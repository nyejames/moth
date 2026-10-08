//! Runtime slot application lowering.
//!
//! WHAT: consumes the finalized AST-owned handoff and lowers it with
//! ordinary string accumulators.
//! WHY: HIR should execute owned runtime slot-application metadata without
//! holding raw TIR IDs or rediscovering source-level `$slot` / `$insert(...)`
//! semantics.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotContributionSource,
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_expression::LoweredExpression;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::return_hir_transformation_error;

use super::append_context::{
    RuntimeSlotSourceAccumulatorContext, RuntimeSlotSourceLocals, RuntimeTemplateAppendContext,
};
use super::render_append::RuntimeTemplateEmission;

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_runtime_slot_application_template_expression(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        span_ref: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let output_accumulator = self.initialize_runtime_template_accumulator(span_ref)?;
        let append_context = RuntimeTemplateAppendContext::new(output_accumulator);
        self.append_runtime_slot_application_with_context(handoff, append_context, span_ref)?;

        let region = self.current_region_or_error(span_ref)?;
        let value = self.make_expression(
            span_ref,
            HirExpressionKind::Copy(HirPlace::local(output_accumulator)),
            builtin_type_ids::STRING,
            ValueKind::RValue,
            region,
        )?;

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    // WHAT: Appends an AST-owned runtime slot application into an existing template output
    // accumulator. WHY: slot applications inside template loops must participate in the same
    // append-mode emission bookkeeping as nested runtime template `if` / `loop` bodies.
    pub(super) fn append_runtime_slot_application_with_context(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        let source_accumulators =
            self.initialize_runtime_slot_source_accumulators(handoff, span_ref)?;

        let emitted_any_contribution =
            self.append_runtime_slot_contributions(handoff, &source_accumulators, span_ref)?;

        if let Some(emitted_any_contribution) = emitted_any_contribution {
            return self.append_runtime_slot_wrapper_if_contributed(
                handoff,
                append_context,
                &source_accumulators,
                emitted_any_contribution,
                span_ref,
            );
        }

        self.append_runtime_slot_wrapper(handoff, append_context, &source_accumulators, span_ref)
    }

    fn initialize_runtime_slot_source_accumulators(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeSlotSourceAccumulatorContext, HirConstructionFailure> {
        let mut context = RuntimeSlotSourceAccumulatorContext::new();

        for source in &handoff.contribution_sources {
            let guarantees_output = owned_runtime_template_node_guarantees_output(
                &source.render_root,
                self.string_table,
            );
            let accumulator = self.initialize_runtime_template_accumulator(span_ref)?;
            let emitted_output = if guarantees_output {
                None
            } else {
                Some(self.initialize_runtime_template_emitted_flag(span_ref)?)
            };
            context.insert(
                source.source,
                RuntimeSlotSourceLocals {
                    accumulator,
                    emitted_output,
                },
            );
        }

        Ok(context)
    }

    /// Returns a contribution flag only when existing proofs leave wrapper selection conditional.
    fn append_runtime_slot_contributions(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        source_accumulators: &RuntimeSlotSourceAccumulatorContext,
        span_ref: &Option<SourceSpan>,
    ) -> Result<Option<LocalId>, HirConstructionFailure> {
        // Wrapper-owned output, such as a list shell, survives when conditional
        // children contribute nothing; conditional sources keep their emission tracking.
        let mut renders_wrapper_unconditionally = handoff.contribution_sources.is_empty()
            || owned_runtime_template_node_guarantees_output(&handoff.wrapper, self.string_table);

        // Reuse the per-source proof gathered during local initialization. Only
        // allocate a shared contribution flag when it still controls wrapper selection.
        for source in &handoff.contribution_sources {
            if renders_wrapper_unconditionally {
                break;
            }

            let Some(source_locals) = source_accumulators.for_source(source.source) else {
                return_hir_transformation_error!(
                    "Runtime slot contribution referenced a source with no allocated accumulator.",
                    self.hir_error_location(&source.span)
                );
            };

            if source.renders_wrapper_unconditionally && source_locals.emitted_output.is_none() {
                renders_wrapper_unconditionally = true;
            }
        }

        let emitted_any_contribution = if renders_wrapper_unconditionally {
            None
        } else {
            Some(self.initialize_runtime_template_emitted_flag(span_ref)?)
        };

        for source in &handoff.contribution_sources {
            let Some(source_locals) = source_accumulators.for_source(source.source) else {
                return_hir_transformation_error!(
                    "Runtime slot contribution referenced a source with no allocated accumulator.",
                    self.hir_error_location(&source.span)
                );
            };

            self.append_runtime_slot_contribution_content(source, source_locals, span_ref)?;

            let current_block = self.current_block_id_or_error(span_ref)?;
            if self.block_has_explicit_terminator(current_block, span_ref)? {
                break;
            }

            if let Some(emitted_any_contribution) = emitted_any_contribution {
                if let Some(source_emitted_output) = source_locals.emitted_output {
                    self.accumulate_runtime_slot_source_emission(
                        source_emitted_output,
                        emitted_any_contribution,
                        span_ref,
                    )?;
                } else {
                    self.mark_runtime_template_output_emitted(emitted_any_contribution, span_ref)?;
                }
            }
        }

        Ok(emitted_any_contribution)
    }

    fn append_runtime_slot_contribution_content(
        &mut self,
        source: &OwnedRuntimeSlotContributionSource,
        source_locals: RuntimeSlotSourceLocals,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let append_context = match source_locals.emitted_output {
            Some(emitted_output) => RuntimeTemplateAppendContext::new(source_locals.accumulator)
                .with_emitted_output(emitted_output),
            None => RuntimeTemplateAppendContext::new(source_locals.accumulator),
        };

        self.append_owned_runtime_template_node_to_accumulator(
            &source.render_root,
            append_context,
            None,
            span_ref,
        )?;

        Ok(())
    }

    fn accumulate_runtime_slot_source_emission(
        &mut self,
        source_emitted_output: LocalId,
        any_source_emitted_output: LocalId,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let region = self.current_region_or_error(span_ref)?;
        let any_source_value = self.make_local_load_expression(
            any_source_emitted_output,
            builtin_type_ids::BOOL,
            span_ref,
            region,
        )?;
        let source_value = self.make_local_load_expression(
            source_emitted_output,
            builtin_type_ids::BOOL,
            span_ref,
            region,
        )?;
        let emitted_output = self.make_expression(
            span_ref,
            HirExpressionKind::BinOp {
                left: any_source_value,
                op: HirBinOp::Or,
                right: source_value,
            },
            builtin_type_ids::BOOL,
            ValueKind::RValue,
            region,
        )?;

        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(HirPlace::local(any_source_emitted_output)),
                value: emitted_output,
            },
            span_ref,
        )
    }

    fn append_runtime_slot_wrapper(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        append_context: RuntimeTemplateAppendContext<'_>,
        source_accumulators: &RuntimeSlotSourceAccumulatorContext,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        let wrapper_context = append_context
            .with_runtime_slot_sites(source_accumulators, &handoff.slot_sites)
            .rejecting_unresolved_slots();
        let emission = self.append_owned_runtime_template_node_to_accumulator(
            &handoff.wrapper,
            wrapper_context,
            None,
            span_ref,
        )?;

        if append_context.emitted_output().is_some() && emission == RuntimeTemplateEmission::Output
        {
            return Ok(RuntimeTemplateEmission::NoOutput);
        }

        Ok(emission)
    }

    fn append_runtime_slot_wrapper_if_contributed(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        append_context: RuntimeTemplateAppendContext<'_>,
        source_accumulators: &RuntimeSlotSourceAccumulatorContext,
        emitted_any_contribution: LocalId,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        let condition_block = self.current_block_id_or_error(span_ref)?;
        let parent_region = self.current_region_or_error(span_ref)?;
        let rendered_region = self.create_child_region(parent_region);
        let skipped_region = self.create_child_region(parent_region);
        let rendered_block =
            self.create_block(rendered_region, span_ref, "runtime-slot-rendered")?;
        let skipped_block = self.create_block(skipped_region, span_ref, "runtime-slot-skipped")?;
        let condition = self.make_local_load_expression(
            emitted_any_contribution,
            builtin_type_ids::BOOL,
            span_ref,
            parent_region,
        )?;

        self.emit_terminator(
            condition_block,
            HirTerminator::If {
                condition,
                then_block: rendered_block,
                else_block: skipped_block,
            },
            span_ref,
        )?;

        // Runtime-only contribution plans render their wrapper only when a
        // contribution produced structural output. This preserves the documented
        // no-output behavior for false branches and no-output loops.
        self.set_current_block(rendered_block, span_ref)?;
        self.append_runtime_slot_wrapper(handoff, append_context, source_accumulators, span_ref)?;
        let rendered_tail = self.current_block_id_or_error(span_ref)?;
        let rendered_terminated = self.block_has_explicit_terminator(rendered_tail, span_ref)?;

        self.set_current_block(skipped_block, span_ref)?;
        let skipped_tail = self.current_block_id_or_error(span_ref)?;

        let emission = if append_context.emitted_output().is_some() {
            RuntimeTemplateEmission::NoOutput
        } else {
            RuntimeTemplateEmission::Output
        };

        if rendered_terminated {
            self.set_current_block(skipped_tail, span_ref)?;
            return Ok(emission);
        }

        let merge_block = self.create_block(parent_region, span_ref, "runtime-slot-merge")?;
        self.emit_jump_to(
            rendered_tail,
            merge_block,
            span_ref,
            "runtime-slot.rendered.merge",
        )?;
        self.emit_jump_to(
            skipped_tail,
            merge_block,
            span_ref,
            "runtime-slot.skipped.merge",
        )?;

        self.set_current_block(merge_block, span_ref)?;
        Ok(emission)
    }
}

