//! Borrow-checker metadata construction helpers.
//!
//! WHAT: builds the retained public-call summaries and per-function layouts that the fixed-point
//! driver needs before it can run transfer over reachable blocks.
//! WHY: HIR remains immutable during borrow validation, so signature and layout facts are cached
//! beside the analysis rather than written back into HIR nodes.

use super::diagnostics::BorrowDiagnostics;
use super::engine::BorrowChecker;
use super::state::{FunctionLayout, FunctionLayoutInputs, RootSet};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalAccessKind};
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionReturnContract};
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::{BlockIdRemap, HirModule};
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::utils::{for_each_terminator_target, terminator_targets};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallSummary, PublicCallTransferEffect,
    typed::{PortId, SupportId},
    validate_public_call_summary_transition,
};
#[cfg(feature = "detailed_timers")]
use crate::timing::TimingMetric;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{BTreeSet, VecDeque};

#[path = "return_alias.rs"]
mod return_alias;
pub(super) use return_alias::ProjectedSupport;

type ReturnAliasIndices = FxHashMap<FunctionId, return_alias::SparseFunction>;
type DisplacedSupportSchemas = FxHashMap<FunctionId, return_alias::DisplacedSupportSchema>;

struct SummaryIndices<'a> {
    current: &'a ReturnAliasIndices,
    displaced: &'a mut DisplacedSupportSchemas,
}
#[cfg(test)]
type ReturnAliasCallers = FxHashMap<FunctionId, Vec<usize>>;

/// `FunctionId` is stable within this module; block coordinates are not.
#[derive(Debug, Clone, Default)]
pub(crate) struct BorrowMetadataCache {
    bodies: FxHashMap<FunctionId, FunctionMetadata>,
    indices: ReturnAliasIndices,
    // Retain the first displaced support descriptors while their settled row is still live.
    prior_supports: DisplacedSupportSchemas,
    callers: FxHashMap<FunctionId, BTreeSet<usize>>,
    pending: FxHashSet<FunctionId>,
    #[cfg(test)]
    total_visits: FxHashMap<FunctionId, usize>,
    unindexed: BTreeSet<usize>,
    #[cfg(test)]
    visits: FxHashMap<FunctionId, usize>,
}

#[derive(Debug, Clone)]
struct FunctionMetadata {
    position: usize,
    revision: u64,
    indexed_revision: Option<u64>,
    schema: Option<BodySchema>,
    #[cfg(any(feature = "detailed_timers", feature = "benchmark_counters"))]
    extracted: bool,
}

/// Entry and declaration support available before a full sparse index exists.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BodySchema {
    entry: BlockId,
    returns: HirFunctionReturnContract,
}

impl From<&HirFunction> for BodySchema {
    fn from(function: &HirFunction) -> Self {
        Self {
            entry: function.entry,
            returns: function.return_contract.clone(),
        }
    }
}

impl BorrowMetadataCache {
    pub(super) fn bootstrap(module: &HirModule) -> Self {
        let function_count = module.functions.len();
        let mut cache = Self {
            bodies: FxHashMap::with_capacity_and_hasher(function_count, Default::default()),
            pending: FxHashSet::with_capacity_and_hasher(function_count, Default::default()),
            ..Self::default()
        };
        for (position, function) in module.functions.iter().enumerate() {
            cache.bodies.insert(
                function.id,
                FunctionMetadata {
                    position,
                    revision: 0,
                    indexed_revision: None,
                    schema: Some(BodySchema::from(function)),
                    #[cfg(any(feature = "detailed_timers", feature = "benchmark_counters"))]
                    extracted: false,
                },
            );
            cache.unindexed.insert(position);
            cache.pending.insert(function.id);
        }
        cache
    }

    /// Reevaluate summaries when the sparse index inputs are unchanged.
    pub(super) fn enqueue(&mut self, functions: impl IntoIterator<Item = FunctionId>) {
        for id in functions {
            assert!(
                self.bodies.contains_key(&id),
                "unknown metadata function {id:?}"
            );
            self.pending.insert(id);
        }
    }

    pub(super) fn invalidate_body(&mut self, id: FunctionId) {
        let body = self.bodies.get_mut(&id).expect("known metadata function");
        body.revision += 1;
        body.schema = None;
        self.unindexed.insert(body.position);
        self.pending.insert(id);
        self.prior_supports.remove(&id);
        // Rewrites can narrow structural envelopes, so a replacement cannot start from
        // stale assumptions about this body, including in a local cycle.
        if let Some(index) = self.indices.remove(&id) {
            update_reverse_callers(&mut self.callers, body.position, &index.callees, &[]);
        }
    }

    /// Apply the installer's layout receipt before enqueueing work on the new HIR. Mere
    /// renumbering invalidates coordinates, but does not reset the semantic summary.
    pub(in crate::compiler_frontend) fn apply_block_remap(&mut self, remap: &BlockIdRemap) {
        for (&id, body) in &mut self.bodies {
            if body.indexed_revision == Some(body.revision)
                && self.indices[&id].has_moved_blocks(remap)
            {
                body.revision += 1;
                body.schema = None;
                self.unindexed.insert(body.position);
            }
        }
    }

    /// Nonlocal summary inputs can widen within one body revision. Rebuild the sparse
    /// demand schema without discarding the previous allocation summary.
    pub(in crate::compiler_frontend) fn refresh_inputs(
        &mut self,
        functions: impl IntoIterator<Item = FunctionId>,
    ) {
        for id in functions {
            self.unindexed.insert(self.bodies[&id].position);
            self.pending.insert(id);
        }
    }

    #[cfg(test)]
    pub(in crate::compiler_frontend) fn pending(&self) -> &FxHashSet<FunctionId> {
        &self.pending
    }

    #[cfg(test)]
    pub(in crate::compiler_frontend) fn visit_count(&self, id: FunctionId) -> usize {
        self.total_visits.get(&id).copied().unwrap_or_default()
    }

