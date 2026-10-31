//! Match-arm boundary scanning.
//!
//! WHAT: determines whether the current token position begins a new match arm header.
//! WHY: match arms are line-oriented, so body parsing needs a shared, deterministic way to
//! know when one arm ends and the next begins.
//!
//! Boundary rule:
//! - A candidate normal arm starts only when the current token is the first real token
//!   of a logical line and its header contains a top-level `=>` before `End` or `Eof`.
//!   A top-level `->` also starts a candidate: the arm parser then reports the authored
//!   arrow mistake rather than treating the whole line as an unparseable header.
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
fn token_is_line_initial(token_stream: &AstCursor, index: usize) -> bool {
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
    find_top_level_match_arm_separator(token_stream, start_index, false).is_some()
}

/// Finds the top-level arm separator, optionally accepting `->` as a candidate.
///
/// WHAT: returns the index of `=>`, or of `->` when `accept_arrow` is set, before the header's
///       end. A guarded header accepts a separator only once the guard expression has started.
/// WHY: a line-initial `<pattern> ->` must reach `parse_match_arm` so the parser reports the
///      authored arrow mistake. Treating it as a malformed header would hide the reason behind
///      the generic expected-header diagnostic.
fn find_top_level_match_arm_separator(
    token_stream: &AstCursor,
    start_index: usize,
    accept_arrow: bool,
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
            TokenTag::FAT_ARROW | TokenTag::ARROW
                if nesting_depth.is_top_level() && (tag == TokenTag::FAT_ARROW || accept_arrow) =>
            {
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
pub(crate) fn token_index_starts_match_arm_header(
    token_stream: &AstCursor,
    start_index: usize,
    _required_column: Option<i32>,
) -> Option<MatchArmHeaderCandidate> {
    if !token_is_line_initial(token_stream, start_index) {
        return None;
    }

    let start_tag = token_stream
        .token_ref_at(start_index)
        .map(|token| token.tag())?;
    // `else` is handled separately by the match parser.
    if matches!(
        start_tag,
        TokenTag::ELSE | TokenTag::FAT_ARROW | TokenTag::ARROW | TokenTag::COLON
    ) {
        return None;
    }

    let arrow_index = find_top_level_match_arm_separator(token_stream, start_index, true)?;

    Some(MatchArmHeaderCandidate {
        start_index,
        arrow_index,
    })
}

/// Check whether the current token starts a line-initial arm header with a region colon.
///
/// WHAT: recognises a `name:` spelling where the match body expects an arm header.
/// WHY: declared regions and malformed symbol-shaped arms share that spelling. A match body
///      accepts `=>` only, so the arm body scan stops here and lets the match parser report the
///      missing separator instead of the statement scan claiming a declared region.
pub(crate) fn current_token_starts_colon_separated_arm_header(token_stream: &AstCursor) -> bool {
    token_is_line_initial(token_stream, token_stream.position())
        && token_stream.current_tag() == TokenTag::SYMBOL
        && token_stream.peek_next_tag() == Some(TokenTag::COLON)
}

/// True when the current token ends a match-arm body rather than continuing an expression.
///
/// WHY: a bare expression that stops at `:` or another in-line token is a malformed header,
///      not a finished statement-match arm body, so it keeps the ordinary unexpected-token path.
///      Single-value expressions skip their trailing newline, so a line-initial token also
///      marks the end of the body statement.
pub(crate) fn current_token_ends_match_arm_body(token_stream: &AstCursor) -> bool {
    matches!(
        token_stream.current_tag(),
        TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF | TokenTag::ELSE
    ) || token_is_line_initial(token_stream, token_stream.position())
}

/// Returns true when the current logical line contains a top-level `=>` at any
/// position, regardless of whether the current token is line-initial.
///
/// Used by `body_dispatch.rs` to detect same-line accidental second arms.
pub(crate) fn current_line_contains_top_level_fat_arrow(token_stream: &AstCursor) -> bool {
    let mut walk = token_stream
        .subcursor_window(token_stream.position(), token_stream.length())
        .expect("match-arm line window stays inside the active parser view");
    let mut nesting_depth = NestingDepth::default();
    while !walk.is_at_end() {
        let tag = walk.current_tag();
        match tag {
            TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF => break,
            TokenTag::FAT_ARROW if nesting_depth.is_top_level() => return true,
            _ => nesting_depth.step_tag(walk.current_tag()),
        }
        walk.advance();
    }
    false
}
