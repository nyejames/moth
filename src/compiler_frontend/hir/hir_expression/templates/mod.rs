//! Runtime-template lowering entry points for HIR expression construction.
//!
//! WHAT: routes finalized AST runtime templates through the inline accumulator path.
//! WHY: AST owns template composition, foldability, and runtime handoff preparation. HIR only
//! lowers the owned runtime surface that remains after those decisions are complete.
//!
//! Submodule map:
//! - `append_context`: shared append target and runtime slot source/site context.
//! - `linear`: ordinary owned-node appending for runtime templates without control flow.
//! - `control_flow`: structured `if` / `loop` dispatch that mutates the enclosing CFG lazily.
//! - `render_append`: owned-node appending and string coercion shared by runtime paths.
//! - `option_capture`: option-present template `if` capture lowering.
//! - `aggregate`: shared aggregate wrapping after a loop or child emitted output.
//! - `slot_application`: AST-planned runtime slots lowered through slot accumulators.

use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::LoweredExpression;

mod aggregate;
mod append_context;
mod control_flow;
mod linear;
mod option_capture;
mod render_append;
mod slot_application;

impl<'a> HirBuilder<'a> {
    // WHAT: Dispatches a runtime template that has already been materialized
    // into the neutral AST-owned handoff shape.
    // WHY: runtime templates cross the AST/HIR boundary as owned TIR-derived
    // handoff payloads after AST finalization replaces raw template expressions.
    pub(crate) fn lower_runtime_template_expression_from_owned_handoff(
        &mut self,
        handoff: &OwnedRuntimeTemplateHandoff,
        span_ref: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, CompilerError> {
        self.validate_runtime_template_handoff_lowering_input(handoff, span_ref)?;

        match &handoff.body {
            OwnedRuntimeTemplateBody::RuntimeSlotApplication(handoff) => {
                self.lower_runtime_slot_application_template_expression(handoff, span_ref)
            }

            OwnedRuntimeTemplateBody::Render(node) => {
                if is_owned_runtime_template_node_control_flow(node) {
                    self.lower_runtime_control_flow_template_expression(node, span_ref)
                } else if self.owned_runtime_template_node_has_runtime_dependency(node) {
                    self.lower_runtime_reactive_linear_template_expression_from_owned_node(
                        node, span_ref,
                    )
                } else {
                    self.lower_runtime_linear_template_expression(node, span_ref)
                }
            }
        }
    }

    // WHAT: Lowers a runtime slot application after AST has already routed the
    // wrapper, contribution sources, and slot-site render plan into owned data.
    // WHY: this is the direct HIR entry point for the final expression variant.
    // It keeps slot lowering on the same accumulator path as the owned runtime
    // template handoff without exposing TIR identities.
    pub(crate) fn lower_runtime_slot_application_expression_from_owned_handoff(
        &mut self,
        handoff: &OwnedRuntimeSlotApplicationHandoff,
        span_ref: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, CompilerError> {
        self.lower_runtime_slot_application_template_expression(handoff, span_ref)
    }

    fn validate_runtime_template_handoff_lowering_input(
        &mut self,
        handoff: &OwnedRuntimeTemplateHandoff,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        if runtime_template_handoff_has_top_level_loop_control(handoff) {
            return_hir_transformation_error!(
                "Template loop-control signal reached HIR outside an owned template loop body.",
                self.hir_error_location(span_ref)
            );
        }

        Ok(())
    }
}

fn is_owned_runtime_template_node_control_flow(node: &OwnedRuntimeTemplateNode) -> bool {
    matches!(
        node,
        OwnedRuntimeTemplateNode::BranchChain { .. }
            | OwnedRuntimeTemplateNode::Loop { .. }
            | OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
            | OwnedRuntimeTemplateNode::LoopControl { .. }
    )
}

fn runtime_template_handoff_has_top_level_loop_control(
    handoff: &OwnedRuntimeTemplateHandoff,
) -> bool {
    matches!(
        &handoff.body,
        OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::LoopControl { .. })
    )
}