    pub(in crate::compiler_frontend) fn work_functions<'a>(
        &'a self,
        module: &'a HirModule,
    ) -> impl Iterator<Item = FunctionId> + 'a {
        self.pending.iter().copied().chain(
            self.unindexed
                .iter()
                .map(|&position| module.functions[position].id)
                .filter(|id| !self.pending.contains(id)),
        )
    }

    pub(in crate::compiler_frontend) fn has_work_for(&self, scope: &FxHashSet<FunctionId>) -> bool {
        scope.iter().any(|id| {
            self.pending.contains(id) || self.unindexed.contains(&self.bodies[id].position)
        })
    }

    pub(super) fn has_work(&self) -> bool {
        !self.unindexed.is_empty() || !self.pending.is_empty()
    }

    pub(super) fn solve(
        &mut self,
        checker: &mut BorrowChecker<'_>,
        scope: Option<&FxHashSet<FunctionId>>,
    ) -> Result<Vec<(FunctionId, bool)>, BorrowCheckError> {
        self.refresh_indices(checker, scope)?;
        let queue = self.dependency_ready(checker.module, scope);
        #[cfg(test)]
        let on_visit = {
            self.visits.clear();
            let visits = &mut self.visits;
            let total_visits = &mut self.total_visits;
            move |id| {
                *visits.entry(id).or_default() += 1;
                *total_visits.entry(id).or_default() += 1;
            }
        };
        #[cfg(not(test))]
        let on_visit = |_| {};
        let module = checker.module;
        let result = checker.stabilize_return_alias_summaries(
            SummaryIndices {
                current: &self.indices,
                displaced: &mut self.prior_supports,
            },
            |id| self.callers.get(&id).into_iter().flatten(),
            |position| scope.is_none_or(|scope| scope.contains(&module.functions[position].id)),
            scope.is_some(),
            queue,
            on_visit,
            #[cfg(any(feature = "detailed_timers", feature = "benchmark_counters"))]
            Some(&mut self.bodies),
        );
        if let Ok(changed) = &result {
            if let Some(scope) = scope {
                for id in scope {
                    self.pending.remove(id);
                }
                // A local caller in a later global component was deliberately excluded from
                // the scoped queue. Retain its exact work rather than activating it early.
                for &(source, notify) in changed {
                    if notify {
                        for &caller in self.callers.get(&source).into_iter().flatten() {
                            let id = module.functions[caller].id;
                            if !scope.contains(&id) {
                                self.pending.insert(id);
                            }
                        }
                    }
                }
            } else {
                self.pending.clear();
            }
        }
        result
    }

    fn refresh_indices(
        &mut self,
        checker: &BorrowChecker<'_>,
        scope: Option<&FxHashSet<FunctionId>>,
    ) -> Result<(), BorrowCheckError> {
        loop {
            let rebuilding = if let Some(scope) = scope {
                scope
                    .iter()
                    .filter_map(|id| {
                        let position = self.bodies[id].position;
                        self.unindexed.remove(&position).then_some(position)
                    })
                    .collect()
            } else {
                std::mem::take(&mut self.unindexed)
            };
            if rebuilding.is_empty() {
                return Ok(());
            }
            #[cfg(feature = "benchmark_counters")]
            add_frontend_counter(
                FrontendCounter::BorrowIndexRebuildFunctions,
                rebuilding.len(),
            );
            #[cfg(feature = "detailed_timers")]
            let _rebuild_timing = checker.timing.map(|timing| {
                timing.start(
                    TimingMetric::FrontendBorrowConvergeIndexRebuild,
                    TimingMetric::FrontendGeneratedBorrowIndexRebuild,
                )
            });
            let mut changed_envelopes = FxHashSet::default();
            let callers = &mut self.callers;
            let bodies = &self.bodies;
            let prior_supports = &mut self.prior_supports;
            return_alias::SparseFunction::build_sparse_for(
                checker,
                rebuilding.iter().copied(),
                &mut self.indices,
                |id| bodies[&id].indexed_revision == Some(bodies[&id].revision),
                |id, before, after, changed| {
                    update_reverse_callers(
                        callers,
                        bodies[&id].position,
                        before.as_ref().map_or(&[], |old| old.callees.as_slice()),
                        &after.callees,
                    );
                    if let Some(before) = before
                        && checker.public_call_summaries[&id]
                            .return_alias
                            .facts()
                            .is_some()
                    {
                        prior_supports
                            .entry(id)
                            .or_insert_with(|| before.into_support_schema());
                    }
                    if changed {
                        changed_envelopes.insert(id);
                    }
                },
            )?;
            for position in &rebuilding {
                let function = &checker.module.functions[*position];
                let body = self
                    .bodies
                    .get_mut(&function.id)
                    .expect("known metadata function");
                body.indexed_revision = Some(body.revision);
                if body.schema.as_ref().is_none_or(|schema| {
                    schema.entry != function.entry || schema.returns != function.return_contract
                }) {
                    body.schema = Some(BodySchema::from(function));
                }
            }
            for id in changed_envelopes {
                self.pending.insert(id);
                for &caller in self.callers.get(&id).into_iter().flatten() {
                    self.pending.insert(checker.module.functions[caller].id);
                    if !rebuilding.contains(&caller) {
                        self.unindexed.insert(caller);
                    }
                }
            }
        }
    }

    fn dependency_ready(
        &self,
        module: &HirModule,
        scope: Option<&FxHashSet<FunctionId>>,
    ) -> VecDeque<usize> {
        let in_scope = |id: &FunctionId| scope.is_none_or(|scope| scope.contains(id));
        let mut roots: Vec<_> = if let Some(scope) = scope {
            scope
                .iter()
                .filter(|id| self.pending.contains(*id))
                .map(|id| self.bodies[id].position)
                .collect()
        } else {
            self.pending
                .iter()
                .map(|id| self.bodies[id].position)
                .collect()
        };
        roots.sort_unstable();
        let mut seen = FxHashSet::default();
        let mut stack = Vec::new();
        let mut ready = VecDeque::with_capacity(roots.len());
        // DFS finish order puts all pending callees outside a caller's SCC first.
        // Backedges within the SCC are handled by the existing fair requeue.
        for root in roots
            .into_iter()
            .map(|position| module.functions[position].id)
        {
            if !seen.insert(root) {
                continue;
            }
            stack.push((root, self.indices[&root].callees.iter()));
            while let Some((id, callees)) = stack.last_mut() {
                if let Some(&callee) = callees.next() {
                    if in_scope(&callee) && self.pending.contains(&callee) && seen.insert(callee) {
                        stack.push((callee, self.indices[&callee].callees.iter()));
                    }
                } else {
                    let body = &self.bodies[id];
                    assert_eq!(body.indexed_revision, Some(body.revision));
                    ready.push_back(body.position);
                    stack.pop();
                }
            }
        }
        ready
    }
}

/// The live Known row is the per-function epoch's port high-water mark. Fixed-schema
/// validation never changes it, and rebasing is allowed only for a strict canonical superset,
/// even when the last extraction had no reachable continuation. A graph reset erases the
/// mark by replacing the row with Pending.
fn commit_allocation_transition(
    previous: &mut PublicCallSummary,
    next: PublicCallSummary,
    embed_supports: impl FnOnce(&[PortId], usize) -> Option<Vec<SupportId>>,
) -> Result<(), InfrastructureFailure> {
    if let (Some(before), Some(after)) = (previous.return_alias.facts(), next.return_alias.facts())
        && let Some(result) = before.validate_port_growth(after, embed_supports)
    {
        result?;
        // The typed boundary checked transfer at the available schema precision.
        // Declaration and inferred parameter mutation still use the normal widening check.
        let prior_alias = std::mem::replace(
            &mut previous.return_alias,
            FunctionReturnAliasSummary::Pending,
        );
        let result = validate_public_call_summary_transition(previous, &next);
        previous.return_alias = prior_alias;
        result?;
    } else {
        validate_public_call_summary_transition(previous, &next)?;
    }
    *previous = next;
    Ok(())
}

fn update_reverse_callers(
    callers: &mut FxHashMap<FunctionId, BTreeSet<usize>>,
    position: usize,
    before: &[FunctionId],
    after: &[FunctionId],
) {
    if before == after {
        return;
    }
    for callee in before {
        let owners = callers.get_mut(callee).expect("indexed reverse edge");
        owners.remove(&position);
        if owners.is_empty() {
            callers.remove(callee);
        }
    }
    for &callee in after {
        callers.entry(callee).or_default().insert(position);
    }
}

/// Per-block read liveness and direct-assignment facts for one function.
///
/// Read liveness kills the incoming value at a definition so rebound locals do not stay live
/// across backedges. Direct assignments remain separate: the borrow-state consumer decides
/// whether assigning a local writes through a source alias rather than rebinding storage.
struct BlockLiveness {
    upward_exposed_reads: RootSet,
    definitions: RootSet,
    direct_assignments: RootSet,
}

