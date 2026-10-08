//! Value-use context lowering for the JavaScript backend.
//!
//! WHAT: centralizes how HIR `Load` and `Copy` expressions are lowered depending on the JS
//! consumption context. User-function return values use a raw-value ABI: the caller receives
//! a fresh binding holding the raw JS value, never a Moth binding cell.
//! WHY: Moth calls, host/external calls, assignments, returns, and plain expressions each
//! use a different value policy. Explicit contexts prevent duplicated `Load`/`Copy` branches
//! across expression and statement lowering, and make the ABI boundary between Moth
//! reference bindings and raw JS values explicit. User-function arguments may use the
//! internal binding-reference parameter ABI, but results always cross as raw values into
//! fresh caller bindings. Inferred alias summaries drive borrow validation and do not select
//! JS assignment mode. Only source `copy` selects deep cloning.

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::HirValueId;

/// Context in which a lowered JS expression will be consumed.
///
/// WHAT: names the value-use site so the emitter can apply the correct ABI policy.
/// WHY: Moth functions speak a reference ABI for arguments (places are passed as binding
/// refs, rvalues are wrapped in `__moth_binding`), while host/external JS calls cross into
/// raw JS and receive concrete values without binding wrappers. User-function results use
/// a raw-value ABI: the caller receives a fresh binding holding the raw value.
/// Assignments need concrete values suitable for write-through or rebinding.
pub(crate) enum JsValueUse {
    /// Ordinary expression position: produces a concrete JS value.
    PlainExpression,

    /// Assignment or write target: produces the concrete JS value to store.
    AssignmentValue,

    /// Argument to a Moth (user-defined or source-backed package) function call.
    /// Places are passed as binding references; rvalues are wrapped in `__moth_binding(...)`.
    MothCallArgument,

    /// Argument to a host or external JS function call.
    /// These cross the Moth-reference ABI boundary and receive raw JS values.
    HostCallArgument,
}

impl<'hir> JsEmitter<'hir> {
    /// Lower a HIR expression according to the consumption context.
    ///
    /// WHAT: selects the correct lowering for `Load`, `Copy`, and recursive contexts
    /// (such as tuple returns) based on how the resulting JS value will be used.
    pub(crate) fn lower_expression_for_use(
        &mut self,
        expression_id: HirValueId,
        use_context: JsValueUse,
    ) -> Result<String, CompilerError> {
        match use_context {
            JsValueUse::PlainExpression
            | JsValueUse::AssignmentValue
            | JsValueUse::HostCallArgument => self.lower_concrete_value(expression_id),
            JsValueUse::MothCallArgument => self.lower_call_argument_value(expression_id),
        }
    }

    /// Lower a HIR expression for a user-function return.
    ///
    /// WHAT: produces the raw JS value that the caller will receive in a fresh binding.
    /// WHY: ordinary returns preserve allocation identity by reading the raw value. Only
    /// explicit `copy` requests an independent graph via `__moth_clone_value`. This name avoids
    /// "fresh" to prevent confusion with the semantic `FunctionReturnAliasSummary::Fresh` lattice
    /// value, which describes whether the returned root aliases a parameter root.
    pub(crate) fn lower_moth_return_value(
        &mut self,
        expression_id: HirValueId,
    ) -> Result<String, CompilerError> {
        let hir = self.hir;
        let expression = hir.expressions.expression(expression_id);
        match &expression.kind {
            // Ordinary return: read the raw value. This preserves allocation identity. The
            // caller receives it in a fresh binding, but the underlying allocation is shared.
            HirExpressionKind::Load(place) => {
                Ok(format!("__moth_read({})", self.lower_place(place)?))
            }
            // Explicit copy: deep-clone the value to create an independent result graph.
            HirExpressionKind::Copy(place) => Ok(format!(
                "__moth_clone_value(__moth_read({}))",
                self.lower_place(place)?
            )),
            HirExpressionKind::TupleConstruct { elements } => {
                let lowered = hir
                    .expressions
                    .values(*elements)
                    .iter()
                    .map(|element| self.lower_moth_return_value(*element))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("[{}]", lowered.join(", ")))
            }
            _ => self.lower_expr(expression_id),
        }
    }

    fn lower_concrete_value(&mut self, expression_id: HirValueId) -> Result<String, CompilerError> {
        let hir = self.hir;
        let expression = hir.expressions.expression(expression_id);
        match &expression.kind {
            HirExpressionKind::Load(place) => {
                Ok(format!("__moth_read({})", self.lower_place(place)?))
            }
            HirExpressionKind::Copy(place) => Ok(format!(
                "__moth_clone_value(__moth_read({}))",
                self.lower_place(place)?
            )),
            _ => self.lower_expr(expression_id),
        }
    }

    fn lower_call_argument_value(
        &mut self,
        expression_id: HirValueId,
    ) -> Result<String, CompilerError> {
        let hir = self.hir;
        let expression = hir.expressions.expression(expression_id);
        match expression.value_kind {
            ValueKind::Place => {
                let HirExpressionKind::Load(place) = &expression.kind else {
                    return Err(CompilerError::compiler_error(
                        "JavaScript backend received a place-valued expression without a direct place load",
                    ));
                };
                self.lower_place(place)
            }
            ValueKind::RValue | ValueKind::Const => Ok(format!(
                "__moth_binding({})",
                self.lower_concrete_value(expression_id)?
            )),
        }
    }
}
