//! Return lowering helpers for HIR statements.
//!
//! WHAT: lowers success and error returns into final HIR return terminators.
//! WHY: return coercion/alias handling is distinct from branch and loop CFG construction.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_return_statement(
        &mut self,
        values: &[Expression],
        span_ref: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let function_id = self.current_function_id_or_error(span_ref)?;

        let handled_direct_propagation = values.len() == 1
            && self.lower_fallible_propagating_direct_return(
                &values[0],
                span_ref,
                authored_span,
            )?;
        if handled_direct_propagation {
            return Ok(());
        }

        let mut lowered_values = Vec::with_capacity(values.len());

        for value in values.iter() {
            let lowered_value = self.lower_expression_value_to_current_block(value)?;
            lowered_values.push(lowered_value);
        }

        let return_value = self.expression_from_return_values(&lowered_values, span_ref)?;
        let function_return_type = self
            .function_by_id_or_error(function_id, span_ref)?
            .return_type;
        let current_block = self.current_block_id_or_error(span_ref)?;

        if let Some((ok, _)) = self
            .type_environment
            .fallible_carrier_slots(function_return_type)
        {
            if return_value.ty != ok {
                return_hir_transformation_error!(
                    "Lowered success return does not match function result ok type",
                    self.hir_error_location(span_ref)
                );
            }

            return self.emit_terminator_with_span(
                current_block,
                HirTerminator::ReturnSuccess(return_value),
                span_ref,
                authored_span,
            );
        }

        self.emit_terminator_with_span(
            current_block,
            HirTerminator::Return(return_value),
            span_ref,
            authored_span,
        )
    }

    pub(super) fn lower_error_return_statement(
        &mut self,
        value: &Expression,
        span_ref: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let function_id = self.current_function_id_or_error(span_ref)?;
        let function_return_type = self
            .function_by_id_or_error(function_id, span_ref)?
            .return_type;
        let err = match self
            .type_environment
            .fallible_carrier_slots(function_return_type)
        {
            Some((_, err)) => err,
            None => {
                return_hir_transformation_error!(
                    "return! reached HIR lowering in a function without a Result return type",
                    self.hir_error_location(span_ref)
                );
            }
        };

        let lowered_value = self.lower_expression_value_to_current_block(value)?;

        let lowered_error = match lowered_value.kind {
            HirExpressionKind::Load(place) => {
                // The copy preserves the incoming expression span; generated values stay spanless.
                let span = lowered_value.span;
                let ty = lowered_value.ty;
                let region = lowered_value.region;
                self.make_expression(
                    &span,
                    HirExpressionKind::Copy(place),
                    ty,
                    ValueKind::RValue,
                    region,
                )
            }
            _ => lowered_value,
        };

        if lowered_error.ty != err {
            return_hir_transformation_error!(
                "Lowered error return does not match function result error type",
                self.hir_error_location(span_ref)
            );
        }
        let current_block = self.current_block_id_or_error(span_ref)?;
        self.emit_terminator_with_span(
            current_block,
            HirTerminator::ReturnError(lowered_error),
            span_ref,
            authored_span,
        )
    }
}
