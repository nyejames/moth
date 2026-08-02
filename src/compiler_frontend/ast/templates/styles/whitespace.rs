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
    FormatterInput, FormatterInputPiece, FormatterOpaquePiece, FormatterOutput,
    FormatterOutputPiece,
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

/// Scans the start of a text buffer for the leading whitespace pattern
/// (optional spaces + newline + blank lines + indentation) and returns the shared
/// dedent width and the char-index end of the leading boundary block.
pub(crate) fn compute_leading_dedent_info(content: &str) -> (usize, usize) {
    let chars: Vec<char> = content.chars().collect();
    let mut char_index = 0usize;

    // Skip optional leading horizontal whitespace before the first newline.
    while char_index < chars.len() && chars[char_index].is_non_newline_whitespace() {
        char_index += 1;
    }

    if char_index < chars.len() && chars[char_index] == '\n' {
        char_index += 1;
        let leading_boundary_end = char_index;
        let mut saw_blank_line = false;
        let mut first_blank_line_indentation = 0usize;

        loop {
            let indentation_start = char_index;

            // Measure indentation on this line. A line containing only this
            // whitespace is blank, so keep scanning for the first content line.
            while char_index < chars.len() && chars[char_index].is_non_newline_whitespace() {
                char_index += 1;
            }

            let indentation_width = char_index.saturating_sub(indentation_start);

            match chars.get(char_index) {
                Some('\n') => {
                    if !saw_blank_line {
                        first_blank_line_indentation = indentation_width;
                    }
                    saw_blank_line = true;
                    char_index += 1;
                }
                Some(_) => {
                    // Preserve authored blank-line newlines. When one precedes
                    // the first content line, the normal dedent pass removes
                    // that line's baseline after its retained newline.
                    let leading_trim_end = if saw_blank_line {
                        leading_boundary_end + first_blank_line_indentation
                    } else {
                        char_index
                    };
                    return (indentation_width, leading_trim_end);
                }
                None => {
                    // A body containing only formatting whitespace has no
                    // content baseline. Retain the existing boundary behavior
                    // for a single trailing indentation run.
                    return if saw_blank_line {
                        (0, leading_boundary_end + first_blank_line_indentation)
                    } else {
                        (indentation_width, char_index)
                    };
                }
            }
        }
    } else {
        (0, 0)
    }
}