fn owned_runtime_template_node_guarantees_output(
    node: &OwnedRuntimeTemplateNode,
    string_table: &StringTable,
) -> bool {
    match node {
        OwnedRuntimeTemplateNode::Sequence { children, .. } => children
            .iter()
            .any(|child| owned_runtime_template_node_guarantees_output(child, string_table)),

        OwnedRuntimeTemplateNode::Text { text, .. } => match text {
            OwnedFoldedString::Text(text) => !text.trim().is_empty(),
            OwnedFoldedString::Pieces(pieces) => pieces.iter().any(|piece| match piece {
                OwnedFoldedStringPiece::Text(text) => !text.trim().is_empty(),
                OwnedFoldedStringPiece::Resource(_) | OwnedFoldedStringPiece::SiteRoot => true,
            }),
        },

        OwnedRuntimeTemplateNode::AggregateOutput => true,

        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            dynamic_expression_guarantees_output(expression, string_table)
        }

        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
            runtime_template_handoff_guarantees_output(template, string_table)
        }

        // Runtime template control flow can structurally produce no output
        // after HIR evaluates its condition or iterable. Even when the body
        // shape is otherwise const-renderable, the slot wrapper must stay
        // guarded by the emitted-output flag.
        OwnedRuntimeTemplateNode::Conditional { .. }
        | OwnedRuntimeTemplateNode::Loop { .. }
        | OwnedRuntimeTemplateNode::ConditionalWrapper { .. } => false,

        OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => false,
    }
}

