//! Shared default template-body whitespace normalization.
//!
//! WHAT:
//! - Applies the compiler's default template-body trimming + dedent behavior.
//! - Provides both a string-based entry point for single buffers and a structured
//!   entry point that operates directly on `FormatterInput` / `FormatterOutput` pieces.
//!
//! WHY:
//! - Template bodies are commonly authored with indentation for readability.
//! - Normalizing at parse/fold time keeps output stable without requiring explicit `$raw`.
//! - The structured entry point allows whitespace passes to run without flattening
//!   opaque child-template anchors into a temporary string buffer.

use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterInputPiece, FormatterOutput, FormatterOutputPiece,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::utilities::basic::CharacterParsing;

/// Shared whitespace passes that template formatters and default template parsing can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplateWhitespacePass {
    /// The compiler's default template dedent/trim pass.
    DefaultTemplateBody,
}

/// Position of the current body run within the whole template stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplateBodyRunPosition {
    Only,
    First,
    Middle,
    Last,
}

impl TemplateBodyRunPosition {
    /// `true` when this run is at the start boundary of the template body stream.
    pub(crate) fn is_first(self) -> bool {
        matches!(self, Self::Only | Self::First)
    }

    /// `true` when this run is at the end boundary of the template body stream.
    pub(crate) fn is_last(self) -> bool {
        matches!(self, Self::Only | Self::Last)
    }
}

/// Configuration for running one whitespace pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TemplateWhitespacePassProfile {
    pub pass: TemplateWhitespacePass,
    pub trim_leading_boundary: bool,
    pub trim_trailing_boundary: bool,
}

impl TemplateWhitespacePassProfile {
    pub(crate) const fn new(
        pass: TemplateWhitespacePass,
        trim_leading_boundary: bool,
        trim_trailing_boundary: bool,
    ) -> Self {
        Self {
            pass,
            trim_leading_boundary,
            trim_trailing_boundary,
        }
    }

    /// Default template dedent/trim profile used by plain templates and markdown pre-pass.
    pub(crate) const fn default_template_body() -> Self {
        Self::new(TemplateWhitespacePass::DefaultTemplateBody, true, true)
    }
}

/// Returns the byte length of the leading run of horizontal (non-newline) whitespace.
/// It scans by char but reports bytes, so callers can slice the same string directly.
fn horizontal_whitespace_len(text: &str) -> usize {
    text.find(|ch: char| !ch.is_non_newline_whitespace())
        .unwrap_or(text.len())
}

/// Scans the start of a text buffer for the leading whitespace pattern
/// (optional spaces + newline + blank lines + indentation) and returns the shared
/// dedent width and the byte end of the leading boundary block.
///
/// WHY: the width counts chars because `dedent_after_newlines` removes chars, while the
/// end is a byte offset so the caller can drain it without rebuilding the string.
pub(crate) fn compute_leading_dedent_info(content: &str) -> (usize, usize) {
    // Skip optional leading horizontal whitespace before the first newline.
    let mut cursor = horizontal_whitespace_len(content);
    if !content[cursor..].starts_with('\n') {
        return (0, 0);
    }

    cursor += 1;
    let leading_boundary_end = cursor;
    let mut saw_blank_line = false;
    let mut first_blank_line_indentation = 0usize;

    loop {
        let indentation_start = cursor;

        // Measure indentation on this line. A line containing only this
        // whitespace is blank, so keep scanning for the first content line.
        cursor += horizontal_whitespace_len(&content[cursor..]);
        let indentation = &content[indentation_start..cursor];

        match content[cursor..].chars().next() {
            Some('\n') => {
                if !saw_blank_line {
                    first_blank_line_indentation = indentation.len();
                }
                saw_blank_line = true;
                cursor += 1;
            }
            Some(_) => {
                // Preserve authored blank-line newlines. When one precedes
                // the first content line, the normal dedent pass removes
                // that line's baseline after its retained newline.
                let leading_trim_end = if saw_blank_line {
                    leading_boundary_end + first_blank_line_indentation
                } else {
                    cursor
                };
                return (indentation.chars().count(), leading_trim_end);
            }
            None => {
                // A body containing only formatting whitespace has no
                // content baseline. Retain the existing boundary behavior
                // for a single trailing indentation run.
                return if saw_blank_line {
                    (0, leading_boundary_end + first_blank_line_indentation)
                } else {
                    (indentation.chars().count(), cursor)
                };
            }
        }
    }
}