impl BlockLiveness {
    fn new(local_count: usize) -> Self {
        Self {
            upward_exposed_reads: RootSet::empty(local_count),
            definitions: RootSet::empty(local_count),
            direct_assignments: RootSet::empty(local_count),
        }
    }

    fn record_read(&mut self, local_index: usize) {
        // A read that follows a definition in the same block observes the new value, so it says
        // nothing about whether the value arriving at block entry is still needed.
        if !self.definitions.contains(local_index) {
            self.upward_exposed_reads.insert(local_index);
        }
    }

    fn record_definition(&mut self, local_index: usize) {
        self.definitions.insert(local_index);
    }
}

/// Selects successor combination and the owned block facts to propagate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockPropagation {
    /// MAY future read, with definition kills.
    AnyPath,
    /// MUST future read, with definition kills.
    EveryPath,
    /// MAY direct assignment reachability, without definition kills.
    AnyAssignment,
}

/// Per-local facts gathered while the dense local index is built.
///
/// WHY: the layout projects three parallel per-local vectors from these facts, and positional
///      access to two adjacent booleans makes the projection easy to misread.
struct LocalLayoutFacts {
    mutable: bool,
    region: RegionId,
    has_in_place_mutation_surface: bool,
}

impl<'a> BorrowChecker<'a> {
    pub(super) fn seed_public_call_summaries(
        module: &HirModule,
        diagnostics: &BorrowDiagnostics<'_>,
    ) -> Result<FxHashMap<FunctionId, PublicCallSummary>, BorrowCheckError> {
        // Parameter mutability is stored on locals. Gather it once globally, then project
        // parameter-order access and transfer facts into the single retained call contract.
        let mut local_mutability_by_id = FxHashMap::default();
        for block in &module.blocks {
            for local in &block.locals {
                local_mutability_by_id.insert(local.id, local.mutable);
            }
        }

        let mut summaries = FxHashMap::default();
        let mut parameter_owner_locals = FxHashSet::default();

        for function in &module.functions {
            if summaries.contains_key(&function.id) {
                return Err(diagnostics.internal_error(
                    format!(
                        "Borrow checker found duplicate local function id '{}' while building public call summaries",
                        function.id
                    ),
                ));
            }

            let mut parameters = Vec::with_capacity(function.params.len());
            let mut parameter_locals = FxHashSet::default();

            for (position, param) in function.params.iter().enumerate() {
                if !parameter_locals.insert(*param) {
                    return Err(diagnostics.internal_error(
                        format!(
                            "Borrow checker found duplicate parameter local '{}' at position {} in function '{}'",
                            diagnostics.local_name(*param),
                            position,
                            diagnostics.function_name(function.id)
                        ),
                    ));
                }
                if !parameter_owner_locals.insert(*param) {
                    return Err(diagnostics.internal_error(format!(
                        "Borrow checker found parameter local '{}' owned by more than one function",
                        diagnostics.local_name(*param)
                    )));
                }

                let Some(is_mutable) = local_mutability_by_id.get(param).copied() else {
                    return Err(diagnostics.internal_error(
                        format!(
                            "Borrow checker could not resolve mutability for parameter local '{}' in function '{}'",
                            diagnostics.local_name(*param),
                            diagnostics.function_name(function.id)
                        ),
                    ));
                };

                let access = if is_mutable {
                    PublicCallParameterAccess::Mutable
                } else {
                    PublicCallParameterAccess::Shared
                };

                parameters.push(PublicCallParameterSummary {
                    access,
                    mutation: PublicCallMutationEffect::NoWrite,
                    transfer_effect: PublicCallTransferEffect::MayConsume,
                });
            }

            summaries.insert(
                function.id,
                PublicCallSummary {
                    parameters,
                    return_alias: FunctionReturnAliasSummary::Pending,
                    escapes_builtin_failure: false,
                    uses_fast_semantics: false,
                    may_trap: false,
                    may_fail: false,
                },
            );
        }

        Ok(summaries)
    }

    #[cfg(test)]
    pub(super) fn build_public_call_summaries(&mut self) -> Result<(), BorrowCheckError> {
        self.public_call_summaries =
            Self::seed_public_call_summaries(self.module, &self.diagnostics)?;

        let indices = return_alias::SparseFunction::build_sparse(self)?;
        let mut callers = ReturnAliasCallers::default();
        for (position, function) in self.module.functions.iter().enumerate() {
            for &callee in &indices[&function.id].callees {
                callers.entry(callee).or_default().push(position);
            }
        }
        self.stabilize_return_alias_summaries(
            SummaryIndices {
                current: &indices,
                displaced: &mut DisplacedSupportSchemas::default(),
            },
            |id| callers.get(&id).into_iter().flatten(),
            |_| true,
            false,
            (0..self.module.functions.len()).collect(),
            |_| {},
            #[cfg(any(feature = "detailed_timers", feature = "benchmark_counters"))]
            None,
        )
        .map(|_| ())
    }

    fn stabilize_return_alias_summaries<'b, I: IntoIterator<Item = &'b usize>>(
        &mut self,
        indices: SummaryIndices<'_>,
        callers: impl Fn(FunctionId) -> I,
        in_scope: impl Fn(usize) -> bool,
        record_changes: bool,
        mut queue: VecDeque<usize>,
        mut on_visit: impl FnMut(FunctionId),
        #[cfg(any(feature = "detailed_timers", feature = "benchmark_counters"))] mut bodies: Option<
            &mut FxHashMap<FunctionId, FunctionMetadata>,
        >,
    ) -> Result<Vec<(FunctionId, bool)>, BorrowCheckError> {
        if queue.is_empty() {
            return Ok(Vec::new());
        }
        let mut changed = FxHashMap::<FunctionId, bool>::default();
        let mut queued = FxHashSet::default();
        for &position in &queue {
            queued.insert(position);
        }
        while let Some(position) = queue.pop_front() {
            queued.remove(&position);
            let function = &self.module.functions[position];
            on_visit(function.id);
            let previous = &self.public_call_summaries[&function.id];
            let index = &indices.current[&function.id];
            let return_alias = {
                #[cfg(any(feature = "detailed_timers", feature = "benchmark_counters"))]
                let first = bodies.as_deref_mut().is_some_and(|bodies| {
                    !std::mem::replace(
                        &mut bodies.get_mut(&function.id).expect("known body").extracted,
                        true,
                    )
                });
                #[cfg(feature = "benchmark_counters")]
                if bodies.is_some() {
                    let counter = if first {
                        FrontendCounter::BorrowFirstExtractions
                    } else if matches!(previous.return_alias, FunctionReturnAliasSummary::Pending) {
                        FrontendCounter::BorrowDirtyReextractions
                    } else {
                        FrontendCounter::BorrowDependencyRevisits
                    };
                    add_frontend_counter(counter, 1);
                }
                #[cfg(feature = "detailed_timers")]
                let _extraction_timing = self.timing.map(|timing| {
                    if first {
                        timing.start(
                            TimingMetric::FrontendBorrowConvergeFirstExtraction,
                            TimingMetric::FrontendGeneratedBorrowFirstExtraction,
                        )
                    } else {
                        timing.start(
                            TimingMetric::FrontendBorrowConvergeReextraction,
                            TimingMetric::FrontendGeneratedBorrowReextraction,
                        )
                    }
                });
                self.classify_function_return_alias(function, index)?
            };
            if previous.return_alias == return_alias {
                indices.displaced.remove(&function.id);
                continue;
            }
            // Settling an empty body need not revisit callers if its executed effects and
            // both continuation lanes still transfer exactly as before.
            let notify = !previous
                .return_alias
                .same_allocation_transfer_as(&return_alias);
            let summary = PublicCallSummary {
                parameters: previous.parameters.clone(),
                return_alias,
                escapes_builtin_failure: previous.escapes_builtin_failure,
                uses_fast_semantics: previous.uses_fast_semantics,
                may_trap: previous.may_trap,
                may_fail: previous.may_fail,
            };
            self.validate_local_allocation_summary(function, &summary)?;
            commit_allocation_transition(
                self.public_call_summaries
                    .get_mut(&function.id)
                    .expect("prior summary"),
                summary,
                |ports, support_count| match indices.displaced.get(&function.id) {
                    Some(old) => old.embed_supports(index, ports),
                    None => Some((0..support_count).map(|id| SupportId(id as u32)).collect()),
                },
            )?;
            indices.displaced.remove(&function.id);
            if record_changes {
                *changed.entry(function.id).or_default() |= notify;
            }
            if notify {
                for &caller in callers(function.id) {
                    if in_scope(caller) && queued.insert(caller) {
                        queue.push_back(caller);
                    }
                }
            }
        }
        Ok(changed.into_iter().collect())
    }

