//! Runtime slot application lowering.
//!
//! WHAT: consumes the finalized AST-owned handoff and lowers it with
//! ordinary string accumulators.
//! WHY: HIR should execute owned runtime slot-application metadata without
//! holding raw TIR IDs or rediscovering source-level `$slot` / `$insert(...)`
//! semantics.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateBodyEmission;
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotContributionSource,
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_expression::LoweredExpression;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::return_hir_transformation_error;

use super::append_context::{
    RuntimeSlotLoopControlFlush, RuntimeSlotSourceAccumulatorContext, RuntimeTemplateAppendContext,
};

struct RuntimeSlotContributionResult {
    emission: TemplateBodyEmission,
    emitted_any_contribution: LocalId,
    renders_wrapper_unconditionally: bool,
}

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_runtime_slot_application_template_expression(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        let output_accumulator = self.initialize_runtime_template_accumulator(location)?;
        let append_context = RuntimeTemplateAppendContext::new(output_accumulator);
        let emission =
            self.append_runtime_slot_application_with_context(handoff, append_context, location)?;

        if matches!(
            emission,
            TemplateBodyEmission::Break | TemplateBodyEmission::Continue
        ) {
            return_hir_transformation_error!(
                "Runtime slot application emitted loop control outside a template loop body.",
                self.hir_error_location(location)
            );
        }

        let region = self.current_region_or_error(location)?;
        let value = self.make_expression(
            location,
            HirExpressionKind::Copy(HirPlace::Local(output_accumulator)),
            builtin_type_ids::STRING,
            ValueKind::RValue,
            region,
        );

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    // WHAT: Appends an AST-owned runtime slot application into an existing template output
    // accumulator. WHY: slot applications inside template loops must participate in the same
    // append-mode control-flow propagation as nested runtime template `if` / `loop` bodies.
    pub(super) fn append_runtime_slot_application_with_context(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        append_context: RuntimeTemplateAppendContext<'_>,
        location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let source_accumulators =
            self.initialize_runtime_slot_source_accumulators(handoff, location)?;

        let contribution_result = self.append_runtime_slot_contributions(
            handoff,
            append_context,
            &source_accumulators,
            location,
        )?;
        if matches!(
            contribution_result.emission,
            TemplateBodyEmission::Break | TemplateBodyEmission::Continue
        ) {
            return Ok(contribution_result.emission);
        }

        if contribution_result.renders_wrapper_unconditionally {
            return self.append_runtime_slot_wrapper(
                handoff,
                append_context,
                &source_accumulators,
                location,
            );
        }

        self.append_runtime_slot_wrapper_if_contributed(
            handoff,
            append_context,
            &source_accumulators,
            contribution_result.emitted_any_contribution,
            location,
        )
    }

    fn initialize_runtime_slot_source_accumulators(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        location: &SourceLocation,
    ) -> Result<RuntimeSlotSourceAccumulatorContext, CompilerError> {
        let mut context = RuntimeSlotSourceAccumulatorContext::new();

        for source in &handoff.contribution_sources {
            let accumulator = self.initialize_runtime_template_accumulator(location)?;
            context.insert(source.source, accumulator);
        }

        Ok(context)
    }

    fn append_runtime_slot_contributions(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        append_context: RuntimeTemplateAppendContext<'_>,
        source_accumulators: &RuntimeSlotSourceAccumulatorContext,
        fallback_location: &SourceLocation,
    ) -> Result<RuntimeSlotContributionResult, CompilerError> {
        let contribution_emitted_flag =
            self.initialize_runtime_template_emitted_flag(fallback_location)?;
        let mut renders_wrapper_unconditionally = handoff.contribution_sources.is_empty();
        let loop_control_flush = RuntimeSlotLoopControlFlush {
            wrapper_plan: &handoff.wrapper,
            target_accumulator: append_context.target_accumulator(),
            source_accumulators,
            slot_sites: &handoff.slot_sites,
            contribution_emitted_flag,
            parent_emitted_flag: append_context.emitted_output(),
        };

        for source in &handoff.contribution_sources {
            let Some(target_accumulator) = source_accumulators.local_for(source.source) else {
                return_hir_transformation_error!(
                    "Runtime slot contribution referenced a source with no allocated accumulator.",
                    self.hir_error_location(&source.location)
                );
            };

            // Missing slots and const-renderable contributions still render through the
            // wrapper with empty slot accumulators when needed. Runtime-only
            // contribution plans use the emitted flag below so false branches
            // and no-output loops can skip wrapper output.
            let source_renders_wrapper_unconditionally = source.renders_wrapper_unconditionally
                && owned_runtime_template_node_guarantees_output(
                    &source.render_root,
                    self.string_table,
                );
            if source_renders_wrapper_unconditionally {
                renders_wrapper_unconditionally = true;
            }

            let emission = self.append_runtime_slot_contribution_content(
                source,
                target_accumulator,
                loop_control_flush,
                contribution_emitted_flag,
                fallback_location,
            )?;

            match emission {
                TemplateBodyEmission::NoOutput | TemplateBodyEmission::Output => {}
                TemplateBodyEmission::Break | TemplateBodyEmission::Continue => {
                    return Ok(RuntimeSlotContributionResult {
                        emission,
                        emitted_any_contribution: contribution_emitted_flag,
                        renders_wrapper_unconditionally,
                    });
                }
            }

            let current_block = self.current_block_id_or_error(fallback_location)?;
            if self.block_has_explicit_terminator(current_block, fallback_location)? {
                break;
            }
        }

        Ok(RuntimeSlotContributionResult {
            emission: TemplateBodyEmission::NoOutput,
            emitted_any_contribution: contribution_emitted_flag,
            renders_wrapper_unconditionally,
        })
    }

    fn append_runtime_slot_contribution_content(
        &mut self,
        source: &OwnedRuntimeSlotContributionSource,
        target_accumulator: LocalId,
        loop_control_flush: RuntimeSlotLoopControlFlush<'_>,
        contribution_emitted_flag: LocalId,
        fallback_location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let append_context = RuntimeTemplateAppendContext::new(target_accumulator)
            .with_emitted_output(Some(contribution_emitted_flag))
            .with_loop_control_flush(loop_control_flush);

        self.append_owned_runtime_template_node_to_accumulator(
            &source.render_root,
            append_context,
            None,
            fallback_location,
        )
    }

    fn append_runtime_slot_wrapper(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        append_context: RuntimeTemplateAppendContext<'_>,
        source_accumulators: &RuntimeSlotSourceAccumulatorContext,
        location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let wrapper_context = append_context
            .with_runtime_slot_sites(source_accumulators, &handoff.slot_sites)
            .rejecting_unresolved_slots();
        let emission = self.append_owned_runtime_template_node_to_accumulator(
            &handoff.wrapper,
            wrapper_context,
            None,
            location,
        )?;

        if append_context.emitted_output().is_some() && emission == TemplateBodyEmission::Output {
            return Ok(TemplateBodyEmission::NoOutput);
        }

        Ok(emission)
    }

    fn append_runtime_slot_wrapper_if_contributed(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        append_context: RuntimeTemplateAppendContext<'_>,
        source_accumulators: &RuntimeSlotSourceAccumulatorContext,
        emitted_any_contribution: LocalId,
        location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let condition_block = self.current_block_id_or_error(location)?;
        let parent_region = self.current_region_or_error(location)?;
        let rendered_region = self.create_child_region(parent_region);
        let skipped_region = self.create_child_region(parent_region);
        let rendered_block =
            self.create_block(rendered_region, location, "runtime-slot-rendered")?;
        let skipped_block = self.create_block(skipped_region, location, "runtime-slot-skipped")?;
        let condition = self.make_local_load_expression(
            emitted_any_contribution,
            builtin_type_ids::BOOL,
            location,
            parent_region,
        );

        self.emit_terminator(
            condition_block,
            HirTerminator::If {
                condition,
                then_block: rendered_block,
                else_block: skipped_block,
            },
            location,
        )?;

        // Runtime-only contribution plans render their wrapper only when a
        // contribution produced structural output. This preserves the documented
        // no-output behavior for false branches and no-output loops.
        self.set_current_block(rendered_block, location)?;
        self.append_runtime_slot_wrapper(handoff, append_context, source_accumulators, location)?;
        let rendered_tail = self.current_block_id_or_error(location)?;
        let rendered_terminated = self.block_has_explicit_terminator(rendered_tail, location)?;

        self.set_current_block(skipped_block, location)?;
        let skipped_tail = self.current_block_id_or_error(location)?;

        let emission = if append_context.emitted_output().is_some() {
            TemplateBodyEmission::NoOutput
        } else {
            TemplateBodyEmission::Output
        };

        if rendered_terminated {
            self.set_current_block(skipped_tail, location)?;
            return Ok(emission);
        }

        let merge_block = self.create_block(parent_region, location, "runtime-slot-merge")?;
        self.emit_jump_to(
            rendered_tail,
            merge_block,
            location,
            "runtime-slot.rendered.merge",
        )?;
        self.emit_jump_to(
            skipped_tail,
            merge_block,
            location,
            "runtime-slot.skipped.merge",
        )?;

        self.set_current_block(merge_block, location)?;
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

        OwnedRuntimeTemplateNode::Text { text, .. } => {
            !string_table.resolve(*text).trim().is_empty()
        }

        OwnedRuntimeTemplateNode::AggregateOutput => true,

        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            dynamic_expression_guarantees_output(expression, string_table)
        }

        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => match &template.body {
            OwnedRuntimeTemplateBody::Render(node) => {
                owned_runtime_template_node_guarantees_output(node, string_table)
            }
            OwnedRuntimeTemplateBody::RuntimeSlotApplication(_) => false,
        },

        // Runtime template control flow can structurally produce no output
        // after HIR evaluates its condition or iterable. Even when the body
        // shape is otherwise const-renderable, the slot wrapper must stay
        // guarded by the emitted-output flag.
        OwnedRuntimeTemplateNode::BranchChain { .. }
        | OwnedRuntimeTemplateNode::Loop { .. }
        | OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
        | OwnedRuntimeTemplateNode::LoopControl { .. } => false,

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

        ExpressionKind::RuntimeSlotApplicationHandoff(_) => false,

        ExpressionKind::Coerced { value, .. } => {
            dynamic_expression_guarantees_output(value, string_table)
        }

        ExpressionKind::Runtime(rpn) if rpn.items.len() == 1 => match &rpn.items[0] {
            ExpressionRpnItem::Operand(expression) => {
                dynamic_expression_guarantees_output(expression, string_table)
            }
            ExpressionRpnItem::Operator { .. } => true,
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
        OwnedRuntimeTemplateBody::RuntimeSlotApplication(_) => false,
    }
}
