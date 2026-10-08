//! Expression-local catch CFG lowering.
//!
//! Protected producers branch through error-only adapters into one shared handler. The
//! previous continuation is restored before lowering that handler, so its failures go outward.

use crate::compiler_frontend::ast::expressions::expression::FallibleHandling;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::{
    CatchHandlerTarget, HirBuilder, ValueBlockTarget,
};
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::statements::HirWriteTarget;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::super::LoweredExpression;
use super::{EmittedFallibleCarrier, FallibleBranchingContext};

impl<'a> HirBuilder<'a> {
    /// Installs the shared handler before evaluating any protected operands, then merges
    /// successful values and handler `then` values through the same result locals.
    pub(crate) fn lower_fallible_carrier_with_branching(
        &mut self,
        context: FallibleBranchingContext<'_>,
        emit_protected: impl FnOnce(
            &mut HirBuilder<'_>,
        ) -> Result<LoweredExpression, HirConstructionFailure>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let FallibleBranchingContext {
            result_type_ids,
            handling,
            err_type,
            span,
        } = context;
        let region = self.current_region_or_error(span)?;
        let error_region = self.create_child_region(region);
        let error_block = self.create_block(error_region, span, "fallible-handled-err")?;
        let merge_block = self.create_block(region, span, "fallible-handled-merge")?;
        let error_local = self.allocate_temp_local(err_type, None)?;
        let mut result_locals = Vec::with_capacity(result_type_ids.len());
        for type_id in result_type_ids {
            let result_type = self.lower_type_id(*type_id, span)?;
            result_locals.push(self.allocate_temp_local(result_type, None)?);
        }

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

        // No success payload or merge-local write is executed on a producer's error edge.
        match result_locals.as_slice() {
            [] => {}
            [result_local] => self.emit_write_statement(
                HirWriteTarget::DefineLocal(*result_local),
                lowered.value,
                span,
            )?,
            _ => {
                let tuple_type = self.module.expressions.expression(lowered.value).ty;
                let tuple_local = self.allocate_temp_local(tuple_type, None)?;
                self.emit_write_statement(
                    HirWriteTarget::DefineLocal(tuple_local),
                    lowered.value,
                    span,
                )?;
                for (index, result_local) in result_locals.iter().enumerate() {
                    let slot_type = self.lower_type_id(result_type_ids[index], span)?;
                    let success_region = self.current_region_or_error(span)?;
                    let tuple = self.make_local_load_expression(
                        tuple_local,
                        tuple_type,
                        &None,
                        success_region,
                    )?;
                    let value = self.make_expression(
                        &None,
                        HirExpressionKind::TupleGet { tuple, index },
                        slot_type,
                        ValueKind::RValue,
                        success_region,
                    )?;
                    self.emit_write_statement(
                        HirWriteTarget::DefineLocal(*result_local),
                        value,
                        span,
                    )?;
                }
            }
        }
        let success_tail = self.current_block_id_or_error(span)?;
        self.emit_jump_to(
            success_tail,
            merge_block,
            span,
            "fallible-handled.success.merge",
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
                    self.lower_statement_sequence(body)?;
                } else {
                    self.with_active_value_block_target(
                        ValueBlockTarget {
                            result_locals: result_locals.clone(),
                            merge_block,
                        },
                        |builder| builder.lower_statement_sequence(body),
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
        let value = if result_locals.is_empty() {
            self.unit_expression(span, region)?
        } else {
            self.value_block_result_expression(&result_locals, result_type_ids, span, region)?
        };
        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    /// Each carrier retains its own success type. Only its error edge transfers the
    /// compatible error payload into the handler's shared local.
    pub(crate) fn lower_carrier_to_active_catch_success(
        &mut self,
        carrier: EmittedFallibleCarrier,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let Some(handler) = self.active_catch_handler else {
            return_hir_transformation_error!(
                "Catch carrier lowered without an active handler",
                self.hir_error_location(span)
            );
        };
        if handler.error_type != carrier.err_type {
            return_hir_transformation_error!(
                "Catch carrier error type does not match its handler",
                self.hir_error_location(span)
            );
        }
        let branch = self.emit_result_carrier_branch(
            carrier.result_local,
            carrier.carrier_type,
            span,
            *span,
            "fallible-handled-ok",
            "fallible-handled-error-transfer",
        )?;
        self.set_current_block(branch.error_block, span)?;
        let error_region = self.current_region_or_error(span)?;
        let error_result = self.make_local_load_expression(
            carrier.result_local,
            carrier.carrier_type,
            &None,
            error_region,
        )?;
        let error_payload = self.make_expression(
            &None,
            HirExpressionKind::FallibleUnwrapError {
                result: error_result,
            },
            carrier.err_type,
            ValueKind::RValue,
            error_region,
        )?;
        self.emit_write_statement(
            HirWriteTarget::DefineLocal(handler.error_local),
            error_payload,
            span,
        )?;
        self.emit_jump_to(
            branch.error_block,
            handler.block,
            span,
            "fallible-handled.error.handler",
        )?;

        self.set_current_block(branch.success_block, span)?;
        let success_region = self.current_region_or_error(span)?;
        let success_result = self.make_local_load_expression(
            carrier.result_local,
            carrier.carrier_type,
            &None,
            success_region,
        )?;
        let success_payload = self.make_expression(
            &None,
            HirExpressionKind::FallibleUnwrapSuccess {
                result: success_result,
            },
            carrier.ok_type,
            ValueKind::RValue,
            success_region,
        )?;
        if carrier.validate_float_success {
            self.emit_validated_float_value(success_payload, span)
        } else {
            Ok(success_payload)
        }
    }
}
