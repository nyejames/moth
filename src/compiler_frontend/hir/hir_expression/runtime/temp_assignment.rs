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
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

impl<'a> HirBuilder<'a> {
    pub(super) fn materialize_short_circuit_jump_argument_local(
        &mut self,
        value: HirExpression,
        location: &SourceLocation,
    ) -> Result<LocalId, CompilerError> {
        let value = self.materialize_short_circuit_assignment_value(value, location);
        let local = self.allocate_temp_local(value.ty, Some(location.to_owned()))?;
        self.emit_assign_local_statement(local, value, location)?;
        Ok(local)
    }

    fn materialize_short_circuit_assignment_value(
        &mut self,
        value: HirExpression,
        location: &SourceLocation,
    ) -> HirExpression {
        // Assigning a place expression directly into the branch-merge temp can preserve aliasing
        // edges to user locals. Materialize as a copied value so branch-local temps stay detached.
        // The copy preserves the incoming expression span; generated values stay spanless.
        let span = value.span;
        if let HirExpressionKind::Load(place) = value.kind {
            let mut copied = self.make_expression(
                location,
                HirExpressionKind::Copy(place),
                value.ty,
                ValueKind::RValue,
                value.region,
            );
            copied.span = span;
            return copied;
        }

        value
    }
}
