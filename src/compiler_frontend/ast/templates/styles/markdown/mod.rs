//! Built-in `$md` template style support.
//!
//! WHAT:
//! - Converts template body text into a narrow, deterministic HTML-flavoured markdown output.
//! - Supports unordered and ordered list blocks with indentation-based nesting.
//! - Preserves child-template and dynamic-expression anchors as opaque inline/block boundaries.
//!
//! WHY:
//! - Templates need lightweight markdown support without adding a full markdown dependency.
//! - Parent markdown runs must keep child template output sealed while still preserving
//!   paragraph and list structure across opaque anchors.
//!
//! Data flow: formatter input splits into lines of atoms here, `blocks` groups them into
//! headings, lists and paragraphs, and `inline` renders each block's atoms. `parsing` and
//! `inline_code` own the grammar, `heading_id` the automatic heading ID normaliser, `types`
//! the shared records and `output` the piece buffer.
//! Plain Markdown `.md` content has its own renderer and does not use this module.

use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterAnchorId, FormatterInput, FormatterInputPiece, FormatterOpaqueKind,
    FormatterOpaquePiece, FormatterOutput, FormatterOutputPiece,
};
use crate::compiler_frontend::ast::templates::styles::whitespace::TemplateWhitespacePassProfile;
use crate::compiler_frontend::ast::templates::template::{
    Formatter, FormatterResult, TemplateFormatter,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::style_directives::StyleDirectiveArgumentValue;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;

mod blocks;
mod heading_id;
mod inline;
mod inline_code;
mod output;
mod parsing;
mod types;

use output::MarkdownOutputBuilder;
use types::*;

pub struct MarkdownTemplateFormatter;

impl TemplateFormatter for MarkdownTemplateFormatter {
    fn format(
        &self,
        input: FormatterInput,
        string_table: &mut StringTable,
    ) -> Result<FormatterResult, CompilerMessages> {
        let lines = split_formatter_input_into_lines(input, string_table);
        let pieces = render_markdown_stream(&lines, "p");

        Ok(FormatterResult {
            output: FormatterOutput { pieces },
            warnings: Vec::new(),
        })
    }
}

pub fn markdown_formatter() -> Formatter {
    Formatter {
        // `$md` opts into the shared default body dedent/trim pass explicitly.
        pre_format_whitespace_passes: vec![TemplateWhitespacePassProfile::default_template_body()],
        formatter: Arc::new(MarkdownTemplateFormatter),
        post_format_whitespace_passes: Vec::new(),
    }
}

pub(crate) fn markdown_formatter_factory(
    argument: Option<&StyleDirectiveArgumentValue>,
) -> Result<Formatter, String> {
    if argument.is_some() {
        return Err("'$md' does not accept arguments.".to_string());
    }

    Ok(markdown_formatter())
}

/// Converts formatter input into newline-delimited markdown lines without flattening anchors.
///
/// WHAT:
/// - Splits text pieces on `\n`.
/// - Preserves opaque anchors inline in the current line.
///
/// WHY:
/// - Block parsing needs line boundaries for headings/lists while inline rendering still
///   needs anchored child/dynamic pieces in the original order.
fn split_formatter_input_into_lines(
    input: FormatterInput,
    string_table: &StringTable,
) -> Vec<MarkdownLine> {
    let mut lines = Vec::new();
    let mut current_line = MarkdownLine::default();

    for piece in input.pieces {
        match piece {
            FormatterInputPiece::Text(text_piece) => {
                for ch in string_table.resolve(text_piece.text).chars() {
                    if ch == '\n' {
                        lines.push(std::mem::take(&mut current_line));
                    } else {
                        current_line.atoms.push(MarkdownInlineAtom::Char(ch));
                    }
                }
            }
            FormatterInputPiece::Opaque(anchor) => {
                current_line.atoms.push(MarkdownInlineAtom::Opaque(anchor));
            }
        }
    }

    lines.push(current_line);
    lines
}

/// Renders the full markdown line stream while keeping block/list state across anchors.
fn render_markdown_stream(lines: &[MarkdownLine], default_tag: &str) -> Vec<FormatterOutputPiece> {
    let mut output = MarkdownOutputBuilder::default();
    let mut line_index = 0usize;
    let mut has_rendered_block = false;

    while line_index < lines.len() {
        if line_is_blank(&lines[line_index]) {
            let mut blank_run = 0usize;
            while line_index + blank_run < lines.len()
                && line_is_blank(&lines[line_index + blank_run])
            {
                blank_run += 1;
            }

            let break_count = if has_rendered_block {
                blank_run / 2
            } else {
                blank_run.saturating_sub(1) / 2
            };
            append_break_tags(&mut output, break_count);
            line_index += blank_run;
            continue;
        }

        if parsing::parse_list_item_line(&lines[line_index]).is_some() {
            let (rendered, consumed_lines) =
                blocks::render_list_block(&lines[line_index..], default_tag);
            if consumed_lines == 0 {
                break;
            }

            output.append_pieces(rendered);
            has_rendered_block = true;
            line_index += consumed_lines;
            continue;
        }

        if let Some(heading) = parsing::parse_heading_line(&lines[line_index]) {
            blocks::render_heading_line(&mut output, &heading);
            has_rendered_block = true;
            line_index += 1;
            continue;
        }

        let region_start = line_index;
        while line_index < lines.len() {
            if line_is_blank(&lines[line_index])
                || parsing::parse_list_item_line(&lines[line_index]).is_some()
                || parsing::parse_heading_line(&lines[line_index]).is_some()
            {
                break;
            }
            line_index += 1;
        }

        output.append_pieces(blocks::render_plain_region(
            &lines[region_start..line_index],
            default_tag,
        ));
        has_rendered_block = true;
    }

    output.finish()
}

fn append_break_tags(output: &mut MarkdownOutputBuilder, break_count: usize) {
    for _ in 0..break_count {
        output.push_raw("<br>");
    }
}

fn line_is_blank(line: &MarkdownLine) -> bool {
    line.atoms.iter().all(|atom| match atom {
        MarkdownInlineAtom::Char(ch) => ch.is_whitespace(),
        MarkdownInlineAtom::Opaque(_) => false,
    })
}

#[cfg(test)]
#[path = "../../tests/markdown_tests.rs"]
mod markdown_tests;
