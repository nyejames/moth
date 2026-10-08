//! Runtime-template render lowering through the inline accumulator path.
//!
//! WHAT: appends every AST-owned render handoff node directly into the enclosing CFG.
//! WHY: control-flow nodes already preserve lazy evaluation while using the same string
//!      accumulator as ordinary template content.

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
    // WHAT: Lowers a render handoff by appending its AST-owned node inline.
    // WHY: one entry preserves the same accumulator semantics for ordinary content and lazy
    //      conditional or loop nodes.
    pub(super) fn lower_runtime_template_render_expression(
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
