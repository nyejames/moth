//! Linear runtime-template lowering through the inline accumulator path.
//!
//! WHAT: appends ordinary runtime template owned handoff nodes directly into the enclosing CFG.
//! WHY: linear and control-flow templates must share one runtime concatenation path so future
//!      template features do not have to preserve separate call-based semantics.

use crate::compiler_frontend::ast::templates::OwnedRuntimeTemplateNode;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_expression::LoweredExpression;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::source::SourceSpan;

use super::append_context::RuntimeTemplateAppendContext;

impl<'a> HirBuilder<'a> {
    // WHAT: Lowers ordinary runtime templates by appending their AST-owned handoff node inline.
    // WHY: this keeps string coercion and chunk ordering owned by `render_append` for every
    //      runtime template shape instead of splitting linear templates into helper functions.
    pub(super) fn lower_runtime_linear_template_expression(
        &mut self,
        node: &OwnedRuntimeTemplateNode,
        span_ref: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
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
            HirExpressionKind::Copy(HirPlace::local(accumulator)),
            builtin_type_ids::STRING,
            ValueKind::RValue,
            region,
        )?;

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }
}
