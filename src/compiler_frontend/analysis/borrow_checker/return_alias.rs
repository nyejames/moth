//! Return-alias summary classification.
//!
//! WHAT: classifies the provenance of function return values and projects callee summaries
//!       through call arguments.
//! WHY: keeping this recursive classifier separate from metadata construction leaves the metadata
//!      module focused on retained layouts and effects.

use super::BorrowChecker;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalReturnAlias};
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use rustc_hash::{FxHashMap, FxHashSet};

/// Immutable inputs for projecting one callee return-alias summary through a caller.
struct AliasProjectionContext<'a> {
    function: &'a HirFunction,
    return_alias: &'a FunctionReturnAliasSummary,
    args: &'a [HirValueId],
    param_index_by_local: &'a FxHashMap<LocalId, usize>,
    reachable_blocks: &'a [BlockId],
    callee_description: &'a str,
}

impl<'a> BorrowChecker<'a> {
    pub(super) fn classify_function_return_alias(
        &self,
        function: &HirFunction,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        // Summary lattice:
        // Fresh < AliasParams(bitset) < Unknown
        // If any reachable return shape is ambiguous, we escalate to Unknown.
        let reachable_blocks = self.collect_reachable_blocks(function)?;
        let mut param_index_by_local = FxHashMap::default();
        for (param_index, param_local) in function.params.iter().enumerate() {
            param_index_by_local.insert(*param_local, param_index);
        }

        let mut summary = FunctionReturnAliasSummary::Fresh;
        let mut saw_return = false;

        for &block_id in &reachable_blocks {
            let block = self.block_by_id_or_error(block_id, function.id)?;
            // Fallible functions return successful values through ReturnSuccess. They still
            // participate in the same signature-level alias contract as plain returns.
            let value = match &block.terminator {
                HirTerminator::Return(value) | HirTerminator::ReturnSuccess(value) => value,
                _ => continue,
            };

            saw_return = true;
            let return_summary = self.classify_return_expression(
                function,
                &reachable_blocks,
                *value,
                &param_index_by_local,
            )?;
            summary = merge_return_alias(summary, return_summary);

            if matches!(summary, FunctionReturnAliasSummary::Unknown) {
                // Unknown is the lattice top; no additional scanning can improve it.
                break;
            }
        }

        if !saw_return {
            return Ok(FunctionReturnAliasSummary::Unknown);
        }

        Ok(summary)
    }

    fn classify_return_expression(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        expression: HirValueId,
        param_index_by_local: &FxHashMap<LocalId, usize>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let mut visiting_locals = FxHashSet::default();
        self.classify_return_expression_with_visiting(
            function,
            reachable_blocks,
            expression,
            param_index_by_local,
            &mut visiting_locals,
        )
    }

    fn classify_return_expression_with_visiting(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        expression: HirValueId,
        param_index_by_local: &FxHashMap<LocalId, usize>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        match &self.module.expressions.expression(expression).kind {
            HirExpressionKind::FallibleUnwrapSuccess { result } => self
                .classify_unwrapped_success_payload(
                    function,
                    reachable_blocks,
                    *result,
                    param_index_by_local,
                    visiting_locals,
                ),
            HirExpressionKind::Load(place) => {
                let root_local = place.root;

                if let Some(param_index) = param_index_by_local.get(&root_local).copied() {
                    return Ok(FunctionReturnAliasSummary::AliasParams(vec![param_index]));
                }

                self.classify_return_local(
                    function,
                    reachable_blocks,
                    root_local,
                    param_index_by_local,
                    visiting_locals,
                )
            }
            HirExpressionKind::TupleConstruct { elements } => {
                // Multi-return summaries currently expose one conservative function-wide
                // union. Each projected result receives that union until the public summary
                // grows per-result provenance.
                let mut summary = FunctionReturnAliasSummary::Fresh;
                for element in self.module.expressions.values(*elements) {
                    summary = merge_return_alias(
                        summary,
                        self.classify_return_expression_with_visiting(
                            function,
                            reachable_blocks,
                            *element,
                            param_index_by_local,
                            visiting_locals,
                        )?,
                    );
                    if matches!(summary, FunctionReturnAliasSummary::Unknown) {
                        break;
                    }
                }
                Ok(summary)
            }
            HirExpressionKind::TupleGet { tuple, .. } => self
                .classify_return_expression_with_visiting(
                    function,
                    reachable_blocks,
                    *tuple,
                    param_index_by_local,
                    visiting_locals,
                ),
            // Copies and computed/constructed expressions produce independent results. Their
            // input aliases do not become aliases of the result.
            _ => Ok(FunctionReturnAliasSummary::Fresh),
        }
    }