pub(crate) fn dedent_after_newlines(input: &str, dedent_width: usize) -> String {
    if dedent_width == 0 {
        return input.to_owned();
    }

    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(current) = chars.next() {
        output.push(current);

        if current != '\n' {
            continue;
        }

        // Drop up to `dedent_width` horizontal whitespace chars after each newline.
        for _ in 0..dedent_width {
            if chars
                .next_if(|next| next.is_non_newline_whitespace())
                .is_none()
            {
                break;
            }
        }
    }

    output
}

/// Removes horizontal indentation from leading blank lines while preserving their newlines.
///
/// WHAT:
/// - Strips all formatting-only indentation before every retained leading blank-line newline.
/// - Leaves the first content line and every later line for the shared baseline dedent pass.
///
/// WHY:
/// - A blank line can be indented more deeply than the first content baseline, so baseline
///   dedenting alone must not leave formatting spaces in the preserved leading boundary.
fn trim_leading_blank_line_indentation(content: &mut String) {
    let mut output = String::with_capacity(content.len());
    let mut line_start = 0usize;

    loop {
        let rest = &content[line_start..];
        let indentation_len = horizontal_whitespace_len(rest);
        match rest[indentation_len..].chars().next() {
            Some('\n') => {
                output.push('\n');
                line_start += indentation_len + 1;
            }
            Some(_) => {
                output.push_str(rest);
                break;
            }
            None => break,
        }
    }

    *content = output;
}

pub(crate) fn trim_trailing_whitespace_from_final_newline(content: &mut String) {
    let Some(last_newline) = content.rfind('\n') else {
        return;
    };

    if content[last_newline + 1..]
        .chars()
        .all(|ch| ch.is_non_newline_whitespace())
    {
        content.truncate(last_newline);
    }
}

// ---------------------------------------------------------------------------
// Structured-input whitespace passes
// ---------------------------------------------------------------------------

/// Runs whitespace passes directly on structured `FormatterInput` pieces.
///
/// WHAT:
/// - Computes dedent width from the first text piece's leading whitespace pattern,
///   then applies normalization to each text piece while preserving opaque anchors.
///
/// WHY:
/// - Allows whitespace normalization without flattening structured formatter input
///   into a guard-character-delimited string buffer.
pub(crate) fn apply_whitespace_passes_to_input(
    input: FormatterInput,
    passes: &[TemplateWhitespacePassProfile],
    run_position: TemplateBodyRunPosition,
    string_table: &mut StringTable,
) -> FormatterOutput {
    // Resolve interned text once; opaque anchors pass through unchanged.
    let mut pieces: Vec<FormatterOutputPiece> = input
        .pieces
        .into_iter()
        .map(|piece| match piece {
            FormatterInputPiece::Text(t) => {
                FormatterOutputPiece::Text(string_table.resolve(t.text).to_owned())
            }
            FormatterInputPiece::Opaque(id) => FormatterOutputPiece::Opaque(id),
        })
        .collect();

    // Run each pass sequentially over the resolved piece list.
    for pass in passes {
        pieces = apply_structured_whitespace_pass(pieces, *pass, run_position);
    }

    FormatterOutput { pieces }
}

