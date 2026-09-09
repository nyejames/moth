//! Runtime template aggregate lowering.
//!
//! WHAT: appends aggregate wrapper output only when the source accumulator structurally emitted
//! output.
//! WHY: loop heads and conditional child wrappers both apply around aggregate output once, while
//! skipped branches and zero-output loops preserve structural no-output semantics. Conditional
//! child wrappers now consume a TIR-derived owned wrapper node; the shared emitted-guard
//! infrastructure below is reused by both paths.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;

use super::append_context::RuntimeTemplateAppendContext;

pub(super) struct RuntimeTemplateAggregateAppend<'context> {
    pub(super) aggregate: LocalId,
    pub(super) emitted_output: LocalId,
    pub(super) append_context: RuntimeTemplateAppendContext<'context>,
}

impl<'a> HirBuilder<'a> {
    pub(super) fn append_runtime_template_aggregate_when_emitted(
        &mut self,
        append: RuntimeTemplateAggregateAppend,
        span_ref: &Option<SourceSpan>,
        append_aggregate: impl FnOnce(
            &mut Self,
            RuntimeTemplateAggregateAppend,
            &Option<SourceSpan>,
        ) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        let condition_block = self.current_block_id_or_error(span_ref)?;
        let parent_region = self.current_region_or_error(span_ref)?;
        let then_region = self.create_child_region(parent_region);
        let else_region = self.create_child_region(parent_region);
        let then_block = self.create_block(then_region, span_ref, "template-aggregate-emitted")?;
        let else_block = self.create_block(else_region, span_ref, "template-aggregate-skipped")?;
        let condition = self.make_local_load_expression(
            append.emitted_output,
            builtin_type_ids::BOOL,
            span_ref,
            parent_region,
        );

        self.emit_terminator(
            condition_block,
            HirTerminator::If {
                condition,
                then_block,
                else_block,
            },
            span_ref,
        )?;
        self.set_current_block(then_block, span_ref)?;
        if let Some(parent_flag) = append.append_context.emitted_output() {
            self.mark_runtime_template_output_emitted(parent_flag, span_ref)?;
        }
        append_aggregate(self, append, span_ref)?;

        let then_tail_block = self.current_block_id_or_error(span_ref)?;
        let then_terminated = self.block_has_explicit_terminator(then_tail_block, span_ref)?;

        self.set_current_block(else_block, span_ref)?;
        let else_tail_block = self.current_block_id_or_error(span_ref)?;

        if then_terminated {
            return self.set_current_block(else_tail_block, span_ref);
        }

        let merge_block = self.create_block(parent_region, span_ref, "template-aggregate-merge")?;
        self.emit_jump_to(
            then_tail_block,
            merge_block,
            span_ref,
            "template-aggregate.emitted.merge",
        )?;
        self.emit_jump_to(
            else_tail_block,
            merge_block,
            span_ref,
            "template-aggregate.skipped.merge",
        )?;

        self.set_current_block(merge_block, span_ref)
    }

    pub(super) fn initialize_runtime_template_emitted_flag(
        &mut self,
        span_ref: &Option<SourceSpan>,
    ) -> Result<LocalId, CompilerError> {
        let flag = self.allocate_temp_local(builtin_type_ids::BOOL, None)?;
        let region = self.current_region_or_error(span_ref)?;
        let false_value = self.make_expression(
            span_ref,
            HirExpressionKind::Bool(false),
            builtin_type_ids::BOOL,
            ValueKind::Const,
            region,
        );

        self.emit_statement_kind(
            crate::compiler_frontend::hir::statements::HirStatementKind::Assign {
                target: HirPlace::Local(flag),
                value: false_value,
            },
            span_ref,
        )?;

        Ok(flag)
    }

    pub(super) fn mark_runtime_template_output_emitted(
        &mut self,
        emitted_output: LocalId,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let region = self.current_region_or_error(span_ref)?;
        let true_value = self.make_expression(
            span_ref,
            HirExpressionKind::Bool(true),
            builtin_type_ids::BOOL,
            ValueKind::Const,
            region,
        );

        self.emit_statement_kind(
            crate::compiler_frontend::hir::statements::HirStatementKind::Assign {
                target: HirPlace::Local(emitted_output),
                value: true_value,
            },
            span_ref,
        )
    }
}
