//! Token-line structural scanning helpers.
//!
//! WHAT: exposes small utilities for finding top-level separators on the current
//! physical source line after tokenization, plus the narrow logical-header scan
//! needed by match-arm parsing.
//! WHY: header splitting and AST statement parsing both need token-boundary facts,
//! but neither stage should duplicate delimiter-depth scans or depend on the other.

use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::utilities::token_scan::NestingDepth;

fn find_top_level_token_on_line(
    token_stream: &FileTokens,
    start_index: usize,
    matches_target: impl Fn(&TokenKind) -> bool,
) -> Option<usize> {
    let mut nesting_depth = NestingDepth::default();

    for index in start_index..token_stream.length {
        let kind = &token_stream.tokens[index].kind;
        match kind {
            TokenKind::Newline | TokenKind::End | TokenKind::Eof => break,
            _ if nesting_depth.is_top_level() && matches_target(kind) => return Some(index),
            _ => nesting_depth.step(kind),
        }
    }

    None
}

pub(crate) fn find_top_level_fat_arrow_on_line(
    token_stream: &FileTokens,
    start_index: usize,
) -> Option<usize> {
    find_top_level_token_on_line(token_stream, start_index, |kind| {
        matches!(kind, TokenKind::FatArrow)
    })
}

/// Find the top-level arrow that terminates one match-arm header.
///
/// Match headers normally end on their physical line. A guarded header also
/// permits parser-owned newline skipping immediately after its `if` keyword,
/// before the guard expression begins. Keeping that exception here prevents
/// generic line scans from acquiring multiline expression semantics.
pub(crate) fn find_top_level_match_arm_fat_arrow(
    token_stream: &FileTokens,
    start_index: usize,
) -> Option<usize> {
    let mut nesting_depth = NestingDepth::default();
    let mut guard_started = false;
    let mut guard_expression_started = false;

    for index in start_index..token_stream.length {
        let kind = &token_stream.tokens[index].kind;

        match kind {
            TokenKind::End | TokenKind::Eof => break,
            TokenKind::Newline => {
                if nesting_depth.is_top_level() && guard_started && !guard_expression_started {
                    continue;
                }

                break;
            }
            TokenKind::FatArrow if nesting_depth.is_top_level() => {
                if !guard_started || guard_expression_started {
                    return Some(index);
                }

                break;
            }
            TokenKind::If if nesting_depth.is_top_level() && !guard_started => {
                guard_started = true;
            }
            _ => {
                if guard_started && nesting_depth.is_top_level() {
                    guard_expression_started = true;
                }
                nesting_depth.step(kind);
            }
        }
    }

    None
}

pub(crate) fn find_top_level_colon_on_line(
    token_stream: &FileTokens,
    start_index: usize,
) -> Option<usize> {
    find_top_level_token_on_line(token_stream, start_index, |kind| {
        matches!(kind, TokenKind::Colon)
    })
}
