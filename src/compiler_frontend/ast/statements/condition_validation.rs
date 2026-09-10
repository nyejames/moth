//! Shared condition-type validation for control-flow statement headers.
//!
//! WHAT: validates that a condition expression resolves to `Bool`.
//! WHY: `if`, match guards, and conditional `loop` headers should all emit the same
//! typed diagnostic payload so that error messages and suggestions are consistent.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, TypeMismatchContext};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Stage-local result for condition-type validation helpers.
///
/// The helper returns the plain `CompilerDiagnostic` used by the diagnosed
/// frontend lane; callers preserve that value until their owning boundary.
type ConditionValidationResult = Result<(), CompilerDiagnostic>;

/// Shared helper that checks a condition expression against `Bool`.
///
/// WHY: all control-flow condition sites (`if`, `loop`, match guard) share one
/// diagnostic path so the message, context label, and suggestion stay uniform.
pub(crate) fn ensure_boolean_condition(
    condition: &Expression,
    span: Option<SourceSpan>,
    type_environment: &TypeEnvironment,
) -> ConditionValidationResult {
    if condition.type_id == type_environment.builtins().bool {
        return Ok(());
    }

    Err(CompilerDiagnostic::type_mismatch(
        type_environment.builtins().bool,
        condition.type_id,
        TypeMismatchContext::Condition,
        span,
    ))
}

/// Validate `if` statement condition type with centralized diagnostics policy.
pub(crate) fn ensure_if_statement_condition(
    condition: &Expression,
    type_environment: &TypeEnvironment,
) -> ConditionValidationResult {
    ensure_boolean_condition(condition, condition.span, type_environment)
}

/// Validate `<pattern> if <guard> =>` guard type with match-specific diagnostics.
pub(crate) fn ensure_match_guard_condition(
    condition: &Expression,
    type_environment: &TypeEnvironment,
) -> ConditionValidationResult {
    ensure_boolean_condition(condition, condition.span, type_environment)
}

/// Validate conditional-loop header type with centralized diagnostics policy.
pub(crate) fn ensure_loop_condition(
    condition: &Expression,
    type_environment: &TypeEnvironment,
) -> ConditionValidationResult {
    ensure_boolean_condition(condition, condition.span, type_environment)
}

/// Return whether the token after `if` is a boundary rather than a condition.
pub(crate) fn if_condition_is_missing(token_stream: &FileTokens) -> bool {
    matches!(
        token_stream.current_token_kind(),
        TokenKind::Colon
            | TokenKind::Then
            | TokenKind::Else
            | TokenKind::Newline
            | TokenKind::End
            | TokenKind::Eof
    )
}
