//! Common language-mismatch mistakes in expression position.
//!
//! WHAT: Detects patterns like `==`, `!=`, `&&`, `||`, `!expr`, `&expr` that
//! users from C-family languages write when they first encounter Moth.
//!
//! WHY: These are unambiguous syntax errors at the token level. Catching them
//! early with specific guidance prevents confusion before generic "invalid token"
//! messages waste the user's time.

use super::common_syntax_mistake;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::{CommonSyntaxMistakeReason, CompilerDiagnostic};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Check for common expression-position mistakes before falling back to a generic error.
///
/// WHAT: inspects the current token (and sometimes the next token) for patterns
/// that are valid in other languages but not in Moth.
///
/// Returns `Some(diagnostic)` when a known mistake is detected, `None` otherwise.
pub(crate) fn check_expression_common_mistake(
    token_stream: &AstCursor,
    expression_is_empty: bool,
) -> Option<CompilerDiagnostic> {
    let current = token_stream.current_tag();
    let next = token_stream.peek_next_tag();
    let location = token_stream.current_span();

    match current {
        // `==`  →  `is`
        TokenTag::ASSIGN if next == Some(TokenTag::ASSIGN) => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::EqualityOperator,
            location,
        )),

        // `!=`  →  `is not`
        TokenTag::BANG if next == Some(TokenTag::ASSIGN) => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::InequalityOperator,
            location,
        )),

        // `&&`  →  `and`
        TokenTag::AMPERSAND if next == Some(TokenTag::AMPERSAND) => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::LogicalAndOperator,
            location,
        )),

        // `||`  →  `or`
        TokenTag::TYPE_PARAMETER_BRACKET if next == Some(TokenTag::TYPE_PARAMETER_BRACKET) => Some(
            common_syntax_mistake(CommonSyntaxMistakeReason::LogicalOrOperator, location),
        ),

        // `!` used as boolean negation (not fallible handling)
        // Fallible handling `!` is parsed as a postfix suffix after the primary expression,
        // so encountering `Bang` at the start of an operand or after an operator means
        // the user is trying to use it as unary negation.
        TokenTag::BANG if expression_is_empty => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::BooleanBangNegation,
            location,
        )),

        // Single `=` in expression position where it is not valid.
        // `=` is only valid in declarations and assignments (statement position).
        TokenTag::ASSIGN => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::ExpressionAssignment,
            location,
        )),

        // Single `&` in expression position (likely Rust borrow attempt)
        TokenTag::AMPERSAND if expression_is_empty => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::RustBorrowPrefix,
            location,
        )),

        // `as` outside its three supported domains
        TokenTag::AS => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::InvalidAsOperator,
            location,
        )),

        _ => None,
    }
}
