//! Shared identifier shape and reserved user-name policy.
//!
//! WHAT: defines the compiler-independent character rules and whole-name reservations used by
//! source identifiers and literal-data schemas.
//! WHY: compiler and MON readers must agree on identifier shape and reserved word families without
//! making the lexical crate depend on compiler token or diagnostic types.

use crate::words::is_reserved_source_word;

/// Returns whether `character` can begin an identifier.
pub fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

/// Returns whether `character` can continue an identifier.
pub fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

/// Returns whether `text` has the source identifier shape.
///
/// Identifier spelling is case-sensitive and is not normalised. This checks shape only; reserved
/// user-name policy is a separate question.
pub fn is_identifier(text: &str) -> bool {
    let mut characters = text.chars();
    characters.next().is_some_and(is_identifier_start) && characters.all(is_identifier_continue)
}

/// Returns whether `name`, after leading underscores are removed, belongs to the reserved `Dec`
/// family.
///
/// The bare family name and ASCII-digit suffixes reserve every case spelling. Trailing letters and
/// non-ASCII digits do not match.
pub fn is_reserved_dec_family_name(name: &str) -> bool {
    is_dec_family_spelling(name.trim_start_matches('_'))
}

/// Returns whether `name` is reserved for a user-chosen identifier.
///
/// Leading underscores do not reclaim a name. The empty spelling left by stripping underscores is
/// not reserved. Source-word matches are ASCII-case-insensitive and whole-name only; the broader
/// `Dec` family is checked separately because its digit suffixes are not exact source words.
pub fn is_reserved_user_name(name: &str) -> bool {
    let stripped = name.trim_start_matches('_');
    !stripped.is_empty() && (is_reserved_source_word(stripped) || is_dec_family_spelling(stripped))
}

fn is_dec_family_spelling(spelling: &str) -> bool {
    let Some(prefix) = spelling.get(..3) else {
        return false;
    };

    prefix.eq_ignore_ascii_case("Dec") && spelling[3..].bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
#[path = "tests/identifier_tests.rs"]
mod tests;