    pub(super) fn finalize_public_call_summary_effects(
        &mut self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        report: &mut super::types::BorrowCheckReport,
    ) -> Result<bool, BorrowCheckError> {
        let mut parameter_positions = FxHashMap::default();
        for (position, parameter) in function.params.iter().enumerate() {
            parameter_positions.insert(*parameter, position);
        }

        let mut mutation_positions = FxHashSet::default();
        let mut record_parameter_mutation = |root: &LocalId| {
            if let Some(position) = parameter_positions.get(root) {
                mutation_positions.insert(*position);
            }
        };
        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function.id)?;
            for statement in &block.statements {
                // A call with no reachable success lane can stop transfer inside this block.
                // Do not ascribe effects to a syntactically present but unexecuted suffix.
                let Some(statement_fact) = report.analysis.statement_facts.get(&statement.id)
                else {
                    break;
                };

                match &statement.kind {
                    // Only a write to an existing place mutates a parameter. HIR validation keeps
                    // definitions off parameter locals. `written_roots` excludes roots accessed
                    // exclusively to issue a mutation-capable alias, so a slot rebind does not
                    // publish a write to its source.
                    HirStatementKind::Write {
                        target: HirWriteTarget::AssignPlace(_),
                        ..
                    } => {
                        for root in &statement_fact.written_roots {
                            record_parameter_mutation(root);
                        }
                    }
                    HirStatementKind::Call { target, args, .. } => {
                        for (argument_index, argument) in
                            self.module.expressions.values(*args).iter().enumerate()
                        {
                            if !self.call_argument_writes(target, argument_index)? {
                                continue;
                            }

                            let Some(argument_fact) = report.analysis.value_facts.get(argument)
                            else {
                                return Err(self.diagnostics.internal_error(
                                    format!(
                                        "Borrow checker is missing value facts for argument {} while finalizing public call summary for function '{}'",
                                        argument_index,
                                        self.diagnostics.function_name(function.id)
                                    ),
                                ));
                            };
                            for root in &argument_fact.roots {
                                record_parameter_mutation(root);
                            }
                        }
                    }
                    HirStatementKind::MapOp { op, receiver, .. }
                        if op.requires_mutable_receiver() =>
                    {
                        let Some(receiver_fact) = report.analysis.value_facts.get(receiver) else {
                            return Err(self.diagnostics.internal_error(
                                format!(
                                    "Borrow checker is missing map receiver value facts while finalizing public call summary for function '{}'",
                                    self.diagnostics.function_name(function.id)
                                ),
                            ));
                        };
                        for root in &receiver_fact.roots {
                            record_parameter_mutation(root);
                        }
                    }
                    _ => {}
                }
            }
            if !report.analysis.terminator_facts.contains_key(block_id) {
                continue;
            }
            // A terminating call mutates its arguments before divergence exactly like an ordinary
            // call. Its effects must still reach this function's public summary even though the
            // block has no successor.
            if let HirTerminator::Invoke {
                operation: HirFallibleOperation::Call { target, args },
                ..
            }
            | HirTerminator::NeverCall { target, args } = &block.terminator
            {
                for (index, argument) in self.module.expressions.values(*args).iter().enumerate() {
                    if !self.call_argument_writes(target, index)? {
                        continue;
                    }
                    let Some(fact) = report.analysis.value_facts.get(argument) else {
                        return Err(self.diagnostics.internal_error(
                            "Missing call argument borrow fact during call-summary publication",
                        ));
                    };
                    for root in &fact.roots {
                        record_parameter_mutation(root);
                    }
                }
            }
            if let HirTerminator::Invoke {
                operation: HirFallibleOperation::MapOp { op, receiver, .. },
                ..
            } = &block.terminator
                && op.requires_mutable_receiver()
            {
                let Some(fact) = report.analysis.value_facts.get(receiver) else {
                    return Err(self.diagnostics.internal_error(
                        "Missing Invoke map receiver borrow fact during call-summary publication",
                    ));
                };
                // Map keys and stored values are borrowed inputs, not mutated parameters.
                for root in &fact.roots {
                    record_parameter_mutation(root);
                }
            }
        }

        let Some(summary) = self.public_call_summaries.get_mut(&function.id) else {
            return Err(self.diagnostics.internal_error(format!(
                "Borrow checker is missing the public call summary for function '{}'",
                self.diagnostics.function_name(function.id)
            )));
        };

        let mut changed = false;
        for (position, parameter) in summary.parameters.iter_mut().enumerate() {
            if mutation_positions.contains(&position)
                && parameter.mutation != PublicCallMutationEffect::Writes
            {
                parameter.mutation = PublicCallMutationEffect::Writes;
                changed = true;
            }
        }

        Ok(changed)
    }

    fn call_argument_writes(
        &self,
        target: &CallTarget,
        argument_index: usize,
    ) -> Result<bool, BorrowCheckError> {
        match target {
            CallTarget::Local(function_id) => {
                let Some(summary) = self.public_call_summaries.get(function_id) else {
                    return Err(self.diagnostics.internal_error(format!(
                        "Borrow checker is missing the public call summary for function '{}'",
                        self.diagnostics.function_name(*function_id)
                    )));
                };
                let Some(parameter) = summary.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {} while finalizing call summary for function '{}'",
                            argument_index,
                            self.diagnostics.function_name(*function_id)
                        ),
                    ));
                };
                Ok(parameter.mutation == PublicCallMutationEffect::Writes)
            }
            CallTarget::CrossModule(origin) => {
                let Some(summary) = self.module.imported_call_summaries.get(origin) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker is missing the provider call summary for imported function {origin:?}"
                        ),
                    ));
                };
                let Some(parameter) = summary.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {argument_index} while finalizing imported call summary for {origin:?}"
                        ),
                    ));
                };
                Ok(parameter.mutation == PublicCallMutationEffect::Writes)
            }
            CallTarget::ModulePrivate(identity) => {
                let Some(summary) = self.module.module_private_call_summaries.get(identity) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker is missing the call summary for module-private function {identity:?}"
                        ),
                    ));
                };
                let Some(parameter) = summary.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {argument_index} while finalizing module-private call summary for {identity:?}"
                        ),
                    ));
                };
                Ok(parameter.mutation == PublicCallMutationEffect::Writes)
            }
            CallTarget::Generated(identity) => {
                let Some(summary) = self.module.generated_call_summaries.get(identity.as_ref())
                else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker is missing the call summary for generated function {identity:?}"
                        ),
                    ));
                };
                let Some(parameter) = summary.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {argument_index} while finalizing generated call summary for {identity:?}"
                        ),
                    ));
                };
                Ok(parameter.mutation == PublicCallMutationEffect::Writes)
            }
            CallTarget::External(function_id) => {
                let Some(definition) = self
                    .external_package_registry
                    .get_function_by_id(*function_id)
                else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker could not resolve host call target '{}' while finalizing public call summary",
                            function_id.name()
                        ),
                    ));
                };
                let Some(parameter) = definition.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {} while finalizing host call summary for '{}'",
                            argument_index, definition.name
                        ),
                    ));
                };
                Ok(parameter.access_kind == ExternalAccessKind::Mutable)
            }
        }
    }

    /// Classifies one local's type for mutable-access root selection.
    ///
    /// WHY: production HIR always registers its type ids. An unregistered id means the layout
    ///      consumer was handed a module without its defining `TypeEnvironment`, which is an
    ///      infrastructure failure rather than a conservative semantic answer.
    fn local_has_in_place_mutation_surface(
        &self,
        type_id: TypeId,
    ) -> Result<bool, BorrowCheckError> {
        let class = classify_hir_type(type_id, self.type_environment)?;
        Ok(class.has_in_place_mutation_surface())
    }

    pub(super) fn build_function_layout(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
    ) -> Result<FunctionLayout, BorrowCheckError> {
        // Borrow state uses dense indices for speed.
        // Build one stable LocalId -> dense index layout from reachable blocks.
        let mut local_info_by_id = FxHashMap::default();

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function.id)?;
            for local in &block.locals {
                local_info_by_id.insert(
                    local.id,
                    LocalLayoutFacts {
                        mutable: local.mutable,
                        region: local.region,
                        has_in_place_mutation_surface: self
                            .local_has_in_place_mutation_surface(local.ty)?,
                    },
                );
            }
        }

        for param in &function.params {
            if !local_info_by_id.contains_key(param) {
                return Err(self.diagnostics.internal_error(format!(
                    "Function '{}' parameter '{}' is missing from reachable local layout",
                    self.diagnostics.function_name(function.id),
                    self.diagnostics.local_name(*param)
                )));
            }
        }

        let mut local_ids = local_info_by_id.keys().copied().collect::<Vec<_>>();
        local_ids.sort_by_key(|local_id| local_id.0);

        let local_mutable = local_ids
            .iter()
            .map(|local_id| local_info_by_id[local_id].mutable)
            .collect::<Vec<_>>();
        let local_has_in_place_mutation_surface = local_ids
            .iter()
            .map(|local_id| local_info_by_id[local_id].has_in_place_mutation_surface)
            .collect::<Vec<_>>();
        let local_regions = local_ids
            .iter()
            .map(|local_id| local_info_by_id[local_id].region)
            .collect::<Vec<_>>();
        let mut local_index_by_id = FxHashMap::default();
        for (index, local_id) in local_ids.iter().enumerate() {
            local_index_by_id.insert(*local_id, index);
        }

        // HIR loop lowering materialises an iterable into a compiler temporary so the loop can
        // reuse one stable place across its header and backedge. Future-use analysis works on
        // local IDs, so carry uses of those direct alias carriers back to their source roots.
        // Without this bridge, an item read can look like the final use of the source collection
        // even though the hidden iterable carrier still reads it on the next iteration.
        let compiler_alias_source_roots_by_local = self.compiler_alias_source_roots(
            function,
            reachable_blocks,
            &local_index_by_id,
            local_ids.len(),
        )?;

        let reachable_block_set = reachable_blocks.iter().copied().collect::<FxHashSet<_>>();
        let mut local_first_write_order = vec![-1; local_ids.len()];
        let mut local_last_use_order = vec![-1; local_ids.len()];
        let mut statement_order_by_id = FxHashMap::default();
        let mut terminator_order_by_block = FxHashMap::default();
        let mut block_successors = FxHashMap::default();
        let mut block_local_max_use_order = FxHashMap::default();
        let mut block_liveness = FxHashMap::default();
        let mut entry_has_predecessor = false;
        let mut next_order_key = 0i32;

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function.id)?;
            let mut max_use_order = vec![-1; local_ids.len()];
            #[cfg(feature = "benchmark_counters")]
            add_frontend_counter(
                FrontendCounter::BorrowValidationDenseLayoutSlots,
                max_use_order.len(),
            );
            let mut liveness = BlockLiveness::new(local_ids.len());

            for statement in &block.statements {
                // WHAT: assign a deterministic ordinal key for this statement.
                // WHY: same-line source locations are not sufficient for precise move decisions.
                let order_key = next_order_key;
                next_order_key += 1;
                statement_order_by_id.insert(statement.id, order_key);

                collect_statement_written_locals(statement, &mut |local_id| {
                    if let Some(index) = local_index_by_id.get(&local_id).copied() {
                        let first_write = &mut local_first_write_order[index];
                        if *first_write < 0 || order_key < *first_write {
                            *first_write = order_key;
                        }

                        local_last_use_order[index] = local_last_use_order[index].max(order_key);
                        max_use_order[index] = max_use_order[index].max(order_key);
                    }
                });
                collect_statement_loaded_locals(
                    &self.module.expressions,
                    statement,
                    &mut |local_id| {
                        if let Some(index) = local_index_by_id.get(&local_id).copied() {
                            local_last_use_order[index] =
                                local_last_use_order[index].max(order_key);
                            max_use_order[index] = max_use_order[index].max(order_key);
                        }
                    },
                );

                record_statement_liveness(
                    &self.module.expressions,
                    statement,
                    &local_index_by_id,
                    &mut liveness,
                );
            }

            for (target_index, source_roots) in
                compiler_alias_source_roots_by_local.iter().enumerate()
            {
                let target_use_order = max_use_order[target_index];
                if target_use_order < 0 {
                    continue;
                }

                for source_index in source_roots.iter_ones() {
                    local_last_use_order[source_index] =
                        local_last_use_order[source_index].max(target_use_order);
                    max_use_order[source_index] = max_use_order[source_index].max(target_use_order);
                }
            }

            // WHAT: terminators also participate in future-use classification.
            // WHY: return/branch conditions can be the last read of a root in the block.
            let terminator_order = next_order_key;
            next_order_key += 1;
            terminator_order_by_block.insert(*block_id, terminator_order);

            collect_terminator_loaded_locals(
                &self.module.expressions,
                &block.terminator,
                &mut |local_id| {
                    if let Some(index) = local_index_by_id.get(&local_id).copied() {
                        local_last_use_order[index] =
                            local_last_use_order[index].max(terminator_order);
                        max_use_order[index] = max_use_order[index].max(terminator_order);
                        liveness.record_read(index);
                    }
                },
            );
            if let HirTerminator::Jump { args, .. } = &block.terminator {
                // Transport reads the old source occurrences before defining the merge slots.
                // These are liveness dependencies, not additional source-semantic access facts.
                for argument in args {
                    if let Some(index) = local_index_by_id.get(&argument.source).copied() {
                        liveness.record_read(index);
                        local_last_use_order[index] =
                            local_last_use_order[index].max(terminator_order);
                        max_use_order[index] = max_use_order[index].max(terminator_order);
                    }
                }
                for argument in args {
                    if let Some(index) = local_index_by_id.get(&argument.destination).copied() {
                        liveness.record_definition(index);
                        if local_first_write_order[index] < 0 {
                            local_first_write_order[index] = terminator_order;
                        }
                    }
                }
            }
            if let HirTerminator::Invoke {
                success_results,
                error_result,
                ..
            } = &block.terminator
            {
                // Both outcomes replace the preceding iteration's values, after operand reads.
                for local in success_results.iter().chain(std::iter::once(error_result)) {
                    if let Some(index) = local_index_by_id.get(local).copied() {
                        liveness.record_definition(index);
                        if local_first_write_order[index] < 0 {
                            local_first_write_order[index] = terminator_order;
                        }
                    }
                }
            }

            // A read of a direct alias carrier is a read of the roots it carries. Union the source
            // roots in unconditionally: an in-block definition of a source root cannot be proven to
            // precede the carrier read here, and over-approximating liveness is the safe direction.
            for (carrier_index, source_roots) in
                compiler_alias_source_roots_by_local.iter().enumerate()
            {
                if liveness.upward_exposed_reads.contains(carrier_index) {
                    liveness.upward_exposed_reads.union_with(source_roots);
                }
            }

            block_liveness.insert(*block_id, liveness);
            block_local_max_use_order.insert(*block_id, max_use_order);
            let successors = terminator_targets(&block.terminator)
                .into_iter()
                .filter(|successor| reachable_block_set.contains(successor))
                .collect::<Vec<_>>();
            entry_has_predecessor |= successors.contains(&function.entry);
            block_successors.insert(*block_id, successors);
        }

        // Share lexical visibility with forward state kills so a later loop-local declaration
        // cannot keep the previous iteration's alias active across its region exit.
        let visible_locals_by_block =
            self.build_visibility_masks(function.id, &local_regions, reachable_blocks)?;

        let (may_use_from_block, must_use_from_block) = compute_future_use_sets(
            local_ids.len(),
            reachable_blocks,
            &block_successors,
            &block_liveness,
            &visible_locals_by_block,
        );
        let may_assign_from_block = propagate_block_facts(
            local_ids.len(),
            reachable_blocks,
            &block_successors,
            &block_liveness,
            &visible_locals_by_block,
            BlockPropagation::AnyAssignment,
        );

        let mut layout = FunctionLayout::new(FunctionLayoutInputs {
            local_ids,
            local_mutable,
            local_has_in_place_mutation_surface,
            local_first_write_order,
            local_last_use_order,
            statement_order_by_id,
            terminator_order_by_block,
            block_local_max_use_order,
            block_successors,
            visible_locals_by_block,
            may_use_from_block,
            must_use_from_block,
            may_assign_from_block,
        });
        layout.acyclic_entry_block = (!entry_has_predecessor).then_some(function.entry);
        Ok(layout)
    }

    fn compiler_alias_source_roots(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        local_index_by_id: &FxHashMap<LocalId, usize>,
        local_count: usize,
    ) -> Result<Vec<RootSet>, BorrowCheckError> {
        let mut source_roots_by_local = (0..local_count)
            .map(|_| RootSet::empty(local_count))
            .collect::<Vec<_>>();

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function.id)?;
            for statement in &block.statements {
                let (target_local, value) = match &statement.kind {
                    HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(local),
                        value,
                    } => (*local, *value),
                    HirStatementKind::Write {
                        target: HirWriteTarget::AssignPlace(place),
                        value,
                    } if self
                        .module
                        .expressions
                        .projections(place.projections)
                        .is_empty() =>
                    {
                        (place.root, *value)
                    }
                    _ => continue,
                };

                if self.module.side_table.local_origin_kind(target_local)
                    != Some(HirLocalOriginKind::CompilerTemp)
                {
                    continue;
                }

                let HirExpressionKind::Load(place) =
                    &self.module.expressions.expression(value).kind
                else {
                    continue;
                };
                if self.module.expressions.expression(value).value_kind
                    != crate::compiler_frontend::hir::expressions::ValueKind::Place
                {
                    continue;
                }
                let source_local = place.root;

                let Some(target_index) = local_index_by_id.get(&target_local).copied() else {
                    continue;
                };
                let Some(source_index) = local_index_by_id.get(&source_local).copied() else {
                    continue;
                };
                source_roots_by_local[target_index].insert(source_index);
            }
        }

        // Compiler temporaries can be assigned from another direct alias carrier. Close that tiny
        // relation once so future-use propagation reaches the original source root without adding a
        // second runtime alias-analysis path.
        let mut changed = true;
        while changed {
            changed = false;
            for target_index in 0..local_count {
                let direct_sources = source_roots_by_local[target_index].clone();
                let mut closed_sources = direct_sources.clone();

                for source_index in direct_sources.iter_ones() {
                    closed_sources.union_with(&source_roots_by_local[source_index]);
                }

                if closed_sources != source_roots_by_local[target_index] {
                    source_roots_by_local[target_index] = closed_sources;
                    changed = true;
                }
            }
        }

        Ok(source_roots_by_local)
    }

    fn build_visibility_masks(
        &self,
        function_id: FunctionId,
        local_regions: &[RegionId],
        reachable_blocks: &[BlockId],
    ) -> Result<FxHashMap<BlockId, RootSet>, BorrowCheckError> {
        // A local is visible in a block when local.region is an ancestor
        // of block.region in the lexical region tree.
        let mut masks = FxHashMap::default();

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function_id)?;
            let mut mask = RootSet::empty(local_regions.len());

            for (local_index, local_region) in local_regions.iter().enumerate() {
                if self.is_region_ancestor_of(*local_region, block.region, *block_id)? {
                    mask.insert(local_index);
                }
            }

            masks.insert(*block_id, mask);
        }

        Ok(masks)
    }

    pub(super) fn is_region_ancestor_of(
        &self,
        ancestor: crate::compiler_frontend::hir::ids::RegionId,
        mut region: crate::compiler_frontend::hir::ids::RegionId,
        block_id: BlockId,
    ) -> Result<bool, BorrowCheckError> {
        // Walk parent links from `region` to root.
        loop {
            if region == ancestor {
                return Ok(true);
            }

            let Some(parent) = self.region_parent_by_id.get(&region).copied() else {
                return Err(self.diagnostics.internal_error(format!(
                    "Borrow checker could not resolve region '{}' while analyzing block '{}'",
                    region.0, block_id
                )));
            };

            let Some(parent) = parent else {
                return Ok(false);
            };
            region = parent;
        }
    }

    pub(super) fn collect_reachable_blocks(
        &self,
        function: &HirFunction,
    ) -> Result<Vec<BlockId>, BorrowCheckError> {
        // Breadth-first traversal over explicit terminator successors.
        crate::compiler_frontend::hir::utils::collect_reachable_blocks(
            function.entry,
            |block_id, visit| {
                let block = self.block_by_id_or_error(block_id, function.id)?;
                for_each_terminator_target(&block.terminator, visit);
                Ok(())
            },
        )
    }
}

