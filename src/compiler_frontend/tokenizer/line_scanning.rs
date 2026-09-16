//! Token-line structural scanning helpers.
//!
//! WHAT: exposes small utilities for finding top-level separators on the current
//! physical source line after tokenization, plus the narrow logical-header scan
//! needed by match-arm parsing.
//! WHY: header splitting and AST statement parsing both need token-boundary facts,
//! but neither stage should duplicate delimiter-depth scans or depend on the other.

use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
use crate::compiler_frontend::utilities::token_scan::{NestingDepth, TokenFactView};

fn find_top_level_token_on_line(
    tokens: &[Token],
    start_index: usize,
    matches_target: impl Fn(&TokenKind) -> bool,
) -> Option<usize> {
    let mut nesting_depth = NestingDepth::default();

    for (index, token) in tokens.iter().enumerate().skip(start_index) {
        let kind = &token.kind;
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
    let Some(cursor) = DeclarationCursor::from_file_tokens(token_stream).ok() else {
        return find_top_level_fat_arrow_on_line_in_tokens(&token_stream.tokens, start_index);
    };
    let mut nesting_depth = NestingDepth::default();
    for index in start_index..cursor.length {
        let Some(kind) = cursor.token_kind_at(index) else {
            break;
        };
        match kind {
            TokenKind::Newline | TokenKind::End | TokenKind::Eof => break,
            TokenKind::FatArrow if nesting_depth.is_top_level() => return Some(index),
            _ => nesting_depth.step(&kind),
        }
    }

    None
}

pub(crate) fn find_top_level_fat_arrow_on_line_in_tokens(
    tokens: &[Token],
    start_index: usize,
) -> Option<usize> {
    find_top_level_fat_arrow_on_line_in_view(TokenFactView::from_slice(tokens), start_index)
}

/// Tag-based fat-arrow scan over short-lived indexed facts.
///
/// WHAT: scans one physical line from `start_index` without collecting the source.
/// WHY: dependency-adjacent classifiers only need their line; per-query whole-source
///      projection pays allocation for untouched tokens.
pub(crate) fn find_top_level_fat_arrow_on_line_in_scanned_tokens(
    tokens: TokenFactView<'_>,
    start_index: usize,
) -> Option<usize> {
    find_top_level_fat_arrow_on_line_in_view(tokens, start_index)
}

fn find_top_level_fat_arrow_on_line_in_view(
    tokens: TokenFactView<'_>,
    start_index: usize,
) -> Option<usize> {
    use crate::compiler_frontend::tokenizer::tokens::TokenTag;

    let mut nesting_depth = NestingDepth::default();
    for index in start_index..tokens.len() {
        let token = tokens.get(index)?;
        if token.tag == TokenTag::NEWLINE
            || token.tag == TokenTag::END
            || token.tag == TokenTag::EOF
        {
            break;
        }
        if nesting_depth.is_top_level() && token.tag == TokenTag::FAT_ARROW {
            return Some(index);
        }
        nesting_depth.step_tag(token.tag);
    }
    None
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
    let Some(cursor) = DeclarationCursor::from_file_tokens(token_stream).ok() else {
        return match_arm_fat_arrow_in_tokens(&token_stream.tokens, start_index);
    };
    let mut nesting_depth = NestingDepth::default();
    let mut guard_started = false;
    let mut guard_expression_started = false;

    for index in start_index..cursor.length {
        let Some(kind) = cursor.token_kind_at(index) else {
            break;
        };

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
                nesting_depth.step(&kind);
            }
        }
    }

    None
}

fn match_arm_fat_arrow_in_tokens(tokens: &[Token], start_index: usize) -> Option<usize> {
    let mut nesting_depth = NestingDepth::default();
    let mut guard_started = false;
    let mut guard_expression_started = false;

    for (offset, token) in tokens.iter().enumerate().skip(start_index) {
        let kind = &token.kind;
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
                    return Some(offset);
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
    let Some(cursor) = DeclarationCursor::from_file_tokens(token_stream).ok() else {
        return find_top_level_token_on_line(&token_stream.tokens, start_index, |kind| {
            matches!(kind, TokenKind::Colon)
        });
    };
    let mut nesting_depth = NestingDepth::default();
    for index in start_index..cursor.length {
        let Some(kind) = cursor.token_kind_at(index) else {
            break;
        };
        match kind {
            TokenKind::Newline | TokenKind::End | TokenKind::Eof => break,
            _ if nesting_depth.is_top_level() && matches!(kind, TokenKind::Colon) => {
                return Some(index);
            }
            _ => nesting_depth.step(&kind),
        }
    }

    None
}