/// Applies a single whitespace pass across a list of resolved pieces.
///
/// Computes dedent width from the first text piece, then normalizes each text piece
/// while keeping opaque anchors untouched.
fn apply_structured_whitespace_pass(
    pieces: Vec<FormatterOutputPiece>,
    pass: TemplateWhitespacePassProfile,
    run_position: TemplateBodyRunPosition,
) -> Vec<FormatterOutputPiece> {
    let trim_leading = pass.trim_leading_boundary && run_position.is_first();
    let trim_trailing = pass.trim_trailing_boundary && run_position.is_last();

    match pass.pass {
        TemplateWhitespacePass::DefaultTemplateBody => {
            normalize_structured_default_whitespace(pieces, trim_leading, trim_trailing)
        }
    }
}

/// Structured version of `normalize_default_template_body_whitespace`.
///
/// WHAT:
/// - Computes dedent width from the first content line of the first text piece.
/// - Applies leading boundary trim to the first text piece, dedent to all text pieces,
///   and trailing boundary trim to the last text piece.
///
/// WHY:
/// - Opaque anchors represent child templates whose content is sealed. They do not
///   contribute whitespace and must not be flattened into a text buffer.
fn normalize_structured_default_whitespace(
    pieces: Vec<FormatterOutputPiece>,
    trim_leading: bool,
    trim_trailing: bool,
) -> Vec<FormatterOutputPiece> {
    // Find the first text piece to compute dedent width.
    let first_text = pieces.iter().find_map(|p| match p {
        FormatterOutputPiece::Text(t) if !t.is_empty() => Some(t.as_str()),
        _ => None,
    });

    let (dedent_width, leading_trim_end) = match first_text {
        Some(text) => compute_leading_dedent_info(text),
        None => (0, 0),
    };

    // Find the index of the last non-empty text piece for trailing trim.
    let last_text_index = pieces
        .iter()
        .rposition(|p| matches!(p, FormatterOutputPiece::Text(t) if !t.is_empty()));

    // Find the index of the first non-empty text piece for leading trim.
    let first_text_index = pieces
        .iter()
        .position(|p| matches!(p, FormatterOutputPiece::Text(t) if !t.is_empty()));

    // Boundary trimming only applies to actual run boundaries. Opaque anchors
    // before the first text or after the last text represent already-emitted
    // child templates or expressions, so adjacent text is interior separator
    // whitespace and must not be trimmed as if it began or ended the template.
    let leading_text_is_run_boundary = first_text_index.is_some_and(|first_index| {
        pieces[..first_index]
            .iter()
            .all(|piece| matches!(piece, FormatterOutputPiece::Text(text) if text.is_empty()))
    });
    let trailing_text_is_run_boundary = last_text_index.is_some_and(|last_index| {
        pieces[last_index + 1..]
            .iter()
            .all(|piece| matches!(piece, FormatterOutputPiece::Text(text) if text.is_empty()))
    });

    let mut result = Vec::with_capacity(pieces.len());

    for (index, piece) in pieces.into_iter().enumerate() {
        match piece {
            FormatterOutputPiece::Opaque(id) => {
                result.push(FormatterOutputPiece::Opaque(id));
            }
            FormatterOutputPiece::Text(mut text) => {
                if text.is_empty() {
                    result.push(FormatterOutputPiece::Text(text));
                    continue;
                }

                let is_first_text = first_text_index == Some(index);
                let is_last_text = last_text_index == Some(index);

                // Leading boundary trim: strip the leading whitespace + newline + indentation
                // block from the first text piece only.
                if trim_leading
                    && is_first_text
                    && leading_text_is_run_boundary
                    && leading_trim_end > 0
                {
                    // Byte offset from compute_leading_dedent_info; drain it in place.
                    text.drain(..leading_trim_end);
                    trim_leading_blank_line_indentation(&mut text);
                }

                // Dedent after every newline using the shared dedent width.
                if dedent_width > 0 {
                    text = dedent_after_newlines(&text, dedent_width);
                }

                // Trailing boundary trim: remove whitespace-only content after the last
                // newline in the final text piece.
                if trim_trailing && is_last_text && trailing_text_is_run_boundary {
                    trim_trailing_whitespace_from_final_newline(&mut text);
                }

                result.push(FormatterOutputPiece::Text(text));
            }
        }
    }

    result
}
