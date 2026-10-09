//! Fallible propagation lowering.
//!
//! WHAT: statement-position `call()!`, value-position `expr!`, and nested expression propagation.
//! WHY: postfix propagation is control flow, not a value expression. These helpers emit the
//! explicit success/error CFG edges that borrow validation and backend lowering expect.

use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling,
};
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::statements::HirWriteTarget;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::carrier::EmittedFallibleCarrier;

impl<'a> HirBuilder<'a> {
    /// Lowers value-position `expr!` when the surrounding statement owns the continuation.
    pub(crate) fn lower_fallible_expression_to_success_value(
        &mut self,
        value: &Expression,
        value_span: &Option<SourceSpan>,
    ) -> Result<Option<HirValueId>, HirConstructionFailure> {
        let Some(result_carrier) = self.emit_result_propagation_carrier_to_current_block(value)?
        else {
            return Ok(None);
        };

        let propagation_span = value.propagation_span().or(*value_span);
        let success_value =
            self.lower_fallible_carrier_to_success_value(result_carrier, &propagation_span)?;
        let success_value =
            self.replace_expression_metadata(success_value, value_span, None, None)?;
        Ok(Some(success_value))
    }

    /// Builds the list of result type IDs for a handled expression.
    pub(crate) fn handled_expression_result_type_ids(
        &self,
        expr_type_id: FrontendTypeId,
    ) -> Vec<FrontendTypeId> {
        if expr_type_id == self.type_environment.builtins().none {
            return vec![];
        }

        self.type_environment
            .tuple_field_ids(expr_type_id)
            .map_or_else(|| vec![expr_type_id], ToOwned::to_owned)
    }

    /// Emits a fallible call carrier after evaluating its operands in source order.
    pub(in crate::compiler_frontend::hir) fn emit_result_call_carrier_to_current_block(
        &mut self,
        target: CallTarget,
        args: &[CallArgument],
        result_type_ids: &[FrontendTypeId],
        call_span: &Option<SourceSpan>,
    ) -> Result<EmittedFallibleCarrier, HirConstructionFailure> {
        let (carrier_type, ok_type, err_type) =
            self.result_call_carrier_slots(&target, call_span)?;
        let requested_ok_type = self.lower_call_result_type(result_type_ids, call_span)?;

        if requested_ok_type != ok_type {
            return_hir_transformation_error!(
                "Direct fallible propagation return lowered with mismatched success type",
                self.hir_error_location(call_span)
            );
        }

        let result_local =
            self.emit_result_call_to_current_block(target, args, carrier_type, call_span)?;

        Ok(EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type,
            err_type,
            validate_float_success: false,
        })
    }

    /// Emits a fallible expression carrier for direct propagation.
    pub(super) fn emit_result_expression_to_current_block(
        &mut self,
        value: &Expression,
    ) -> Result<EmittedFallibleCarrier, HirConstructionFailure> {
        let lowered = self.lower_expression(value)?;
        self.emit_lowered_result_expression_to_current_block(lowered, &value.span)
    }

    /// Emits an already-lowered fallible expression carrier to the current block.
    pub(super) fn emit_lowered_result_expression_to_current_block(
        &mut self,
        lowered: super::super::LoweredExpression,
        span: &Option<SourceSpan>,
    ) -> Result<EmittedFallibleCarrier, HirConstructionFailure> {
        let lowered_type = self.module.expressions.expression(lowered.value).ty;
        let Some((ok_type, err_type)) = self.type_environment.fallible_carrier_slots(lowered_type)
        else {
            return_hir_transformation_error!(
                "Fallible expression reached HIR lowering without an internal carrier type",
                self.hir_error_location(span)
            );
        };
        let carrier_type = lowered_type;

        for prelude in lowered.prelude {
            self.emit_statement_to_current_block(prelude, span)?;
        }

        let result_local = self.allocate_temp_local(carrier_type, None)?;
        self.emit_write_statement(
            HirWriteTarget::DefineLocal(result_local),
            lowered.value,
            span,
        )?;

        Ok(EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type,
            err_type,
            validate_float_success: false,
        })
    }

    /// Probes an expression for postfix propagation and emits the carrier if present.
    pub(super) fn emit_result_propagation_carrier_to_current_block(
        &mut self,
        value: &Expression,
    ) -> Result<Option<EmittedFallibleCarrier>, HirConstructionFailure> {
        match &value.kind {
            ExpressionKind::HandledFallibleFunctionCall {
                name,
                args,
                result_type_ids,
                handling: FallibleExpressionHandling::Propagate,
                ..
            } => {
                let target = self.resolve_call_target_or_error(name, &value.span)?;
                let carrier = self.emit_result_call_carrier_to_current_block(
                    target,
                    args,
                    result_type_ids,
                    &value.span,
                )?;

                Ok(Some(carrier))
            }

            ExpressionKind::HandledFallibleHostFunctionCall {
                id,
                args,
                result_type_ids,
                requires_external_float_validation,
                error_type_id,
                handling: FallibleExpressionHandling::Propagate,
                ..
            } => Ok(Some(
                self.emit_external_result_call_carrier_to_current_block(
                    *id,
                    args,
                    result_type_ids,
                    *requires_external_float_validation,
                    *error_type_id,
                    &value.span,
                )?,
            )),

            ExpressionKind::HandledFallibleExpression {
                value: result_value,
                handling: FallibleExpressionHandling::Propagate,
                ..
            } => Ok(Some(
                self.emit_result_expression_to_current_block(result_value)?,
            )),

            _ => Ok(None),
        }
    }
}
