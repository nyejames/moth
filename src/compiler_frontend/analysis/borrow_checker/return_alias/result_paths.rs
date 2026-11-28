//! The summary evaluator keeps projected provenance and simultaneous sharing separate.
//! A block has one joined input; loops join finite typed paths and site identities, rather
//! than expanding the set of execution paths or comparing independently joined exits.

use super::super::{
    collect_statement_defined_locals, collect_statement_loaded_locals,
    collect_statement_written_locals, collect_terminator_loaded_locals,
};
use super::{BorrowCheckError, BorrowChecker};
use crate::compiler_frontend::analysis::borrow_checker::state::{
    BindingSpec, BorrowState, OrderKey, RootSet, UNKNOWN_ORDER_KEY, cyclic_blocks,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::call_member_result;
use crate::compiler_frontend::analysis::borrow_checker::transfer::call_semantics::{
    BorrowedCall, CallReturns,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    OperationView, SparseBlockView,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalReturnAlias};
use crate::compiler_frontend::hir::expression_store::{HirExpressionStore, HirProjection};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{BlockId, FieldId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::reachability::{statement_call_target, terminator_call_target};
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::utils::for_each_terminator_target;
use crate::compiler_frontend::instrumentation::frontend_counters::{
    FrontendCounter, add_frontend_counter,
};
use crate::compiler_frontend::public_call_summary::typed::{
    EntryPort, IdentityKind, PortId, Predicate, ReachKind, Role, SupportId,
};
use crate::compiler_frontend::public_call_summary::{
    CallAllocationFacts, CallField, CallLaneSummary, CallOutputPath, CallParameterPath,
    CallPathSource, CallResultPath, CallSharing, FunctionReturnAliasSummary,
    PublicCallParameterAccess, PublicCallSummary,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Bound::{Included, Unbounded};
use std::rc::Rc;

#[path = "call.rs"]
mod call;
#[path = "graph.rs"]
mod graph;
#[cfg(test)]
#[path = "result_paths_tests.rs"]
mod result_paths_tests;

use graph::{Cell, Graph, Origin, Owner, Path, Site, Spec};

#[derive(Clone, Copy)]
struct Local {
    mutable: bool,
    mutable_parameter: Option<usize>,
    authored: bool,
}

pub(in crate::compiler_frontend::analysis::borrow_checker) struct IndexedFunction {
    locals: FxHashMap<LocalId, Local>,
    retirements: FxHashMap<BlockId, Vec<(usize, LocalId)>>,
    pub callees: Vec<FunctionId>,
    tracks_values: bool,
}

impl IndexedFunction {
    pub(in crate::compiler_frontend::analysis::borrow_checker) fn new(
        checker: &BorrowChecker<'_>,
        function: &HirFunction,
        collect_callees: bool,
    ) -> Result<Self, BorrowCheckError> {
        let mut locals = FxHashMap::default();
        let mut callees = Vec::new();
        let mut retirements = FxHashMap::<_, Vec<_>>::default();
        let mut tracks_values = false;
        if let Some(shape) = function.return_contract.ordinary() {
            for ty in shape.success().iter().copied().chain(shape.error()) {
                tracks_values |= is_heap(checker, ty)?;
            }
            let access = &checker.public_call_summaries[&function.id].parameters;
            for (&local, parameter) in function.params.iter().zip(access) {
                if !tracks_values && parameter.access == PublicCallParameterAccess::Mutable {
                    tracks_values = is_heap(checker, checker.local_types[&local])?;
                }
            }
        }
        if !tracks_values && !collect_callees {
            return Ok(Self {
                locals,
                retirements,
                callees,
                tracks_values,
            });
        }
        let expressions = &checker.module.expressions;
        let mut uses = FxHashMap::<LocalId, LocalUses>::default();
        for block in checker.collect_reachable_blocks(function)? {
            let block = checker.block_by_id_or_error(block, function.id)?;
            if tracks_values {
                locals.extend(block.locals.iter().map(|local| {
                    let authored = checker
                        .module
                        .side_table
                        .local_origin_kind(local.id)
                        .is_none_or(|origin| origin == HirLocalOriginKind::User);
                    (
                        local.id,
                        Local {
                            mutable: local.mutable,
                            mutable_parameter: None,
                            authored,
                        },
                    )
                }));
            }
            for (index, statement) in block.statements.iter().enumerate() {
                if collect_callees
                    && let Some(CallTarget::Local(callee)) = statement_call_target(&statement.kind)
                {
                    callees.push(*callee);
                }
                if !tracks_values {
                    continue;
                }
                collect_statement_loaded_locals(expressions, statement, &mut |local| {
                    LocalUses::record(&mut uses, local, block.id, index);
                });
                collect_statement_defined_locals(statement, &mut |local| {
                    uses.entry(local).or_insert(LocalUses {
                        defining_block: Some(block.id),
                        last: index,
                    });
                });
                collect_statement_written_locals(statement, &mut |local| {
                    LocalUses::record(&mut uses, local, block.id, index);
                });
            }
            if tracks_values {
                let mut use_in_terminator = |local| {
                    LocalUses::record(&mut uses, local, block.id, block.statements.len());
                };
                // Plumbing is not a user read, but the extractor still needs to capture it.
                if let HirTerminator::Jump { args, .. } = &block.terminator {
                    for arg in args {
                        use_in_terminator(arg.source);
                        use_in_terminator(arg.destination);
                    }
                } else {
                    collect_terminator_loaded_locals(
                        expressions,
                        &block.terminator,
                        &mut use_in_terminator,
                    );
                }
            }
            if collect_callees
                && let Some(CallTarget::Local(callee)) = terminator_call_target(&block.terminator)
            {
                callees.push(*callee);
            }
        }
        if tracks_values {
            for (index, parameter) in function.params.iter().enumerate() {
                let local = locals.get_mut(parameter).ok_or_else(|| {
                    checker
                        .diagnostics
                        .internal_error("Missing summary parameter local")
                })?;
                local.mutable_parameter = local.mutable.then_some(index);
            }
        }
        for (local, uses) in uses {
            if let Some(block) = uses.defining_block {
                retirements
                    .entry(block)
                    .or_default()
                    .push((uses.last, local));
            }
        }
        for events in retirements.values_mut() {
            events.sort_unstable_by_key(|&(index, local)| (index, local.0));
        }
        callees.sort_unstable_by_key(|callee| callee.0);
        callees.dedup();
        Ok(Self {
            locals,
            retirements,
            callees,
            tracks_values,
        })
    }
}

/// A HIR revision owns these coordinates. Published contracts use the typed
/// ports, not the local, operation or block IDs of the producer.
#[allow(dead_code)] // Queried directly until semantic transfer uses the sparse view.
pub(super) struct SparseFunction {
    pub(super) ports: Vec<EntryPort>,
    pub(super) port_supports: Vec<SupportId>,
    pub(super) guards: Vec<Predicate>,
    pub(super) compatibility: Vec<CompatibilityConstraint>,
    #[cfg(test)]
    pub(super) operations: BTreeMap<IndexedSite, Box<[SupportId]>>,
    operations_by_support: BTreeMap<(SupportId, BlockId), Box<[usize]>>,
    pub(super) conditional_supports: BTreeSet<SupportId>,
    pub(super) conditional_arm_count: usize,
    pub(super) counts: IndexCounts,
    transfer: Rc<SparseTransferIndex>,
    envelope: BTreeMap<(usize, Box<[CallField]>, DemandKind), SupportId>,
    result_sources: BTreeMap<(bool, usize), ResultSources>,
}

/// Transfer coordinates are unchanged when a callee's structural envelope widens.
struct SparseTransferIndex {
    local_ids: Vec<LocalId>,
    local_indices: FxHashMap<LocalId, usize>,
    mutable: Vec<bool>,
    heap: Vec<bool>,
    first_write: Vec<i32>,
    successors: FxHashMap<BlockId, Vec<BlockId>>,
    predecessors: FxHashMap<BlockId, Vec<BlockId>>,
    recurring: Rc<FxHashSet<BlockId>>,
    ranks: Rc<FxHashMap<BlockId, usize>>,
    postdom_jumps: Vec<FxHashMap<BlockId, BlockId>>,
    structural_barriers: BTreeSet<usize>,
    acyclic_entry: Option<BlockId>,
    blocks: FxHashMap<BlockId, SparseBlockOrders>,
}

struct SparseBlockOrders {
    statements: Box<[i32]>,
    operands: Box<[Box<[usize]>]>,
    terminator: i32,
}

impl SparseTransferIndex {
    fn new(checker: &BorrowChecker<'_>, snapshot: &Snapshot<'_>) -> Result<Self, BorrowCheckError> {
        let blocks = || {
            snapshot
                .blocks
                .iter()
                .map(|id| &checker.module.blocks[checker.block_index_by_id[id]])
        };
        let mut locals: Vec<_> = blocks().flat_map(|block| &block.locals).collect();
        locals.sort_unstable_by_key(|local| local.id.0);
        let mut result = Self {
            local_ids: locals.iter().map(|local| local.id).collect(),
            local_indices: locals
                .iter()
                .enumerate()
                .map(|(i, local)| (local.id, i))
                .collect(),
            mutable: locals.iter().map(|local| local.mutable).collect(),
            heap: locals
                .iter()
                .map(|local| is_heap(checker, local.ty))
                .collect::<Result<_, _>>()?,
            first_write: vec![UNKNOWN_ORDER_KEY; locals.len()],
            successors: FxHashMap::default(),
            predecessors: FxHashMap::default(),
            recurring: Rc::default(),
            acyclic_entry: Some(snapshot.function.entry),
            blocks: FxHashMap::default(),
            ranks: Rc::default(),
            postdom_jumps: Vec::new(),
            structural_barriers: BTreeSet::new(),
        };
        let mut prior_statements = 0;
        for (block_index, block) in blocks().enumerate() {
            // The preceding blocks each contribute one terminator in addition to their statements.
            let first_order = (prior_statements + block_index) as OrderKey;
            prior_statements += block.statements.len();
            let order = (prior_statements + block_index) as OrderKey;
            let mut first_write = |local, order| {
                let slot = &mut result.first_write[result.local_indices[&local]];
                if *slot == UNKNOWN_ORDER_KEY {
                    *slot = order;
                }
            };
            let mut operands = Vec::with_capacity(block.statements.len() + 1);
            let statements = block
                .statements
                .iter()
                .enumerate()
                .map(|(position, statement)| {
                    let at = first_order + position as OrderKey;
                    collect_statement_written_locals(statement, &mut |local| {
                        first_write(local, at)
                    });
                    let mut slots = Vec::new();
                    let mut push = |local| slots.push(result.local_indices[&local]);
                    collect_statement_loaded_locals(
                        &checker.module.expressions,
                        statement,
                        &mut push,
                    );
                    collect_statement_defined_locals(statement, &mut push);
                    collect_statement_written_locals(statement, &mut push);
                    slots.sort_unstable();
                    slots.dedup();
                    operands.push(slots.into_boxed_slice());
                    at
                })
                .collect();
            let mut slots = Vec::new();
            let mut push = |local| slots.push(result.local_indices[&local]);
            collect_terminator_loaded_locals(
                &checker.module.expressions,
                &block.terminator,
                &mut push,
            );
            match &block.terminator {
                HirTerminator::Jump { args, .. } => {
                    for argument in args {
                        first_write(argument.destination, order);
                        push(argument.source);
                        push(argument.destination);
                    }
                }
                HirTerminator::Invoke {
                    success_results,
                    error_result,
                    ..
                } => {
                    for &local in success_results.iter().chain(std::iter::once(error_result)) {
                        first_write(local, order);
                        push(local);
                    }
                }
                _ => {}
            }
            slots.sort_unstable();
            slots.dedup();
            operands.push(slots.into_boxed_slice());
            let successors = result.successors.entry(block.id).or_default();
            for_each_terminator_target(&block.terminator, |next| successors.push(next));
            for &next in successors.iter() {
                result.predecessors.entry(next).or_default().push(block.id);
            }
            if successors.contains(&snapshot.function.entry) {
                result.acyclic_entry = None;
            }
            result.blocks.insert(
                block.id,
                SparseBlockOrders {
                    statements,
                    operands: operands.into_boxed_slice(),
                    terminator: order,
                },
            );
        }
        result.ranks = Rc::new(
            crate::compiler_frontend::analysis::borrow_checker::engine::reverse_postorder_ranks(
                snapshot.function.entry,
                &result.successors,
            ),
        );
        for block in blocks() {
            if matches!(block.terminator, HirTerminator::Uninitialized) {
                result.structural_barriers.insert(result.ranks[&block.id]);
            }
        }
        let cfg = SparseCfg::new(checker, snapshot, &mut 0);
        result.recurring = Rc::new(cyclic_blocks(&result.successors));
        if result.recurring.is_empty() {
            let parents = cfg.postdominators(&mut 0);
            let immediate: FxHashMap<_, _> = snapshot
                .blocks
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(i, block)| snapshot.blocks.get(parents[i]).map(|&next| (block, next)))
                .collect();
            result.postdom_jumps.push(immediate);
            // Jump along the postdominator chain without walking each identity diamond.
            while let Some(previous) = result.postdom_jumps.last() {
                let next: FxHashMap<_, _> = previous
                    .iter()
                    .filter_map(|(&from, through)| previous.get(through).map(|&to| (from, to)))
                    .collect();
                if next.is_empty() {
                    break;
                }
                result.postdom_jumps.push(next);
            }
        }
        Ok(result)
    }
}

#[allow(dead_code)] // Prepared for support-local execution after typed publication is enabled.
pub(in crate::compiler_frontend::analysis::borrow_checker) struct ProjectedSupport<'a> {
    function: &'a SparseFunction,
    support: SupportId,
    barriers: BTreeSet<usize>,
    visibility: FxHashMap<BlockId, RootSet>,
    #[cfg(test)]
    compress: bool,
    local_ids: Vec<LocalId>,
    local_indices: FxHashMap<LocalId, usize>,
    mutable: Vec<bool>,
    heap: Vec<bool>,
    first_write: Vec<i32>,
}

#[allow(dead_code)] // Prepared for guarded extraction.
impl SparseFunction {
    fn project_support(
        &self,
        support: SupportId,
        anchors: impl IntoIterator<Item = LocalId>,
        visibility: impl IntoIterator<Item = (BlockId, RootSet)>,
    ) -> ProjectedSupport<'_> {
        let coordinates = &self.transfer;
        let mut barriers = coordinates.structural_barriers.clone();
        let mut locals: BTreeSet<_> = anchors
            .into_iter()
            .map(|local| coordinates.local_indices[&local])
            .collect();
        for (&(_, block), positions) in self
            .operations_by_support
            .range((support, BlockId(u32::MIN))..=(support, BlockId(u32::MAX)))
        {
            barriers.insert(coordinates.ranks[&block]);
            for &position in positions.iter() {
                locals.extend(
                    coordinates.blocks[&block].operands[position]
                        .iter()
                        .copied(),
                );
            }
        }
        let mut projected = ProjectedSupport {
            function: self,
            support,
            barriers,
            visibility: FxHashMap::default(),
            #[cfg(test)]
            compress: true,
            local_ids: Vec::with_capacity(locals.len()),
            local_indices: FxHashMap::default(),
            mutable: Vec::with_capacity(locals.len()),
            heap: Vec::with_capacity(locals.len()),
            first_write: Vec::with_capacity(locals.len()),
        };
        projected.local_indices.reserve(locals.len());
        for i in locals {
            let local = coordinates.local_ids[i];
            projected
                .local_indices
                .insert(local, projected.local_ids.len());
            projected.local_ids.push(local);
            projected.mutable.push(coordinates.mutable[i]);
            projected.heap.push(coordinates.heap[i]);
            projected.first_write.push(coordinates.first_write[i]);
        }
        for (target, mask) in visibility {
            assert_eq!(mask.bit_len(), coordinates.local_ids.len());
            let mut visible = RootSet::empty(projected.local_ids.len());
            for (index, local) in projected.local_ids.iter().enumerate() {
                if mask.contains(coordinates.local_indices[local]) {
                    visible.insert(index);
                }
            }
            if visible.covers_all(projected.local_ids.len()) {
                continue;
            }
            projected.visibility.insert(target, visible);
            projected.barriers.insert(coordinates.ranks[&target]);
            // Narrowing does not distribute over a join. Keep the authored edge
            // source so all its incoming facts are joined before the mask applies.
            if let Some(predecessors) = coordinates.predecessors.get(&target) {
                projected
                    .barriers
                    .extend(predecessors.iter().map(|block| coordinates.ranks[block]));
            }
        }
        projected
    }
}

