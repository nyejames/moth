//! Shared condition-type validation for control-flow statement headers.
//!
//! WHAT: validates that a condition expression resolves to `Bool`.
//! WHY: `if`, match guards, and conditional `loop` headers should all emit the same
//! typed diagnostic payload so that error messages and suggestions are consistent.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionView;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets;
use crate::compiler_frontend::compiler_messages::{TypeMismatchContext, ValueReceiverContext};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Stage-local result for condition-type validation helpers.
type ConditionValidationResult = Result<(), DiagnosticDraft>;

/// Shared helper that checks a condition expression against `Bool`.
///
/// WHY: all control-flow condition sites (`if`, `loop`, match guard) share one
/// diagnostic path so the message, context label, and suggestion stay uniform.
pub(crate) fn ensure_boolean_condition(
    condition: ExpressionView<'_>,
    span: Option<SourceSpan>,
    type_environment: &TypeEnvironment,
) -> ConditionValidationResult {
    let condition_type = condition.require_single(ValueReceiverContext::Condition)?;
    if condition_type == type_environment.builtins().bool {
        return Ok(());
    }

    Err(types_borrows_targets::type_mismatch(
        type_environment.builtins().bool,
        condition_type,
        TypeMismatchContext::Condition,
        span,
    ))
}

/// Validate `if` statement condition type with centralized diagnostics policy.
pub(crate) fn ensure_if_statement_condition(
    condition: ExpressionView<'_>,
    type_environment: &TypeEnvironment,
) -> ConditionValidationResult {
    ensure_boolean_condition(condition, condition.span(), type_environment)
}

/// Validate `<pattern> if <guard> =>` guard type with match-specific diagnostics.
pub(crate) fn ensure_match_guard_condition(
    condition: ExpressionView<'_>,
    type_environment: &TypeEnvironment,
) -> ConditionValidationResult {
    ensure_boolean_condition(condition, condition.span(), type_environment)
}

/// Validate conditional-loop header type with centralized diagnostics policy.
pub(crate) fn ensure_loop_condition(
    condition: ExpressionView<'_>,
    type_environment: &TypeEnvironment,
) -> ConditionValidationResult {
    ensure_boolean_condition(condition, condition.span(), type_environment)
}

/// Return whether the token after `if` is a boundary rather than a condition.
pub(crate) fn if_condition_is_missing(token_stream: &AstCursor) -> bool {
    matches!(
        token_stream.current_tag(),
        TokenTag::COLON
            | TokenTag::THEN
            | TokenTag::ELSE
            | TokenTag::NEWLINE
            | TokenTag::END
            | TokenTag::EOF
    )
}
