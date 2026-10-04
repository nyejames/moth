//! HIR fallible-expression lowering.
//!
//! WHAT: lowers postfix propagation, `catch` handling, and the temporary fallible carrier
//! branches used at the HIR boundary.
//! WHY: fallible calls are control flow, not ordinary call values. Keeping this code separate from
//! plain call lowering makes the success/error CFG joins explicit and confines the temporary
//! fallible carrier machinery to one HIR-owned lowering module.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` / `return_hir_transformation_error!` in this module means an internal
//! HIR transformation or lowering invariant failure only.
//!
//! Submodule map:
//! - `carrier`: carrier slot lookup, branch creation, success/error unwrap helpers, and postfix
//!   error payload wrapping.
//! - `propagation`: statement-position `call()!`, value-position `expr!`, and nested expression
//!   propagation.
//! - `catch`: catch handler CFG lowering, binding, merge behavior, and catch block fallthrough.
//! - `external`: external fallible call carrier creation and metadata checks used only by HIR lowering.
//! - `direct_return`: `return fallible_call()!` success/error direct return branches.

use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::LoweredExpression;

mod carrier;
mod catch;
mod direct_return;
mod external;
mod propagation;

pub(crate) use self::carrier::EmittedFallibleCarrier;
pub(crate) use self::external::ExternalFallibleCallLoweringInput;

/// Shared result locals and handler metadata for expression-wide recovery.
pub(crate) struct FallibleBranchingContext<'a> {
    pub(crate) result_type_ids: &'a [FrontendTypeId],
    pub(crate) handling: &'a FallibleHandling,
    pub(crate) err_type: TypeId,
    pub(crate) span: &'a Option<SourceSpan>,
}

impl<'a> HirBuilder<'a> {
    /// Lowers a handled fallible expression (value-position `expr!` or `expr catch:`).
    pub(crate) fn lower_handled_fallible_expression(
        &mut self,
        value: &Expression,
        handling: &FallibleExpressionHandling,
        value_span: &Option<SourceSpan>,
        propagation_span: &Option<SourceSpan>,
        expr_type_id: FrontendTypeId,
    ) -> Result<LoweredExpression, CompilerError> {
        let lowered = self.lower_expression(value)?;
        let ok_type = match self
            .type_environment
            .fallible_carrier_slots(lowered.value.ty)
        {
            Some((ok, _)) => ok,
            None => {
                return_hir_transformation_error!(
                    "Handled fallible expression reached HIR lowering without an internal carrier type",
                    self.hir_error_location(value_span)
                );
            }
        };

        let result_type_ids = self.handled_expression_result_type_ids(expr_type_id);
        let expected_ok_type = self.lower_call_result_type(&result_type_ids, value_span)?;
        if expected_ok_type != ok_type {
            return_hir_transformation_error!(
                "Handled fallible expression lowered with mismatched success type",
                self.hir_error_location(value_span)
            );
        }

        if matches!(handling, FallibleExpressionHandling::Propagate) {
            let result_carrier =
                self.emit_lowered_result_expression_to_current_block(lowered, value_span)?;
            let mut success_value =
                self.lower_fallible_carrier_to_success_value(result_carrier, propagation_span)?;
            success_value.span = *value_span;
            self.side_table
                .map_value(*value_span, success_value.id, success_value.span);

            return Ok(LoweredExpression {
                prelude: vec![],
                value: success_value,
            });
        }

        return_hir_transformation_error!(
            "Recovering fallible expression reached HIR outside a value catch block",
            self.hir_error_location(value_span)
        )
    }

    pub(crate) fn lower_recovering_fallible_expression(
        &mut self,
        value: &Expression,
        result_type_ids: &[FrontendTypeId],
        source_span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, CompilerError> {
        let lowered = self.lower_expression(value)?;
        let result_carrier =
            self.emit_lowered_result_expression_to_current_block(lowered, source_span)?;
        let expected_ok_type = self.lower_call_result_type(result_type_ids, source_span)?;
        if expected_ok_type != result_carrier.ok_type {
            return_hir_transformation_error!(
                "Recovering fallible expression lowered with mismatched success type",
                self.hir_error_location(source_span)
            );
        }
        let value = self.lower_carrier_to_active_catch_success(result_carrier, source_span)?;
        Ok(LoweredExpression { prelude: vec![], value })
    }

    pub(crate) fn lower_handled_fallible_call_expression(
        &mut self,
        target: CallTarget,
        args: &[CallArgument],
        result_type_ids: &[FrontendTypeId],
        handling: &FallibleExpressionHandling,
        call_span: &Option<SourceSpan>,
        propagation_span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, CompilerError> {
        let (_, ok_type, _) = self.result_call_carrier_slots(&target, call_span)?;

        let requested_ok_type = self.lower_call_result_type(result_type_ids, call_span)?;
        if requested_ok_type != ok_type {
            return_hir_transformation_error!(
                "Handled fallible call lowered with mismatched success type",
                self.hir_error_location(call_span)
            );
        }

        if matches!(handling, FallibleExpressionHandling::Propagate) {
            let result_carrier = self.emit_result_call_carrier_to_current_block(
                target,
                args,
                result_type_ids,
                call_span,
            )?;
            let mut success_value =
                self.lower_fallible_carrier_to_success_value(result_carrier, propagation_span)?;
            success_value.span = *call_span;
            self.side_table
                .map_value(*call_span, success_value.id, success_value.span);
            self.log_call_result_binding(call_span, None, &success_value);

            return Ok(LoweredExpression {
                prelude: vec![],
                value: success_value,
            });
        }

        return_hir_transformation_error!(
            "Recovering fallible call reached HIR outside a value catch block",
            self.hir_error_location(call_span)
        )
    }

    pub(crate) fn lower_handled_external_fallible_call_expression(
        &mut self,
        input: ExternalFallibleCallLoweringInput<'_>,
    ) -> Result<LoweredExpression, CompilerError> {
        let ExternalFallibleCallLoweringInput {
            id,
            args,
            result_type_ids,
            error_type_id,
            handling,
            call_span,
            propagation_span,
        } = input;

        if matches!(handling, FallibleExpressionHandling::Propagate) {
            let result_carrier = self.emit_external_result_call_carrier_to_current_block(
                id,
                args,
                result_type_ids,
                error_type_id,
                call_span,
            )?;
            let mut success_value =
                self.lower_fallible_carrier_to_success_value(result_carrier, propagation_span)?;
            success_value.span = *call_span;
            self.side_table
                .map_value(*call_span, success_value.id, success_value.span);
            self.log_call_result_binding(call_span, None, &success_value);

            return Ok(LoweredExpression {
                prelude: vec![],
                value: success_value,
            });
        }

        return_hir_transformation_error!(
            "Recovering external fallible call reached HIR outside a value catch block",
            self.hir_error_location(call_span)
        )
    }
}
