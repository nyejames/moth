//! Pure numeric-text grammar predicates.
//!
//! WHAT: tiny, testable building blocks for recognizing characters that may appear
//!       in Moth numeric literals.
//! WHY: keeping these predicates separate from the scanner makes the grammar rules
//!      explicit and avoids duplicating character classes across tokenizer and casts.

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
