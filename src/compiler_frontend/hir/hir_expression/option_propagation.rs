//! HIR lowering for postfix option propagation.
//!
//! WHAT: turns `expr?` into an explicit option match with an early `none`
//! return and a present-value continuation.
//! WHY: propagation is control flow. Lowering it into blocks keeps borrow
//! validation and backend lowering from treating it as a panic unwrap.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::return_hir_transformation_error;

use super::LoweredExpression;

impl<'a> HirBuilder<'a> {
    pub(crate) fn lower_option_expression_to_present_value(
        &mut self,
        value: &Expression,
        location: &SourceLocation,
        span: Option<SourceSpan>,
    ) -> Result<LoweredExpression, CompilerError> {
        let lowered = self.lower_expression(value)?;
        let option_type = lowered.value.ty;
        let Some(inner_type) = self.type_environment.option_inner_type(option_type) else {
            return_hir_transformation_error!(
                "Option propagation reached HIR with a non-option value",
                self.hir_error_location(location)
            );
        };

        for prelude in lowered.prelude {
            self.emit_statement_to_current_block(prelude, location)?;
        }

        let option_local = self.allocate_temp_local(option_type, Some(location.to_owned()))?;
        self.emit_assign_local_statement(option_local, lowered.value, location)?;

        let branch_region = self.current_region_or_error(location)?;
        let branch_block = self.current_block_id_or_error(location)?;
        let present_block = self.create_block(branch_region, location, "propagate-option-some")?;
        let none_block = self.create_block(branch_region, location, "propagate-option-none")?;
        let option_for_branch =
            self.make_local_load_expression(option_local, option_type, location, branch_region);

        self.emit_terminator_with_span(
            branch_block,
            HirTerminator::Match {
                scrutinee: option_for_branch,
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::OptionPresent,
                        guard: None,
                        body: present_block,
                    },
                    HirMatchArm {
                        pattern: HirPattern::OptionNone,
                        guard: None,
                        body: none_block,
                    },
                ],
            },
            location,
            span,
        )?;

        self.emit_option_none_return(none_block, option_type, location, span)?;

        self.set_current_block(present_block, location)?;
        let present_region = self.current_region_or_error(location)?;
        let option_for_payload =
            self.make_local_load_expression(option_local, option_type, location, present_region);
        let present_value = self.make_expression(
            location,
            HirExpressionKind::VariantPayloadGet {
                carrier: HirVariantCarrier::Option,
                source: Box::new(option_for_payload),
                variant_index: OPTION_SOME_VARIANT_INDEX,
                field_index: 0,
            },
            inner_type,
            ValueKind::RValue,
            present_region,
        );

        Ok(LoweredExpression {
            prelude: vec![],
            value: present_value,
        })
    }

    fn emit_option_none_return(
        &mut self,
        none_block: crate::compiler_frontend::hir::ids::BlockId,
        propagated_option_type: crate::compiler_frontend::datatypes::ids::TypeId,
        location: &SourceLocation,
        span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        self.set_current_block(none_block, location)?;
        let none_region = self.current_region_or_error(location)?;
        let current_function_id = self.current_function_id_or_error(location)?;
        let function_return_type = self
            .function_by_id_or_error(current_function_id, location)?
            .return_type;

        if let Some((success_type, _)) = self
            .type_environment
            .fallible_carrier_slots(function_return_type)
        {
            let none = self.option_none_expression(success_type, none_region, location)?;
            return self.emit_terminator_with_span(
                none_block,
                HirTerminator::ReturnSuccess(none),
                location,
                span,
            );
        }

        let none = self.option_none_expression(function_return_type, none_region, location)?;
        if none.ty != propagated_option_type {
            // AST compatibility checks should have guaranteed that the early
            // return uses the current function's option type.
            return_hir_transformation_error!(
                "Option propagation reached HIR with a mismatched function return type",
                self.hir_error_location(location)
            );
        }

        self.emit_terminator_with_span(none_block, HirTerminator::Return(none), location, span)
    }

    fn option_none_expression(
        &mut self,
        option_type: crate::compiler_frontend::datatypes::ids::TypeId,
        region: crate::compiler_frontend::hir::ids::RegionId,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        if !self.type_environment.is_option(option_type) {
            return_hir_transformation_error!(
                "Option propagation none return targeted a non-option type",
                self.hir_error_location(location)
            );
        }

        Ok(self.make_expression(
            location,
            HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: 0,
                fields: vec![],
            },
            option_type,
            ValueKind::RValue,
            region,
        ))
    }
}
