//! Built-in `$md` template style support.
//!
//! WHAT:
//! - Converts template body text into a narrow, deterministic HTML-flavoured markdown output.
//! - Supports unordered and ordered list blocks with indentation-based nesting.
//! - Preserves child-template and dynamic-expression anchors as opaque inline/block boundaries.
//! - Assigns explicit `@#id` section IDs to the Markdown element that owns the marker.
//!
//! WHY:
//! - Templates need lightweight markdown support without adding a full markdown dependency.
//! - Parent markdown runs must keep child template output sealed while still preserving
//!   paragraph and list structure across opaque anchors.
//! - Explicit markers keep generated fragment IDs stable instead of deriving them from labels.
//!
//! Data flow: formatter input splits into lines of atoms here, `blocks` groups them into
//! headings, lists and paragraphs, and `inline` renders each block's atoms. `parsing` and
//! `inline_code` own the grammar, `types` the shared records and `output` the piece buffer.
//! Plain Markdown `.md` content has its own renderer and does not use this module.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterAnchorId, FormatterInput, FormatterInputPiece, FormatterOpaqueKind,
    FormatterOpaquePiece, FormatterOutput, FormatterOutputPiece,
};
use crate::compiler_frontend::ast::templates::styles::whitespace::TemplateWhitespacePassProfile;
use crate::compiler_frontend::ast::templates::template::{
    Formatter, FormatterResult, TemplateFormatter,
};
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::InvalidTemplateStructureReason;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::invalid_template_structure;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::style_directives::StyleDirectiveArgumentValue;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;

mod blocks;
mod inline;
mod inline_code;
mod output;
mod parsing;
mod types;

use output::MarkdownOutputBuilder;
use types::*;

pub struct MarkdownTemplateFormatter;

impl TemplateFormatter for MarkdownTemplateFormatter {
    fn requires_authored_text_spans(&self) -> bool {
        true
    }

    fn format(
        &self,
        input: FormatterInput,
        diagnostic_strings: &mut DiagnosticStringBuilder<'_>,
    ) -> Result<FormatterResult, TemplateError> {
        let lines = split_formatter_input_into_lines(input, diagnostic_strings.strings());
        let pieces = render_markdown_stream(&lines, "p")?;

        Ok(FormatterResult {
            output: FormatterOutput { pieces },
            warnings: DiagnosticBag::new(),
        })
    }
}

pub(crate) fn markdown_formatter() -> Formatter {
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
/// - Consumes a `\r` immediately before `\n` as part of that line ending.
/// - Preserves opaque anchors inline in the current line.
///
/// WHY:
/// - Block parsing needs line boundaries for headings/lists while inline rendering still
///   needs anchored child/dynamic pieces in the original order.
/// - A CRLF terminator is one structural line ending; retaining its `\r` would leave a
///   trailing space in headings and other block text.
fn split_formatter_input_into_lines(
    input: FormatterInput,
    string_table: &StringTable,
) -> Vec<MarkdownLine> {
    let mut lines = Vec::new();
    let mut current_line = MarkdownLine::default();

    for piece in input.pieces {
        match piece {
            FormatterInputPiece::Text(text_piece) => {
                let text = string_table.resolve(text_piece.text);
                let mut byte_offset = 0usize;
                let mut segment_origins = Vec::new();

                for ch in text.chars() {
                    if ch == '\n' {
                        discard_crlf_carriage_return(&mut current_line);
                        close_marker_origin_segment(&mut current_line, &mut segment_origins);
                        lines.push(std::mem::take(&mut current_line));
                        byte_offset += ch.len_utf8();
                        continue;
                    }

                    if ch == '@'
                        && let Some(span) = text_piece.span
                    {
                        segment_origins.push(MarkdownMarkerOrigin {
                            atom_index: current_line.atoms.len(),
                            atom_end: 0,
                            span,
                            byte_offset,
                            text_byte_len: u32::try_from(text.len()).unwrap_or(u32::MAX),
                        });
                    }

                    current_line.atoms.push(MarkdownInlineAtom::Char(ch));
                    byte_offset += ch.len_utf8();
                }

                close_marker_origin_segment(&mut current_line, &mut segment_origins);
            }
            FormatterInputPiece::Opaque(anchor) => {
                current_line.atoms.push(MarkdownInlineAtom::Opaque(anchor));
            }
        }
    }

    lines.push(current_line);
    lines
}

/// Removes the retained `\r` from a line terminated by `\n`.
///
/// The `\r` may have arrived in a separate contiguous text piece, so line splitting must
/// recognise it after the fact. Opaque atoms are never removed, which keeps a line ending
/// from collapsing across a child-template or dynamic-expression boundary. Marker origins
/// keep their authored byte offsets; only the atom end is clamped when the carriage return
/// was the final atom of their source piece.
fn discard_crlf_carriage_return(line: &mut MarkdownLine) {
    if !matches!(line.atoms.last(), Some(MarkdownInlineAtom::Char('\r'))) {
        return;
    }

    line.atoms.pop();
    let atom_end = line.atoms.len();
    for origin in &mut line.marker_origins {
        if origin.atom_end > atom_end {
            origin.atom_end = atom_end;
        }
    }
}

/// Finishes the marker origins contributed by one authored text piece segment.
///
/// The origin's `atom_end` is the end of the atoms that came from the same piece, so a
/// marker candidate crossing into another piece falls back to the containing authored span.
fn close_marker_origin_segment(
    line: &mut MarkdownLine,
    segment_origins: &mut Vec<MarkdownMarkerOrigin>,
) {
    let atom_end = line.atoms.len();
    for mut origin in segment_origins.drain(..) {
        origin.atom_end = atom_end;
        line.marker_origins.push(origin);
    }
}

/// Renders the full markdown line stream while keeping block/list state across anchors.
fn render_markdown_stream(
    lines: &[MarkdownLine],
    default_tag: &str,
) -> Result<Vec<FormatterOutputPiece>, TemplateError> {
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
                blocks::render_list_block(&lines[line_index..], default_tag)?;
            if consumed_lines == 0 {
                break;
            }

            output.append_pieces(rendered);
            has_rendered_block = true;
            line_index += consumed_lines;
            continue;
        }

        if let Some(heading) = parsing::parse_heading_line(&lines[line_index]) {
            blocks::render_heading_line(&mut output, &heading)?;
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
        )?);
        has_rendered_block = true;
    }

    Ok(output.finish())
}

/// Builds the typed source error for a malformed `@#id` section marker.
pub(super) fn invalid_section_marker(span: Option<SourceSpan>) -> TemplateError {
    TemplateError::Diagnostic(invalid_template_structure(
        InvalidTemplateStructureReason::InvalidMarkdownSectionMarker,
        span,
    ))
}

/// Builds the typed source error for two markers on one Markdown block.
pub(super) fn duplicate_section_marker(
    second_span: Option<SourceSpan>,
    first_span: Option<SourceSpan>,
) -> TemplateError {
    TemplateError::Diagnostic(invalid_template_structure(
        InvalidTemplateStructureReason::DuplicateMarkdownSectionMarker {
            earlier_span: first_span,
        },
        second_span,
    ))
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
