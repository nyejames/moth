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
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};

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
        expression: &HirExpression,
        use_context: JsValueUse,
    ) -> Result<String, CompilerError> {
        match use_context {
            JsValueUse::PlainExpression | JsValueUse::HostCallArgument => {
                // Ordinary string contexts receive a plain string snapshot.
                self.lower_concrete_value(expression)
            }

            JsValueUse::AssignmentValue => {
                // Assignment preserves reactive template values so they can be inserted later.
                if self.value_is_reactive_template(expression.id) {
                    self.lower_reactive_template_value(expression)
                } else {
                    self.lower_concrete_value(expression)
                }
            }

            JsValueUse::MothCallArgument => {
                // String parameters that receive reactive templates should pass the template value
                // object, not a binding wrapper, so the callee can preserve reactivity.
                if self.value_is_reactive_template(expression.id) {
                    self.lower_reactive_template_value(expression)
                } else {
                    self.lower_call_argument_value(expression)
                }
            }
        }
    }

    /// Lower a HIR expression for a user-function return.
    ///
    /// WHAT: produces the raw JS value that the caller will receive in a fresh binding.
    /// WHY: ordinary returns preserve allocation identity by reading the raw value. Only
    /// explicit `copy` requests an independent graph via `__moth_clone_value`. Reactive
    /// template values preserve their specialised representation. This name avoids "fresh"
    /// to prevent confusion with the semantic `FunctionReturnAliasSummary::Fresh` lattice
    /// value, which describes whether the returned root aliases a parameter root.
    pub(crate) fn lower_moth_return_value(
        &mut self,
        expression: &HirExpression,
    ) -> Result<String, CompilerError> {
        // Reactive template values preserve their specialised representation. They must not
        // pass through generic deep-object cloning or template snapshotting.
        if self.value_is_reactive_template(expression.id) {
            return self.lower_reactive_template_value(expression);
        }

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
                let lowered = elements
                    .iter()
                    .map(|element| self.lower_moth_return_value(element))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("[{}]", lowered.join(", ")))
            }
            _ => self.lower_expr(expression),
        }
    }

    fn lower_concrete_value(
        &mut self,
        expression: &HirExpression,
    ) -> Result<String, CompilerError> {
        match &expression.kind {
            HirExpressionKind::Load(place) => {
                Ok(format!("__moth_read({})", self.lower_place(place)?))
            }
            HirExpressionKind::Copy(place) => Ok(format!(
                "__moth_clone_value(__moth_read({}))",
                self.lower_place(place)?
            )),
            _ => self.lower_expr(expression),
        }
    }

    fn lower_call_argument_value(
        &mut self,
        expression: &HirExpression,
    ) -> Result<String, CompilerError> {
        match &expression.kind {
            HirExpressionKind::Load(place) => self.lower_place(place),
            HirExpressionKind::Copy(place) => Ok(format!(
                "__moth_binding(__moth_clone_value(__moth_read({})))",
                self.lower_place(place)?
            )),
            _ => Ok(format!("__moth_binding({})", self.lower_expr(expression)?)),
        }
    }
}
