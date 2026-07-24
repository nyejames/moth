//! Shared numeric text grammar for the Moth frontend.
//!
//! WHAT: classifies and parses numeric literal text without depending on AST, HIR,
//!       or backend concepts.
//! WHY: source literals and future string casts must agree on separator, exponent,
//!      sign, and digit-count rules.

pub mod format;
pub mod parse;
pub mod token;

pub(crate) mod grammar;
