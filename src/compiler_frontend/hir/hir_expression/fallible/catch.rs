//! Expression-local catch CFG lowering.
//!
//! Protected producers branch through error-only adapters into one shared handler. The
//! previous continuation is restored before lowering that handler, so its failures go outward.

use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::hir_builder::{
    CatchHandlerTarget, HirBuilder, ValueBlockTarget,
};
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::statements::HirWriteTarget;
use crate::compiler_frontend::hir::terminators::{HirJumpArgument, HirTerminator};
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::super::LoweredResults;
use super::{EmittedFallibleResults, FallibleBranchingContext};

impl<'a> HirBuilder<'a> {
    /// Installs the shared handler before evaluating any protected operands, then merges
    /// successful values and handler `then` values through the same result locals.
    pub(crate) fn lower_fallible_results_with_branching(
        &mut self,
        context: FallibleBranchingContext<'_>,
        emit_protected: impl FnOnce(
            &mut HirBuilder<'_>,
        ) -> Result<LoweredResults, HirConstructionFailure>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let FallibleBranchingContext {
            expressions,
            result_type_ids,
            handling,
            err_type,
            span,
        } = context;
        let source_block = self.current_block_id_or_error(span)?;
        let region = self.current_region_or_error(span)?;
        let error_region = self.create_child_region(region);
        let error_block = self.create_block(error_region, span, "fallible-handled-err")?;
        let merge_block = self.create_block(region, span, "fallible-handled-merge")?;
        // Edge destinations belong to their target blocks, not the protected predecessor.
        self.set_current_block(error_block, span)?;
        let error_local = self.allocate_temp_local(err_type, None)?;
        self.set_current_block(merge_block, span)?;
        let mut result_locals = Vec::with_capacity(result_type_ids.len());
        for type_id in result_type_ids {
            let result_type = self.lower_type_id(*type_id, span)?;
            result_locals.push(self.allocate_temp_local(result_type, None)?);
        }
        self.set_current_block(source_block, span)?;

        let lowered = self.with_active_catch_handler(
            CatchHandlerTarget {
                block: error_block,
                error_local,
                error_type: err_type,
            },
            emit_protected,
        )?;
        for statement in lowered.prelude {
            self.emit_statement_to_current_block(statement, span)?;
        }

        // The merge edge transfers each slot only after all protected work succeeds.
        let mut args = Vec::with_capacity(result_locals.len());
        for (destination, value) in result_locals.iter().copied().zip(lowered.values) {
            let ty = self.module.expressions.expression(value).ty;
            let source = self.allocate_temp_local(ty, None)?;
            self.emit_write_statement(HirWriteTarget::DefineLocal(source), value, span)?;
            args.push(HirJumpArgument {
                source,
                destination,
            });
        }
        let success_tail = self.current_block_id_or_error(span)?;
        self.emit_terminator(
            success_tail,
            HirTerminator::Jump {
                target: merge_block,
                args,
            },
            span,
        )?;

        // The enclosing handler is active again here, never this catch's own handler.
        self.set_current_block(error_block, span)?;
        match handling {
            FallibleHandling::Handler { error, body } => {
                if let Some(error_binding) = error {
                    let handler_error_local = self.allocate_named_local(
                        error_binding.error_binding.to_owned(),
                        err_type,
                        false,
                        None,
                    )?;
                    let error_payload = self.make_local_load_expression(
                        error_local,
                        err_type,
                        &None,
                        error_region,
                    )?;
                    self.emit_write_statement(
                        HirWriteTarget::DefineLocal(handler_error_local),
                        error_payload,
                        span,
                    )?;
                }
                if result_locals.is_empty() {
                    self.lower_statement_sequence(expressions, body)?;
                } else {
                    self.with_active_value_block_target(
                        ValueBlockTarget {
                            result_locals: result_locals.clone(),
                            merge_block,
                        },
                        |builder| builder.lower_statement_sequence(expressions, body),
                    )?;
                }
                let error_tail = self.current_block_id_or_error(span)?;
                if !self.block_has_explicit_terminator(error_tail, span)? {
                    if !result_locals.is_empty() {
                        return_hir_transformation_error!(
                            "Catch handler reached HIR fallthrough while a value continuation is required",
                            self.hir_error_location(span)
                        );
                    }
                    self.emit_jump_to(
                        error_tail,
                        merge_block,
                        span,
                        "fallible-handled.error.merge",
                    )?;
                }
            }
            FallibleHandling::Propagate => return_hir_transformation_error!(
                "Propagation handling unexpectedly reached fallible branching lowering",
                self.hir_error_location(span)
            ),
        }

        self.set_current_block(merge_block, span)?;
        let values =
            self.value_block_result_expressions(&result_locals, result_type_ids, span, region)?;
        Ok(LoweredResults {
            prelude: vec![],
            values,
        })
    }

    /// Error adapters transfer only the error local, never success slots.
    pub(crate) fn lower_results_to_active_catch_success_values(
        &mut self,
        results: EmittedFallibleResults,
        span: &Option<SourceSpan>,
    ) -> Result<Vec<HirValueId>, HirConstructionFailure> {
        let Some(handler) = self.active_catch_handler else {
            return_hir_transformation_error!(
                "Producer lowered without an active catch",
                self.hir_error_location(span)
            );
        };
        if handler.error_type != results.err_type {
            return_hir_transformation_error!(
                "Producer error type does not match catch",
                self.hir_error_location(span)
            );
        }
        self.set_current_block(results.error_block, span)?;
        self.emit_terminator(
            results.error_block,
            HirTerminator::Jump {
                target: handler.block,
                args: vec![HirJumpArgument {
                    source: results.error_local,
                    destination: handler.error_local,
                }],
            },
            span,
        )?;
        self.fallible_success_values(&results, span)
    }

    pub(crate) fn lower_results_to_active_catch_success_value(
        &mut self,
        results: EmittedFallibleResults,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let values = self.lower_results_to_active_catch_success_values(results, span)?;
        self.single_result_value(values, span)
    }
}
