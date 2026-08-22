//! Fallible propagation lowering.
//!
//! WHAT: statement-position `call()!`, value-position `expr!`, and nested expression propagation.
//! WHY: postfix propagation is control flow, not a value expression. These helpers emit the
//! explicit success/error CFG edges that borrow validation and backend lowering expect.

use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::return_hir_transformation_error;

use super::carrier::EmittedFallibleCarrier;

impl<'a> HirBuilder<'a> {
    /// Lowers value-position `expr!` when the surrounding statement owns the continuation.
    ///
    /// WHAT: emits the fallible carrier, branches on success/error, returns the error edge from
    /// the current function, and leaves the builder positioned on the success continuation with
    /// the unwrapped success payload available to the caller.
    /// WHY: declaration, assignment, and multi-bind boundaries need a value to continue with, but
    /// the propagation itself is control flow. Keeping the split here prevents those statement
    /// lowerers from manufacturing expression-only propagation nodes.
    pub(crate) fn lower_fallible_expression_to_success_value(
        &mut self,
        value: &Expression,
        location: &SourceLocation,
    ) -> Result<Option<HirExpression>, CompilerError> {
        let Some(result_carrier) = self.emit_result_propagation_carrier_to_current_block(value)?
        else {
            return Ok(None);
        };

        let propagation_location = value.propagation_location().unwrap_or(location);
        self.lower_fallible_carrier_to_success_value(result_carrier, propagation_location)
            .map(Some)
    }

    /// Builds the list of result type IDs for a handled expression.
    ///
    /// WHAT: converts the expression's type into a vector of success-slot type IDs.
    /// WHY: multi-success fallible calls return tuples, and the fallback path needs the same arity.
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

    /// Emits a fallible call carrier for direct propagation and returns its metadata.
    ///
    /// WHAT: looks up carrier slots, validates the success type, emits the call, and packages the
    /// result local into an `EmittedFallibleCarrier`.
    pub(super) fn emit_result_call_carrier_to_current_block(
        &mut self,
        target: CallTarget,
        args: &[CallArgument],
        result_type_ids: &[FrontendTypeId],
        location: &SourceLocation,
    ) -> Result<EmittedFallibleCarrier, CompilerError> {
        let (carrier_type, ok_type, err_type) =
            self.result_call_carrier_slots(&target, location)?;
        let requested_ok_type = self.lower_call_result_type(result_type_ids, location)?;

        if requested_ok_type != ok_type {
            return_hir_transformation_error!(
                "Direct fallible propagation return lowered with mismatched success type",
                self.hir_error_location(location)
            );
        }

        let result_local =
            self.emit_result_call_to_current_block(target, args, carrier_type, location)?;

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
        location: &SourceLocation,
    ) -> Result<EmittedFallibleCarrier, CompilerError> {
        let lowered = self.lower_expression(value)?;
        self.emit_lowered_result_expression_to_current_block(lowered, location)
    }

    /// Emits an already-lowered fallible expression carrier to the current block.
    pub(super) fn emit_lowered_result_expression_to_current_block(
        &mut self,
        lowered: super::super::LoweredExpression,
        location: &SourceLocation,
    ) -> Result<EmittedFallibleCarrier, CompilerError> {
        let Some((ok_type, err_type)) = self
            .type_environment
            .fallible_carrier_slots(lowered.value.ty)
        else {
            return_hir_transformation_error!(
                "Fallible expression reached HIR lowering without an internal carrier type",
                self.hir_error_location(location)
            );
        };
        let carrier_type = lowered.value.ty;

        for prelude in lowered.prelude {
            self.emit_statement_to_current_block(prelude, location)?;
        }

        let result_local = self.allocate_temp_local(carrier_type, Some(location.to_owned()))?;
        self.emit_assign_local_statement(result_local, lowered.value, location)?;

        Ok(EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type,
            err_type,
            validate_float_success: false,
        })
    }

    /// Probes an expression for postfix propagation and emits the carrier if present.
    ///
    /// WHAT: matches on `HandledFallibleFunctionCall`, `HandledFallibleHostFunctionCall`, and
    /// `HandledFallibleExpression` with `Propagate` handling, emitting the carrier for each.
    /// WHY: nested expression propagation needs a uniform probe so callers can decide whether to
    /// branch or fall back to ordinary lowering.
    pub(super) fn emit_result_propagation_carrier_to_current_block(
        &mut self,
        value: &Expression,
    ) -> Result<Option<EmittedFallibleCarrier>, CompilerError> {
        match &value.kind {
            ExpressionKind::HandledFallibleFunctionCall {
                name,
                args,
                result_type_ids,
                handling: FallibleExpressionHandling::Propagate,
                ..
            } => {
                let target = self.resolve_call_target_or_error(name, &value.location)?;
                let carrier = self.emit_result_call_carrier_to_current_block(
                    target,
                    args,
                    result_type_ids,
                    &value.location,
                )?;

                Ok(Some(carrier))
            }

            ExpressionKind::HandledFallibleHostFunctionCall {
                id,
                args,
                result_type_ids,
                error_type_id,
                handling: FallibleExpressionHandling::Propagate,
                ..
            } => Ok(Some(
                self.emit_external_result_call_carrier_to_current_block(
                    *id,
                    args,
                    result_type_ids,
                    *error_type_id,
                    &value.location,
                )?,
            )),

            ExpressionKind::HandledFallibleExpression {
                value: result_value,
                handling: FallibleExpressionHandling::Propagate,
                ..
            } => Ok(Some(self.emit_result_expression_to_current_block(
                result_value,
                &result_value.location,
            )?)),

            _ => Ok(None),
        }
    }
}
