//! Pure numeric-text grammar predicates.
//!
//! WHAT: tiny, testable building blocks for recognizing characters that may appear
//!       in Moth numeric literals.
//! WHY: keeping these predicates separate from the scanner makes the grammar rules
//!      explicit and avoids duplicating character classes across tokenizer and casts.

/// Lexical classification of a numeric literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericLiteralKind {
    /// A whole-number literal such as `42` or `1_000`.
    WholeNumber,
    /// A decimal-point literal such as `3.14`.
    DecimalPoint,
    /// An exponent literal such as `1e6` or `1.0e-21`.
    Exponent,
}

/// Sign attached to a numeric literal token.
///
/// The tokenizer front-loads attached `-`; normalized text stays unsigned so materialization can
/// apply range checks with the sign as explicit metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericLiteralSign {
    /// The literal is positive (no leading `-`).
    Positive,
    /// The literal has an attached leading `-` sign.
    Negative,
}

/// Sign explicitly written on an exponent, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericExponentSign {
    /// No explicit sign marker on the exponent (e.g. `1e6`).
    None,
    /// Explicit `+` after the exponent marker (e.g. `1e+21`).
    Positive,
    /// Explicit `-` after the exponent marker (e.g. `1e-21`).
    Negative,
}
/// Digits allowed in any digit run (integer, fractional, or exponent).
pub fn is_numeric_digit(character: char) -> bool {
    character.is_ascii_digit()
}

/// The only lowercase exponent marker supported in Moth numeric literals.
pub fn is_exponent_marker(character: char) -> bool {
    character == 'e' || character == 'E'
}

/// Signs that may appear immediately after an exponent marker.
pub fn is_exponent_sign(character: char) -> bool {
    character == '+' || character == '-'
}

/// Digit separator allowed between digits, but never adjacent, at edges, or next to
/// a decimal point or exponent marker.
pub fn is_digit_separator(character: char) -> bool {
    character == '_'
}