fn dynamic_expression_guarantees_output(
    expression: &Expression,
    string_table: &StringTable,
) -> bool {
    match &expression.kind {
        ExpressionKind::StringSlice(text) => !string_table.resolve(*text).is_empty(),

        ExpressionKind::Template(_) => false,

        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            runtime_template_handoff_guarantees_output(handoff, string_table)
        }

        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            owned_runtime_template_node_guarantees_output(&handoff.wrapper, string_table)
        }

        ExpressionKind::Coerced { value, .. } => {
            dynamic_expression_guarantees_output(value, string_table)
        }

        ExpressionKind::Runtime(rpn) if rpn.items.len() == 1 => match &rpn.items[0] {
            ExpressionRpnItem::Operand(expression) => {
                dynamic_expression_guarantees_output(expression, string_table)
            }
            ExpressionRpnItem::Operator { .. } => true,
            // Pending syntax never survives evaluation; flag it in debug builds so the invalid
            // boundary is visible instead of silently assuming output.
            ExpressionRpnItem::PendingNumericLiteral { .. }
            | ExpressionRpnItem::PendingGroup { .. } => {
                debug_assert!(
                    false,
                    "pending expression syntax reached HIR output-guarantee query"
                );
                true
            }
        },

        _ => true,
    }
}

fn runtime_template_handoff_guarantees_output(
    handoff: &OwnedRuntimeTemplateHandoff,
    string_table: &StringTable,
) -> bool {
    match &handoff.body {
        OwnedRuntimeTemplateBody::Render(node) => {
            owned_runtime_template_node_guarantees_output(node, string_table)
        }
        OwnedRuntimeTemplateBody::RuntimeSlotApplication(handoff) => {
            owned_runtime_template_node_guarantees_output(&handoff.wrapper, string_table)
        }
    }
}

#[cfg(test)]
#[path = "tests/slot_application_tests.rs"]
mod slot_application_tests;