fn compute_future_use_sets(
    local_count: usize,
    reachable_blocks: &[BlockId],
    block_successors: &FxHashMap<BlockId, Vec<BlockId>>,
    block_liveness: &FxHashMap<BlockId, BlockLiveness>,
    visible_locals_by_block: &FxHashMap<BlockId, RootSet>,
) -> (FxHashMap<BlockId, RootSet>, FxHashMap<BlockId, RootSet>) {
    // WHAT: derives per-block MAY/MUST future-use summaries by backward liveness propagation.
    // WHY: transfer needs O(1) future-use classification when deciding borrow versus move, and
    //      alias-conflict checks need to know whether the value arriving at a block is still
    //      required. Both are liveness questions, so the definition kill is what stops a rebound
    //      local from looking used forever once a loop back-edge makes its own body reachable.
    //
    //      live_in[b]  = upward_exposed_reads[b] | (live_out[b] & !definitions[b])
    //      live_out[b] = combine over successors: union for MAY, intersection for MUST
    let may_use_from_block = propagate_block_facts(
        local_count,
        reachable_blocks,
        block_successors,
        block_liveness,
        visible_locals_by_block,
        BlockPropagation::AnyPath,
    );

    let must_use_from_block = propagate_block_facts(
        local_count,
        reachable_blocks,
        block_successors,
        block_liveness,
        visible_locals_by_block,
        BlockPropagation::EveryPath,
    );

    (may_use_from_block, must_use_from_block)
}

