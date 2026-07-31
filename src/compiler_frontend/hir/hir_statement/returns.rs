//! Return lowering helpers for HIR statements.
//!
//! WHAT: lowers success and error returns into final HIR return terminators.
//! WHY: return coercion/alias handling is distinct from branch and loop CFG construction.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_return_statement(
        &mut self,
        values: &[Expression],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let function_id = self.current_function_id_or_error(location)?;

        let handled_direct_propagation = values.len() == 1
            && self.lower_fallible_propagating_direct_return(&values[0], location)?;
        if handled_direct_propagation {
            return Ok(());
        }

        let mut lowered_values = Vec::with_capacity(values.len());

        for value in values.iter() {
            let lowered_value = self.lower_expression_value_to_current_block(value)?;
            lowered_values.push(lowered_value);
        }

        let return_value = self.expression_from_return_values(&lowered_values, location)?;
        let function_return_type = self
            .function_by_id_or_error(function_id, location)?
            .return_type;
        let current_block = self.current_block_id_or_error(location)?;

        if let Some((ok, _)) = self
            .type_environment
            .fallible_carrier_slots(function_return_type)
        {
            if return_value.ty != ok {
                return_hir_transformation_error!(
                    "Lowered success return does not match function result ok type",
                    self.hir_error_location(location)
                );
            }

            return self.emit_terminator(
                current_block,
                HirTerminator::ReturnSuccess(return_value),
                location,
            );
        }

        self.emit_terminator(current_block, HirTerminator::Return(return_value), location)
    }

    pub(super) fn lower_error_return_statement(
        &mut self,
        value: &Expression,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let function_id = self.current_function_id_or_error(location)?;
        let function_return_type = self
            .function_by_id_or_error(function_id, location)?
            .return_type;
        let err = match self
            .type_environment
            .fallible_carrier_slots(function_return_type)
        {
            Some((_, err)) => err,
            None => {
                return_hir_transformation_error!(
                    "return! reached HIR lowering in a function without a Result return type",
                    self.hir_error_location(location)
                );
            }
        };

        let lowered_value = self.lower_expression_value_to_current_block(value)?;

        let lowered_error = match lowered_value.kind {
            HirExpressionKind::Load(place) => self.make_expression(
                location,
                HirExpressionKind::Copy(place),
                lowered_value.ty,
                ValueKind::RValue,
                lowered_value.region,
            ),
            _ => lowered_value,
        };

        if lowered_error.ty != err {
            return_hir_transformation_error!(
                "Lowered error return does not match function result error type",
                self.hir_error_location(location)
            );
        }
        let current_block = self.current_block_id_or_error(location)?;
        self.emit_terminator(
            current_block,
            HirTerminator::ReturnError(lowered_error),
            location,
        )
    }
}