pub(crate) fn dedent_after_newlines(input: &str, dedent_width: usize) -> String {
    if dedent_width == 0 {
        return input.to_owned();
    }

    let chars: Vec<char> = input.chars().collect();
    let mut output = String::with_capacity(input.len());
    let mut char_index = 0usize;

    while char_index < chars.len() {
        let current = chars[char_index];
        output.push(current);
        char_index += 1;

        if current != '\n' {
            continue;
        }

        let mut removed = 0usize;
        while char_index < chars.len() && removed < dedent_width {
            let next = chars[char_index];
            if !next.is_non_newline_whitespace() {
                break;
            }

            char_index += 1;
            removed += 1;
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
    let chars: Vec<char> = content.chars().collect();
    let mut output = String::with_capacity(content.len());
    let mut line_start = 0usize;

    loop {
        let mut cursor = line_start;
        while cursor < chars.len() && chars[cursor].is_non_newline_whitespace() {
            cursor += 1;
        }

        match chars.get(cursor) {
            Some('\n') => {
                output.push('\n');
                line_start = cursor + 1;
            }
            Some(_) => {
                output.extend(chars[line_start..].iter().copied());
                break;
            }
            None => break,
        }
    }

    *content = output;
}

pub(crate) fn trim_trailing_whitespace_from_final_newline(content: &mut String) {
    let chars: Vec<char> = content.chars().collect();
    let Some(last_newline) = chars.iter().rposition(|ch| *ch == '\n') else {
        return;
    };

    if chars[last_newline + 1..]
        .iter()
        .all(|ch| ch.is_non_newline_whitespace())
    {
        *content = chars[..last_newline].iter().collect();
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
    if passes.is_empty() {
        // No passes to run — convert input directly to output.
        let pieces = input
            .pieces
            .into_iter()
            .map(|piece| match piece {
                FormatterInputPiece::Text(t) => {
                    FormatterOutputPiece::Text(string_table.resolve(t.text).to_owned())
                }
                FormatterInputPiece::Opaque(id) => FormatterOutputPiece::Opaque(id),
            })
            .collect();
        return FormatterOutput { pieces };
    }

    // Resolve all text pieces up-front so we can compute shared whitespace context.
    let resolved_pieces: Vec<ResolvedInputPiece> = input
        .pieces
        .into_iter()
        .map(|piece| match piece {
            FormatterInputPiece::Text(t) => {
                ResolvedInputPiece::Text(string_table.resolve(t.text).to_owned())
            }
            FormatterInputPiece::Opaque(id) => ResolvedInputPiece::Opaque(id),
        })
        .collect();

    // Run each pass sequentially over the resolved piece list.
    let mut text_pieces = resolved_pieces;
    for pass in passes {
        text_pieces = apply_structured_whitespace_pass(text_pieces, *pass, run_position);
    }

    // Convert resolved pieces back to formatter output.
    let pieces = text_pieces
        .into_iter()
        .map(|piece| match piece {
            ResolvedInputPiece::Text(t) => FormatterOutputPiece::Text(t),
            ResolvedInputPiece::Opaque(id) => FormatterOutputPiece::Opaque(id),
        })
        .collect();

    FormatterOutput { pieces }
}

/// Intermediate piece type used during structured whitespace processing.
/// Text pieces hold owned strings so they can be modified in-place across passes.
enum ResolvedInputPiece {
    Text(String),
    Opaque(FormatterOpaquePiece),
}

/// Applies a single whitespace pass across a list of resolved pieces.
///
/// Computes dedent width from the first text piece, then normalizes each text piece
/// while keeping opaque anchors untouched.
fn apply_structured_whitespace_pass(
    pieces: Vec<ResolvedInputPiece>,
    pass: TemplateWhitespacePassProfile,
    run_position: TemplateBodyRunPosition,
) -> Vec<ResolvedInputPiece> {
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
    pieces: Vec<ResolvedInputPiece>,
    trim_leading: bool,
    trim_trailing: bool,
) -> Vec<ResolvedInputPiece> {
    // Find the first text piece to compute dedent width.
    let first_text = pieces.iter().find_map(|p| match p {
        ResolvedInputPiece::Text(t) if !t.is_empty() => Some(t.as_str()),
        _ => None,
    });

    let (dedent_width, leading_trim_end) = match first_text {
        Some(text) => compute_leading_dedent_info(text),
        None => (0, 0),
    };

    // Find the index of the last non-empty text piece for trailing trim.
    let last_text_index = pieces
        .iter()
        .rposition(|p| matches!(p, ResolvedInputPiece::Text(t) if !t.is_empty()));

    // Find the index of the first non-empty text piece for leading trim.
    let first_text_index = pieces
        .iter()
        .position(|p| matches!(p, ResolvedInputPiece::Text(t) if !t.is_empty()));

    // Boundary trimming only applies to actual run boundaries. Opaque anchors
    // before the first text or after the last text represent already-emitted
    // child templates or expressions, so adjacent text is interior separator
    // whitespace and must not be trimmed as if it began or ended the template.
    let leading_text_is_run_boundary = first_text_index.is_some_and(|first_index| {
        pieces[..first_index]
            .iter()
            .all(|piece| matches!(piece, ResolvedInputPiece::Text(text) if text.is_empty()))
    });
    let trailing_text_is_run_boundary = last_text_index.is_some_and(|last_index| {
        pieces[last_index + 1..]
            .iter()
            .all(|piece| matches!(piece, ResolvedInputPiece::Text(text) if text.is_empty()))
    });

    let mut result = Vec::with_capacity(pieces.len());

    for (index, piece) in pieces.into_iter().enumerate() {
        match piece {
            ResolvedInputPiece::Opaque(id) => {
                result.push(ResolvedInputPiece::Opaque(id));
            }
            ResolvedInputPiece::Text(mut text) => {
                if text.is_empty() {
                    result.push(ResolvedInputPiece::Text(text));
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
                    text = text.chars().skip(leading_trim_end).collect();
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

                result.push(ResolvedInputPiece::Text(text));
            }
        }
    }

    result
}
