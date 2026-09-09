//! Literal lowering helpers for HIR expression construction.
//!
//! WHAT: centralizes the common lowering path for compile-time scalar literals.
//! WHY: these cases differ only by the final HIR expression kind, so one helper keeps the main
//! dispatcher smaller and removes repeated region/type boilerplate.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::source::SourceSpan;

use super::LoweredExpression;

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_literal_expression(
        &mut self,
        span: &Option<SourceSpan>,
        type_id: FrontendTypeId,
        kind: HirExpressionKind,
    ) -> Result<LoweredExpression, CompilerError> {
        let region = self.current_region_or_error(span)?;
        let ty = self.lower_type_id(type_id, span)?;
        Ok(LoweredExpression {
            prelude: vec![],
            value: self.make_expression(span, kind, ty, ValueKind::Const, region),
        })
    }
}