fn propagate_block_facts(
    local_count: usize,
    reachable_blocks: &[BlockId],
    block_successors: &FxHashMap<BlockId, Vec<BlockId>>,
    block_liveness: &FxHashMap<BlockId, BlockLiveness>,
    visible_locals_by_block: &FxHashMap<BlockId, RootSet>,
    propagation: BlockPropagation,
) -> FxHashMap<BlockId, RootSet> {
    // MAY grows from nothing; MUST shrinks from everything so loops converge on the roots that
    // survive on every path rather than on the first path visited.
    let seed = match propagation {
        BlockPropagation::AnyPath | BlockPropagation::AnyAssignment => RootSet::empty(local_count),
        BlockPropagation::EveryPath => RootSet::full(local_count),
    };

    let mut live_in = FxHashMap::default();
    for block_id in reachable_blocks {
        live_in.insert(*block_id, seed.clone());
    }

    let mut changed = true;
    while changed {
        changed = false;

        for block_id in reachable_blocks.iter().rev() {
            let successors = block_successors
                .get(block_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);

            let mut next = combine_successor_facts(local_count, successors, &live_in, propagation);

            if let Some(liveness) = block_liveness.get(block_id) {
                match propagation {
                    BlockPropagation::AnyPath | BlockPropagation::EveryPath => {
                        next.subtract_with(&liveness.definitions);
                        next.union_with(&liveness.upward_exposed_reads);
                    }

                    // Assignment can be a source alias holder use even when no old value is read.
                    // Keep this reachability fact independent of definition-killing read liveness.
                    BlockPropagation::AnyAssignment => {
                        next.union_with(&liveness.direct_assignments);
                        if let Some(mask) = visible_locals_by_block.get(block_id) {
                            next.intersect_with(mask);
                        }
                    }
                }
            }

            let should_update = live_in
                .get(block_id)
                .map(|existing| existing != &next)
                .unwrap_or(true);

            if should_update {
                live_in.insert(*block_id, next);
                changed = true;
            }
        }
    }

    live_in
}

