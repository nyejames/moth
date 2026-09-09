//! Direct return propagation lowering.
//!
//! WHAT: `return fallible_call()!` and `return expr!` into explicit success/error return edges.
//! WHY: direct-return propagation is a control-flow operation. HIR should expose both return
//! edges instead of hiding the error path inside an expression helper.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::type_coercion::compatibility::is_postfix_error_compatible;
use crate::return_hir_transformation_error;

use super::carrier::EmittedFallibleCarrier;

impl<'a> HirBuilder<'a> {
    /// Lowers direct `return fallible_expression!` into explicit success/error return edges.
    pub(crate) fn lower_fallible_propagating_direct_return(
        &mut self,
        value: &Expression,
        value_span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<bool, CompilerError> {
        let Some(result_carrier) = self.emit_result_propagation_carrier_to_current_block(value)?
        else {
            return Ok(false);
        };

        let propagation_span = value.propagation_span().or(*value_span);
        self.emit_result_carrier_direct_return(result_carrier, &propagation_span, authored_span)?;
        Ok(true)
    }

    pub(super) fn emit_result_carrier_direct_return(
        &mut self,
        result_carrier: EmittedFallibleCarrier,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let current_function_id = self.current_function_id_or_error(span)?;
        let current_return_type = self
            .function_by_id_or_error(current_function_id, span)?
            .return_type;
        let Some((current_ok_type, current_error_type)) = self
            .type_environment
            .fallible_carrier_slots(current_return_type)
        else {
            return_hir_transformation_error!(
                "Direct fallible propagation return reached HIR outside a fallible function",
                self.hir_error_location(span)
            );
        };

        if current_ok_type != result_carrier.ok_type {
            return_hir_transformation_error!(
                "Direct fallible propagation success type does not match the enclosing function",
                self.hir_error_location(span)
            );
        }

        if !is_postfix_error_compatible(
            current_error_type,
            result_carrier.err_type,
            &self.type_environment,
        ) {
            return_hir_transformation_error!(
                "Direct fallible propagation error type does not match the enclosing function",
                self.hir_error_location(span)
            );
        }

        let branch = self.emit_result_carrier_branch(
            result_carrier.result_local,
            result_carrier.carrier_type,
            span,
            authored_span,
            "return-fallible-ok",
            "return-fallible-err",
        )?;

        self.set_current_block(branch.success_block, span)?;
        let success_region = self.current_region_or_error(span)?;
        let success_result = self.make_local_load_expression(
            result_carrier.result_local,
            result_carrier.carrier_type,
            &None,
            success_region,
        );
        let mut success_payload = self.make_expression(
            &None,
            HirExpressionKind::FallibleUnwrapSuccess {
                result: Box::new(success_result),
            },
            result_carrier.ok_type,
            ValueKind::RValue,
            success_region,
        );
        if result_carrier.validate_float_success {
            success_payload = self.emit_validated_float_value(success_payload, span)?;
        }
        self.emit_terminator_with_span(
            branch.success_block,
            HirTerminator::ReturnSuccess(success_payload),
            span,
            authored_span,
        )?;

        self.emit_result_carrier_error_return(
            branch.error_block,
            result_carrier.result_local,
            result_carrier.carrier_type,
            current_error_type,
            result_carrier.err_type,
            span,
            authored_span,
        )?;

        self.set_current_block(branch.success_block, span)
    }
}
