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
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::type_coercion::compatibility::is_postfix_error_compatible;
use crate::return_hir_transformation_error;

use super::carrier::EmittedFallibleCarrier;

impl<'a> HirBuilder<'a> {
    /// Lowers direct `return fallible_expression!` into explicit success/error return edges.
    ///
    /// WHAT: emits the fallible carrier, branches on it, returns the success payload from the
    /// success edge, and returns the error payload from the error edge.
    /// WHY: direct-return propagation is a control-flow operation. HIR should expose both return
    /// edges instead of hiding the error path inside an expression helper.
    pub(crate) fn lower_fallible_propagating_direct_return(
        &mut self,
        value: &Expression,
        location: &SourceLocation,
    ) -> Result<bool, CompilerError> {
        let Some(result_carrier) = self.emit_result_propagation_carrier_to_current_block(value)?
        else {
            return Ok(false);
        };

        let propagation_location = value.propagation_location().unwrap_or(location);
        self.emit_result_carrier_direct_return(result_carrier, propagation_location)?;
        Ok(true)
    }

    /// Emits the success and error return terminators for a direct-return carrier.
    ///
    /// WHAT: branches on the carrier, returns the unwrapped success payload from the success block,
    /// and returns the coerced error payload from the error block.
    pub(super) fn emit_result_carrier_direct_return(
        &mut self,
        result_carrier: EmittedFallibleCarrier,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let current_function_id = self.current_function_id_or_error(location)?;
        let current_return_type = self
            .function_by_id_or_error(current_function_id, location)?
            .return_type;
        let Some((current_ok_type, current_error_type)) = self
            .type_environment
            .fallible_carrier_slots(current_return_type)
        else {
            return_hir_transformation_error!(
                "Direct fallible propagation return reached HIR outside a fallible function",
                self.hir_error_location(location)
            );
        };

        if current_ok_type != result_carrier.ok_type {
            return_hir_transformation_error!(
                "Direct fallible propagation success type does not match the enclosing function",
                self.hir_error_location(location)
            );
        }

        if !is_postfix_error_compatible(
            current_error_type,
            result_carrier.err_type,
            &self.type_environment,
        ) {
            return_hir_transformation_error!(
                "Direct fallible propagation error type does not match the enclosing function",
                self.hir_error_location(location)
            );
        }

        let branch = self.emit_result_carrier_branch(
            result_carrier.result_local,
            result_carrier.carrier_type,
            location,
            "return-fallible-ok",
            "return-fallible-err",
        )?;

        self.set_current_block(branch.success_block, location)?;
        let success_region = self.current_region_or_error(location)?;
        let success_result = self.make_local_load_expression(
            result_carrier.result_local,
            result_carrier.carrier_type,
            location,
            success_region,
        );
        let mut success_payload = self.make_expression(
            location,
            HirExpressionKind::FallibleUnwrapSuccess {
                result: Box::new(success_result),
            },
            result_carrier.ok_type,
            ValueKind::RValue,
            success_region,
        );
        if result_carrier.validate_float_success {
            success_payload = self.emit_validated_float_value(success_payload, location)?;
        }
        self.emit_terminator(
            branch.success_block,
            HirTerminator::ReturnSuccess(success_payload),
            location,
        )?;

        self.emit_result_carrier_error_return(
            branch.error_block,
            result_carrier.result_local,
            result_carrier.carrier_type,
            current_error_type,
            result_carrier.err_type,
            location,
        )?;

        self.set_current_block(branch.success_block, location)
    }
}