fn combine_successor_facts(
    local_count: usize,
    successors: &[BlockId],
    live_in: &FxHashMap<BlockId, RootSet>,
    propagation: BlockPropagation,
) -> RootSet {
    // An exit block has no successor, so nothing is required after it on either lattice.
    if successors.is_empty() {
        return RootSet::empty(local_count);
    }

    match propagation {
        BlockPropagation::AnyPath | BlockPropagation::AnyAssignment => {
            let mut union = RootSet::empty(local_count);
            for successor in successors {
                if let Some(successor_live_in) = live_in.get(successor) {
                    union.union_with(successor_live_in);
                }
            }
            union
        }

        BlockPropagation::EveryPath => {
            let mut intersection = RootSet::full(local_count);
            for successor in successors {
                match live_in.get(successor) {
                    Some(successor_live_in) => {
                        intersection.intersect_with(successor_live_in);
                    }
                    // An unreachable successor contributes no guaranteed future use.
                    None => return RootSet::empty(local_count),
                }
            }
            intersection
        }
    }
}

// WHAT: records one statement's contribution to its block's liveness facts.
// WHY: operands are read before the result is written, so a self-referential rebind such as
//      `total = total + 1` still exposes a read at block entry while ending the old value's life.
fn record_statement_liveness(
    expressions: &HirExpressionStore,
    statement: &HirStatement,
    local_index_by_id: &FxHashMap<LocalId, usize>,
    liveness: &mut BlockLiveness,
) {
    if let HirStatementKind::Write { target, .. } = &statement.kind {
        let local = match target {
            HirWriteTarget::DefineLocal(local) => Some(*local),
            HirWriteTarget::AssignPlace(place)
                if expressions.projections(place.projections).is_empty() =>
            {
                Some(place.root)
            }
            HirWriteTarget::AssignPlace(_) => None,
        };
        if let Some(index) = local.and_then(|local| local_index_by_id.get(&local).copied()) {
            liveness.direct_assignments.insert(index);
        }
    }

    collect_statement_loaded_locals(expressions, statement, &mut |local_id| {
        if let Some(index) = local_index_by_id.get(&local_id).copied() {
            liveness.record_read(index);
        }
    });

    // A write through a projection updates part of the base and leaves the rest observable, so the
    // base counts as a read and never kills.
    collect_statement_written_locals(statement, &mut |local_id| {
        let mut is_definition = false;
        collect_statement_defined_locals(statement, &mut |defined| {
            is_definition |= defined == local_id
        });
        if is_definition {
            return;
        }
        if let Some(index) = local_index_by_id.get(&local_id).copied() {
            liveness.record_read(index);
        }
    });

    collect_statement_defined_locals(statement, &mut |local_id| {
        if let Some(index) = local_index_by_id.get(&local_id).copied() {
            liveness.record_definition(index);
        }
    });
}

// WHAT: reports every local this statement redefines outright.
// WHY: only whole-local writes end the previous value's life; projected writes leave the
//      surrounding value observable.
fn collect_statement_defined_locals(statement: &HirStatement, visitor: &mut impl FnMut(LocalId)) {
    let mut visit_destination = |destination: &HirLocalDestination| {
        if matches!(destination, HirLocalDestination::Define(_)) {
            visitor(destination.local());
        }
    };
    match &statement.kind {
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(local),
            ..
        } => visitor(*local),
        HirStatementKind::Call { results, .. } => {
            for result in results {
                visit_destination(result);
            }
        }
        HirStatementKind::MapOp { result, .. } => {
            if let Some(result) = result {
                visit_destination(result);
            }
        }
        HirStatementKind::NumericOp { result, .. }
        | HirStatementKind::FastStoreConversion { result, .. }
        | HirStatementKind::LoopStepFailure { result, .. }
        | HirStatementKind::FormatFloat { result, .. }
        | HirStatementKind::ValidateFloat { result, .. } => visit_destination(result),
        HirStatementKind::RangeCandidate {
            candidate_result,
            in_range_result,
            ..
        } => {
            visit_destination(candidate_result);
            visit_destination(in_range_result);
        }
        HirStatementKind::Write { .. }
        | HirStatementKind::Expr(_)
        | HirStatementKind::PushRuntimeFragment { .. } => {}
    }
}

