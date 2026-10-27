//! HIR lowering for postfix option propagation.
//!
//! WHAT: turns `expr?` into an explicit option match with an early `none`
//! return and a present-value continuation.
//! WHY: propagation is control flow. Lowering it into blocks keeps borrow
//! validation and backend lowering from treating it as a panic unwrap.

use crate::compiler_frontend::ast::expressions::expression_store::ExpressionView;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::statements::HirWriteTarget;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;

use super::LoweredExpression;

impl<'a> HirBuilder<'a> {
    pub(crate) fn lower_option_expression_to_present_value(
        &mut self,
        value: ExpressionView<'_>,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let lowered = self.lower_expression(value)?;
        let option_type = self.module.expressions.expression(lowered.value).ty;
        let Some(inner_type) = self.type_environment.option_inner_type(option_type) else {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Option propagation reached HIR with a non-option value",
            );
        };

        for prelude in lowered.prelude {
            self.emit_statement_to_current_block(prelude)?;
        }

        let option_local = self.allocate_temp_local(option_type, None)?;
        self.emit_write_statement(HirWriteTarget::DefineLocal(option_local), lowered.value)?;

        let branch_region = self.current_region_or_error()?;
        let branch_block = self.current_block_id_or_error()?;
        let no_span = None;
        let present_block = self.create_block(branch_region, &no_span, "propagate-option-some");
        let none_block = self.create_block(branch_region, &no_span, "propagate-option-none");
        let option_for_branch =
            self.make_local_load_expression(option_local, option_type, &no_span, branch_region)?;

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
            span,
            *span,
        )?;

        self.emit_option_none_return(none_block, option_type, span)?;
        self.set_current_block(present_block)?;
        let present_region = self.current_region_or_error()?;
        let option_for_payload =
            self.make_local_load_expression(option_local, option_type, &no_span, present_region)?;
        let present_value = self.make_expression(
            &no_span,
            HirExpressionKind::VariantPayloadGet {
                carrier: HirVariantCarrier::Option,
                source: option_for_payload,
                variant_index: OPTION_SOME_VARIANT_INDEX,
                field_index: 0,
            },
            inner_type,
            ValueKind::RValue,
            present_region,
        )?;

        Ok(LoweredExpression {
            prelude: vec![],
            value: present_value,
        })
    }

    fn emit_option_none_return(
        &mut self,
        none_block: crate::compiler_frontend::hir::ids::BlockId,
        propagated_option_type: crate::compiler_frontend::datatypes::ids::TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.set_current_block(none_block)?;
        let none_region = self.current_region_or_error()?;
        let current_function_id = self.current_function_id_or_error()?;
        let returns = &self.function_by_id_or_error(current_function_id)?.returns;
        let [function_return_type] = returns.as_ref() else {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Option propagation requires one success return slot",
            );
        };
        let function_return_type = *function_return_type;
        let no_span = None;

        let none = self.option_none_expression(function_return_type, none_region, &no_span)?;
        if self.module.expressions.expression(none).ty != propagated_option_type {
            // AST compatibility checks should have guaranteed that the early
            // return uses the current function's option type.
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Option propagation reached HIR with a mismatched function return type",
            );
        }
        let values = self.module.expressions.append_values(&[none], *span)?;
        self.emit_terminator_with_span(none_block, HirTerminator::Return(values), span, *span)
    }

    fn option_none_expression(
        &mut self,
        option_type: crate::compiler_frontend::datatypes::ids::TypeId,
        region: crate::compiler_frontend::hir::ids::RegionId,
        span: &Option<SourceSpan>,
    ) -> Result<crate::compiler_frontend::hir::ids::HirValueId, HirConstructionFailure> {
        if !self.type_environment.is_option(option_type) {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Option propagation none return targeted a non-option type",
            );
        }

        let fields = self.module.expressions.append_variant_fields(&[], *span)?;
        self.make_expression(
            span,
            HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: 0,
                fields,
            },
            option_type,
            ValueKind::RValue,
            region,
        )
    }
}
