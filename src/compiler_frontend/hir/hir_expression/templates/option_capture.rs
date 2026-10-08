//! Option-present template `if` capture lowering.
//!
//! WHAT: lowers runtime `[if option is |value|:]` templates into a match terminator with a
//! branch-local capture binding.
//! WHY: option capture has extra local-registration and payload-extraction rules that are easier
//! to audit apart from ordinary Bool branch lowering.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    pub(super) fn append_runtime_option_present_template_branch(
        &mut self,
        scrutinee: &Expression,
        pattern: &MatchPattern,
        span_ref: &Option<SourceSpan>,
        append_present: impl FnOnce(&mut HirBuilder<'a>) -> Result<(), HirConstructionFailure>,
    ) -> Result<(), HirConstructionFailure> {
        let MatchPattern::OptionPresentCapture {
            binding_path,
            inner_type_id,
            span: pattern_span,
            binding_span,
            ..
        } = pattern
        else {
            return_hir_transformation_error!(
                "Runtime template option-present if reached HIR without an option-present capture pattern.",
                self.hir_error_location(span_ref)
            );
        };
        let capture_span = (*binding_span).or(*pattern_span);

        let lowered_scrutinee = self.lower_expression_value_to_current_block(scrutinee)?;
        let option_type = self.module.expressions.expression(lowered_scrutinee).ty;
        if self
            .type_environment
            .option_inner_type(option_type)
            .is_none()
        {
            return_hir_transformation_error!(
                "Runtime template option-present if reached HIR with a non-option scrutinee.",
                self.hir_error_location(span_ref)
            );
        }

        // Materialize the scrutinee once so matching and payload extraction share its value.
        let option_local = self.allocate_temp_local(option_type, None)?;
        self.emit_write_statement(
            HirWriteTarget::DefineLocal(option_local),
            lowered_scrutinee,
            span_ref,
        )?;

        let match_block = self.current_block_id_or_error(span_ref)?;
        let parent_region = self.current_region_or_error(span_ref)?;
        let present_region = self.create_child_region(parent_region);
        let absent_region = self.create_child_region(parent_region);
        let present_block =
            self.create_block(present_region, span_ref, "template-if-option-present")?;
        let absent_block = self.create_block(absent_region, span_ref, "template-if-option-none")?;
        let scrutinee_for_match =
            self.make_local_load_expression(option_local, option_type, span_ref, parent_region)?;

        self.emit_terminator(
            match_block,
            HirTerminator::Match {
                scrutinee: scrutinee_for_match,
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::OptionPresent,
                        guard: None,
                        body: present_block,
                    },
                    HirMatchArm {
                        pattern: HirPattern::OptionNone,
                        guard: None,
                        body: absent_block,
                    },
                ],
            },
            span_ref,
        )?;

        // The match has terminated the parent block; capture binding and present-body output
        // belong to the present arm rather than being appended after that terminator.
        self.set_current_block(present_block, span_ref)?;
        let capture_local = self.register_template_option_capture_local(
            binding_path,
            *inner_type_id,
            &capture_span,
            *binding_span,
        )?;
        self.emit_template_option_capture_assignment(
            capture_local,
            option_local,
            option_type,
            *inner_type_id,
            &capture_span,
            *binding_span,
        )?;
        self.with_temporary_local_bindings([(*binding_path, capture_local)], |builder| {
            append_present(builder)
        })?;

        let present_tail_block = self.current_block_id_or_error(span_ref)?;
        let present_terminated =
            self.block_has_explicit_terminator(present_tail_block, span_ref)?;

        let merge_block = self.create_block(parent_region, span_ref, "template-if-option-merge")?;
        if !present_terminated {
            self.emit_jump_to(
                present_tail_block,
                merge_block,
                span_ref,
                "template-if-option.present.merge",
            )?;
        }

        self.set_current_block(absent_block, span_ref)?;
        self.emit_jump_to(
            absent_block,
            merge_block,
            span_ref,
            "template-if-option.none.merge",
        )?;

        self.set_current_block(merge_block, span_ref)?;
        Ok(())
    }

    fn register_template_option_capture_local(
        &mut self,
        binding_path: &PathId,
        inner_type_id: TypeId,
        span_ref: &Option<SourceSpan>,
        binding_span: Option<SourceSpan>,
    ) -> Result<LocalId, HirConstructionFailure> {
        let ty = self.lower_type_id(inner_type_id, span_ref)?;
        let region = self.current_region_or_error(span_ref)?;
        let block_id = self.current_block_id_or_error(span_ref)?;
        let local_id = self.allocate_local_id();
        let local = HirLocal {
            id: local_id,
            ty,
            mutable: false,
            region,
            span: binding_span,
        };

        self.register_local_in_block(block_id, local, span_ref)?;
        self.side_table.bind_local_name(local_id, *binding_path);

        Ok(local_id)
    }

    fn emit_template_option_capture_assignment(
        &mut self,
        capture_local: LocalId,
        option_local: LocalId,
        option_type: TypeId,
        inner_type_id: TypeId,
        span_ref: &Option<SourceSpan>,
        binding_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let field_ty = self.lower_type_id(inner_type_id, span_ref)?;
        let region = self.current_region_or_error(span_ref)?;
        let source =
            self.make_local_load_expression(option_local, option_type, span_ref, region)?;
        // Authored option capture materialization carries the binding span. Constructing the
        // value with that span keeps its side-table mapping aligned with the node itself.
        let payload_get = self.make_expression(
            &binding_span,
            HirExpressionKind::VariantPayloadGet {
                carrier: HirVariantCarrier::Option,
                source,
                variant_index: OPTION_SOME_VARIANT_INDEX,
                field_index: 0,
            },
            field_ty,
            ValueKind::RValue,
            region,
        )?;

        self.emit_statement_kind_with_span(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(capture_local),
                value: payload_get,
            },
            span_ref,
            binding_span,
        )
    }
}