fn collect_statement_loaded_locals(
    expressions: &HirExpressionStore,
    statement: &HirStatement,
    visitor: &mut impl FnMut(LocalId),
) {
    match &statement.kind {
        HirStatementKind::Write { target, value } => {
            if let HirWriteTarget::AssignPlace(place) = target {
                for index in expressions.place_index_values(*place) {
                    collect_expression_loaded_locals(expressions, index, visitor);
                }
            }
            collect_expression_loaded_locals(expressions, *value, visitor);
        }
        HirStatementKind::Call { args, .. } => {
            for arg in expressions.values(*args) {
                collect_expression_loaded_locals(expressions, *arg, visitor);
            }
        }
        HirStatementKind::MapOp { receiver, args, .. } => {
            collect_expression_loaded_locals(expressions, *receiver, visitor);
            for arg in expressions.values(*args) {
                collect_expression_loaded_locals(expressions, *arg, visitor);
            }
        }
        HirStatementKind::NumericOp { operands, .. } => match operands {
            HirNumericOperands::Unary { operand } => {
                collect_expression_loaded_locals(expressions, *operand, visitor);
            }
            HirNumericOperands::Binary { left, right } => {
                collect_expression_loaded_locals(expressions, *left, visitor);
                collect_expression_loaded_locals(expressions, *right, visitor);
            }
        },
        HirStatementKind::RangeCandidate {
            current, step, end, ..
        } => {
            collect_expression_loaded_locals(expressions, *current, visitor);
            collect_expression_loaded_locals(expressions, *step, visitor);
            collect_expression_loaded_locals(expressions, *end, visitor);
        }
        HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. }
        | HirStatementKind::FastStoreConversion { source, .. } => {
            collect_expression_loaded_locals(expressions, *source, visitor);
        }
        HirStatementKind::Expr(expression) => {
            collect_expression_loaded_locals(expressions, *expression, visitor);
        }
        HirStatementKind::LoopStepFailure { .. } => {}
        HirStatementKind::PushRuntimeFragment { vec_local, value } => {
            visitor(*vec_local);
            collect_expression_loaded_locals(expressions, *value, visitor);
        }
    }
}

fn collect_statement_written_locals(statement: &HirStatement, visitor: &mut impl FnMut(LocalId)) {
    match &statement.kind {
        HirStatementKind::Write { target, .. } => match target {
            HirWriteTarget::DefineLocal(local) => visitor(*local),
            HirWriteTarget::AssignPlace(place) => collect_place_written_local(place, visitor),
        },
        HirStatementKind::Call { results, .. } => {
            for result in results {
                visitor(result.local());
            }
        }
        HirStatementKind::MapOp {
            result: Some(local),
            ..
        } => visitor(local.local()),
        HirStatementKind::NumericOp { result, .. }
        | HirStatementKind::FastStoreConversion { result, .. }
        | HirStatementKind::LoopStepFailure { result, .. } => visitor(result.local()),
        HirStatementKind::RangeCandidate {
            candidate_result,
            in_range_result,
            ..
        } => {
            visitor(candidate_result.local());
            visitor(in_range_result.local());
        }
        HirStatementKind::FormatFloat { result, .. }
        | HirStatementKind::ValidateFloat { result, .. } => visitor(result.local()),
        HirStatementKind::MapOp { result: None, .. }
        | HirStatementKind::Expr(_)
        | HirStatementKind::PushRuntimeFragment { .. } => {}
    }
}

fn collect_place_written_local(place: &HirPlace, visitor: &mut impl FnMut(LocalId)) {
    visitor(place.root);
}

fn collect_terminator_loaded_locals(
    expressions: &HirExpressionStore,
    terminator: &HirTerminator,
    visitor: &mut impl FnMut(LocalId),
) {
    match terminator {
        // Jump argument passing is CFG plumbing, not a semantic value use.
        HirTerminator::Jump { .. } => {}
        HirTerminator::If { condition, .. } => {
            collect_expression_loaded_locals(expressions, *condition, visitor);
        }
        HirTerminator::Invoke { operation, .. } => {
            for value in operation.operands(expressions) {
                collect_expression_loaded_locals(expressions, value, visitor);
            }
        }
        HirTerminator::Match { scrutinee, arms } => {
            collect_expression_loaded_locals(expressions, *scrutinee, visitor);
            for arm in arms {
                if let Some(value) = arm.pattern.value() {
                    collect_expression_loaded_locals(expressions, value, visitor);
                }
                if let Some(guard) = arm.guard {
                    collect_expression_loaded_locals(expressions, guard, visitor);
                }
            }
        }
        HirTerminator::Return(values) => {
            for value in expressions.values(*values) {
                collect_expression_loaded_locals(expressions, *value, visitor);
            }
        }
        HirTerminator::ReturnError(value) => {
            collect_expression_loaded_locals(expressions, *value, visitor);
        }
        HirTerminator::NeverCall { args, .. } => {
            for value in expressions.values(*args) {
                collect_expression_loaded_locals(expressions, *value, visitor);
            }
        }
        HirTerminator::AssertFailure { message, .. } => {
            collect_expression_loaded_locals(expressions, *message, visitor);
        }

        HirTerminator::RuntimeFailure { .. } => {
            // Runtime-failure messages are backend-facing text, not HIR expressions.
        }

        HirTerminator::Uninitialized => {
            // Internal placeholder — no expressions to visit.
        }
        HirTerminator::Break { .. } | HirTerminator::Continue { .. } => {}
    }
}

/// Visits every local read by one expression tree, in evaluation order.
///
/// WHAT: walks the tree in pre-order with an explicit stack of child iterators. Each node is
/// visited before its children, and siblings keep source order.
/// WHY: nested arithmetic can be deeper than the thread stack allows, so the walk must not
/// recurse once per nesting level.
fn collect_expression_loaded_locals(
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
    visitor: &mut impl FnMut(LocalId),
) {
    visit_expression_read(expressions, expression_id, visitor);
    let mut frames = vec![expressions.child_values(expression_id)];
    while let Some(frame) = frames.last_mut() {
        match frame.next() {
            Some(child) => {
                visit_expression_read(expressions, child, visitor);
                frames.push(expressions.child_values(child));
            }
            None => {
                frames.pop();
            }
        }
    }
}

/// Reports the local that a load or copy node reads directly.
///
/// WHY: index operands are separate child expressions, so the caller's child iteration walks
/// them; reporting them here as well would count each read twice.
fn visit_expression_read(
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
    visitor: &mut impl FnMut(LocalId),
) {
    if let HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) =
        &expressions.expression(expression_id).kind
    {
        visitor(place.root);
    }
}

#[cfg(test)]
#[path = "tests/borrow_checker_metadata_tests.rs"]
mod borrow_checker_metadata_tests;
