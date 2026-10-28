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

#[cfg(test)]
mod tests;
