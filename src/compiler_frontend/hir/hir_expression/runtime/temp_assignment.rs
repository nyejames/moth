//! Temp-local materialisation for short-circuit merge values.
//!
//! WHAT: allocates a temporary `LocalId` and emits a definition so that the result
//!       of a short-circuit expression can be consumed uniformly from the merge block.
//! WHY: branch-local expressions must be lifted to named locals before the merge
//!      block so the merge block can read a single stable place.

use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::statements::HirWriteTarget;
use crate::compiler_frontend::source::SourceSpan;

impl<'a> HirBuilder<'a> {
    pub(super) fn materialize_short_circuit_jump_argument_local(
        &mut self,
        value: HirValueId,
        source_span: &Option<SourceSpan>,
    ) -> Result<LocalId, HirConstructionFailure> {
        let value = self.materialize_short_circuit_result_value(value)?;
        let value_type = self.module.expressions.expression(value).ty;
        let local = self.allocate_temp_local(value_type, None)?;
        self.emit_write_statement(HirWriteTarget::DefineLocal(local), value, source_span)?;
        Ok(local)
    }

    fn materialize_short_circuit_result_value(
        &mut self,
        value: HirValueId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        // The expression row decides whether this load is still a binding-cell place. The new
        // local receives the produced value while retaining allocation provenance.
        let place_value = {
            let row = self.module.expressions.expression(value);
            match (&row.kind, row.value_kind) {
                (HirExpressionKind::Load(place), ValueKind::Place) => {
                    Some((*place, row.span, row.ty, row.region))
                }
                _ => None,
            }
        };
        if let Some((place, span, ty, region)) = place_value {
            return self.make_expression(
                &span,
                HirExpressionKind::Load(place),
                ty,
                ValueKind::RValue,
                region,
            );
        }

        Ok(value)
    }
}
