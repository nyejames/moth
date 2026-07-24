//! Common language-mismatch mistakes in declaration/signature position.
//!
//! WHAT: Detects patterns like `name(a, b)` and `name(a: Int)` that users from
//! C-family languages write when declaring functions or structs in Moth,
//! and misplaced `as` in variable declarations.
//!
//! WHY: Parameter and field delimiters are one of the most visually striking
//! differences between Moth and other languages, so they deserve targeted
//! guidance at the point of failure. `as` is also rejected here because it is
//! called from both signature-members and body-local declaration parsers.

use super::common_syntax_mistake;
use crate::compiler_frontend::compiler_messages::{CommonSyntaxMistakeReason, CompilerDiagnostic};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Check for common signature-position mistakes before falling back to a generic error.
///
/// Called from declaration-shell and signature-members parsers when an
/// unexpected token appears while parsing parameter lists or struct fields.
pub(crate) fn check_signature_common_mistake(
    token_stream: &FileTokens,
) -> Option<CompilerDiagnostic> {
    let current = token_stream.current_token_kind();
    let location = token_stream.current_location();

    match current {
        // `(` where `|` is expected for parameters/fields
        TokenKind::OpenParenthesis => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::SignatureParenthesisDelimiter,
            location,
        )),

        // `as` is not valid in parameter/field or declaration position
        TokenKind::As => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::SignatureAsKeyword,
            location,
        )),

        _ => None,
    }
}
