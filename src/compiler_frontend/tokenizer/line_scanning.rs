//! Token-line structural scanning helpers.
//!
//! WHAT: exposes the tag-based fat-arrow scan over short-lived indexed token facts.
//! WHY: dependency-adjacent classifiers only need their line; per-query whole-source
//!      projection pays allocation for untouched tokens.

use crate::compiler_frontend::utilities::token_scan::{NestingDepth, TokenFactView};

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
