//! Borrow-checker metadata construction helpers.
//!
//! WHAT: builds the retained public-call summaries and per-function layouts that the fixed-point
//! driver needs before it can run transfer over reachable blocks.
//! WHY: HIR remains immutable during borrow validation, so signature and layout facts are cached
//! beside the analysis rather than written back into HIR nodes.

use super::engine::BorrowChecker;
use super::state::{FunctionLayout, FunctionLayoutInputs, RootSet};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalAccessKind};

use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_side_table::{HirLocalOriginKind, HirLocation};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId};
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::reactivity::HirReactiveSourceKind;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::terminator_targets;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallSummary,
    PublicCallTransferEffect, PublicCallTransferEligibility,
};
use rustc_hash::{FxHashMap, FxHashSet};

#[path = "return_alias.rs"]
mod return_alias;

/// Per-block liveness facts for one function.
///
/// WHAT: the locals a block reads before redefining them, and the locals it redefines outright.
/// WHY: future-use propagation is a liveness problem. Without the definition kill it collapses into
/// "used anywhere reachable from here", which keeps a rebound alias active across a loop back-edge
/// even though every path redefines it before the next read.
struct BlockLiveness {
    upward_exposed_reads: RootSet,
    definitions: RootSet,
}

impl BlockLiveness {
    fn new(local_count: usize) -> Self {
        Self {
            upward_exposed_reads: RootSet::empty(local_count),
            definitions: RootSet::empty(local_count),
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

/// Selects how a block combines the liveness facts of its successors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuccessorCombine {
    /// MAY future use: a root survives when any successor still reads it.
    AnyPath,
    /// MUST future use: a root survives only when every successor still reads it.
    EveryPath,
}

impl<'a> BorrowChecker<'a> {
    pub(super) fn build_public_call_summaries(&mut self) -> Result<(), BorrowCheckError> {
        // Parameter mutability is stored on locals. Gather it once globally, then project
        // parameter-order access and transfer facts into the single retained call contract.
        let mut local_mutability_by_id = FxHashMap::default();
        for block in &self.module.blocks {
            for local in &block.locals {
                local_mutability_by_id.insert(local.id, local.mutable);
            }
        }

        let mut parameter_owner_by_local = FxHashMap::default();
        let mut function_ids = FxHashSet::default();

        for function in &self.module.functions {
            if !function_ids.insert(function.id) {
                return Err(self.diagnostics.internal_error(
                    format!(
                        "Borrow checker found duplicate local function id '{}' while building public call summaries",
                        function.id
                    ),
                    self.diagnostics.function_error_span(function.id),
                ));
            }

            let mut parameters = Vec::with_capacity(function.params.len());
            let mut parameter_locals = FxHashSet::default();

            for (position, param) in function.params.iter().enumerate() {
                if !parameter_locals.insert(*param) {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found duplicate parameter local '{}' at position {} in function '{}'",
                            self.diagnostics.local_name(*param),
                            position,
                            self.diagnostics.function_name(function.id)
                        ),
                        self.diagnostics.function_error_span(function.id),
                    ));
                }

                let Some(is_mutable) = local_mutability_by_id.get(param).copied() else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker could not resolve mutability for parameter local '{}' in function '{}'",
                            self.diagnostics.local_name(*param),
                            self.diagnostics.function_name(function.id)
                        ),
                        self.diagnostics.function_error_span(function.id),
                    ));
                };

                if parameter_owner_by_local
                    .insert(*param, (function.id, position))
                    .is_some()
                {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found parameter local '{}' owned by more than one function",
                            self.diagnostics.local_name(*param)
                        ),
                        self.diagnostics.function_error_span(function.id),
                    ));
                }

                let access = match self.module.side_table.reactive_source_id_for_local(*param) {
                    Some(source_id) => {
                        let Some(source) = self.module.side_table.reactive_source(source_id) else {
                            return Err(self.diagnostics.internal_error(
                                format!(
                                    "Borrow checker could not resolve reactive source metadata for parameter local '{}'",
                                    self.diagnostics.local_name(*param)
                                ),
                                self.diagnostics.function_error_span(function.id),
                            ));
                        };

                        match source.kind {
                            HirReactiveSourceKind::Parameter if is_mutable => {
                                return Err(self.diagnostics.internal_error(
                                    format!(
                                        "Reactive parameter '{}' in function '{}' is marked mutable",
                                        self.diagnostics.local_name(*param),
                                        self.diagnostics.function_name(function.id)
                                    ),
                                    source.span,
                                ));
                            }
                            HirReactiveSourceKind::Parameter => PublicCallParameterAccess::Reactive,
                            HirReactiveSourceKind::Declaration => {
                                return Err(self.diagnostics.internal_error(
                                    format!(
                                        "Reactive declaration metadata is attached to parameter local '{}' in function '{}'",
                                        self.diagnostics.local_name(*param),
                                        self.diagnostics.function_name(function.id)
                                    ),
                                    source.span,
                                ));
                            }
                        }
                    }
                    None if is_mutable => PublicCallParameterAccess::Mutable,
                    None => PublicCallParameterAccess::Shared,
                };

                let (transfer_eligibility, transfer_effect) = match access {
                    PublicCallParameterAccess::Reactive => (
                        PublicCallTransferEligibility::Ineligible,
                        PublicCallTransferEffect::NeverConsumes,
                    ),
                    PublicCallParameterAccess::Shared | PublicCallParameterAccess::Mutable => (
                        PublicCallTransferEligibility::Eligible,
                        PublicCallTransferEffect::MayConsume,
                    ),
                };

                parameters.push(PublicCallParameterSummary {
                    access,
                    mutation: PublicCallMutationEffect::NoWrite,
                    transfer_eligibility,
                    transfer_effect,
                    reactive_effect: PublicCallReactiveEffect::None,
                });
            }

            self.public_call_summaries.insert(
                function.id,
                PublicCallSummary {
                    parameters,
                    return_alias: FunctionReturnAliasSummary::Fresh,
                },
            );
        }

        for source in self.module.side_table.reactive_sources() {
            if source.kind != HirReactiveSourceKind::Parameter {
                continue;
            }

            if parameter_owner_by_local.contains_key(&source.local_id) {
                continue;
            }

            return Err(self.diagnostics.internal_error(
                format!(
                    "Reactive parameter source metadata points at local '{}' that is not a function parameter",
                    self.diagnostics.local_name(source.local_id)
                ),
                source.span,
            ));
        }

        self.retain_hir_reactive_parameter_effects(&parameter_owner_by_local)?;

        self.stabilize_return_alias_summaries()?;

        if self.public_call_summaries.len() != self.module.functions.len() {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker retained {} public call summaries for {} local functions",
                    self.public_call_summaries.len(),
                    self.module.functions.len()
                ),
                self.diagnostics.module_error_span(),
            ));
        }

        Ok(())
    }

    fn stabilize_return_alias_summaries(&mut self) -> Result<(), BorrowCheckError> {
        // Return aliases form a finite monotone lattice. Recompute every function from the same
        // retained map, then publish the whole pass so local call chains cannot depend on HIR
        // declaration order.
        let recursive_functions = self.recursive_return_alias_functions()?;
        for function_id in &recursive_functions {
            let Some(summary) = self.public_call_summaries.get_mut(function_id) else {
                return Err(self.diagnostics.internal_error(
                    format!(
                        "Borrow checker is missing the public call summary for recursive function '{}'",
                        self.diagnostics.function_name(*function_id)
                    ),
                    self.diagnostics.function_error_span(*function_id),
                ));
            };
            // A recursive return-summary cycle has no finite body summary to project through.
            // Publish Unknown at the cycle boundary so callers cannot mistake the initial Fresh
            // seed for a proven fresh result.
            summary.return_alias = FunctionReturnAliasSummary::Unknown;
        }

        let parameter_count = self
            .module
            .functions
            .iter()
            .map(|function| function.params.len())
            .sum::<usize>();
        let max_iterations = parameter_count
            .saturating_add(self.module.functions.len().saturating_mul(2))
            .saturating_add(1);

        for _ in 0..max_iterations {
            let mut retained_by_function = Vec::with_capacity(self.module.functions.len());
            for function in &self.module.functions {
                let classified = if recursive_functions.contains(&function.id) {
                    FunctionReturnAliasSummary::Unknown
                } else {
                    self.classify_function_return_alias(function)?
                };
                retained_by_function.push((function.id, classified.clone(), classified));
            }

            let mut changed = false;
            for (function_id, _, retained) in &retained_by_function {
                let Some(summary) = self.public_call_summaries.get_mut(function_id) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker is missing the public call summary for function '{}'",
                            self.diagnostics.function_name(*function_id)
                        ),
                        self.diagnostics.function_error_span(*function_id),
                    ));
                };

                if summary.return_alias != *retained {
                    summary.return_alias = retained.clone();
                    changed = true;
                }
            }

            if !changed {
                return Ok(());
            }
        }

        Err(self.diagnostics.internal_error(
            "Borrow checker could not stabilize local return-alias summaries",
            self.diagnostics.module_error_span(),
        ))
    }

    fn recursive_return_alias_functions(&self) -> Result<FxHashSet<FunctionId>, BorrowCheckError> {
        let mut edges = FxHashMap::<FunctionId, Vec<FunctionId>>::default();
        for function in &self.module.functions {
            let mut callees = Vec::new();
            let reachable_blocks = self.collect_reachable_blocks(function)?;
            for block_id in reachable_blocks {
                let block = self.block_by_id_or_error(block_id, function.id)?;

                for statement in &block.statements {
                    let HirStatementKind::Call {
                        target: CallTarget::Local(callee),
                        ..
                    } = &statement.kind
                    else {
                        continue;
                    };
                    callees.push(*callee);
                }
            }
            callees.sort_unstable_by_key(|callee| callee.0);
            callees.dedup();
            edges.insert(function.id, callees);
        }

        let mut recursive = FxHashSet::default();
        for function in &self.module.functions {
            let mut visited = FxHashSet::default();
            if reaches_function(function.id, function.id, &edges, &mut visited) {
                recursive.insert(function.id);
            }
        }
        Ok(recursive)
    }

    fn retain_hir_reactive_parameter_effects(
        &mut self,
        parameter_owner_by_local: &FxHashMap<LocalId, (FunctionId, usize)>,
    ) -> Result<(), BorrowCheckError> {
        for template in self.module.side_table.reactive_templates() {
            for dependency in &template.dependencies {
                let Some(source) = self.module.side_table.reactive_source(dependency.source) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Reactive template metadata references unknown source {:?}",
                            dependency.source
                        ),
                        dependency.span,
                    ));
                };

                if source.kind != HirReactiveSourceKind::Parameter {
                    continue;
                }

                self.mark_reactive_subscription(parameter_owner_by_local, source.local_id)?;
            }

            for dependency in &template.template_value_parameters {
                self.mark_reactive_subscription(parameter_owner_by_local, dependency.parameter)?;
            }
        }

        Ok(())
    }

    fn mark_reactive_subscription(
        &mut self,
        parameter_owner_by_local: &FxHashMap<LocalId, (FunctionId, usize)>,
        parameter_local: LocalId,
    ) -> Result<(), BorrowCheckError> {
        let Some((function_id, position)) = parameter_owner_by_local.get(&parameter_local) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Reactive template metadata references local '{}' that is not a function parameter",
                    self.diagnostics.local_name(parameter_local)
                ),
                self.diagnostics.module_error_span(),
            ));
        };

        let Some(summary) = self.public_call_summaries.get_mut(function_id) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the public call summary for function '{}'",
                    self.diagnostics.function_name(*function_id)
                ),
                self.diagnostics.function_error_span(*function_id),
            ));
        };

        let Some(parameter) = summary.parameters.get_mut(*position) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Reactive template metadata references out-of-range parameter position {} in function '{}'",
                    position,
                    self.diagnostics.function_name(*function_id)
                ),
                self.diagnostics.function_error_span(*function_id),
            ));
        };
        parameter.reactive_effect = parameter.reactive_effect.with_subscription();
        Ok(())
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
        let mut invalidated_positions = FxHashSet::default();

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function.id)?;
            for statement in &block.statements {
                let Some(statement_fact) = report.analysis.statement_facts.get(&statement.id)
                else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker is missing statement facts while finalizing public call summary for function '{}'",
                            self.diagnostics.function_name(function.id)
                        ),
                        self.diagnostics.statement_error_span(statement),
                    ));
                };

                let mut mutates_parameter = |root: &LocalId| {
                    if let Some(position) = parameter_positions.get(root) {
                        mutation_positions.insert(*position);
                    }
                };

                match &statement.kind {
                    HirStatementKind::Assign { .. } => {
                        for root in &statement_fact.mutable_roots {
                            mutates_parameter(root);
                        }
                    }
                    HirStatementKind::Call { target, args, .. } => {
                        for (argument_index, argument) in args.iter().enumerate() {
                            if !self.call_argument_writes(target, argument_index, statement)? {
                                continue;
                            }

                            let Some(argument_fact) = report.analysis.value_facts.get(&argument.id)
                            else {
                                return Err(self.diagnostics.internal_error(
                                    format!(
                                        "Borrow checker is missing value facts for argument {} while finalizing public call summary for function '{}'",
                                        argument_index,
                                        self.diagnostics.function_name(function.id)
                                    ),
                                    self.diagnostics.statement_error_span(statement),
                                ));
                            };
                            for root in &argument_fact.roots {
                                mutates_parameter(root);
                            }
                        }
                    }
                    HirStatementKind::MapOp { op, receiver, .. }
                        if op.requires_mutable_receiver() =>
                    {
                        let Some(receiver_fact) = report.analysis.value_facts.get(&receiver.id)
                        else {
                            return Err(self.diagnostics.internal_error(
                                format!(
                                    "Borrow checker is missing map receiver value facts while finalizing public call summary for function '{}'",
                                    self.diagnostics.function_name(function.id)
                                ),
                                self.diagnostics.statement_error_span(statement),
                            ));
                        };
                        for root in &receiver_fact.roots {
                            mutates_parameter(root);
                        }
                    }
                    _ => {}
                }
            }

            for statement in &block.statements {
                let Some(invalidations) = report.analysis.reactive_invalidations.get(&statement.id)
                else {
                    continue;
                };
                for invalidation in invalidations {
                    let Some(source) = self.module.side_table.reactive_source(invalidation.source)
                    else {
                        return Err(self.diagnostics.internal_error(
                            format!(
                                "Borrow checker reactive invalidation references unknown source {:?}",
                                invalidation.source
                            ),
                            invalidation.span,
                        ));
                    };
                    if let Some(position) = parameter_positions.get(&source.local_id) {
                        invalidated_positions.insert(*position);
                    }
                }
            }
        }

        let Some(summary) = self.public_call_summaries.get_mut(&function.id) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the public call summary for function '{}'",
                    self.diagnostics.function_name(function.id)
                ),
                self.diagnostics.function_error_span(function.id),
            ));
        };

        let mut changed = false;
        for (position, parameter) in summary.parameters.iter_mut().enumerate() {
            if mutation_positions.contains(&position)
                && parameter.mutation != PublicCallMutationEffect::Writes
            {
                parameter.mutation = PublicCallMutationEffect::Writes;
                changed = true;
            }
            if invalidated_positions.contains(&position) {
                parameter.reactive_effect = parameter.reactive_effect.with_invalidation();
            }
        }

        Ok(changed)
    }

    fn call_argument_writes(
        &self,
        target: &CallTarget,
        argument_index: usize,
        statement: &HirStatement,
    ) -> Result<bool, BorrowCheckError> {
        match target {
            CallTarget::Local(function_id) => {
                let Some(summary) = self.public_call_summaries.get(function_id) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker is missing the public call summary for function '{}'",
                            self.diagnostics.function_name(*function_id)
                        ),
                        self.diagnostics.function_error_span(*function_id),
                    ));
                };
                let Some(parameter) = summary.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {} while finalizing call summary for function '{}'",
                            argument_index,
                            self.diagnostics.function_name(*function_id)
                        ),
                        self.diagnostics.function_error_span(*function_id),
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
                        self.diagnostics.statement_error_span(statement),
                    ));
                };
                let Some(parameter) = summary.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {argument_index} while finalizing imported call summary for {origin:?}"
                        ),
                        self.diagnostics.statement_error_span(statement),
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
                        self.diagnostics.statement_error_span(statement),
                    ));
                };
                let Some(parameter) = summary.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {argument_index} while finalizing module-private call summary for {identity:?}"
                        ),
                        self.diagnostics.statement_error_span(statement),
                    ));
                };
                Ok(parameter.mutation == PublicCallMutationEffect::Writes)
            }
            CallTarget::Generated(identity) => {
                let Some(summary) = self.module.generated_call_summaries.get(identity) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker is missing the call summary for generated function {identity:?}"
                        ),
                        self.diagnostics.statement_error_span(statement),
                    ));
                };
                let Some(parameter) = summary.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {argument_index} while finalizing generated call summary for {identity:?}"
                        ),
                        self.diagnostics.statement_error_span(statement),
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
                        self.diagnostics.statement_error_span(statement),
                    ));
                };
                let Some(parameter) = definition.parameters.get(argument_index) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker found out-of-range argument {} while finalizing host call summary for '{}'",
                            argument_index, definition.name
                        ),
                        self.diagnostics.statement_error_span(statement),
                    ));
                };
                Ok(parameter.access_kind == ExternalAccessKind::Mutable)
            }
        }
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
                local_info_by_id.insert(local.id, (local.mutable, local.region));
            }
        }

        for param in &function.params {
            if !local_info_by_id.contains_key(param) {
                return Err(self.diagnostics.internal_error(
                    format!(
                        "Function '{}' parameter '{}' is missing from reachable local layout",
                        self.diagnostics.function_name(function.id),
                        self.diagnostics.local_name(*param)
                    ),
                    self.diagnostics.function_error_span(function.id),
                ));
            }
        }

        let mut local_ids = local_info_by_id.keys().copied().collect::<Vec<_>>();
        local_ids.sort_by_key(|local_id| local_id.0);

        let local_mutable = local_ids
            .iter()
            .map(|local_id| local_info_by_id[local_id].0)
            .collect::<Vec<_>>();
        let local_regions = local_ids
            .iter()
            .map(|local_id| local_info_by_id[local_id].1)
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
        let mut next_order_key = 0i32;

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function.id)?;
            let mut max_use_order = vec![-1; local_ids.len()];
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
                collect_statement_loaded_locals(statement, &mut |local_id| {
                    if let Some(index) = local_index_by_id.get(&local_id).copied() {
                        local_last_use_order[index] = local_last_use_order[index].max(order_key);
                        max_use_order[index] = max_use_order[index].max(order_key);
                    }
                });

                record_statement_liveness(statement, &local_index_by_id, &mut liveness);
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

            collect_terminator_loaded_locals(&block.terminator, &mut |local_id| {
                if let Some(index) = local_index_by_id.get(&local_id).copied() {
                    local_last_use_order[index] = local_last_use_order[index].max(terminator_order);
                    max_use_order[index] = max_use_order[index].max(terminator_order);
                    liveness.record_read(index);
                }
            });

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
            block_successors.insert(
                *block_id,
                terminator_targets(&block.terminator)
                    .into_iter()
                    .filter(|successor| reachable_block_set.contains(successor))
                    .collect(),
            );
        }

        let (may_use_from_block, must_use_from_block) = compute_future_use_sets(
            local_ids.len(),
            reachable_blocks,
            &block_successors,
            &block_liveness,
        );

        Ok(FunctionLayout::new(FunctionLayoutInputs {
            local_ids,
            local_mutable,
            local_regions,
            local_first_write_order,
            local_last_use_order,
            statement_order_by_id,
            terminator_order_by_block,
            block_local_max_use_order,
            block_successors,
            may_use_from_block,
            must_use_from_block,
        }))
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
                let HirStatementKind::Assign {
                    target: HirPlace::Local(target_local),
                    value,
                } = &statement.kind
                else {
                    continue;
                };

                if self.module.side_table.local_origin_kind(*target_local)
                    != Some(HirLocalOriginKind::CompilerTemp)
                {
                    continue;
                }

                let HirExpressionKind::Load(place) = &value.kind else {
                    continue;
                };
                let Some(source_local) = root_local_for_place(place) else {
                    continue;
                };

                let Some(target_index) = local_index_by_id.get(target_local).copied() else {
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

    pub(super) fn build_visibility_masks(
        &self,
        function_id: crate::compiler_frontend::hir::ids::FunctionId,
        layout: &FunctionLayout,
        reachable_blocks: &[BlockId],
    ) -> Result<FxHashMap<BlockId, RootSet>, BorrowCheckError> {
        // A local is visible in a block when local.region is an ancestor
        // of block.region in the lexical region tree.
        let mut masks = FxHashMap::default();

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function_id)?;
            let mut mask = RootSet::empty(layout.local_count());

            for (local_index, local_region) in layout.local_regions.iter().enumerate() {
                if self.is_region_ancestor_of(
                    *local_region,
                    block.region,
                    function_id,
                    *block_id,
                )? {
                    mask.insert(local_index);
                }
            }

            masks.insert(*block_id, mask);
        }

        Ok(masks)
    }

    fn is_region_ancestor_of(
        &self,
        ancestor: crate::compiler_frontend::hir::ids::RegionId,
        mut region: crate::compiler_frontend::hir::ids::RegionId,
        function_id: crate::compiler_frontend::hir::ids::FunctionId,
        block_id: BlockId,
    ) -> Result<bool, BorrowCheckError> {
        // Walk parent links from `region` to root.
        loop {
            if region == ancestor {
                return Ok(true);
            }

            let Some(parent) = self.region_parent_by_id.get(&region).copied() else {
                let span = self
                    .module
                    .side_table
                    .hir_source_span_for_hir(HirLocation::Block(block_id))
                    .or_else(|| {
                        self.module
                            .side_table
                            .ast_span_for_hir(HirLocation::Block(block_id))
                    })
                    .or_else(|| self.diagnostics.function_error_span(function_id));

                return Err(self.diagnostics.internal_error(
                    format!(
                        "Borrow checker could not resolve region '{}' while analyzing block '{}'",
                        region.0, block_id
                    ),
                    span,
                ));
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
        crate::compiler_frontend::hir::utils::collect_reachable_blocks(function.entry, |block_id| {
            let block = self.block_by_id_or_error(block_id, function.id)?;
            Ok(terminator_targets(&block.terminator))
        })
    }
}

fn compute_future_use_sets(
    local_count: usize,
    reachable_blocks: &[BlockId],
    block_successors: &FxHashMap<BlockId, Vec<BlockId>>,
    block_liveness: &FxHashMap<BlockId, BlockLiveness>,
) -> (FxHashMap<BlockId, RootSet>, FxHashMap<BlockId, RootSet>) {
    // WHAT: derives per-block MAY/MUST future-use summaries by backward liveness propagation.
    // WHY: transfer needs O(1) future-use classification when deciding borrow versus move, and
    //      alias-conflict checks need to know whether the value arriving at a block is still
    //      required. Both are liveness questions, so the definition kill is what stops a rebound
    //      local from looking used forever once a loop back-edge makes its own body reachable.
    //
    //      live_in[b]  = upward_exposed_reads[b] | (live_out[b] & !definitions[b])
    //      live_out[b] = combine over successors: union for MAY, intersection for MUST
    let may_use_from_block = propagate_block_liveness(
        local_count,
        reachable_blocks,
        block_successors,
        block_liveness,
        SuccessorCombine::AnyPath,
    );

    let must_use_from_block = propagate_block_liveness(
        local_count,
        reachable_blocks,
        block_successors,
        block_liveness,
        SuccessorCombine::EveryPath,
    );

    (may_use_from_block, must_use_from_block)
}

fn propagate_block_liveness(
    local_count: usize,
    reachable_blocks: &[BlockId],
    block_successors: &FxHashMap<BlockId, Vec<BlockId>>,
    block_liveness: &FxHashMap<BlockId, BlockLiveness>,
    combine: SuccessorCombine,
) -> FxHashMap<BlockId, RootSet> {
    // MAY grows from nothing; MUST shrinks from everything so loops converge on the roots that
    // survive on every path rather than on the first path visited.
    let seed = match combine {
        SuccessorCombine::AnyPath => RootSet::empty(local_count),
        SuccessorCombine::EveryPath => RootSet::full(local_count),
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

            let mut next = combine_successor_liveness(local_count, successors, &live_in, combine);

            if let Some(liveness) = block_liveness.get(block_id) {
                next.subtract_with(&liveness.definitions);
                next.union_with(&liveness.upward_exposed_reads);
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

fn combine_successor_liveness(
    local_count: usize,
    successors: &[BlockId],
    live_in: &FxHashMap<BlockId, RootSet>,
    combine: SuccessorCombine,
) -> RootSet {
    // An exit block has no successor, so nothing is required after it on either lattice.
    if successors.is_empty() {
        return RootSet::empty(local_count);
    }

    match combine {
        SuccessorCombine::AnyPath => {
            let mut union = RootSet::empty(local_count);
            for successor in successors {
                if let Some(successor_live_in) = live_in.get(successor) {
                    union.union_with(successor_live_in);
                }
            }
            union
        }

        SuccessorCombine::EveryPath => {
            let mut intersection = RootSet::full(local_count);
            for successor in successors {
                match live_in.get(successor) {
                    Some(successor_live_in) => intersection.intersect_with(successor_live_in),
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
    statement: &HirStatement,
    local_index_by_id: &FxHashMap<LocalId, usize>,
    liveness: &mut BlockLiveness,
) {
    let defined_local = statement_defined_local(statement);

    collect_statement_loaded_locals(statement, &mut |local_id| {
        if let Some(index) = local_index_by_id.get(&local_id).copied() {
            liveness.record_read(index);
        }
    });

    // A write through a projection updates part of the base and leaves the rest observable, so the
    // base counts as a read and never kills.
    collect_statement_written_locals(statement, &mut |local_id| {
        if Some(local_id) == defined_local {
            return;
        }
        if let Some(index) = local_index_by_id.get(&local_id).copied() {
            liveness.record_read(index);
        }
    });

    if let Some(local_id) = defined_local
        && let Some(index) = local_index_by_id.get(&local_id).copied()
    {
        liveness.record_definition(index);
    }
}

// WHAT: reports the local this statement redefines outright, if any.
// WHY: only a whole-local write ends the previous value's life. Field and index writes reach the
//      same root through `collect_statement_written_locals` but leave the surrounding value intact.
fn statement_defined_local(statement: &HirStatement) -> Option<LocalId> {
    match &statement.kind {
        HirStatementKind::Assign {
            target: HirPlace::Local(local),
            ..
        } => Some(*local),
        HirStatementKind::Call {
            result: Some(local),
            ..
        }
        | HirStatementKind::MapOp {
            result: Some(local),
            ..
        }
        | HirStatementKind::CastOp {
            result: Some(local),
            ..
        } => Some(*local),
        HirStatementKind::NumericOp { result, .. }
        | HirStatementKind::FormatFloat { result, .. }
        | HirStatementKind::ValidateFloat { result, .. } => Some(*result),

        HirStatementKind::Assign { .. }
        | HirStatementKind::Call { result: None, .. }
        | HirStatementKind::MapOp { result: None, .. }
        | HirStatementKind::CastOp { result: None, .. }
        | HirStatementKind::Expr(_)
        | HirStatementKind::Drop(_)
        | HirStatementKind::PushRuntimeFragment { .. } => None,
    }
}

fn reaches_function(
    start: FunctionId,
    current: FunctionId,
    edges: &FxHashMap<FunctionId, Vec<FunctionId>>,
    visited: &mut FxHashSet<FunctionId>,
) -> bool {
    let Some(callees) = edges.get(&current) else {
        return false;
    };

    for callee in callees {
        if *callee == start {
            return true;
        }
        if visited.insert(*callee) && reaches_function(start, *callee, edges, visited) {
            return true;
        }
    }

    false
}

fn root_local_for_place(place: &HirPlace) -> Option<LocalId> {
    match place {
        HirPlace::Local(local) => Some(*local),
        HirPlace::Field { base, .. } | HirPlace::Index { base, .. } => root_local_for_place(base),
    }
}

fn collect_statement_loaded_locals(statement: &HirStatement, visitor: &mut impl FnMut(LocalId)) {
    match &statement.kind {
        HirStatementKind::Assign { target, value } => {
            collect_place_index_loaded_locals(target, visitor);
            collect_expression_loaded_locals(value, visitor);
        }
        HirStatementKind::Call { args, .. } => {
            for arg in args {
                collect_expression_loaded_locals(arg, visitor);
            }
        }
        HirStatementKind::MapOp { receiver, args, .. } => {
            collect_expression_loaded_locals(receiver, visitor);
            for arg in args {
                collect_expression_loaded_locals(arg, visitor);
            }
        }
        HirStatementKind::CastOp { source, .. } => {
            collect_expression_loaded_locals(source, visitor);
        }
        HirStatementKind::NumericOp { operands, .. } => match operands {
            HirNumericOperands::Unary { operand } => {
                collect_expression_loaded_locals(operand, visitor);
            }
            HirNumericOperands::Binary { left, right } => {
                collect_expression_loaded_locals(left, visitor);
                collect_expression_loaded_locals(right, visitor);
            }
        },
        HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. } => {
            collect_expression_loaded_locals(source, visitor);
        }
        HirStatementKind::Expr(expression) => {
            collect_expression_loaded_locals(expression, visitor);
        }
        HirStatementKind::Drop(local) => visitor(*local),
        HirStatementKind::PushRuntimeFragment { vec_local, value } => {
            visitor(*vec_local);
            collect_expression_loaded_locals(value, visitor);
        }
    }
}

fn collect_statement_written_locals(statement: &HirStatement, visitor: &mut impl FnMut(LocalId)) {
    match &statement.kind {
        HirStatementKind::Assign { target, .. } => collect_place_written_local(target, visitor),
        HirStatementKind::Call {
            result: Some(local),
            ..
        } => visitor(*local),
        HirStatementKind::MapOp {
            result: Some(local),
            ..
        }
        | HirStatementKind::CastOp {
            result: Some(local),
            ..
        } => visitor(*local),
        HirStatementKind::NumericOp { result, .. } => visitor(*result),
        HirStatementKind::FormatFloat { result, .. }
        | HirStatementKind::ValidateFloat { result, .. } => visitor(*result),
        HirStatementKind::Call { result: None, .. }
        | HirStatementKind::MapOp { result: None, .. }
        | HirStatementKind::CastOp { result: None, .. }
        | HirStatementKind::Expr(_)
        | HirStatementKind::Drop(_)
        | HirStatementKind::PushRuntimeFragment { .. } => {}
    }
}

fn collect_place_written_local(place: &HirPlace, visitor: &mut impl FnMut(LocalId)) {
    match place {
        HirPlace::Local(local) => visitor(*local),
        HirPlace::Field { base, .. } | HirPlace::Index { base, .. } => {
            collect_place_written_local(base, visitor)
        }
    }
}

fn collect_place_index_loaded_locals(place: &HirPlace, visitor: &mut impl FnMut(LocalId)) {
    match place {
        HirPlace::Local(_) => {}
        HirPlace::Field { base, .. } => collect_place_index_loaded_locals(base, visitor),
        HirPlace::Index { base, index } => {
            collect_place_index_loaded_locals(base, visitor);
            collect_expression_loaded_locals(index, visitor);
        }
    }
}

fn collect_terminator_loaded_locals(terminator: &HirTerminator, visitor: &mut impl FnMut(LocalId)) {
    match terminator {
        // Jump argument passing is CFG plumbing, not a semantic value use.
        HirTerminator::Jump { .. } => {}
        HirTerminator::If { condition, .. } => {
            collect_expression_loaded_locals(condition, visitor);
        }
        HirTerminator::FallibleBranch { result, .. } => {
            collect_expression_loaded_locals(result, visitor);
        }
        HirTerminator::Match { scrutinee, arms } => {
            collect_expression_loaded_locals(scrutinee, visitor);
            for arm in arms {
                if let HirPattern::Literal(expression)
                | HirPattern::OptionValue { value: expression }
                | HirPattern::OptionRelational {
                    value: expression, ..
                } = &arm.pattern
                {
                    collect_expression_loaded_locals(expression, visitor);
                }
                if let Some(guard) = &arm.guard {
                    collect_expression_loaded_locals(guard, visitor);
                }
            }
        }
        HirTerminator::Return(value)
        | HirTerminator::ReturnSuccess(value)
        | HirTerminator::ReturnError(value) => {
            collect_expression_loaded_locals(value, visitor);
        }
        HirTerminator::AssertFailure { message, .. } => {
            collect_expression_loaded_locals(message, visitor);
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

fn collect_expression_loaded_locals(expression: &HirExpression, visitor: &mut impl FnMut(LocalId)) {
    match &expression.kind {
        HirExpressionKind::Load(place) => collect_place_loaded_locals(place, visitor),
        HirExpressionKind::Copy(place) => collect_place_loaded_locals(place, visitor),
        HirExpressionKind::BinOp { left, right, .. } => {
            collect_expression_loaded_locals(left, visitor);
            collect_expression_loaded_locals(right, visitor);
        }
        HirExpressionKind::UnaryOp { operand, .. } => {
            collect_expression_loaded_locals(operand, visitor);
        }
        HirExpressionKind::StructConstruct { fields, .. } => {
            for (_, value) in fields {
                collect_expression_loaded_locals(value, visitor);
            }
        }
        HirExpressionKind::Collection(elements)
        | HirExpressionKind::TupleConstruct { elements } => {
            for element in elements {
                collect_expression_loaded_locals(element, visitor);
            }
        }
        HirExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                collect_expression_loaded_locals(&entry.key, visitor);
                collect_expression_loaded_locals(&entry.value, visitor);
            }
        }
        HirExpressionKind::TupleGet { tuple, .. } => {
            collect_expression_loaded_locals(tuple, visitor);
        }
        HirExpressionKind::Range { start, end } => {
            collect_expression_loaded_locals(start, visitor);
            collect_expression_loaded_locals(end, visitor);
        }
        HirExpressionKind::VariantConstruct { fields, .. } => {
            for field in fields {
                collect_expression_loaded_locals(&field.value, visitor);
            }
        }
        HirExpressionKind::FallibleUnwrapSuccess { result }
        | HirExpressionKind::FallibleUnwrapError { result }
        | HirExpressionKind::Cast { source: result, .. } => {
            collect_expression_loaded_locals(result, visitor);
        }
        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. } => {}

        HirExpressionKind::VariantPayloadGet { source, .. } => {
            collect_expression_loaded_locals(source, visitor);
        }
    }
}

fn collect_place_loaded_locals(place: &HirPlace, visitor: &mut impl FnMut(LocalId)) {
    match place {
        HirPlace::Local(local) => visitor(*local),
        HirPlace::Field { base, .. } => collect_place_loaded_locals(base, visitor),
        HirPlace::Index { base, index } => {
            collect_place_loaded_locals(base, visitor);
            collect_expression_loaded_locals(index, visitor);
        }
    }
}

#[cfg(test)]
#[path = "tests/borrow_checker_metadata_tests.rs"]
mod borrow_checker_metadata_tests;
