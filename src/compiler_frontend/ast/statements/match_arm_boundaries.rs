//! Match-arm boundary scanning.
//!
//! WHAT: determines whether the current token position begins a new match arm header.
//! WHY: without the `case` keyword, body parsing needs a shared, deterministic way to
//! know when one arm ends and the next begins.
//!
//! Boundary rule:
//! - A candidate normal arm starts only when the current token is the first real token
//!   of a logical line and its header contains a top-level `=>` before `End` or `Eof`.
//! - Headers normally end on their physical line. A guarded header may continue across
//!   parser-supported newlines immediately after `if`, before the guard expression.
//! - `else` is handled separately by the match parser and is never reported as a
//!   normal-arm candidate by this helper.
//! - Delimiter depth is tracked so `=>` inside nested parentheses, collections, or
//!   templates is not mistaken for an arm separator.

use crate::compiler_frontend::tokenizer::line_scanning::{
    find_top_level_colon_on_line, find_top_level_fat_arrow_on_line,
    find_top_level_match_arm_fat_arrow,
};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};

pub(crate) struct MatchArmHeaderCandidate {
    pub(crate) start_index: usize,
    pub(crate) arrow_index: usize,
    pub(crate) start_location: SourceLocation,
}

/// Returns true when the token at `index` is the first real token of a logical line.
///
/// A token starts a logical line when:
/// - it is not `Newline`, `End`, or `Eof`;
/// - either it is at index `0`, or the immediately previous token is `Newline`.
pub(crate) fn token_is_line_initial(token_stream: &FileTokens, index: usize) -> bool {
    if index >= token_stream.length {
        return false;
    }

    let token = &token_stream.tokens[index];
    if matches!(
        token.kind,
        TokenKind::Newline | TokenKind::End | TokenKind::Eof
    ) {
        return false;
    }

    if index == 0 {
        return true;
    }

    let current_line = token.location.start_pos.line_number;

    // Scan backwards to find the nearest preceding token on this or a prior line.
    // A Newline token or any token on an earlier line means this token is line-initial.
    // If we encounter a token on the same line first, this token is not line-initial.
    for previous in token_stream.tokens[..index].iter().rev() {
        if previous.kind == TokenKind::Newline {
            return true;
        }

        if previous.location.start_pos.line_number < current_line {
            return true;
        }

        if previous.location.start_pos.line_number == current_line {
            return false;
        }
    }

    true
}

/// Returns true when the token at `start_index` has a top-level `=>` in a match
/// header, including the narrow guarded-header newline exception.
pub(crate) fn token_index_has_top_level_fat_arrow(
    token_stream: &FileTokens,
    start_index: usize,
) -> bool {
    find_top_level_match_arm_fat_arrow(token_stream, start_index).is_some()
}

/// Check whether the current token starts a line-initial match arm header.
///
/// Returns `Some(candidate)` when:
/// - the current token is line-initial;
/// - the token is not `Else` or punctuation that cannot start a normal arm;
/// - the match header contains a top-level `=>`, with only the parser-supported
///   newline exception after a guard `if`.
pub(crate) fn current_token_starts_match_arm_header(
    token_stream: &FileTokens,
) -> Option<MatchArmHeaderCandidate> {
    token_index_starts_match_arm_header(token_stream, token_stream.index, None)
}

/// Check whether the token at `start_index` begins a match arm header.
///
/// If `required_column` is `Some(column)`, the start token must also be at that
/// character column. This preserves the "same arm column" idea used by semicolon
/// delimiter diagnostics.
pub(crate) fn token_index_starts_match_arm_header(
    token_stream: &FileTokens,
    start_index: usize,
    required_column: Option<i32>,
) -> Option<MatchArmHeaderCandidate> {
    if !token_is_line_initial(token_stream, start_index) {
        return None;
    }

    let start_token = &token_stream.tokens[start_index];
    let start_kind = &start_token.kind;

    // `else` is handled separately by the match parser.
    if matches!(
        start_kind,
        TokenKind::Else | TokenKind::FatArrow | TokenKind::Arrow | TokenKind::Colon
    ) {
        return None;
    }

    if let Some(column) = required_column
        && start_token.location.start_pos.char_column != column
    {
        return None;
    }

    let arrow_index = find_top_level_match_arm_fat_arrow(token_stream, start_index)?;

    Some(MatchArmHeaderCandidate {
        start_index,
        arrow_index,
        start_location: start_token.location.clone(),
    })
}

/// Returns true when the current logical line contains a top-level `=>` at any
/// position, regardless of whether the current token is line-initial.
///
/// Used by `body_dispatch.rs` to detect same-line accidental second arms.
pub(crate) fn current_line_contains_top_level_fat_arrow(token_stream: &FileTokens) -> bool {
    find_top_level_fat_arrow_on_line(token_stream, token_stream.index).is_some()
}

/// Scan forward from the current token looking for a top-level `Colon` on the same
/// logical line. Returns `true` if one is found at delimiter depth `0` before any
/// `Newline`, `End`, or `Eof`.
pub(crate) fn current_line_contains_top_level_colon(token_stream: &FileTokens) -> bool {
    find_top_level_colon_on_line(token_stream, token_stream.index).is_some()
}
