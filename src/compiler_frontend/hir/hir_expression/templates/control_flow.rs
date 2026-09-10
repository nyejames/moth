//! Structured runtime template control-flow lowering.
//!
//! WHAT: lowers runtime template `if` and `loop` control flow inline into the enclosing HIR CFG.
//! WHY: branch and loop bodies must remain lazy; eager helper-call arguments would evaluate
//!      inactive template content before dispatch.

use crate::compiler_frontend::ast::templates::OwnedRuntimeTemplateNode;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_expression::LoweredExpression;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::append_context::RuntimeTemplateAppendContext;

impl<'a> HirBuilder<'a> {
    // WHAT: Lowers structured runtime template control flow directly into the enclosing CFG.
    // WHY: branch content must stay lazy; branch chunks must not be lowered before the condition
    //      chooses the active render path.
    pub(super) fn lower_runtime_control_flow_template_expression(
        &mut self,
        node: &OwnedRuntimeTemplateNode,
        span_ref: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, CompilerError> {
        if !is_control_flow_node(node) {
            return_hir_transformation_error!(
                "Runtime control-flow template lowering was called for a non-control-flow owned node.",
                self.hir_error_location(span_ref)
            );
        }

        let accumulator = self.initialize_runtime_template_accumulator(span_ref)?;

        self.append_owned_runtime_template_node_to_accumulator(
            node,
            RuntimeTemplateAppendContext::new(accumulator),
            None,
            span_ref,
        )?;

        let region = self.current_region_or_error(span_ref)?;
        let value = self.make_expression(
            span_ref,
            HirExpressionKind::Copy(HirPlace::Local(accumulator)),
            builtin_type_ids::STRING,
            ValueKind::RValue,
            region,
        );

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }
}

fn is_control_flow_node(node: &OwnedRuntimeTemplateNode) -> bool {
    matches!(
        node,
        OwnedRuntimeTemplateNode::BranchChain { .. }
            | OwnedRuntimeTemplateNode::Loop { .. }
            | OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
            | OwnedRuntimeTemplateNode::LoopControl { .. }
    )
}
