//! A lexical scanner for Rust source text.
//!
//! WHAT: locates which characters of a Rust file are code, rather than comment or literal text.
//! WHY:  every source-reading gate in this crate has to answer the same question before it can
//!       claim a hit is real. A gate that matches raw text reports its own doc comment, another
//!       gate's fixture strings and any prose example as findings, which is how a source audit
//!       stops being evidence and becomes noise a reader learns to skip.
//!
//! This is a scanner, not a parser. It knows exactly what can carry text resembling code:
//! comments, strings and character literals. Anything beyond that belongs to a real parser.
//!
//! # What this module owns
//! - The code/non-code mask for a Rust file
//! - Small positional helpers every consumer of that mask needs
//!
//! # What this module does NOT own
//! - What any rule means (see `honesty_audit`, `source_audit`, `feature_matrix`)
//! - Which files a gate reads (see `source_tree`)

/// Whether `characters` at `index` starts with `needle`.
pub(crate) fn matches_at(characters: &[char], index: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(offset, character)| characters.get(index + offset) == Some(&character))
}

/// What one character of a Rust file is part of.
///
/// A gate that only knows "code or not" cannot tell a banned call apart from a doc comment
/// describing it, nor from a path literal that names it. Both distinctions decide whether a hit
/// is a finding, so the scanner reports which of the three a character belongs to and lets each
/// rule say which it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextClass {
    /// Source the compiler executes.
    Code,
    /// Line or block comment text, including doc comments.
    Comment,
    /// String, raw string, byte string or character literal text.
    Literal,
}

/// Classify every character of a Rust file.
///
/// This is a scanner, not a parser: it recognises line and nested block comments, normal and raw
/// strings (with the `b` prefix) and character literals, which is everything that can carry text
/// resembling code.
pub(crate) fn classify(characters: &[char]) -> Vec<TextClass> {
    let mut classes = vec![TextClass::Code; characters.len()];
    let mut index = 0;

    while index < characters.len() {
        let skipped = skip_comment(characters, index)
            .map(|next| (next, TextClass::Comment))
            .or_else(|| skip_raw_string(characters, index).map(|next| (next, TextClass::Literal)))
            .or_else(|| {
                (characters[index] == '"')
                    .then(|| (skip_normal_string(characters, index), TextClass::Literal))
            })
            .or_else(|| {
                (characters[index] == '\'')
                    .then(|| skip_character_literal(characters, index))
                    .flatten()
                    .map(|next| (next, TextClass::Literal))
            });

        match skipped {
            Some((next, class)) => {
                let next = next.min(characters.len()).max(index + 1);
                classes[index..next].fill(class);
                index = next;
            }
            None => index += 1,
        }
    }

    classes
}

/// Which characters are code, rather than comment or literal text.
pub(crate) fn code_mask(characters: &[char]) -> Vec<bool> {
    classify(characters)
        .into_iter()
        .map(|class| class == TextClass::Code)
        .collect()
}

/// Index just past a line or nested block comment starting at `index`, if one starts there.
pub(crate) fn skip_comment(characters: &[char], index: usize) -> Option<usize> {
    if characters[index] != '/' {
        return None;
    }

    match characters.get(index + 1) {
        Some('/') => {
            let mut cursor = index + 2;
            while cursor < characters.len() && characters[cursor] != '\n' {
                cursor += 1;
            }
            Some(cursor)
        }
        Some('*') => {
            let mut cursor = index + 2;
            let mut depth = 1_usize;
            while cursor < characters.len() && depth > 0 {
                if characters[cursor] == '/' && characters.get(cursor + 1) == Some(&'*') {
                    depth += 1;
                    cursor += 2;
                } else if characters[cursor] == '*' && characters.get(cursor + 1) == Some(&'/') {
                    depth -= 1;
                    cursor += 2;
                } else {
                    cursor += 1;
                }
            }
            Some(cursor)
        }
        _ => None,
    }
}

/// Index just past a raw string starting at `index`, if one starts there.
///
/// The `r` must not continue an identifier, so `for` and `char` do not open a raw string. An
/// unterminated raw string runs to end of file, which is what the compiler would do; the
/// alternative is reading literal text as code.
pub(crate) fn skip_raw_string(characters: &[char], index: usize) -> Option<usize> {
    let prefix_len = match characters[index] {
        'r' => 1,
        'b' if characters.get(index + 1) == Some(&'r') => 2,
        _ => return None,
    };

    if index > 0 && is_identifier_character(characters[index - 1]) {
        return None;
    }

    let mut cursor = index + prefix_len;
    let mut hashes = 0_usize;
    while characters.get(cursor) == Some(&'#') {
        hashes += 1;
        cursor += 1;
    }

    if characters.get(cursor) != Some(&'"') {
        return None;
    }
    cursor += 1;

    while cursor < characters.len() {
        if characters[cursor] == '"'
            && (1..=hashes).all(|offset| characters.get(cursor + offset) == Some(&'#'))
        {
            return Some(cursor + 1 + hashes);
        }
        cursor += 1;
    }

    Some(characters.len())
}

/// Index just past a normal or byte string starting at `index`.
pub(crate) fn skip_normal_string(characters: &[char], index: usize) -> usize {
    let mut cursor = index + 1;

    while cursor < characters.len() {
        match characters[cursor] {
            '\\' => cursor += 2,
            '"' => return cursor + 1,
            _ => cursor += 1,
        }
    }

    characters.len()
}

/// Index just past a character literal starting at `index`, or `None` for a lifetime.
pub(crate) fn skip_character_literal(characters: &[char], index: usize) -> Option<usize> {
    let body_start = index + 1;

    let close = if characters.get(body_start) == Some(&'\\') {
        match characters.get(body_start + 1) {
            Some('u') if characters.get(body_start + 2) == Some(&'{') => {
                let mut cursor = body_start + 3;
                while cursor < characters.len() && characters[cursor] != '}' {
                    cursor += 1;
                }
                cursor + 1
            }
            Some('x') => body_start + 4,
            Some(_) => body_start + 2,
            None => return None,
        }
    } else {
        body_start + 1
    };

    (characters.get(close) == Some(&'\'')).then_some(close + 1)
}

pub(crate) fn is_identifier_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests;
