//! Compiler-independent identifier, source-word and numeric-text rules shared by Moth and MON.
//!
//! [`identifier`] owns Unicode identifier shape and reserved user-name policy. [`words`] owns
//! exact source-word identities and neutral presentation facts, not compiler token tags or
//! naming-style diagnostics. Spelling stays case-sensitive without Unicode normalisation.
//! Reservation strips leading underscores and matches whole names without regard to ASCII case,
//! including fixed numeric names and the `Dec` family with any ASCII-digit suffix.
//!
//! [`numeric`] accepts literal text and explicit destination facts, then parses, materialises or
//! formats fixed-width integers, `Byte`, F16/F32/F64 and profile-dependent `Int`/`Uint`/`Float`.
//! [`numeric::profile::NumericProfile`] supports all four 32/64-bit width/precision combinations
//! and defaults to Int32/Float64. Decimal text facts cover `Dec`/`Dec0` through `Dec256` without
//! owning arbitrary-precision coefficient arithmetic.
//!
//! The crate provides no file I/O, document parser, schemas, expression evaluation, compiler token
//! storage, type lookup or numeric arithmetic. Callers select destinations and project structured
//! numeric failure reasons into their own error boundary. `moth-mon` owns immutable prepared schemas,
//! owned document values and `MonError` with `Display`/`std::error::Error`, not this crate.
//!
//! ```
//! use moth_lexical::identifier::{is_identifier, is_reserved_user_name};
//! use moth_lexical::numeric::parse::parse_numeric_literal;
//!
//! assert!(is_identifier("Δ_count"));
//! assert!(!is_reserved_user_name("Δ_count"));
//! assert!(is_identifier("__DeC257"));
//! assert!(is_reserved_user_name("__DeC257"));
//! assert!(!is_reserved_user_name("DecBox"));
//!
//! let literal = parse_numeric_literal("1_200.50e-2").expect("valid unsigned literal");
//! assert_eq!(literal.normalized_text, "1200.50e-2");
//! ```

pub mod identifier;
pub mod numeric;
pub mod words;

/// Returns whether `character` is a Moth logical line-break character.
///
/// Only LF and CR are line breaks; CRLF combines these two characters into one boundary.
pub fn is_line_break(character: char) -> bool {
    matches!(character, '\n' | '\r')
}

#[cfg(test)]
#[path = "tests/line_break_tests.rs"]
mod tests;
