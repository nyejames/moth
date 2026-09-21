//! String coercion policy for the Moth compiler frontend.
//!
//! WHAT: defines what expression types are renderable as string content and
//! provides the coercion logic used at template boundaries.
//! WHY: previously, the rules for "what can become a string in a template"
//! were inlined directly into `template_folding.rs`. Moving them here makes
//! the policy explicit and reusable without touching template mechanics.

use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::numeric_text::format::format_finite_float;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Attempts to coerce a constant expression kind to its string representation
/// for use in template folding.
///
/// WHAT: converts compile-time scalar expression kinds to their string content.
/// Returns `None` for expression kinds that cannot be folded into a string at
/// compile time. Template values are handled by the TIR fold owner because
/// their classification requires the module-local store.
/// WHY: centralises the "what can fold to a string" decision that was
/// previously inlined in `template_folding::fold_plan`.
pub(crate) fn fold_expression_kind_to_string(
    kind: &ExpressionKind,
    string_table: &StringTable,
) -> Option<String> {
    match kind {
        ExpressionKind::StringSlice(string) => Some(string_table.resolve(*string).to_owned()),
        ExpressionKind::Float(value) => {
            // Compile-time Float values are finite by language contract, but the
            // formatter still returns a Result. Fold non-finite values away from
            // the compile-time path rather than panicking on an internal invariant.
            format_finite_float(*value).ok()
        }
        ExpressionKind::Int(value) => Some(value.to_string()),
        ExpressionKind::Bool(value) => Some(value.to_string()),
        ExpressionKind::Char(value) => Some(value.to_string()),
        ExpressionKind::Coerced { value, .. } => {
            // Contextual coercion nodes do not change the rendered scalar value;
            // delegate to the inner expression so coerced literals fold the same
            // way as their unwrapped counterparts.
            fold_expression_kind_to_string(&value.kind, string_table)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/string_tests.rs"]
mod string_tests;