#[allow(dead_code)] // Prepared for guarded extraction.
impl ProjectedSupport<'_> {
    pub(in crate::compiler_frontend::analysis::borrow_checker) fn continuation(
        &self,
        mut block: BlockId,
    ) -> BlockId {
        #[cfg(test)]
        if !self.compress {
            return block;
        }
        let transfer = &self.function.transfer;
        let rank = transfer.ranks[&block];
        let barrier = self
            .barriers
            .range(..=rank)
            .next_back()
            .copied()
            .unwrap_or(0);
        // In a DAG an interior block lies strictly between an origin and its
        // postdominator in RPO. Jump only when that whole interval is identity.
        for jumps in transfer.postdom_jumps.iter().rev() {
            if let Some(&next) = jumps.get(&block)
                && transfer.ranks[&next] >= barrier
            {
                block = next;
            }
        }
        block
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn ranks(
        &self,
    ) -> Rc<FxHashMap<BlockId, usize>> {
        Rc::clone(&self.function.transfer.ranks)
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn recurring_blocks(
        &self,
    ) -> Rc<FxHashSet<BlockId>> {
        Rc::clone(&self.function.transfer.recurring)
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn visibility(
        &self,
        block: BlockId,
    ) -> Option<&RootSet> {
        self.visibility.get(&block)
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn block_view(
        &self,
        block: BlockId,
    ) -> SparseBlockView<'_> {
        let coordinates = &self.function.transfer;
        let orders = &coordinates.blocks[&block];
        let positions = self
            .function
            .operations_by_support
            .get(&(self.support, block))
            .map(Box::as_ref)
            .unwrap_or_default();
        let count = orders.statements.len();
        let transfer_terminator = positions.last() == Some(&count);
        SparseBlockView {
            operations: OperationView {
                local_ids: &self.local_ids,
                local_indices: &self.local_indices,
                local_mutable: &self.mutable,
                local_has_in_place_mutation_surface: &self.heap,
                block_successors: &coordinates.successors,
                local_first_write_order: &self.first_write,
                acyclic_entry_block: coordinates.acyclic_entry,
            },
            statement_orders: &orders.statements,
            statement_positions: if transfer_terminator {
                &positions[..positions.len() - 1]
            } else {
                positions
            },
            terminator_order: orders.terminator,
            transfer_terminator,
        }
    }

    fn seed_entry(
        &self,
        bindings: impl IntoIterator<Item = (LocalId, BindingSpec)>,
    ) -> BorrowState {
        let mut entry = BorrowState::new_uninitialized(self.local_ids.len());
        for (local, binding) in bindings {
            entry.initialize_entry_binding(self.local_indices[&local], binding);
        }
        entry
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ResultSource {
    parameter: usize,
    input: Box<[CallField]>,
}

#[derive(Default, PartialEq, Eq)]
struct ResultSources(BTreeMap<Box<[CallField]>, BTreeSet<ResultSource>>);

impl ResultSources {
    // A demand can select above a retained source or continue inside it.
    // Probe ancestors and then the contiguous range beneath the demand.
    fn select<'a>(
        &'a self,
        tail: &'a [CallField],
    ) -> impl Iterator<Item = (&'a ResultSource, &'a [CallField], &'a [CallField])> + 'a {
        let ancestors = (0..tail.len()).filter_map(move |length| {
            self.0
                .get(&tail[..length])
                .map(|sources| (sources, &[][..], &tail[length..]))
        });
        let descendants = self
            .0
            .range::<[CallField], _>((Included(tail), Unbounded))
            .take_while(move |(path, _)| path.starts_with(tail))
            .map(move |(path, sources)| (sources, &path[tail.len()..], &[][..]));
        ancestors
            .chain(descendants)
            .flat_map(|(sources, output, input)| {
                sources.iter().map(move |source| (source, output, input))
            })
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ResultDemand {
    Value(HirValueId, Box<[CallField]>),
    Place(IndexedPlace),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code)] // Matching consumes these compact constraints after indexing.
pub(super) enum CompatibilityConstraint {
    Equality(IdentityKind, Vec<PortId>),
    Reach(Vec<PortId>),
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct IndexCounts {
    pub(super) blocks: usize,
    pub(super) index_builds: usize,
    pub(super) statements: usize,
    /// HIR operands counted once per snapshot, not probes on every build.
    pub(super) arguments: usize,
    pub(super) access_incidences: usize,
    pub(super) selector_incidences: usize,
    pub(super) reach_incidences: usize,
    pub(super) compatibility_candidates: usize,
    pub(super) support_candidates: usize,
    pub(super) reverse_visits: usize,
}

impl std::ops::AddAssign for IndexCounts {
    fn add_assign(&mut self, other: Self) {
        self.blocks += other.blocks;
        self.index_builds += other.index_builds;
        self.statements += other.statements;
        self.arguments += other.arguments;
        self.access_incidences += other.access_incidences;
        self.selector_incidences += other.selector_incidences;
        self.reach_incidences += other.reach_incidences;
        self.compatibility_candidates += other.compatibility_candidates;
        self.support_candidates += other.support_candidates;
        self.reverse_visits += other.reverse_visits;
    }
}

impl IndexCounts {
    fn record(self, entry_ports: usize) {
        use FrontendCounter::*;
        for (counter, count) in [
            (BorrowIndexEntryPorts, entry_ports),
            (BorrowIndexAccessIncidences, self.access_incidences),
            (BorrowIndexSelectorIncidences, self.selector_incidences),
            (BorrowIndexReachIncidences, self.reach_incidences),
            (
                BorrowIndexCompatibilityCandidates,
                self.compatibility_candidates,
            ),
            (BorrowIndexSupportCandidates, self.support_candidates),
            (BorrowIndexReverseVisits, self.reverse_visits),
        ] {
            add_frontend_counter(counter, count);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum DemandKind {
    Value,
    Cell,
    Retarget,
    Reach(ReachKind),
}

impl DemandKind {
    fn writes(self) -> bool {
        matches!(
            self,
            Self::Cell | Self::Retarget | Self::Reach(ReachKind::EntryStructure)
        )
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct IndexedPlace {
    local: LocalId,
    path: Box<[CallField]>,
}

#[derive(Clone, Copy)]
enum Input {
    Value(HirValueId),
    Jump(LocalId),
    Member(HirValueId),
    CallResult(usize, Option<usize>),
}

// HIR IDs deliberately have no global ordering. The block ordinal is only a
// stable order within the owning revision.
pub(super) type IndexedSite = (u32, usize);

struct IndexedCall<'a> {
    site: IndexedSite,
    target: &'a CallTarget,
    args: &'a [HirValueId],
    continuation: CallContinuation,
}

#[derive(Clone, Copy)]
enum CallContinuation {
    Statement,
    Invoke,
    Never,
}

#[derive(Clone, Copy)]
enum Observation {
    Place(HirPlace, DemandKind),
    Value(HirValueId),
    Member(HirValueId, DemandKind),
}

struct Snapshot<'a> {
    function: &'a HirFunction,
    blocks: Vec<BlockId>,
    writes: Vec<(IndexedSite, HirPlace, Option<HirValueId>)>,
    returns: Vec<(IndexedSite, bool, usize, HirValueId)>,
    maps: Vec<(IndexedSite, HirMapOp, HirValueId, &'a [HirValueId])>,
    definitions: FxHashMap<LocalId, Vec<(IndexedSite, Input)>>,
    aliases: FxHashMap<LocalId, Vec<HirPlace>>,
    calls: Vec<IndexedCall<'a>>,
    counts: IndexCounts,
}

impl<'a> Snapshot<'a> {
    fn call(
        &mut self,
        site: IndexedSite,
        target: &'a CallTarget,
        args: &'a [HirValueId],
        continuation: CallContinuation,
        results: impl IntoIterator<Item = (LocalId, Option<usize>)>,
    ) {
        self.counts.arguments += args.len();
        let index = self.calls.len();
        self.calls.push(IndexedCall {
            site,
            target,
            args,
            continuation,
        });
        for (local, slot) in results {
            self.definitions
                .entry(local)
                .or_default()
                .push((site, Input::CallResult(index, slot)));
        }
    }

    fn map(
        &mut self,
        site: IndexedSite,
        op: HirMapOp,
        receiver: HirValueId,
        args: &'a [HirValueId],
        result: Option<LocalId>,
    ) {
        self.counts.arguments += 1 + args.len();
        self.maps.push((site, op, receiver, args));
        if matches!(op, HirMapOp::Get | HirMapOp::Remove)
            && let Some(local) = result
        {
            self.definitions
                .entry(local)
                .or_default()
                .push((site, Input::Member(receiver)));
        }
    }

    fn collect(
        checker: &BorrowChecker<'a>,
        function: &'a HirFunction,
    ) -> Result<Self, BorrowCheckError> {
        let blocks = checker.collect_reachable_blocks(function)?;
        let mut result = Self {
            function,
            blocks: Vec::new(),
            writes: Vec::new(),
            returns: Vec::new(),
            maps: Vec::new(),
            definitions: FxHashMap::default(),
            aliases: FxHashMap::default(),
            calls: Vec::new(),
            counts: IndexCounts::default(),
        };
        for &id in &blocks {
            let block = checker
                .module
                .blocks
                .get(checker.block_index_by_id[&id])
                .ok_or_else(|| {
                    checker
                        .diagnostics
                        .internal_error("Missing block in sparse borrow index")
                })?;
            let expressions = &checker.module.expressions;
            result.counts.blocks += 1;
            result.counts.statements += block.statements.len();
            for (position, statement) in block.statements.iter().enumerate() {
                result.collect_statement((id.0, position), &statement.kind, expressions);
            }
            result.collect_terminator(
                (id.0, block.statements.len()),
                &block.terminator,
                expressions,
            );
        }
        result.blocks = blocks;
        Ok(result)
    }

    fn collect_statement(
        &mut self,
        site: IndexedSite,
        statement: &'a HirStatementKind,
        expressions: &'a HirExpressionStore,
    ) {
        match statement {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } => self.define_value(site, *local, *value, None, expressions),
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(place),
                value,
            } => {
                self.writes.push((site, *place, Some(*value)));
                if place.projections.is_empty() {
                    self.define_value(site, place.root, *value, Some(*place), expressions);
                }
            }
            HirStatementKind::Call {
                target,
                args,
                results,
            } => {
                self.call(
                    site,
                    target,
                    expressions.values(*args),
                    CallContinuation::Statement,
                    results
                        .iter()
                        .enumerate()
                        .map(|(slot, dest)| (dest.local(), Some(slot))),
                );
                for &destination in results {
                    self.collect_update(site, destination);
                }
            }
            HirStatementKind::MapOp {
                op,
                receiver,
                args,
                result,
            } => self.map(
                site,
                *op,
                *receiver,
                expressions.values(*args),
                result.map(HirLocalDestination::local),
            ),
            HirStatementKind::NumericOp { result, .. }
            | HirStatementKind::FastStoreConversion { result, .. }
            | HirStatementKind::LoopStepFailure { result, .. }
            | HirStatementKind::FormatFloat { result, .. }
            | HirStatementKind::ValidateFloat { result, .. } => {
                self.collect_update(site, *result);
            }
            HirStatementKind::RangeCandidate {
                candidate_result,
                in_range_result,
                ..
            } => {
                self.collect_update(site, *candidate_result);
                self.collect_update(site, *in_range_result);
            }
            _ => {}
        }
    }

    fn collect_update(&mut self, site: IndexedSite, destination: HirLocalDestination) {
        if let HirLocalDestination::Update(local) = destination {
            self.writes.push((site, HirPlace::local(local), None));
        }
    }

    fn define_value(
        &mut self,
        site: IndexedSite,
        local: LocalId,
        value: HirValueId,
        update_target: Option<HirPlace>,
        expressions: &HirExpressionStore,
    ) {
        self.definitions
            .entry(local)
            .or_default()
            .push((site, Input::Value(value)));
        let expression = expressions.expression(value);
        if expression.value_kind == ValueKind::Place
            && let HirExpressionKind::Load(source) = expression.kind
            && Some(source) != update_target
        {
            self.aliases.entry(local).or_default().push(source);
        }
    }

    fn collect_terminator(
        &mut self,
        site: IndexedSite,
        terminator: &'a HirTerminator,
        expressions: &'a HirExpressionStore,
    ) {
        match terminator {
            HirTerminator::Jump { args, .. } => {
                self.counts.arguments += args.len();
                for argument in args {
                    self.definitions
                        .entry(argument.destination)
                        .or_default()
                        .push((site, Input::Jump(argument.source)));
                }
            }
            HirTerminator::Invoke {
                operation: HirFallibleOperation::Call { target, args },
                success_results,
                error_result,
                ..
            } => self.call(
                site,
                target,
                expressions.values(*args),
                CallContinuation::Invoke,
                success_results
                    .iter()
                    .enumerate()
                    .map(|(slot, &local)| (local, Some(slot)))
                    .chain([(*error_result, None)]),
            ),
            HirTerminator::Invoke {
                operation: HirFallibleOperation::MapOp { op, receiver, args },
                success_results,
                ..
            } => self.map(
                site,
                *op,
                *receiver,
                expressions.values(*args),
                success_results.first().copied(),
            ),
            HirTerminator::NeverCall { target, args } => self.call(
                site,
                target,
                expressions.values(*args),
                CallContinuation::Never,
                [],
            ),
            HirTerminator::Return(values) => {
                self.returns.extend(
                    expressions
                        .values(*values)
                        .iter()
                        .enumerate()
                        .map(|(slot, &value)| (site, false, slot, value)),
                );
            }
            HirTerminator::ReturnError(value) => self.returns.push((site, true, 0, *value)),
            _ => {}
        }
    }
}

#[derive(Default)]
struct Supports(Vec<usize>);

impl Supports {
    fn insert(&mut self) -> usize {
        let id = self.0.len();
        self.0.push(id);
        id
    }

    fn root(&mut self, mut node: usize) -> usize {
        while self.0[node] != node {
            self.0[node] = self.0[self.0[node]];
            node = self.0[node];
        }
        node
    }

    fn couple(&mut self, left: usize, right: usize) {
        let left = self.root(left);
        let right = self.root(right);
        self.0[left.max(right)] = left.min(right);
    }

    fn connect_equality(
        &mut self,
        kind: IdentityKind,
        ports: &BTreeSet<PortId>,
        nodes: &[usize],
        counts: &mut IndexCounts,
        guards: &mut BTreeSet<Predicate>,
        broad: &mut BTreeSet<CompatibilityConstraint>,
    ) {
        let Some(&first) = ports.first() else {
            return;
        };
        for &other in ports.iter().skip(1) {
            counts.compatibility_candidates += 1;
            counts.support_candidates += 1;
            self.couple(nodes[first.0 as usize], nodes[other.0 as usize]);
        }
        match ports.len() {
            2 => {
                let last = *ports.last().expect("two indexed equality candidates");
                guards.insert(Predicate::Equal(kind, first, last));
            }
            3.. => {
                broad.insert(CompatibilityConstraint::Equality(
                    kind,
                    ports.iter().copied().collect(),
                ));
            }
            _ => {}
        }
    }
}

struct DemandBuilder<'a, 'module> {
    checker: &'a BorrowChecker<'module>,
    call_contracts: Vec<BorrowedCall<'a>>,
    snapshot: &'a Snapshot<'module>,
    callees: &'a FxHashMap<FunctionId, SparseFunction>,
    field_ordinals: &'a FxHashMap<FieldId, CallField>,
    parameter_indices: FxHashMap<LocalId, usize>,
    counts: IndexCounts,
    supports: Supports,
    sites: BTreeMap<IndexedSite, Vec<usize>>,
    local_nodes: FxHashMap<(IndexedPlace, DemandKind), usize>,
    ports: Vec<EntryPort>,
    port_index: FxHashMap<EntryPort, PortId>,
    port_nodes: Vec<usize>,
    // Match locations by the containing allocation's type and the selector,
    // not by the type of what was previously stored there.
    selector_index: BTreeMap<(TypeId, CallField), BTreeMap<PortId, Vec<usize>>>,
    written_selectors: BTreeSet<(TypeId, CallField)>,
    holders_by_type: BTreeMap<TypeId, BTreeMap<PortId, Vec<usize>>>,
    wide_reach: BTreeMap<TypeId, BTreeSet<PortId>>,
    terminals_by_type: BTreeMap<TypeId, BTreeSet<PortId>>,
    edges_by_value_type: BTreeMap<TypeId, BTreeMap<PortId, Vec<usize>>>,
    wrappers_by_type: BTreeMap<TypeId, BTreeSet<PortId>>,
    demanded_roles: BTreeSet<Predicate>,
    constructed_fields: FxHashMap<HirValueId, FxHashMap<CallField, HirValueId>>,
    envelope: BTreeMap<(usize, Box<[CallField]>, DemandKind), usize>,
    work: VecDeque<(IndexedPlace, DemandKind, usize)>,
    value_work: VecDeque<(HirValueId, Box<[CallField]>, DemandKind, usize)>,
    value_nodes: FxHashMap<(HirValueId, Box<[CallField]>, DemandKind), usize>,
    path_ancestors: Vec<TypeId>,
}

impl<'a, 'module> DemandBuilder<'a, 'module> {
    fn new(
        checker: &'a BorrowChecker<'module>,
        snapshot: &'a Snapshot<'module>,
        field_ordinals: &'a FxHashMap<FieldId, CallField>,
        callees: &'a FxHashMap<FunctionId, SparseFunction>,
    ) -> Self {
        Self {
            checker,
            callees,
            call_contracts: Vec::with_capacity(snapshot.calls.len()),
            snapshot,
            field_ordinals,
            parameter_indices: snapshot
                .function
                .params
                .iter()
                .enumerate()
                .map(|(index, &local)| (local, index))
                .collect(),
            counts: IndexCounts {
                index_builds: 1,
                ..IndexCounts::default()
            },
            supports: Supports::default(),
            sites: BTreeMap::new(),
            local_nodes: FxHashMap::default(),
            ports: Vec::new(),
            port_index: FxHashMap::default(),
            port_nodes: Vec::new(),
            terminals_by_type: BTreeMap::new(),
            edges_by_value_type: BTreeMap::new(),
            wrappers_by_type: BTreeMap::new(),
            demanded_roles: BTreeSet::new(),
            constructed_fields: FxHashMap::default(),
            selector_index: BTreeMap::new(),
            written_selectors: BTreeSet::new(),
            holders_by_type: BTreeMap::new(),
            wide_reach: BTreeMap::new(),
            envelope: BTreeMap::new(),
            work: VecDeque::new(),
            value_work: VecDeque::new(),
            value_nodes: FxHashMap::default(),
            path_ancestors: Vec::new(),
        }
    }

    fn couple(&mut self, left: usize, right: usize) {
        self.counts.support_candidates += 1;
        self.supports.couple(left, right);
    }

    fn port(&mut self, port: EntryPort, dependent: Option<usize>) -> PortId {
        let id = match self.port_index.entry(port) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let id = PortId(u32::try_from(self.ports.len()).expect("HIR demand port capacity"));
                self.ports.push(entry.key().clone());
                entry.insert(id);
                self.port_nodes.push(self.supports.insert());
                id
            }
        };
        if let Some(node) = dependent {
            self.couple(node, self.port_nodes[id.0 as usize]);
        }
        id
    }

    fn site(&mut self, site: IndexedSite) -> usize {
        let node = self.supports.insert();
        self.sites.entry(site).or_default().push(node);
        node
    }

    fn place_path(&self, place: HirPlace, tail: &[CallField]) -> IndexedPlace {
        let projections = self
            .checker
            .module
            .expressions
            .projections(place.projections);
        let path = projections
            .iter()
            .map(|projection| match projection {
                HirProjection::Field(field) => self.field_ordinals[field],
                HirProjection::Index(_) => CallField::Elem,
            })
            .chain(tail.iter().copied())
            .collect();
        IndexedPlace {
            local: place.root,
            path,
        }
    }

    fn demand_place(&mut self, mut place: IndexedPlace, mut kind: DemandKind, node: usize) {
        self.counts.access_incidences += 1;
        let prefix = self.finite_prefix(self.checker.local_types[&place.local], &place.path);
        if prefix != place.path.len() {
            // A recursive layout tail is a typed reach family, not a
            // new coordinate each time this path crosses a call or loop.
            place.path = place.path[..prefix].into();
            kind = DemandKind::Reach(if kind.writes() {
                ReachKind::EntryStructure
            } else {
                ReachKind::Referent
            });
        }
        match self.local_nodes.entry((place, kind)) {
            Entry::Occupied(entry) => {
                let first = *entry.get();
                self.couple(first, node);
            }
            Entry::Vacant(entry) => {
                self.work.push_back((entry.key().0.clone(), kind, node));
                entry.insert(node);
            }
        }
    }

    fn demand_value(
        &mut self,
        value: HirValueId,
        path: &[CallField],
        kind: DemandKind,
        node: usize,
    ) {
        self.demand_value_path(value, path.into(), kind, node);
    }

    fn demand_value_path(
        &mut self,
        value: HirValueId,
        path: Box<[CallField]>,
        kind: DemandKind,
        node: usize,
    ) {
        match self.value_nodes.entry((value, path, kind)) {
            Entry::Occupied(entry) => {
                let first = *entry.get();
                self.couple(first, node);
            }
            Entry::Vacant(entry) => {
                self.value_work
                    .push_back((value, entry.key().1.clone(), kind, node));
                entry.insert(node);
            }
        }
    }

    fn entry(&mut self, param: usize, place: &IndexedPlace, kind: DemandKind, node: usize) {
        let path = place.path.as_ref();
        let mut ty = self.checker.local_types[&place.local];
        let access =
            self.checker.public_call_summaries[&self.snapshot.function.id].parameters[param].access;
        let writable = access == PublicCallParameterAccess::Mutable;
        if kind.writes() && !writable {
            return;
        }
        self.envelope
            .insert((param, place.path.clone(), kind), node);
        if path.is_empty()
            && matches!(
                kind,
                DemandKind::Cell | DemandKind::Retarget | DemandKind::Reach(ReachKind::Cell)
            )
        {
            let terminal = self.port(EntryPort::Terminal(param), Some(node));
            self.terminals_by_type
                .entry(ty)
                .or_default()
                .insert(terminal);
            if kind != DemandKind::Cell {
                let wrapper = self.port(EntryPort::Wrapper(param), Some(node));
                self.wrappers_by_type.entry(ty).or_default().insert(wrapper);
                self.demanded_roles
                    .insert(Predicate::Role(wrapper, Role::Slot));
                if kind == DemandKind::Reach(ReachKind::Cell) {
                    self.demanded_roles
                        .insert(Predicate::Role(wrapper, Role::WholeAlias));
                }
                self.port(EntryPort::Immediate(param), Some(node));
            }
            return;
        }
        for (index, &selector) in path.iter().enumerate() {
            self.counts.selector_incidences += 1;
            let final_cell =
                kind.writes() && index + 1 == path.len() && !matches!(kind, DemandKind::Reach(_));
            let holder = self.port(
                EntryPort::Value {
                    parameter: param,
                    path: path[..index].into(),
                },
                None,
            );
            self.holders_by_type
                .entry(ty)
                .or_default()
                .entry(holder)
                .or_default()
                .push(node);
            let edge = self.port(
                EntryPort::Edge(holder, selector),
                final_cell.then_some(node),
            );
            self.selector_index
                .entry((ty, selector))
                .or_default()
                .entry(holder)
                .or_default()
                .push(node);
            if final_cell {
                self.written_selectors.insert((ty, selector));
            }
            ty = self
                .checker
                .call_child_type(ty, selector)
                .expect("checked entry path");
            if !selector.is_collapsed() {
                self.edges_by_value_type
                    .entry(ty)
                    .or_default()
                    .entry(edge)
                    .or_default()
                    .push(node);
            }
        }
        if matches!(kind, DemandKind::Value | DemandKind::Reach(_)) {
            let port = self.port(
                EntryPort::Value {
                    parameter: param,
                    path: path.into(),
                },
                Some(node),
            );
            if matches!(kind, DemandKind::Reach(_))
                && classify_hir_type(ty, self.checker.type_environment)
                    .is_ok_and(|shape| shape.has_in_place_mutation_surface())
            {
                self.counts.reach_incidences += 1;
                self.wide_reach.entry(ty).or_default().insert(port);
            }
        }
    }

    fn visit_place(&mut self, place: IndexedPlace, kind: DemandKind, node: usize) {
        if let Some(&param) = self.parameter_indices.get(&place.local) {
            self.entry(param, &place, kind, node);
        }
        let definitions = self.snapshot.definitions.get(&place.local);
        if let Some(definitions) = definitions {
            for &(site, _) in definitions {
                self.counts.reverse_visits += 1;
                self.sites.entry(site).or_default().push(node);
            }
        }
        if kind.writes() {
            if let Some(sources) = self.snapshot.aliases.get(&place.local) {
                for &source in sources {
                    let next = self.place_path(source, &place.path);
                    let kind = if next.path.is_empty() {
                        kind
                    } else if kind == DemandKind::Retarget {
                        DemandKind::Cell
                    } else {
                        kind
                    };
                    self.demand_place(next, kind, node);
                }
            }
            if place.path.is_empty() && !matches!(kind, DemandKind::Reach(_)) {
                return;
            }
        }
        if let Some(definitions) = definitions {
            for &(_, source) in definitions {
                self.counts.reverse_visits += 1;
                match source {
                    Input::Value(value) => self.demand_value(value, &place.path, kind, node),
                    Input::Jump(local) => self.demand_place(
                        IndexedPlace {
                            local,
                            path: place.path.clone(),
                        },
                        kind,
                        node,
                    ),
                    Input::Member(receiver) => {
                        self.demand_member(receiver, &place.path, kind, node);
                    }
                    Input::CallResult(call_index, slot) => {
                        let call = &self.snapshot.calls[call_index];
                        if let Some(effects) = self.sites.get(&call.site) {
                            for &effect in effects {
                                self.counts.support_candidates += 1;
                                self.supports.couple(node, effect);
                            }
                        }
                        if slot == Some(0)
                            && let Some(receiver) = call_member_result(call.target)
                        {
                            self.demand_member(call.args[receiver], &place.path, kind, node);
                            continue;
                        }
                        let reach = if kind.writes() {
                            ReachKind::EntryStructure
                        } else {
                            ReachKind::Referent
                        };
                        match self.call_contracts[call_index].returns() {
                            CallReturns::External(slots) => {
                                if let Some(slot) = slot.and_then(|index| slots.get(index))
                                    && let ExternalReturnAlias::AliasArgs(indices) = &slot.alias
                                {
                                    for &index in indices {
                                        let actual = call.args[index];
                                        self.demand_opaque_result(
                                            actual, &place, kind, reach, node,
                                        );
                                    }
                                }
                            }
                            CallReturns::Summary(_) => {
                                if let CallTarget::Local(target) = call.target {
                                    for &actual in call.args {
                                        self.demand_value(
                                            actual,
                                            &[],
                                            DemandKind::Reach(reach),
                                            node,
                                        );
                                    }
                                    if place.path.is_empty() {
                                        continue;
                                    }
                                    if let Some(sources) =
                                        self.callees.get(target).and_then(|callee| {
                                            callee.result_sources.get(&outcome_slot(slot))
                                        })
                                    {
                                        self.counts.reverse_visits += place.path.len() + 1;
                                        for (source, _, tail) in sources.select(&place.path) {
                                            self.counts.reverse_visits += 1;
                                            if kind.writes()
                                                && !matches!(kind, DemandKind::Reach(_))
                                                && tail.is_empty()
                                            {
                                                // A cell allocated by the callee is not a cell of its actual.
                                                continue;
                                            }
                                            let path = join_selectors(&source.input, tail);
                                            self.demand_value_path(
                                                call.args[source.parameter],
                                                path,
                                                kind,
                                                node,
                                            );
                                        }
                                    }
                                } else {
                                    for &actual in call.args {
                                        self.demand_opaque_result(
                                            actual, &place, kind, reach, node,
                                        );
                                    }
                                }
                            }
                            CallReturns::Fresh => {}
                        }
                    }
                }
            }
        }
    }

    fn demand_opaque_result(
        &mut self,
        actual: HirValueId,
        result: &IndexedPlace,
        kind: DemandKind,
        reach: ReachKind,
        node: usize,
    ) {
        self.demand_value(actual, &[], DemandKind::Reach(reach), node);
        // An opaque whole-argument alias provides no construction route.
        // Its selectors can only be refined when the result has the same type.
        if !result.path.is_empty()
            && self.checker.module.expressions.expression(actual).ty
                == self.checker.local_types[&result.local]
        {
            self.demand_value(actual, &result.path, kind, node);
        }
    }

    fn demand_member(
        &mut self,
        receiver: HirValueId,
        tail: &[CallField],
        kind: DemandKind,
        node: usize,
    ) {
        let path = join_selectors(&[CallField::Elem], tail);
        self.demand_value_path(receiver, path, kind, node);
        self.demand_value(receiver, &[], DemandKind::Reach(ReachKind::Referent), node);
    }

    fn constructed_field(&mut self, value: HirValueId, selector: CallField) -> Option<HirValueId> {
        let fields = self.constructed_fields.entry(value).or_insert_with(|| {
            let expressions = &self.checker.module.expressions;
            let HirExpressionKind::StructConstruct { fields, .. } =
                expressions.expression(value).kind
            else {
                unreachable!("only struct constructions have indexed fields");
            };
            let fields = expressions.struct_fields(fields);
            self.counts.reach_incidences += fields.len();
            fields
                .iter()
                .map(|&(field, value)| (self.field_ordinals[&field], value))
                .collect()
        });
        fields.get(&selector).copied()
    }

    fn visit_value(
        &mut self,
        value: HirValueId,
        tail: &[CallField],
        kind: DemandKind,
        node: usize,
    ) {
        let checker = self.checker;
        let expressions = &checker.module.expressions;
        match &expressions.expression(value).kind {
            HirExpressionKind::Load(place) => {
                let place = self.place_path(*place, tail);
                self.demand_place(place, kind, node);
            }
            HirExpressionKind::VariantPayloadGet {
                source,
                variant_index,
                field_index,
                ..
            } => {
                let selector = payload(*variant_index, *field_index);
                let path = join_selectors(&[selector], tail);
                self.demand_value_path(*source, path, kind, node);
            }
            kind_expression if kind_expression.is_aggregate_construction() => {
                let selected = tail.split_first();
                if kind.writes() && !matches!(kind, DemandKind::Reach(_)) && tail.len() == 1 {
                    // Replacing a fresh aggregate's own cell cannot write
                    // through the object previously stored in that cell.
                    return;
                }
                match (kind_expression, selected) {
                    (
                        HirExpressionKind::StructConstruct { .. },
                        Some((selector @ CallField::Field(_), tail)),
                    ) => {
                        if let Some(selected) = self.constructed_field(value, *selector) {
                            self.demand_value(selected, tail, kind, node);
                        }
                    }
                    (
                        HirExpressionKind::VariantConstruct {
                            fields,
                            variant_index,
                            ..
                        },
                        Some((CallField::Payload { variant, field }, tail)),
                    ) => {
                        if *variant as usize == *variant_index
                            && let Some(field) =
                                expressions.variant_fields(*fields).get(*field as usize)
                        {
                            self.demand_value(field.value, tail, kind, node);
                        }
                    }
                    (HirExpressionKind::Collection(values), Some((CallField::Elem, tail))) => {
                        for &value in expressions.values(*values) {
                            self.demand_value(value, tail, kind, node);
                        }
                    }
                    (
                        HirExpressionKind::MapLiteral(entries),
                        Some((selector @ (CallField::Elem | CallField::MapKey), tail)),
                    ) => {
                        for entry in expressions.map_entries(*entries) {
                            let value = if *selector == CallField::Elem {
                                entry.value
                            } else {
                                entry.key
                            };
                            self.demand_value(value, tail, kind, node);
                        }
                    }
                    _ => {
                        for child in expressions.child_values(value) {
                            self.demand_value(child, &[], DemandKind::Value, node);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn drain_demands(&mut self) {
        loop {
            while let Some((place, kind, node)) = self.work.pop_front() {
                self.counts.reverse_visits += 1;
                self.visit_place(place, kind, node);
            }
            let Some((value, path, kind, node)) = self.value_work.pop_front() else {
                break;
            };
            self.counts.reverse_visits += 1;
            self.visit_value(value, &path, kind, node);
        }
    }

    fn collect_roots(&mut self) {
        let expressions = &self.checker.module.expressions;
        for &(site, place, value) in &self.snapshot.writes {
            let node = self.site(site);
            let retargets = place.projections.is_empty()
                && value.is_some_and(|value| {
                    let expression = expressions.expression(value);
                    expression.value_kind == ValueKind::Place
                        && !HirWriteTarget::AssignPlace(place).is_direct_self_update_of(expression)
                });
            let kind = if retargets {
                DemandKind::Retarget
            } else {
                DemandKind::Cell
            };
            self.observe(Observation::Place(place, kind), node);
            if let Some(value) = value {
                self.observe(Observation::Value(value), node);
                if retargets
                    && let HirExpressionKind::Load(source) = expressions.expression(value).kind
                {
                    self.observe(
                        Observation::Place(source, DemandKind::Reach(ReachKind::Cell)),
                        node,
                    );
                }
            }
        }

        for &(site, op, receiver, args) in &self.snapshot.maps {
            if op.requires_mutable_receiver() || op == HirMapOp::Get {
                let node = self.site(site);
                let kind = if op.requires_mutable_receiver() {
                    ReachKind::EntryStructure
                } else {
                    ReachKind::Referent
                };
                self.observe(Observation::Member(receiver, DemandKind::Reach(kind)), node);
                if op == HirMapOp::Set {
                    for &argument in args {
                        self.observe(Observation::Value(argument), node);
                    }
                }
            }
        }

        let mut result_nodes = BTreeMap::new();
        for &(site, error, slot, value) in &self.snapshot.returns {
            let node = self.site(site);
            if let Some(&previous) = result_nodes.get(&(error, slot)) {
                self.couple(previous, node);
            } else {
                result_nodes.insert((error, slot), node);
            }
            self.observe(Observation::Value(value), node);
        }
    }

    fn collect_call_roots(&mut self) -> Result<(), BorrowCheckError> {
        let checker = self.checker;
        let expressions = &checker.module.expressions;
        let signatures = checker.call_signatures();
        for call in &self.snapshot.calls {
            self.call_contracts.push(signatures.resolve(
                call.target,
                call.args.len(),
                &checker.diagnostics,
            )?);
        }
        for (position, call) in self.snapshot.calls.iter().enumerate() {
            if let CallTarget::Local(target) = call.target {
                if let Some(callee) = self.callees.get(target) {
                    // Project an entire callee component into an actual support.
                    // Independently certified callee effects are not all joined
                    // just because they occur at one call site.
                    let mut nodes = FxHashMap::default();
                    for ((parameter, path, kind), &support) in &callee.envelope {
                        let Some(&actual) = call.args.get(*parameter) else {
                            return Err(self
                                .checker
                                .diagnostics
                                .internal_error("Sparse call argument is missing"));
                        };
                        let node = *nodes.entry(support).or_insert_with(|| self.site(call.site));
                        self.demand_value(actual, path, *kind, node);
                    }
                }
                continue;
            }

            let contract = self.call_contracts[position];
            let is_mutable = |index| contract.is_mutable(index);
            if (0..call.args.len()).any(is_mutable) {
                let node = self.site(call.site);
                for (index, &actual) in call.args.iter().enumerate() {
                    if is_mutable(index) {
                        self.demand_value(actual, &[], DemandKind::Retarget, node);
                        if is_heap(self.checker, expressions.expression(actual).ty)? {
                            self.demand_value(
                                actual,
                                &[],
                                DemandKind::Reach(ReachKind::EntryStructure),
                                node,
                            );
                        }
                    }
                    self.demand_value(actual, &[], DemandKind::Value, node);
                }
            }
        }
        Ok(())
    }

    fn observe(&mut self, observation: Observation, node: usize) {
        match observation {
            Observation::Place(place, kind) => {
                let place = self.place_path(place, &[]);
                self.demand_place(place, kind, node);
            }
            Observation::Value(value) => self.demand_value(value, &[], DemandKind::Value, node),
            Observation::Member(receiver, kind) => self.demand_value(receiver, &[], kind, node),
        }
    }

    // These incidences conservatively couple supports. Admitting a resolved
    // presentation uses the transfer's typed footprint overlap, not these buckets.
    fn connect_compatibility(&mut self) -> (Vec<Predicate>, Vec<CompatibilityConstraint>) {
        let mut guards = std::mem::take(&mut self.demanded_roles);
        let mut broad = BTreeSet::new();
        for (ty, mut terminals) in std::mem::take(&mut self.terminals_by_type) {
            if let Some(edges) = self.edges_by_value_type.get(&ty) {
                for (&edge, incidences) in edges {
                    terminals.insert(edge);
                    for &site in incidences {
                        self.counts.support_candidates += 1;
                        self.supports.couple(self.port_nodes[edge.0 as usize], site);
                    }
                }
            }
            self.supports.connect_equality(
                IdentityKind::Cell,
                &terminals,
                &self.port_nodes,
                &mut self.counts,
                &mut guards,
                &mut broad,
            );
        }
        for wrappers in self.wrappers_by_type.values() {
            self.supports.connect_equality(
                IdentityKind::Wrapper,
                wrappers,
                &self.port_nodes,
                &mut self.counts,
                &mut guards,
                &mut broad,
            );
        }
        for key in &self.written_selectors {
            let holders = &self.selector_index[key];
            let mut candidates = holders.iter();
            let Some((&first, first_sites)) = candidates.next() else {
                continue;
            };
            let first_site = first_sites[0];
            // An exact selector shared by reads and writes is also a
            // dependency when there is just one observed holder.
            for &site in &first_sites[1..] {
                self.counts.support_candidates += 1;
                self.supports.couple(first_site, site);
            }
            if holders.len() == 1 {
                continue;
            }
            let first_holder = self.port_nodes[first.0 as usize];
            self.counts.support_candidates += 1;
            self.supports.couple(first_holder, first_site);
            for (&port, sites) in candidates {
                self.counts.compatibility_candidates += 1;
                let holder_node = self.port_nodes[port.0 as usize];
                self.counts.support_candidates += 1;
                self.supports.couple(first_holder, holder_node);
                for &site in sites {
                    self.counts.support_candidates += 1;
                    self.supports.couple(holder_node, site);
                }
            }
            if holders.len() == 2 {
                let last = *holders.last_key_value().expect("two indexed holders").0;
                guards.insert(Predicate::Equal(IdentityKind::Allocation, first, last));
            } else {
                broad.insert(CompatibilityConstraint::Equality(
                    IdentityKind::Allocation,
                    holders.keys().copied().collect(),
                ));
            }
        }
        let mut remaining = Vec::new();
        for (ty, mut compatible) in std::mem::take(&mut self.wide_reach) {
            // An unknown write can affect a reached child as well as its root.
            // One constraint represents the compatible set without pairing
            // the roots or building a concrete occurrence graph.
            let mut seen = FxHashSet::default();
            remaining.push(ty);
            while let Some(ty) = remaining.pop() {
                self.counts.reach_incidences += 1;
                if !seen.insert(ty) {
                    continue;
                }
                if let Some(holders) = self.holders_by_type.get(&ty) {
                    self.counts.reach_incidences += holders.len();
                    for (&holder, sites) in holders {
                        compatible.insert(holder);
                        for &site in sites {
                            self.counts.support_candidates += 1;
                            self.supports
                                .couple(self.port_nodes[holder.0 as usize], site);
                        }
                    }
                }
                let result: Result<(), std::convert::Infallible> =
                    CallField::for_each_child(ty, self.checker.type_environment, |_, child| {
                        remaining.push(child);
                        Ok(())
                    });
                match result {
                    Ok(()) => {}
                    Err(never) => match never {},
                }
            }
            if let Some(first) = compatible.first() {
                for port in compatible.iter().skip(1) {
                    self.counts.compatibility_candidates += 1;
                    self.counts.support_candidates += 1;
                    self.supports.couple(
                        self.port_nodes[first.0 as usize],
                        self.port_nodes[port.0 as usize],
                    );
                }
                if compatible.len() > 1 {
                    broad.insert(CompatibilityConstraint::Reach(
                        compatible.into_iter().collect(),
                    ));
                }
            }
        }
        (guards.into_iter().collect(), broad.into_iter().collect())
    }

    fn connect_local_accesses(&mut self) {
        // A local can carry a fresh component that has no entry port. A
        // whole-value observation must still be coupled to writes beneath it.
        for ((place, demand), &node) in &self.local_nodes {
            let related = if demand.writes() {
                [
                    DemandKind::Value,
                    DemandKind::Reach(ReachKind::Referent),
                    DemandKind::Reach(ReachKind::EntryStructure),
                ]
            } else {
                [
                    DemandKind::Cell,
                    DemandKind::Retarget,
                    DemandKind::Reach(ReachKind::EntryStructure),
                ]
            };
            for end in 0..=place.path.len() {
                let mut key = (
                    IndexedPlace {
                        local: place.local,
                        path: place.path[..end].into(),
                    },
                    *demand,
                );
                for demand in related {
                    key.1 = demand;
                    self.counts.access_incidences += 1;
                    if let Some(&dependent) = self.local_nodes.get(&key) {
                        self.counts.support_candidates += 1;
                        self.supports.couple(node, dependent);
                    }
                }
            }
        }
    }

    fn collect_result_sources(&mut self) -> BTreeMap<(bool, usize), ResultSources> {
        let checker = self.checker;
        let expressions = &checker.module.expressions;
        let mut sources = BTreeMap::<_, ResultSources>::new();
        let mut visited = FxHashSet::default();
        let mut work = Vec::new();
        for &(_, error, slot, returned) in &self.snapshot.returns {
            let result_type = expressions.expression(returned).ty;
            work.push((
                ResultDemand::Value(returned, Box::new([])),
                Box::<[CallField]>::default(),
            ));
            while let Some((demand, output)) = work.pop() {
                self.counts.reverse_visits += 1;
                let (input_type, input) = match &demand {
                    ResultDemand::Value(value, input) => (expressions.expression(*value).ty, input),
                    ResultDemand::Place(place) => {
                        (self.checker.local_types[&place.local], &place.path)
                    }
                };
                if self.finite_prefix(input_type, input) != input.len()
                    || self.finite_prefix(result_type, &output) != output.len()
                    || !visited.insert(((error, slot), demand.clone(), output.clone()))
                {
                    continue;
                }
                match demand {
                    ResultDemand::Place(place) => {
                        if let Some(&parameter) = self.parameter_indices.get(&place.local) {
                            sources
                                .entry((error, slot))
                                .or_default()
                                .0
                                .entry(output.clone())
                                .or_default()
                                .insert(ResultSource {
                                    parameter,
                                    input: place.path.clone(),
                                });
                        }
                        for &(_, input) in self
                            .snapshot
                            .definitions
                            .get(&place.local)
                            .into_iter()
                            .flatten()
                        {
                            self.counts.reverse_visits += 1;
                            let actual = match input {
                                Input::Value(value) => {
                                    ResultDemand::Value(value, place.path.clone())
                                }
                                Input::Jump(local) => ResultDemand::Place(IndexedPlace {
                                    local,
                                    path: place.path.clone(),
                                }),
                                Input::Member(receiver) => ResultDemand::Value(
                                    receiver,
                                    join_selectors(&[CallField::Elem], &place.path),
                                ),
                                Input::CallResult(position, slot) => {
                                    let call = &self.snapshot.calls[position];
                                    if slot == Some(0)
                                        && let Some(receiver) = call_member_result(call.target)
                                    {
                                        work.push((
                                            ResultDemand::Value(
                                                call.args[receiver],
                                                join_selectors(&[CallField::Elem], &place.path),
                                            ),
                                            output.clone(),
                                        ));
                                    } else if let CallTarget::Local(target) = call.target
                                        && let Some(rows) =
                                            self.callees.get(target).and_then(|callee| {
                                                callee.result_sources.get(&outcome_slot(slot))
                                            })
                                    {
                                        self.counts.reverse_visits += place.path.len() + 1;
                                        for (source, output_tail, input_tail) in
                                            rows.select(&place.path)
                                        {
                                            self.counts.reverse_visits += 1;
                                            let actual = ResultDemand::Value(
                                                call.args[source.parameter],
                                                join_selectors(&source.input, input_tail),
                                            );
                                            work.push((
                                                actual,
                                                join_selectors(&output, output_tail),
                                            ));
                                        }
                                    }
                                    continue;
                                }
                            };
                            work.push((actual, output.clone()));
                        }
                    }
                    ResultDemand::Value(value, input) => {
                        let mut field = |selector, child| {
                            if let Some((&selected, tail)) = input.split_first() {
                                if selected == selector {
                                    work.push((
                                        ResultDemand::Value(child, tail.into()),
                                        output.clone(),
                                    ));
                                }
                            } else {
                                let output = join_selectors(&output, &[selector]);
                                work.push((ResultDemand::Value(child, Box::new([])), output));
                            }
                        };
                        match &expressions.expression(value).kind {
                            HirExpressionKind::Load(place) => {
                                work.push((
                                    ResultDemand::Place(self.place_path(*place, &input)),
                                    output,
                                ));
                            }
                            HirExpressionKind::VariantPayloadGet {
                                source,
                                variant_index,
                                field_index,
                                ..
                            } => {
                                let selector = payload(*variant_index, *field_index);
                                work.push((
                                    ResultDemand::Value(
                                        *source,
                                        join_selectors(&[selector], &input),
                                    ),
                                    output,
                                ));
                            }
                            HirExpressionKind::StructConstruct { fields, .. } => {
                                if let Some((&selector, _)) = input.split_first() {
                                    if let Some(child) = self.constructed_field(value, selector) {
                                        field(selector, child);
                                    }
                                } else {
                                    let fields = expressions.struct_fields(*fields);
                                    self.counts.selector_incidences += fields.len();
                                    for &(id, child) in fields {
                                        field(self.field_ordinals[&id], child);
                                    }
                                }
                            }
                            HirExpressionKind::VariantConstruct {
                                fields,
                                variant_index,
                                ..
                            } => {
                                for (index, member) in
                                    expressions.variant_fields(*fields).iter().enumerate()
                                {
                                    field(payload(*variant_index, index), member.value);
                                }
                            }
                            HirExpressionKind::Collection(values) => {
                                for &value in expressions.values(*values) {
                                    field(CallField::Elem, value);
                                }
                            }
                            HirExpressionKind::MapLiteral(entries) => {
                                for entry in expressions.map_entries(*entries) {
                                    field(CallField::MapKey, entry.key);
                                    field(CallField::Elem, entry.value);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        sources
    }

    fn finite_prefix(&mut self, mut ty: TypeId, path: &[CallField]) -> usize {
        self.path_ancestors.clear();
        for (index, &selector) in path.iter().enumerate() {
            if self.path_ancestors.contains(&ty) {
                return index;
            }
            let Some(child) = selector.type_child(ty, self.checker.type_environment) else {
                return index;
            };
            if index + 1 != path.len() {
                self.path_ancestors.push(ty);
            }
            ty = child;
        }
        path.len()
    }

    fn seal(mut self, transfer: Rc<SparseTransferIndex>) -> SparseFunction {
        let result_sources = self.collect_result_sources();
        self.drain_demands();
        let (conditional, conditional_arm_count) = self.connect_control();
        self.drain_demands();
        self.connect_local_accesses();
        let (guards, compatibility) = self.connect_compatibility();
        let mut support_indices = BTreeMap::new();
        let mut support_of = |node| {
            let root = self.supports.root(node);
            let next =
                SupportId(u32::try_from(support_indices.len()).expect("HIR support capacity"));
            *support_indices.entry(root).or_insert(next)
        };
        let envelope = self
            .envelope
            .into_iter()
            .map(|(key, node)| (key, support_of(node)))
            .collect();
        let port_supports = self
            .port_nodes
            .iter()
            .copied()
            .map(&mut support_of)
            .collect();
        let mut support_positions = BTreeMap::<_, Vec<_>>::new();
        #[cfg(test)]
        let mut operations = BTreeMap::new();
        for (site, nodes) in self.sites {
            let mut supports: Vec<_> = nodes.into_iter().map(&mut support_of).collect();
            supports.sort_unstable();
            supports.dedup();
            for &support in &supports {
                support_positions
                    .entry((support, BlockId(site.0)))
                    .or_default()
                    .push(site.1);
            }
            #[cfg(test)]
            operations.insert(site, supports.into_boxed_slice());
        }
        let conditional_supports = conditional.into_iter().map(&mut support_of).collect();

        SparseFunction {
            ports: self.ports,
            port_supports,
            guards,
            compatibility,
            #[cfg(test)]
            operations,
            operations_by_support: support_positions
                .into_iter()
                .map(|(key, positions)| (key, positions.into_boxed_slice()))
                .collect(),
            conditional_supports,
            conditional_arm_count,
            counts: self.counts,
            transfer,
            envelope,
            result_sources,
        }
    }

    fn connect_control(&mut self) -> (Vec<usize>, usize) {
        let cfg = SparseCfg::new(self.checker, self.snapshot, &mut self.counts.reverse_visits);
        if cfg.may_recur(&mut self.counts.reverse_visits) {
            // Iterations can share a trip count or skip a reconvergence.
            // Couple their state until there is a local recurrence proof.
            let mut sites = self.sites.values().flatten().copied();
            if let Some(first) = sites.next() {
                for site in sites {
                    self.counts.support_candidates += 1;
                    self.supports.couple(first, site);
                }
                return (vec![first], 0);
            }
            return (Vec::new(), 0);
        }
        let joins = cfg.postdominators(&mut self.counts.reverse_visits);
        let mut conditional = Vec::new();
        let mut arms = 0;
        let mut reuse = FxHashMap::default();
        let mut seen = vec![0; cfg.successors.len()];
        let mut joined_to_exit = None;
        let mut work = Vec::new();
        let mut affected = Vec::new();
        for (branch, successors) in cfg.successors.iter().enumerate() {
            if successors.len() < 2 && !cfg.unproved_continuation[branch] {
                continue;
            }
            let join = joins[branch];
            let block = self.snapshot.blocks[branch];
            let term =
                &self.checker.module.blocks[self.checker.block_index_by_id[&block]].terminator;
            let independent_condition = match term {
                HirTerminator::If { condition, .. } => {
                    let expression = self.checker.module.expressions.expression(*condition);
                    matches!(expression.kind, HirExpressionKind::Load(place)
                    if place.projections.is_empty()
                        && self.parameter_indices.get(&place.root).is_some_and(|&index| {
                            self.checker.public_call_summaries[&self.snapshot.function.id]
                                .parameters[index].access == PublicCallParameterAccess::Shared
                        }))
                }
                _ => false,
            };
            affected.clear();
            let stamp = branch + 1;
            if join == cfg.exit || !independent_condition {
                // An early stop or an unmodelled choice can couple effects
                // on both sides of this region, even across its CFG join.
                if let Some(first) = joined_to_exit {
                    affected.push(first);
                } else {
                    affected.extend(self.sites.values().flatten().copied());
                    joined_to_exit = affected.first().copied();
                }
            } else {
                work.extend_from_slice(successors);
                while let Some(next) = work.pop() {
                    self.counts.reverse_visits += 1;
                    if next == join
                        || next == cfg.exit
                        || std::mem::replace(&mut seen[next], stamp) == stamp
                    {
                        continue;
                    }
                    let block = self.snapshot.blocks[next];
                    affected.extend(
                        self.sites
                            .range((block.0, 0)..=(block.0, usize::MAX))
                            .flat_map(|(_, nodes)| nodes.iter().copied()),
                    );
                    work.extend_from_slice(&cfg.successors[next]);
                }
            }
            let Some(&first) = affected.first() else {
                continue;
            };
            conditional.push(first);
            arms += successors.len();
            for &node in &affected[1..] {
                self.couple(first, node);
            }
            if let HirTerminator::If { condition, .. } = term {
                self.demand_value(*condition, &[], DemandKind::Value, first);
                let expression = self.checker.module.expressions.expression(*condition);
                if let HirExpressionKind::Load(place) = expression.kind {
                    let key = self.place_path(place, &[]);
                    if let Some(previous) = reuse.insert(key, first) {
                        self.couple(previous, first);
                    }
                }
            }
        }
        (conditional, arms)
    }
}

fn outcome_slot(slot: Option<usize>) -> (bool, usize) {
    (slot.is_none(), slot.unwrap_or(0))
}

fn join_selectors(prefix: &[CallField], tail: &[CallField]) -> Box<[CallField]> {
    [prefix, tail].concat().into_boxed_slice()
}

struct SparseCfg {
    successors: Vec<Vec<usize>>,
    predecessors: Vec<Vec<usize>>,
    unproved_continuation: Vec<bool>,
    exit: usize,
}

impl SparseCfg {
    fn new(checker: &BorrowChecker<'_>, snapshot: &Snapshot<'_>, visits: &mut usize) -> Self {
        let blocks = &snapshot.blocks;
        let indices: FxHashMap<_, _> = blocks.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        let exit = blocks.len();
        let mut successors = vec![Vec::new(); exit + 1];
        let mut predecessors = vec![Vec::new(); exit + 1];
        let mut unproved_continuation = vec![false; exit + 1];
        for call in &snapshot.calls {
            *visits += 1;
            if matches!(
                call.continuation,
                CallContinuation::Statement | CallContinuation::Invoke
            ) {
                // The HIR continuation is executable only if the call returns.
                // Keep the stop edge structural, independent of delivered summaries.
                unproved_continuation[indices[&BlockId(call.site.0)]] = true;
            }
        }
        for (i, &id) in blocks.iter().enumerate() {
            let block = &checker.module.blocks[checker.block_index_by_id[&id]];
            for_each_terminator_target(&block.terminator, |target| {
                successors[i].push(indices[&target])
            });
            if successors[i].is_empty() || unproved_continuation[i] {
                successors[i].push(exit);
            }
            for &j in &successors[i] {
                predecessors[j].push(i);
            }
        }
        let mut reaches_exit = vec![false; exit + 1];
        let mut work = vec![exit];
        while let Some(block) = work.pop() {
            *visits += 1;
            if !std::mem::replace(&mut reaches_exit[block], true) {
                work.extend_from_slice(&predecessors[block]);
            }
        }
        // A closed recurrent region is another possible stop, not proof that
        // all paths through it reach an unrelated continuation.
        for i in 0..exit {
            if !reaches_exit[i] {
                successors[i].push(exit);
                predecessors[exit].push(i);
            }
        }
        Self {
            successors,
            predecessors,
            unproved_continuation,
            exit,
        }
    }

    fn may_recur(&self, visits: &mut usize) -> bool {
        let mut inbound: Vec<_> = self.predecessors.iter().map(Vec::len).collect();
        let mut ready: Vec<_> = inbound
            .iter()
            .enumerate()
            .filter_map(|(node, &count)| (count == 0).then_some(node))
            .collect();
        let mut visited = 0;
        while let Some(node) = ready.pop() {
            *visits += 1;
            visited += 1;
            for &next in &self.successors[node] {
                *visits += 1;
                inbound[next] -= 1;
                if inbound[next] == 0 {
                    ready.push(next);
                }
            }
        }
        visited != self.successors.len()
    }

    fn postdominators(&self, visits: &mut usize) -> Vec<usize> {
        let mut visited = vec![false; self.exit + 1];
        let mut order = Vec::new();
        let mut stack = vec![(self.exit, false)];
        while let Some((block, finished)) = stack.pop() {
            *visits += 1;
            if finished {
                order.push(block);
            } else if !std::mem::replace(&mut visited[block], true) {
                stack.push((block, true));
                *visits += self.predecessors[block].len();
                stack.extend(
                    self.predecessors[block]
                        .iter()
                        .rev()
                        .map(|&predecessor| (predecessor, false)),
                );
            }
        }
        let mut ranks = vec![0; self.exit + 1];
        for (rank, &block) in order.iter().rev().enumerate() {
            ranks[block] = rank;
        }
        let mut parent = vec![usize::MAX; self.exit + 1];
        parent[self.exit] = self.exit;
        loop {
            let mut changed = false;
            for &block in order.iter().rev().skip(1) {
                *visits += 1;
                *visits += self.successors[block].len();
                let mut known = self.successors[block]
                    .iter()
                    .copied()
                    .filter(|&next| parent[next] != usize::MAX);
                let Some(mut common) = known.next() else {
                    continue;
                };
                for mut other in known {
                    while common != other {
                        *visits += 1;
                        if ranks[common] > ranks[other] {
                            common = parent[common];
                        } else {
                            other = parent[other];
                        }
                    }
                }
                changed |= parent[block] != common;
                parent[block] = common;
            }
            if !changed {
                return parent;
            }
        }
    }
}

impl IndexedFunction {
    #[allow(dead_code)] // Sparse extraction supplies this view without constructing dense validation layout.
    pub(super) fn build_sparse(
        checker: &BorrowChecker<'_>,
    ) -> Result<FxHashMap<FunctionId, SparseFunction>, BorrowCheckError> {
        let mut fields = FxHashMap::default();
        for layout in &checker.module.structs {
            for (position, field) in layout.fields.iter().enumerate() {
                fields.insert(field.id, CallField::Field(position as u32));
            }
        }
        let snapshots = checker
            .module
            .functions
            .iter()
            .map(|function| Snapshot::collect(checker, function))
            .collect::<Result<Vec<_>, _>>()?;
        let transfer_indices = snapshots
            .iter()
            .map(|snapshot| SparseTransferIndex::new(checker, snapshot).map(Rc::new))
            .collect::<Result<Vec<_>, _>>()?;
        let mut callers = FxHashMap::<FunctionId, Vec<usize>>::default();
        for (caller, snapshot) in snapshots.iter().enumerate() {
            for call in &snapshot.calls {
                if let CallTarget::Local(target) = call.target {
                    callers.entry(*target).or_default().push(caller);
                }
            }
        }
        // Acyclic callees supply their structural envelope before any caller
        // reads it. Only cycles need provisional rows and revisits.
        let positions: FxHashSet<_> = snapshots
            .iter()
            .map(|snapshot| snapshot.function.id)
            .collect();
        let mut unscheduled_dependencies = vec![0; snapshots.len()];
        for (&callee, edges) in &callers {
            if positions.contains(&callee) {
                for &caller in edges {
                    unscheduled_dependencies[caller] += 1;
                }
            }
        }
        let mut ready: VecDeque<_> = unscheduled_dependencies
            .iter()
            .enumerate()
            .filter_map(|(position, &count)| (count == 0).then_some(position))
            .collect();
        let mut work = VecDeque::with_capacity(snapshots.len());
        while let Some(position) = ready.pop_front() {
            work.push_back(position);
            if let Some(caller_edges) = callers.get(&snapshots[position].function.id) {
                for &caller in caller_edges {
                    unscheduled_dependencies[caller] -= 1;
                    if unscheduled_dependencies[caller] == 0 {
                        ready.push_back(caller);
                    }
                }
            }
        }
        work.extend(
            unscheduled_dependencies
                .iter()
                .enumerate()
                .filter_map(|(position, &count)| (count != 0).then_some(position)),
        );
        let mut pending = vec![true; snapshots.len()];
        let mut indices = FxHashMap::<FunctionId, SparseFunction>::default();
        while let Some(position) = work.pop_front() {
            pending[position] = false;
            let snapshot = &snapshots[position];
            let mut builder = DemandBuilder::new(checker, snapshot, &fields, &indices);
            builder.collect_roots();
            builder.collect_call_roots()?;
            let mut index = builder.seal(Rc::clone(&transfer_indices[position]));
            index.counts.record(index.ports.len());
            let previous = indices.get(&snapshot.function.id);
            if let Some(old) = previous {
                // Work evidence includes the previous visits, not just the
                // final reduced graph.
                index.counts += old.counts;
            } else {
                index.counts += snapshot.counts;
            }
            if previous.is_none_or(|old| {
                old.envelope != index.envelope || old.result_sources != index.result_sources
            }) && let Some(callers) = callers.get(&snapshot.function.id)
            {
                for &caller in callers {
                    if !std::mem::replace(&mut pending[caller], true) {
                        work.push_back(caller);
                    }
                }
            }
            indices.insert(snapshot.function.id, index);
        }
        Ok(indices)
    }
}

#[cfg(test)]
fn build_sparse_index_for_test(
    checker: &mut BorrowChecker<'_>,
) -> Result<FxHashMap<FunctionId, SparseFunction>, BorrowCheckError> {
    checker.public_call_summaries =
        BorrowChecker::seed_public_call_summaries(checker.module, &checker.diagnostics)?;
    IndexedFunction::build_sparse(checker)
}

// Only retire named bindings proven dead without a CFG liveness walk. Every reference must
// follow a definition in the same block, so a backedge also redefines before the next use.
struct LocalUses {
    defining_block: Option<BlockId>,
    last: usize,
}

impl LocalUses {
    fn record(uses: &mut FxHashMap<LocalId, Self>, local: LocalId, block: BlockId, index: usize) {
        let uses = uses.entry(local).or_insert(Self {
            defining_block: None,
            last: index,
        });
        if uses.defining_block == Some(block) {
            uses.last = index;
        } else {
            uses.defining_block = None;
        }
    }
}

pub(super) struct FunctionEvidence<'a, 'module> {
    pub(super) checker: &'a BorrowChecker<'module>,
    pub(super) function: &'a HirFunction,
    pub(super) index: &'a IndexedFunction,
}

impl FunctionEvidence<'_, '_> {
    pub(super) fn evaluate(&self) -> Result<CallAllocationFacts, BorrowCheckError> {
        if self.function.return_contract.is_never() {
            return Ok(CallAllocationFacts::default());
        }
        let entry = self.function.entry;
        let mut initial = Graph::default();
        let mut rows = Vec::new();
        if self.index.tracks_values {
            for (index, &local) in self.function.params.iter().enumerate() {
                self.typed_paths(self.ty(local), |path, ty| {
                    let parameter = CallParameterPath {
                        index,
                        path: path.to_vec(),
                    };
                    rows.push(Spec::born(
                        Self::local(local).at(path),
                        ty,
                        Origin::Parameter(parameter),
                    ));
                })?;
            }
        }
        initial.install(rows, &[], None);
        let mut inputs = FxHashMap::default();
        inputs.insert(entry, initial);
        let mut queue = VecDeque::from([entry]);
        let mut queued = rustc_hash::FxHashSet::default();
        queued.insert(entry);
        let mut success = Lane::default();
        let mut error = Lane::default();
        'worklist: while let Some(block_id) = queue.pop_front() {
            queued.remove(&block_id);
            let block = self
                .checker
                .block_by_id_or_error(block_id, self.function.id)?;
            let mut state = inputs[&block_id].clone();
            let mut retiring = self
                .index
                .retirements
                .get(&block_id)
                .into_iter()
                .flatten()
                .peekable();
            let mut protected = Vec::new();
            for (index, statement) in block.statements.iter().enumerate() {
                if !self.statement(
                    &mut state,
                    Site::Operation(block_id.0, index),
                    &statement.kind,
                )? {
                    continue 'worklist;
                }
                state.retire_temporaries();
                while let Some(&(_, local)) = retiring.next_if(|&&(last, _)| last == index) {
                    if !state.retire_local(local.0) {
                        protected.push(local.0);
                    }
                }
                protected.retain(|&local| !state.retire_local(local));
            }
            let mut edge = |target, mut state: Graph| {
                state.retire_temporaries();
                use std::collections::hash_map::Entry;
                let changed = match inputs.entry(target) {
                    Entry::Occupied(mut input) => input.get_mut().join(state),
                    Entry::Vacant(input) => {
                        input.insert(state);
                        true
                    }
                };
                if changed && queued.insert(target) {
                    queue.push_back(target);
                }
            };
            match &block.terminator {
                HirTerminator::Return(values) => {
                    let values = self.checker.module.expressions.values(*values);
                    let results = values
                        .iter()
                        .map(|&value| self.value(&mut state, value))
                        .collect::<Result<Vec<_>, _>>()?;
                    success.extend(&state, &results);
                }
                HirTerminator::ReturnError(value) => {
                    let result = self.value(&mut state, *value)?;
                    error.extend(&state, &[result]);
                }
                HirTerminator::Jump { target, args } => {
                    let mut rows = Vec::new();
                    let mut clear = Vec::with_capacity(args.len());
                    for arg in args {
                        let destination = Self::local(arg.destination);
                        state.snapshot(&destination, &Self::local(arg.source), &mut rows);
                        clear.push(destination);
                        state.aliases.remove(&arg.destination.0);
                    }
                    state.install(rows, &clear, None);
                    edge(*target, state);
                }
                HirTerminator::Invoke {
                    operation,
                    success,
                    success_results,
                    failure,
                    error_result,
                    ..
                } => {
                    let site = Site::Operation(block_id.0, block.statements.len());
                    let mut normal = state.clone();
                    // Evaluate operands against the same predecessor. Retire the unselected
                    // slots after capture; selected definitions replace their own prior bindings.
                    if self.invoke(&mut normal, site, operation, false, success_results)? {
                        self.kill_local(&mut normal, *error_result);
                        edge(*success, normal);
                    }
                    if self.invoke(
                        &mut state,
                        site,
                        operation,
                        true,
                        std::slice::from_ref(error_result),
                    )? {
                        for &local in success_results {
                            self.kill_local(&mut state, local);
                        }
                        edge(*failure, state);
                    }
                }
                HirTerminator::Break { target } | HirTerminator::Continue { target } => {
                    edge(*target, state)
                }
                terminator => {
                    for_each_terminator_target(terminator, |target| edge(target, state.clone()))
                }
            }
        }
        let mut success = success.finish();
        let mut error = error.finish();
        let needs_success = !success.reachable;
        let needs_error = !error.reachable
            && self
                .function
                .return_contract
                .ordinary()
                .is_some_and(|shape| shape.is_fallible());
        if needs_success || needs_error {
            let (may_succeed, may_fail) = self.structural_returns(needs_success, needs_error)?;
            success.structural_only = needs_success && may_succeed;
            error.structural_only = needs_error && may_fail;
        }
        Ok(CallAllocationFacts { success, error })
    }

    /// Find permitted exits without treating absent allocation evidence as a control-flow
    /// predicate. Only needed for lanes the graph transfer did not reach. Calls use the same
    /// least fixed point, but read the structural return bit rather than the graph bottom.
    fn structural_returns(
        &self,
        needs_success: bool,
        needs_error: bool,
    ) -> Result<(bool, bool), BorrowCheckError> {
        let can_return = |target: &CallTarget, error: bool| match target {
            CallTarget::External(_) => true,
            _ => matches!(
                self.call_summary(target).map(|summary| &summary.return_alias),
                Some(FunctionReturnAliasSummary::Known(facts))
                    if (if error { &facts.error } else { &facts.success }).can_return()
            ),
        };
        let entry = self.function.entry;
        let mut seen = rustc_hash::FxHashSet::default();
        seen.insert(entry);
        let mut worklist = vec![entry];
        let (mut success, mut error) = (false, false);
        while let Some(id) = worklist.pop() {
            let block = self.checker.block_by_id_or_error(id, self.function.id)?;
            if block.statements.iter().any(|statement| {
                matches!(&statement.kind, HirStatementKind::Call { target, .. } if !can_return(target, false))
            }) {
                continue;
            }
            let mut push = |id| {
                if seen.insert(id) {
                    worklist.push(id);
                }
            };
            match &block.terminator {
                HirTerminator::Return(_) => success = true,
                HirTerminator::ReturnError(_) => error = true,
                HirTerminator::Invoke {
                    operation: HirFallibleOperation::Call { target, .. },
                    success,
                    failure,
                    ..
                } => {
                    if can_return(target, false) {
                        push(*success);
                    }
                    if can_return(target, true) {
                        push(*failure);
                    }
                }
                terminator => for_each_terminator_target(terminator, push),
            }
            if (success || !needs_success) && (error || !needs_error) {
                break;
            }
        }
        Ok((success, error))
    }

    fn statement(
        &self,
        state: &mut Graph,
        site: Site,
        statement: &HirStatementKind,
    ) -> Result<bool, BorrowCheckError> {
        if !self.index.tracks_values {
            return match statement {
                HirStatementKind::Call {
                    target,
                    args,
                    results,
                } => self.call(state, site, target, *args, false, results),
                _ => Ok(true),
            };
        }
        match statement {
            HirStatementKind::Write { target, value } => match target {
                HirWriteTarget::DefineLocal(local) => {
                    let alias = self.acquired_alias(*local, *value)?;
                    let captured = self.value(state, *value)?;
                    self.define(state, *local, &captured, alias.map(|alias| (site, alias)));
                }
                HirWriteTarget::AssignPlace(place) => {
                    // A direct self-update adds no alias and keeps the binding role, as in live
                    // transfer.
                    if target.is_direct_self_update_of(
                        self.checker.module.expressions.expression(*value),
                    ) {
                        return Ok(true);
                    }
                    let target = self.place(place)?;
                    let alias = if target.path.is_empty() {
                        self.acquired_alias(place.root, *value)?
                    } else {
                        None
                    };
                    let value = self.value(state, *value)?;
                    let Some(alias) = alias else {
                        return Ok(self.assign(state, &target, &value, false));
                    };
                    // The plain-slot alternative acquires the place as its alias, as at a
                    // definition; each existing alias alternative is written through instead.
                    let Some(alternatives) = state.aliases.get(&place.root.0) else {
                        self.define(state, place.root, &value, Some((site, alias)));
                        return Ok(true);
                    };
                    if !alternatives.contains(&target) {
                        return Ok(self.assign(state, &target, &value, false));
                    }
                    let mut aliased = state.clone();
                    if let Some(alternatives) = aliased.aliases.get_mut(&place.root.0) {
                        alternatives.remove(&target);
                    }
                    // The alternatives are joined possibilities: one whose holder no allocation
                    // reaches yet adds nothing, and widening that resolves it only adds facts.
                    let aliased = self
                        .assign(&mut aliased, &target, &value, false)
                        .then_some(aliased);
                    self.define(state, place.root, &value, Some((site, alias)));
                    if let Some(aliased) = aliased {
                        state.join(aliased);
                    }
                }
            },
            HirStatementKind::Call {
                target,
                args,
                results,
            } => {
                return self.call(state, site, target, *args, false, results);
            }
            HirStatementKind::MapOp {
                op,
                receiver,
                args,
                result,
            } => {
                return self.map(state, site, *op, *receiver, *args, result.as_slice());
            }
            HirStatementKind::FormatFloat { result, .. } => {
                let value = self.fresh_operation(state, site, 0, self.ty(result.local()))?;
                return Ok(self.bind(state, *result, &value));
            }
            HirStatementKind::PushRuntimeFragment { vec_local, value } => {
                let value = self.value(state, *value)?;
                return Ok(self.store(
                    state,
                    &Self::local(*vec_local),
                    CallField::Elem,
                    &value,
                    true,
                ));
            }
            HirStatementKind::NumericOp { .. }
            | HirStatementKind::FastStoreConversion { .. }
            | HirStatementKind::LoopStepFailure { .. }
            | HirStatementKind::RangeCandidate { .. }
            | HirStatementKind::ValidateFloat { .. }
            | HirStatementKind::Expr(_) => {}
        }
        Ok(true)
    }

    fn invoke(
        &self,
        state: &mut Graph,
        site: Site,
        op: &HirFallibleOperation,
        error: bool,
        locals: &[LocalId],
    ) -> Result<bool, BorrowCheckError> {
        if !self.index.tracks_values {
            return match op {
                HirFallibleOperation::Call { target, args } => {
                    self.call(state, site, target, *args, error, &[])
                }
                _ => Ok(true),
            };
        }
        let destinations = locals
            .iter()
            .copied()
            .map(HirLocalDestination::Define)
            .collect::<Vec<_>>();
        match op {
            HirFallibleOperation::Call { target, args } => {
                self.call(state, site, target, *args, error, &destinations)
            }
            HirFallibleOperation::MapOp { op, receiver, args } if !error => {
                self.map(state, site, *op, *receiver, *args, &destinations)
            }
            _ => {
                let mut reached = true;
                for (slot, &destination) in destinations.iter().enumerate() {
                    let ty = self.ty(destination.local());
                    let value = if error {
                        self.fresh_builtin_error(state, site, slot, ty)?
                    } else {
                        self.fresh_operation(state, site, slot, ty)?
                    };
                    reached &= self.bind(state, destination, &value);
                }
                Ok(reached)
            }
        }
    }

    fn value(&self, state: &mut Graph, value: HirValueId) -> Result<Cell, BorrowCheckError> {
        let expressions = &self.checker.module.expressions;
        let expression = expressions.expression(value);
        if !self.heap(expression.ty)? {
            return Ok(state.temporary());
        }
        let site = Site::Value(value.0);
        match &expression.kind {
            HirExpressionKind::Load(place) if expression.value_kind != ValueKind::Const => {
                let source = self.place(place)?;
                Ok(state.capture(&source))
            }
            HirExpressionKind::VariantPayloadGet {
                source,
                variant_index,
                field_index,
                ..
            } => {
                let source = self.value(state, *source)?;
                Ok(source.at(&[payload(*variant_index, *field_index)]))
            }
            HirExpressionKind::StructConstruct { fields, .. } => {
                let children = expressions
                    .struct_fields(*fields)
                    .iter()
                    .map(|&(field, child)| Ok((self.field(expression.ty, field)?, child)));
                self.construct(state, value, expression.ty, children)
            }
            HirExpressionKind::VariantConstruct {
                variant_index,
                fields,
                ..
            }
            | HirExpressionKind::RouteConstruct {
                variant_index,
                fields,
                ..
            } => self.construct(
                state,
                value,
                expression.ty,
                expressions
                    .variant_fields(*fields)
                    .iter()
                    .enumerate()
                    .map(|(index, field)| Ok((payload(*variant_index, index), field.value))),
            ),
            HirExpressionKind::Collection(values) => self.construct(
                state,
                value,
                expression.ty,
                expressions
                    .values(*values)
                    .iter()
                    .map(|&child| Ok((CallField::Elem, child))),
            ),
            HirExpressionKind::MapLiteral(entries) => self.construct(
                state,
                value,
                expression.ty,
                expressions.map_entries(*entries).iter().flat_map(|entry| {
                    [
                        Ok((CallField::MapKey, entry.key)),
                        Ok((CallField::Elem, entry.value)),
                    ]
                }),
            ),
            HirExpressionKind::Copy(place) => {
                let source = self.place(place)?;
                let target = state.temporary();
                let mut rows = Vec::new();
                state.snapshot(&target, &source, &mut rows);
                for row in &mut rows {
                    let detached = Spec::born(
                        row.cell.clone(),
                        row.node.ty,
                        Origin::Fresh(site, 0, row.cell.path.clone()),
                    );
                    row.node = detached.node;
                    row.births = detached.births;
                    row.detached = true;
                }
                state.install(rows, &[], None);
                Ok(target)
            }
            HirExpressionKind::Load(_) => self.opaque_value(state, site, 0, expression.ty),
            _ => self.fresh_operation(state, site, 0, expression.ty),
        }
    }

    fn construct(
        &self,
        state: &mut Graph,
        value: HirValueId,
        ty: TypeId,
        children: impl Iterator<Item = Result<(CallField, HirValueId), BorrowCheckError>>,
    ) -> Result<Cell, BorrowCheckError> {
        let result = state.temporary();
        let mut rows = vec![Spec::born(
            result.clone(),
            ty,
            Origin::Fresh(Site::Value(value.0), 0, Vec::new()),
        )];
        for child in children {
            let (field, value) = child?;
            if !self.heap(self.checker.module.expressions.expression(value).ty)? {
                continue;
            }
            let value = self.value(state, value)?;
            state.snapshot(&result.at(&[field]), &value, &mut rows);
        }
        state.install(rows, &[], None);
        Ok(result)
    }

    fn fresh_operation(
        &self,
        state: &mut Graph,
        site: Site,
        slot: usize,
        ty: TypeId,
    ) -> Result<Cell, BorrowCheckError> {
        let result = state.temporary();
        if self.heap(ty)? {
            state.install(
                vec![Spec::born(
                    result.clone(),
                    ty,
                    Origin::Fresh(site, slot, Vec::new()),
                )],
                &[],
                None,
            );
        }
        Ok(result)
    }

    /// Compiler-produced builtin errors initialise their message as well as the record.
    fn fresh_builtin_error(
        &self,
        state: &mut Graph,
        site: Site,
        slot: usize,
        ty: TypeId,
    ) -> Result<Cell, BorrowCheckError> {
        let result = state.temporary();
        let mut rows = Vec::new();
        self.typed_paths(ty, |path, ty| {
            rows.push(Spec::born(
                result.at(path),
                ty,
                Origin::Fresh(site, slot, path.to_vec()),
            ));
        })?;
        state.install(rows, &[], None);
        Ok(result)
    }

    fn opaque_value(
        &self,
        state: &mut Graph,
        site: Site,
        _slot: usize,
        ty: TypeId,
    ) -> Result<Cell, BorrowCheckError> {
        let result = state.temporary();
        let mut specs = Vec::new();
        self.typed_paths(ty, |path, ty| {
            specs.push(Spec::born(
                result.at(path),
                ty,
                Origin::Opaque(site, ty, None),
            ))
        })?;
        state.install(specs, &[], None);
        Ok(result)
    }

    /// The place a mutable authored local starts aliasing when it receives a place load.
    fn acquired_alias(
        &self,
        local: LocalId,
        value: HirValueId,
    ) -> Result<Option<Cell>, BorrowCheckError> {
        let binding = self.index.locals[&local];
        let expression = self.checker.module.expressions.expression(value);
        if binding.mutable
            && binding.authored
            && expression.value_kind == ValueKind::Place
            && let HirExpressionKind::Load(place) = &expression.kind
        {
            return Ok(Some(self.place(place)?));
        }
        Ok(None)
    }

    /// Bind a definite new value to the local's slot. A place load also makes the local that
    /// place's alias; a projected one captures its holder under the acquisition site.
    fn define(&self, state: &mut Graph, local: LocalId, value: &Cell, alias: Option<(Site, Cell)>) {
        let holder = alias.as_ref().and_then(|(_, alias)| {
            let (_, prefix) = alias.path.split_last()?;
            let holder = Cell::root(alias.owner.clone()).at(prefix);
            // A place that runs through the dying slot is captured before the slot is cleared.
            let through_slot = alias.owner == Owner::Local(local.0);
            Some(if through_slot {
                state.capture(&holder)
            } else {
                holder
            })
        });
        self.kill_local(state, local);
        self.rebind(state, Self::local(local), value, false);
        let Some((site, alias)) = alias else { return };
        let targets = match holder {
            Some(holder) => {
                let owner = Cell::root(Owner::Alias(local.0, site));
                self.rebind(state, owner.clone(), &holder, false);
                BTreeSet::from([owner.at(&alias.path[alias.path.len() - 1..])])
            }
            None => state
                .aliases
                .get(&place_local(&alias))
                .cloned()
                .unwrap_or_else(|| BTreeSet::from([alias])),
        };
        state.aliases.insert(local.0, targets);
    }

    /// Returns `false` when an update reaches a definite holder that no allocation reaches yet.
    #[must_use]
    fn bind(&self, state: &mut Graph, destination: HirLocalDestination, value: &Cell) -> bool {
        let local = destination.local();
        if matches!(destination, HirLocalDestination::Update(_)) {
            return self.assign(state, &Self::local(local), value, false);
        }
        self.define(state, local, value, None);
        true
    }

    fn kill_local(&self, state: &mut Graph, local: LocalId) {
        state.aliases.remove(&local.0);
        state.clear_local(local.0);
    }

    fn rebind(&self, state: &mut Graph, target: Cell, value: &Cell, weak: bool) {
        let mut rows = Vec::new();
        state.snapshot(&target, value, &mut rows);
        let clear = if weak {
            &[]
        } else {
            std::slice::from_ref(&target)
        };
        state.install(rows, clear, None);
    }

    /// Write every destination of one assignment. Returns `false` when a definite destination's
    /// holder is still bottom: that path stays incomplete until a widening input resolves the
    /// holder, instead of dropping a write that the resolved state would make strong.
    #[must_use]
    fn assign(&self, state: &mut Graph, target: &Cell, value: &Cell, weak: bool) -> bool {
        let mut seen = BTreeSet::new();
        let mut pending = vec![(target.clone(), weak)];
        let mut reached = true;
        while let Some((target, weak)) = pending.pop() {
            if !seen.insert(target.clone()) {
                continue;
            }
            if let Some((&key, prefix)) = target.path.split_last() {
                // An ambiguous alias target is only a may-destination: a bottom alternative adds
                // nothing, and another alternative carries the write.
                reached &= self.store(
                    state,
                    &Cell::root(target.owner.clone()).at(prefix),
                    key,
                    value,
                    weak,
                ) || weak;
            } else {
                let local = place_local(&target);
                let aliases = state.aliases.get(&local);
                let weak = weak || aliases.is_some_and(|aliases| aliases.len() != 1);
                if let Some(aliases) = aliases {
                    pending.extend(aliases.iter().cloned().map(|alias| (alias, weak)));
                }
                if let Some(index) = self.index.locals[&LocalId(local)].mutable_parameter {
                    let output = Cell::root(Owner::Store(CallParameterPath {
                        index,
                        path: Vec::new(),
                    }));
                    self.rebind(state, output, value, weak);
                }
                self.rebind(state, target, value, weak);
            }
        }
        reached
    }

    /// Returns `false` when the holder is bottom. `weak` only stops the write replacing existing
    /// contents; whether a bottom holder may be skipped is the caller's decision, because a
    /// definite operation on a weakly updated element path still needs its receiver to exist.
    #[must_use]
    fn store(
        &self,
        state: &mut Graph,
        holder: &Cell,
        key: CallField,
        value: &Cell,
        weak: bool,
    ) -> bool {
        let allowed = |index: usize| self.index.locals[&self.function.params[index]].mutable;
        state.write(holder, key, value, weak, allowed)
    }

    fn place(&self, place: &HirPlace) -> Result<Cell, BorrowCheckError> {
        let mut cell = Self::local(place.root);
        let mut ty = self.ty(place.root);
        for projection in self
            .checker
            .module
            .expressions
            .projections(place.projections)
        {
            let field = match projection {
                HirProjection::Field(field) => self.field(ty, *field)?,
                HirProjection::Index(_) => CallField::Elem,
            };
            ty = self.checker.call_child_type(ty, field).ok_or_else(|| {
                self.checker
                    .diagnostics
                    .internal_error("Invalid projection in allocation summary")
            })?;
            cell.path.push(field);
        }
        Ok(cell)
    }

    fn field(&self, ty: TypeId, field: FieldId) -> Result<CallField, BorrowCheckError> {
        self.checker
            .module
            .structs
            .iter()
            .find(|layout| layout.frontend_type_id == ty)
            .and_then(|layout| {
                layout
                    .fields
                    .iter()
                    .position(|candidate| candidate.id == field)
            })
            .map(|ordinal| CallField::Field(ordinal as u32))
            .ok_or_else(|| {
                self.checker
                    .diagnostics
                    .internal_error("Missing field ordinal in allocation summary")
            })
    }

    fn heap(&self, ty: TypeId) -> Result<bool, BorrowCheckError> {
        is_heap(self.checker, ty)
    }

    fn ty(&self, local: LocalId) -> TypeId {
        self.checker.local_types[&local]
    }

    fn local(local: LocalId) -> Cell {
        Cell::root(Owner::Local(local.0))
    }

    fn typed_paths(
        &self,
        ty: TypeId,
        mut visit: impl FnMut(&[CallField], TypeId),
    ) -> Result<(), BorrowCheckError> {
        fn walk(
            context: &FunctionEvidence<'_, '_>,
            ty: TypeId,
            path: &mut Path,
            stack: &mut Vec<TypeId>,
            visit: &mut impl FnMut(&[CallField], TypeId),
        ) -> Result<(), BorrowCheckError> {
            if !context.heap(ty)? {
                return Ok(());
            }
            if stack.contains(&ty) {
                return Err(context
                    .checker
                    .diagnostics
                    .internal_error("Recursive runtime layout reached allocation summary"));
            }
            visit(path, ty);
            stack.push(ty);
            CallField::for_each_child::<BorrowCheckError>(
                ty,
                context.checker.type_environment,
                |field, child| {
                    path.push(field);
                    walk(context, child, path, stack, visit)?;
                    path.pop();
                    Ok(())
                },
            )?;
            stack.pop();
            Ok(())
        }
        walk(self, ty, &mut Vec::new(), &mut Vec::new(), &mut visit)
    }

    fn call_summary(&self, target: &CallTarget) -> Option<&PublicCallSummary> {
        self.checker.call_signatures().summary(target)
    }
}

fn is_heap(checker: &BorrowChecker<'_>, ty: TypeId) -> Result<bool, BorrowCheckError> {
    Ok(classify_hir_type(ty, checker.type_environment)?.has_in_place_mutation_surface())
}

fn payload(variant: usize, field: usize) -> CallField {
    CallField::Payload {
        variant: variant as u32,
        field: field as u32,
    }
}

fn place_local(cell: &Cell) -> u32 {
    match cell.owner {
        Owner::Local(local) => local,
        _ => unreachable!("a HIR place must have a local root"),
    }
}

#[derive(Default)]
struct Lane {
    reachable: bool,
    sources: BTreeSet<CallPathSource>,
    stores: BTreeSet<CallPathSource>,
    sharing: BTreeSet<CallSharing>,
}

impl Lane {
    fn extend(&mut self, state: &Graph, results: &[Cell]) {
        self.reachable = true;
        let mut endpoints = BTreeMap::<&Cell, Vec<CallOutputPath>>::new();
        for (cell, node) in &state.nodes {
            let mut outputs = Vec::new();
            for (slot, result) in results.iter().enumerate() {
                if let Some(path) = cell.below(result) {
                    let output = CallOutputPath::Result(CallResultPath {
                        slot,
                        path: path.to_vec(),
                    });
                    self.sources
                        .extend(node.sources.iter().cloned().map(|origin| CallPathSource {
                            output: output.clone(),
                            source: origin.into_source(),
                        }));
                    outputs.push(output);
                }
            }
            if let Owner::Store(target) = &cell.owner {
                let output = CallOutputPath::Store {
                    target: target.clone(),
                    path: cell.path.clone(),
                };
                self.stores
                    .extend(node.sources.iter().cloned().map(|origin| CallPathSource {
                        output: output.clone(),
                        source: origin.into_source(),
                    }));
                outputs.push(output);
            }
            if !outputs.is_empty() {
                endpoints.insert(cell, outputs);
            }
        }
        for ((left, right), conditions) in &state.pairs {
            let (Some(left), Some(right)) = (endpoints.get(left), endpoints.get(right)) else {
                continue;
            };
            for a in left {
                for b in right {
                    let (left, right) = if a <= b { (a, b) } else { (b, a) };
                    if left != right {
                        self.sharing
                            .extend(conditions.iter().cloned().map(|condition| CallSharing {
                                left: left.clone(),
                                right: right.clone(),
                                condition,
                            }));
                    }
                }
            }
        }
    }

    fn finish(self) -> CallLaneSummary {
        CallLaneSummary {
            reachable: self.reachable,
            structural_only: false,
            sources: self.sources.into_iter().collect(),
            stores: self.stores.into_iter().collect(),
            sharing: self.sharing.into_iter().collect(),
        }
    }
}
