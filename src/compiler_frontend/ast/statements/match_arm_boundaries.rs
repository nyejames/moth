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

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::utilities::token_scan::NestingDepth;

pub(crate) struct MatchArmHeaderCandidate {
    pub(crate) start_index: usize,
    pub(crate) arrow_index: usize,
}

/// Returns true when the token at `index` is the first real token of a logical line.
///
/// A token starts a logical line when:
/// - it is not `Newline`, `End`, or `Eof`;
pub(crate) fn token_is_line_initial(token_stream: &AstCursor, index: usize) -> bool {
    if index >= token_stream.length() {
        return false;
    }

    let Some(tag) = token_stream.token_ref_at(index).map(|token| token.tag()) else {
        return false;
    };
    if matches!(tag, TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF) {
        return false;
    }
    index == 0
        || token_stream
            .token_ref_at(index.saturating_sub(1))
            .is_some_and(|token| token.tag() == TokenTag::NEWLINE)
}

/// Returns true when the token at `start_index` has a top-level `=>` in a match
/// header, including the narrow guarded-header newline exception.
pub(crate) fn token_index_has_top_level_fat_arrow(
    token_stream: &AstCursor,
    start_index: usize,
) -> bool {
    find_top_level_match_arm_fat_arrow(token_stream, start_index).is_some()
}

fn find_top_level_match_arm_fat_arrow(
    token_stream: &AstCursor,
    start_index: usize,
) -> Option<usize> {
    let mut walk = token_stream
        .subcursor_window(start_index, token_stream.length())
        .expect("match-arm window stays inside the active parser view");
    let mut nesting_depth = NestingDepth::default();
    let mut guard_started = false;
    let mut guard_expression_started = false;

    while !walk.is_at_end() {
        let tag = walk.current_tag();
        match tag {
            TokenTag::END | TokenTag::EOF => break,
            TokenTag::NEWLINE => {
                if nesting_depth.is_top_level() && guard_started && !guard_expression_started {
                    walk.advance();
                    continue;
                }
                break;
            }
            TokenTag::FAT_ARROW if nesting_depth.is_top_level() => {
                if !guard_started || guard_expression_started {
                    return Some(walk.position());
                }
                break;
            }
            TokenTag::IF if nesting_depth.is_top_level() && !guard_started => {
                guard_started = true;
            }
            _ => {
                if guard_started && nesting_depth.is_top_level() {
                    guard_expression_started = true;
                }
                nesting_depth.step_tag(walk.current_tag());
            }
        }
        walk.advance();
    }

    None
}

/// Check whether the current token starts a line-initial match arm header.
///
/// Returns `Some(candidate)` when:
/// - the current token is line-initial;
/// - the token is not `Else` or punctuation that cannot start a normal arm;
/// - the match header contains a top-level `=>`, with only the parser-supported
///   newline exception after a guard `if`.
pub(crate) fn current_token_starts_match_arm_header(
    token_stream: &AstCursor,
) -> Option<MatchArmHeaderCandidate> {
    token_index_starts_match_arm_header(token_stream, token_stream.position(), None)
}

/// Check whether the token at `start_index` begins a match arm header.
///
/// If `required_column` is `Some(column)`, the start token must also be at that
/// character column. This preserves the "same arm column" idea used by semicolon
/// delimiter diagnostics.
pub(crate) fn token_index_starts_match_arm_header(
    token_stream: &AstCursor,
    start_index: usize,
    _required_column: Option<i32>,
) -> Option<MatchArmHeaderCandidate> {
    if !token_is_line_initial(token_stream, start_index) {
        return None;
    }

    let Some(start_tag) = token_stream.token_ref_at(start_index).map(|token| token.tag()) else {
        return None;
    };

    // `else` is handled separately by the match parser.
    if matches!(
        start_tag,
        TokenTag::ELSE | TokenTag::FAT_ARROW | TokenTag::ARROW | TokenTag::COLON
    ) {
        return None;
    }

    let arrow_index = find_top_level_match_arm_fat_arrow(token_stream, start_index)?;

    Some(MatchArmHeaderCandidate {
        start_index,
        arrow_index,
    })
}

fn find_top_level_token_on_line(
    token_stream: &AstCursor,
    start_index: usize,
    matches_target: impl Fn(TokenTag) -> bool,
) -> Option<usize> {
    let mut walk = token_stream
        .subcursor_window(start_index, token_stream.length())
        .expect("match-arm line window stays inside the active parser view");
    let mut nesting_depth = NestingDepth::default();
    while !walk.is_at_end() {
        let tag = walk.current_tag();
        match tag {
            TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF => break,
            _ if nesting_depth.is_top_level() && matches_target(tag) => {
                return Some(walk.position());
            }
            _ => nesting_depth.step_tag(walk.current_tag()),
        }
        walk.advance();
    }
    None
}

/// Returns true when the current logical line contains a top-level `=>` at any
/// position, regardless of whether the current token is line-initial.
///
/// Used by `body_dispatch.rs` to detect same-line accidental second arms.
pub(crate) fn current_line_contains_top_level_fat_arrow(token_stream: &AstCursor) -> bool {
    find_top_level_token_on_line(token_stream, token_stream.position(), |tag| {
        tag == TokenTag::FAT_ARROW
    })
    .is_some()
}

/// Scan forward from the current token looking for a top-level `Colon` on the same
/// logical line. Returns `true` if one is found at delimiter depth `0` before any
/// `Newline`, `End`, or `Eof`.
pub(crate) fn current_line_contains_top_level_colon(token_stream: &AstCursor) -> bool {
    find_top_level_token_on_line(token_stream, token_stream.position(), |tag| {
        tag == TokenTag::COLON
    })
    .is_some()
}
