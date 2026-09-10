//! Temp-local materialisation for short-circuit merge values.
//!
//! WHAT: allocates a temporary `LocalId` and emits an assignment so that the result
//!       of a short-circuit expression can be consumed uniformly from the merge block.
//! WHY: branch-local expressions must be lifted to named locals before the merge
//!      block so the merge block can read a single stable place.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::source::SourceSpan;

impl<'a> HirBuilder<'a> {
    pub(super) fn materialize_short_circuit_jump_argument_local(
        &mut self,
        value: HirExpression,
        source_span: &Option<SourceSpan>,
    ) -> Result<LocalId, CompilerError> {
        let value = self.materialize_short_circuit_assignment_value(value);
        let local = self.allocate_temp_local(value.ty, None)?;
        self.emit_assign_local_statement(local, value, source_span)?;
        Ok(local)
    }

    fn materialize_short_circuit_assignment_value(
        &mut self,
        value: HirExpression,
    ) -> HirExpression {
        // Assigning a place expression directly into the branch-merge temp can preserve aliasing
        // edges to user locals. Materialize as a copied value so branch-local temps stay detached.
        // The copy keeps the incoming authored span and its side-table mapping.
        let span = value.span;
        if let HirExpressionKind::Load(place) = value.kind {
            return self.make_expression(
                &span,
                HirExpressionKind::Copy(place),
                value.ty,
                ValueKind::RValue,
                value.region,
            );
        }

        value
    }
}