    fn classify_return_local(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        local: LocalId,
        param_index_by_local: &FxHashMap<LocalId, usize>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        if !visiting_locals.insert(local) {
            return Ok(FunctionReturnAliasSummary::Unknown);
        }

        let mut alias_param_indices = Vec::new();
        let mut saw_fresh_write = false;
        let mut saw_unknown_write = false;
        let mut saw_any_write = false;

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function.id)?;

            for statement in &block.statements {
                let writer_summary = match &statement.kind {
                    HirStatementKind::Write { target, value } => {
                        let target_local = match target {
                            HirWriteTarget::DefineLocal(target_local) => *target_local,
                            HirWriteTarget::AssignPlace(place)
                                if self
                                    .module
                                    .expressions
                                    .projections(place.projections)
                                    .is_empty() =>
                            {
                                place.root
                            }
                            HirWriteTarget::AssignPlace(_) => continue,
                        };
                        if target_local != local {
                            continue;
                        }

                        Some(self.classify_return_expression_with_visiting(
                            function,
                            reachable_blocks,
                            *value,
                            param_index_by_local,
                            visiting_locals,
                        )?)
                    }
                    HirStatementKind::Call {
                        target,
                        args,
                        result: Some(result_local),
                    } if result_local.local() == local => Some(self.classify_call_result(
                        function,
                        target.clone(),
                        self.module.expressions.values(*args),
                        param_index_by_local,
                        reachable_blocks,
                        visiting_locals,
                    )?),
                    HirStatementKind::MapOp {
                        op,
                        receiver,
                        result: Some(result_local),
                        ..
                    } if result_local.local() == local => Some(self.classify_map_result(
                        *op,
                        *receiver,
                        function,
                        reachable_blocks,
                        param_index_by_local,
                        visiting_locals,
                    )?),
                    HirStatementKind::CastOp {
                        result: Some(result_local),
                        ..
                    }
                    | HirStatementKind::NumericOp {
                        result: result_local,
                        ..
                    }
                    | HirStatementKind::RangeStepFailure {
                        result: result_local,
                        ..
                    }
                    | HirStatementKind::FormatFloat {
                        result: result_local,
                        ..
                    }
                    | HirStatementKind::ValidateFloat {
                        result: result_local,
                        ..
                    } if result_local.local() == local => Some(FunctionReturnAliasSummary::Fresh),
                    HirStatementKind::FloatRangeCandidate {
                        candidate_result,
                        in_range_result,
                        ..
                    } if candidate_result.local() == local || in_range_result.local() == local => {
                        Some(FunctionReturnAliasSummary::Fresh)
                    }
                    _ => None,
                };

                let Some(writer_summary) = writer_summary else {
                    continue;
                };

                saw_any_write = true;
                match writer_summary {
                    FunctionReturnAliasSummary::Fresh => saw_fresh_write = true,
                    FunctionReturnAliasSummary::AliasParams(indices) => {
                        alias_param_indices.extend(indices);
                    }
                    FunctionReturnAliasSummary::Unknown => saw_unknown_write = true,
                }
            }
        }

        visiting_locals.remove(&local);

        alias_param_indices.sort_unstable();
        alias_param_indices.dedup();

        Ok(if !alias_param_indices.is_empty() {
            if saw_fresh_write || saw_unknown_write {
                FunctionReturnAliasSummary::Unknown
            } else {
                FunctionReturnAliasSummary::AliasParams(alias_param_indices)
            }
        } else if saw_unknown_write {
            FunctionReturnAliasSummary::Unknown
        } else if saw_fresh_write || saw_any_write {
            FunctionReturnAliasSummary::Fresh
        } else {
            FunctionReturnAliasSummary::Unknown
        })
    }

    fn classify_unwrapped_success_payload(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        result: HirValueId,
        param_index_by_local: &FxHashMap<LocalId, usize>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        // Direct `return fallible_call()!` unwraps the success payload from a fresh carrier local.
        // The carrier itself is not an alias; payload aliasing comes from the callee metadata and
        // must be projected back through the forwarded call arguments.
        let HirExpressionKind::Load(place) = &self.module.expressions.expression(result).kind
        else {
            return Ok(FunctionReturnAliasSummary::Unknown);
        };
        if !self
            .module
            .expressions
            .projections(place.projections)
            .is_empty()
        {
            return Ok(FunctionReturnAliasSummary::Unknown);
        }

        self.classify_unwrapped_success_local(
            function,
            reachable_blocks,
            place.root,
            param_index_by_local,
            visiting_locals,
        )
    }

    fn classify_unwrapped_success_local(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        result_local: LocalId,
        param_index_by_local: &FxHashMap<LocalId, usize>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        if !visiting_locals.insert(result_local) {
            return Ok(FunctionReturnAliasSummary::Unknown);
        }

        let mut summary = FunctionReturnAliasSummary::Fresh;
        let mut saw_writer = false;

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function.id)?;

            for statement in &block.statements {
                let writer_summary = match &statement.kind {
                    HirStatementKind::Call {
                        target,
                        args,
                        result: Some(call_result),
                        ..
                    } if call_result.local() == result_local => {
                        Some(self.classify_call_success_payload(
                            function,
                            target.clone(),
                            self.module.expressions.values(*args),
                            param_index_by_local,
                            reachable_blocks,
                            visiting_locals,
                        )?)
                    }
                    HirStatementKind::MapOp {
                        op,
                        receiver,
                        result: Some(operation_result),
                        ..
                    } if operation_result.local() == result_local => {
                        Some(self.classify_map_result(
                            *op,
                            *receiver,
                            function,
                            reachable_blocks,
                            param_index_by_local,
                            visiting_locals,
                        )?)
                    }
                    HirStatementKind::Write { target, value }
                        if match target {
                            HirWriteTarget::DefineLocal(local) => *local == result_local,
                            HirWriteTarget::AssignPlace(place) => {
                                place.root == result_local
                                    && self
                                        .module
                                        .expressions
                                        .projections(place.projections)
                                        .is_empty()
                            }
                        } =>
                    {
                        match &self.module.expressions.expression(*value).kind {
                            HirExpressionKind::Load(place)
                                if self
                                    .module
                                    .expressions
                                    .projections(place.projections)
                                    .is_empty() =>
                            {
                                Some(self.classify_unwrapped_success_local(
                                    function,
                                    reachable_blocks,
                                    place.root,
                                    param_index_by_local,
                                    visiting_locals,
                                )?)
                            }
                            _ => Some(FunctionReturnAliasSummary::Unknown),
                        }
                    }
                    HirStatementKind::CastOp {
                        result: Some(operation_result),
                        ..
                    }
                    | HirStatementKind::NumericOp {
                        result: operation_result,
                        ..
                    }
                    | HirStatementKind::RangeStepFailure {
                        result: operation_result,
                        ..
                    }
                    | HirStatementKind::FormatFloat {
                        result: operation_result,
                        ..
                    }
                    | HirStatementKind::ValidateFloat {
                        result: operation_result,
                        ..
                    } if operation_result.local() == result_local => {
                        Some(FunctionReturnAliasSummary::Fresh)
                    }
                    HirStatementKind::FloatRangeCandidate {
                        candidate_result,
                        in_range_result,
                        ..
                    } if candidate_result.local() == result_local
                        || in_range_result.local() == result_local =>
                    {
                        Some(FunctionReturnAliasSummary::Fresh)
                    }
                    _ => None,
                };

                let Some(writer_summary) = writer_summary else {
                    continue;
                };

                saw_writer = true;
                summary = merge_return_alias(summary, writer_summary);
                if matches!(summary, FunctionReturnAliasSummary::Unknown) {
                    visiting_locals.remove(&result_local);
                    return Ok(FunctionReturnAliasSummary::Unknown);
                }
            }
        }

        visiting_locals.remove(&result_local);

        if saw_writer {
            Ok(summary)
        } else {
            Ok(FunctionReturnAliasSummary::Unknown)
        }
    }

    fn classify_call_result(
        &self,
        function: &HirFunction,
        target: CallTarget,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        match target {
            CallTarget::Local(callee_id) if self.function_returns_success_channel(callee_id)? => {
                // The call local carries a fallible result object. Its success payload gets
                // classified only by classify_call_success_payload after an explicit unwrap.
                Ok(FunctionReturnAliasSummary::Fresh)
            }
            CallTarget::Local(callee_id) => self.project_local_call_return_alias(
                function,
                callee_id,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::CrossModule(origin) => self.project_imported_call_return_alias(
                function,
                &origin,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::ModulePrivate(identity) => self.project_module_private_call_return_alias(
                function,
                &identity,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::Generated(identity) => self.project_generated_call_return_alias(
                function,
                &identity,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::External(function_id) => {
                let Some(definition) = self
                    .external_package_registry
                    .get_function_by_id(function_id)
                else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker could not resolve host call target '{}' while classifying a return",
                            function_id.name()
                        ),
                        self.diagnostics.function_error_span(function.id),
                    ));
                };

                if definition.is_fallible() {
                    return Ok(FunctionReturnAliasSummary::Fresh);
                }

                self.project_external_return_alias(
                    function,
                    &definition.hir_return_alias(),
                    args,
                    param_index_by_local,
                    reachable_blocks,
                    visiting_locals,
                )
            }
        }
    }

    fn classify_call_success_payload(
        &self,
        function: &HirFunction,
        target: CallTarget,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        match target {
            CallTarget::Local(callee_id) => self.project_local_call_return_alias(
                function,
                callee_id,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::CrossModule(origin) => self.project_imported_call_return_alias(
                function,
                &origin,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::ModulePrivate(identity) => self.project_module_private_call_return_alias(
                function,
                &identity,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::Generated(identity) => self.project_generated_call_return_alias(
                function,
                &identity,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::External(function_id) => {
                let Some(definition) = self
                    .external_package_registry
                    .get_function_by_id(function_id)
                else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker could not resolve host call target '{}' while classifying a success payload",
                            function_id.name()
                        ),
                        self.diagnostics.function_error_span(function.id),
                    ));
                };

                let [return_slot] = definition.returns.as_slice() else {
                    // The compact borrow summary has no per-slot projection for a multi-return
                    // external boundary, so keep this genuinely imprecise shape conservative.
                    return Ok(FunctionReturnAliasSummary::Unknown);
                };

                self.project_external_return_alias(
                    function,
                    &return_slot.alias,
                    args,
                    param_index_by_local,
                    reachable_blocks,
                    visiting_locals,
                )
            }
        }
    }

    fn project_local_call_return_alias(
        &self,
        function: &HirFunction,
        callee_id: FunctionId,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let Some(callee_summary) = self.public_call_summaries.get(&callee_id) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the public call summary for function '{}' while classifying a forwarded return",
                    self.diagnostics.function_name(callee_id)
                ),
                self.diagnostics.function_error_span(function.id),
            ));
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &callee_summary.return_alias,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: &format!(
                    "user function '{}'",
                    self.diagnostics.function_name(callee_id)
                ),
            },
            visiting_locals,
        )
    }

    fn project_imported_call_return_alias(
        &self,
        function: &HirFunction,
        origin: &crate::compiler_frontend::semantic_identity::OriginFunctionId,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let Some(summary) = self.module.imported_call_summaries.get(origin) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the provider call summary for imported function {origin:?} while classifying a forwarded return"
                ),
                self.diagnostics.function_error_span(function.id),
            ));
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &summary.return_alias,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: "imported function",
            },
            visiting_locals,
        )
    }

    fn project_generated_call_return_alias(
        &self,
        function: &HirFunction,
        identity: &crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let Some(summary) = self.module.generated_call_summaries.get(identity) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the call summary for generated function {identity:?} while classifying a forwarded return"
                ),
                self.diagnostics.function_error_span(function.id),
            ));
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &summary.return_alias,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: "generated function",
            },
            visiting_locals,
        )
    }

    fn project_module_private_call_return_alias(
        &self,
        function: &HirFunction,
        identity: &crate::compiler_frontend::semantic_identity::ModulePrivateExecutableIdentity,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let Some(summary) = self.module.module_private_call_summaries.get(identity) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the call summary for module-private function {identity:?} while classifying a forwarded return"
                ),
                self.diagnostics.function_error_span(function.id),
            ));
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &summary.return_alias,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: "module-private function",
            },
            visiting_locals,
        )
    }

    fn project_external_return_alias(
        &self,
        function: &HirFunction,
        return_alias: &ExternalReturnAlias,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let alias_summary = match return_alias {
            ExternalReturnAlias::Fresh => FunctionReturnAliasSummary::Fresh,
            ExternalReturnAlias::AliasArgs(indices) => {
                FunctionReturnAliasSummary::AliasParams(indices.clone())
            }
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &alias_summary,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: "external function",
            },
            visiting_locals,
        )
    }

    fn project_alias_summary_through_arguments(
        &self,
        context: AliasProjectionContext<'_>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let AliasProjectionContext {
            function,
            return_alias,
            args,
            param_index_by_local,
            reachable_blocks,
            callee_description,
        } = context;

        let alias_arg_indices = match return_alias {
            FunctionReturnAliasSummary::Fresh => return Ok(FunctionReturnAliasSummary::Fresh),
            FunctionReturnAliasSummary::Unknown => return Ok(FunctionReturnAliasSummary::Unknown),
            FunctionReturnAliasSummary::AliasParams(indices) => indices,
        };

        let mut caller_param_indices = Vec::new();
        for arg_index in alias_arg_indices {
            let Some(argument) = args.get(*arg_index) else {
                return Err(self.diagnostics.internal_error(
                    format!(
                        "Return alias metadata for function '{}' references call argument index {} but the forwarded call only has {} argument(s)",
                        callee_description,
                        arg_index,
                        args.len()
                    ),
                    self.diagnostics.function_error_span(function.id),
                ));
            };

            match self.classify_return_expression_with_visiting(
                function,
                reachable_blocks,
                *argument,
                param_index_by_local,
                visiting_locals,
            )? {
                FunctionReturnAliasSummary::Fresh => {}
                FunctionReturnAliasSummary::AliasParams(indices) => {
                    caller_param_indices.extend(indices);
                }
                FunctionReturnAliasSummary::Unknown => {
                    return Ok(FunctionReturnAliasSummary::Unknown);
                }
            }
        }

        caller_param_indices.sort_unstable();
        caller_param_indices.dedup();

        Ok(if caller_param_indices.is_empty() {
            FunctionReturnAliasSummary::Fresh
        } else {
            FunctionReturnAliasSummary::AliasParams(caller_param_indices)
        })
    }

    fn classify_map_result(
        &self,
        op: crate::compiler_frontend::hir::expressions::HirMapOp,
        receiver: HirValueId,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        if !matches!(
            op,
            crate::compiler_frontend::hir::expressions::HirMapOp::Get
        ) {
            return Ok(FunctionReturnAliasSummary::Fresh);
        }

        self.classify_return_expression_with_visiting(
            function,
            reachable_blocks,
            receiver,
            param_index_by_local,
            visiting_locals,
        )
    }

    fn function_returns_success_channel(
        &self,
        function_id: FunctionId,
    ) -> Result<bool, BorrowCheckError> {
        let Some(function) = self
            .module
            .functions
            .iter()
            .find(|function| function.id == function_id)
        else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker could not resolve local function '{}' while classifying a call result",
                    self.diagnostics.function_name(function_id)
                ),
                self.diagnostics.function_error_span(function_id),
            ));
        };

        let reachable_blocks = self.collect_reachable_blocks(function)?;
        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(block_id, function.id)?;
            if matches!(block.terminator, HirTerminator::ReturnSuccess(_)) {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

fn merge_return_alias(
    left: FunctionReturnAliasSummary,
    right: FunctionReturnAliasSummary,
) -> FunctionReturnAliasSummary {
    // Conservative join:
    // Unknown dominates; Fresh is neutral; AliasParams unions indices.
    match (left, right) {
        (FunctionReturnAliasSummary::Unknown, _) | (_, FunctionReturnAliasSummary::Unknown) => {
            FunctionReturnAliasSummary::Unknown
        }

        (FunctionReturnAliasSummary::Fresh, other) | (other, FunctionReturnAliasSummary::Fresh) => {
            other
        }

        (
            FunctionReturnAliasSummary::AliasParams(mut left),
            FunctionReturnAliasSummary::AliasParams(right),
        ) => {
            left.extend(right);
            left.sort_unstable();
            left.dedup();
            FunctionReturnAliasSummary::AliasParams(left)
        }
    }
}
