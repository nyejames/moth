//! Return lowering helpers for HIR statements.
//!
//! WHAT: lowers success and error returns into final HIR return terminators.
//! WHY: return coercion/alias handling is distinct from branch and loop CFG construction.

use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionStore, ExpressionView,
};
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_return_statement(
        &mut self,
        expressions: &ExpressionStore,
        values: &[ExprId],
        span_ref: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let function_id = self.current_function_id_or_error(span_ref)?;

        let expected_count = self
            .function_by_id_or_error(function_id, span_ref)?
            .returns
            .len();
        let mut lowered_values = Vec::with_capacity(expected_count);
        for value in values {
            lowered_values
                .extend(self.lower_expression_results_to_current_block(expressions.view(*value)?)?);
        }
        if lowered_values.len() != expected_count {
            return_hir_transformation_error!(
                "Lowered return slot count does not match function signature",
                self.hir_error_location(span_ref)
            );
        }
        for (index, value) in lowered_values.iter_mut().enumerate() {
            let expected_type = self.function_by_id_or_error(function_id, span_ref)?.returns[index];
            *value = self.wrap_value_to_optional_type(*value, expected_type, span_ref)?;
            if self.module.expressions.expression(*value).ty != expected_type {
                return_hir_transformation_error!(
                    "Lowered return slot type does not match function signature",
                    self.hir_error_location(span_ref)
                );
            }
        }
        let current_block = self.current_block_id_or_error(span_ref)?;
        let lowered_values = self
            .module
            .expressions
            .append_values(&lowered_values, *span_ref)?;

        self.emit_terminator_with_span(
            current_block,
            HirTerminator::Return(lowered_values),
            span_ref,
            authored_span,
        )
    }

    pub(super) fn lower_error_return_statement(
        &mut self,
        value: ExpressionView<'_>,
        span_ref: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let function_id = self.current_function_id_or_error(span_ref)?;
        let err = match self
            .function_by_id_or_error(function_id, span_ref)?
            .error_return
        {
            Some(err) => err,
            None => {
                return_hir_transformation_error!(
                    "return! reached HIR lowering in a function without an error channel",
                    self.hir_error_location(span_ref)
                );
            }
        };

        let lowered_value = self.lower_expression_value_to_current_block(value)?;

        let loaded_place = {
            let row = self.module.expressions.expression(lowered_value);
            match &row.kind {
                HirExpressionKind::Load(place) => Some((*place, row.span, row.ty, row.region)),
                _ => None,
            }
        };
        let lowered_error = if let Some((place, span, ty, region)) = loaded_place {
            // The copy preserves the incoming expression span; generated values stay spanless.
            self.make_expression(
                &span,
                HirExpressionKind::Copy(place),
                ty,
                ValueKind::RValue,
                region,
            )?
        } else {
            lowered_value
        };

        if self.module.expressions.expression(lowered_error).ty != err {
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
