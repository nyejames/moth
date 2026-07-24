//! Helpful syntax-error detection for common language-mismatch mistakes.
//!
//! WHAT: Detects patterns that new Moth users write when bringing habits from
//! C, Rust, Python, JavaScript, and other languages. These are not generic syntax
//! errors — they are specific, actionable mistakes that deserve targeted guidance.
//!
//! WHY: Centralizing this logic keeps the parser dispatch loops focused on grammar
//! and makes it easy to add new "did you mean" hints without bloating every parse
//! file. Each position (expression, statement, signature) gets its own module so
//! ownership and context requirements stay explicit.

pub(crate) mod expression_position;
pub(crate) mod signature_position;
pub(crate) mod statement_position;

use crate::compiler_frontend::compiler_messages::{CommonSyntaxMistakeReason, CompilerDiagnostic};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

#[cfg(test)]
mod tests;

/// Build a typed diagnostic for a known syntax habit from another language.
///
/// WHAT: keeps the detector modules focused on recognizing token patterns while the diagnostic
/// payload carries only a stable reason enum.
/// WHY: renderer text belongs at the render boundary, not inside syntax scanning helpers.
pub(crate) fn common_syntax_mistake(
    reason: CommonSyntaxMistakeReason,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::common_syntax_mistake(reason, location)
}
