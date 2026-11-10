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
    FormatterOutputPiece, FormatterTextPiece,
};
use crate::compiler_frontend::source::SourceSpan;
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
/// WHY: the width counts chars because `dedent_keep_ranges` and `WhitespaceTextPiece`
/// dedenting remove chars, while the end is a byte offset so the caller can drain it
/// without rebuilding the string.
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

// ---------------------------------------------------------------------------
// Structured-input whitespace passes
// ---------------------------------------------------------------------------

/// One resolved text piece plus optional authored provenance.
///
/// WHAT: keeps the text seen by a formatter, its authored span and the sparse preserved source
/// byte ranges still present after dedent/trim.
/// WHY: `$md` reports source errors at marker locations after preprocessing. A run-level
/// representative span would point at the wrong bytes, while per-character spans would be
/// heavier than the few sparse ranges a normal body retains.
struct WhitespaceTextPiece {
    text: String,
    span: Option<SourceSpan>,
    /// Preserved source byte ranges in output order. `None` when the caller did not ask for
    /// provenance or the piece had no authored span.
    ranges: Option<Vec<PreservedByteRange>>,
    /// Byte length of the authored text this span covers before preprocessing.
    source_byte_len: u32,
}

#[derive(Clone, Copy, Debug)]
struct PreservedByteRange {
    source_start: usize,
    length: usize,
}

enum WhitespacePiece {
    Text(WhitespaceTextPiece),
    Opaque(FormatterOpaquePiece),
}

impl WhitespaceTextPiece {
    /// Keeps only `kept` byte ranges from the current text and updates the sparse provenance.
    ///
    /// `kept` ranges are sorted, non-overlapping and measured in byte offsets of the current
    /// text. Deletion is the only preprocessing operation, so concatenating the kept ranges
    /// reproduces the old text-transformation output exactly.
    fn retain(&mut self, kept: &[(usize, usize)]) {
        let Some(ranges) = self.ranges.take() else {
            let old_text = std::mem::take(&mut self.text);
            self.text = kept
                .iter()
                .map(|&(start, end)| &old_text[start..end])
                .collect();
            return;
        };

        let old_text = std::mem::take(&mut self.text);
        let mut new_text = String::with_capacity(kept.iter().map(|(start, end)| end - start).sum());
        let mut new_ranges = Vec::new();
        let mut range_index = 0usize;
        let mut range_text_start = 0usize;

        for &(keep_start, keep_end) in kept {
            if keep_start >= keep_end {
                continue;
            }

            let mut cursor = keep_start;
            while cursor < keep_end {
                // Advance past source ranges entirely before this kept range.
                while range_index < ranges.len()
                    && range_text_start + ranges[range_index].length <= cursor
                {
                    range_text_start += ranges[range_index].length;
                    range_index += 1;
                }

                let Some(range) = ranges.get(range_index) else {
                    break;
                };
                let range_text_end = range_text_start + range.length;
                let overlap_end = keep_end.min(range_text_end);
                let length = overlap_end.saturating_sub(cursor);
                if length == 0 {
                    break;
                }

                new_ranges.push(PreservedByteRange {
                    source_start: range.source_start + (cursor - range_text_start),
                    length,
                });
                new_text.push_str(&old_text[cursor..overlap_end]);
                cursor = overlap_end;

                if cursor == range_text_end {
                    range_text_start = range_text_end;
                    range_index += 1;
                }
            }
        }

        debug_assert_eq!(
            new_text.len(),
            new_ranges.iter().map(|range| range.length).sum::<usize>(),
            "preserved text and provenance ranges must stay aligned"
        );
        self.text = new_text;
        self.ranges = Some(new_ranges);
    }

    fn trim_leading_boundary(&mut self, leading_trim_end: usize) {
        let kept = leading_boundary_keep_ranges(&self.text, leading_trim_end);
        self.retain(&kept);
    }

    fn dedent(&mut self, dedent_width: usize) {
        let kept = dedent_keep_ranges(&self.text, dedent_width);
        self.retain(&kept);
    }

    fn trim_trailing_boundary(&mut self) {
        let kept = trailing_boundary_keep_ranges(&self.text);
        self.retain(&kept);
    }
}

/// Returns the ranges retained by the leading boundary drain and blank-line indentation trim.
fn leading_boundary_keep_ranges(text: &str, leading_trim_end: usize) -> Vec<(usize, usize)> {
    let text_len = text.len();
    let mut cursor = leading_trim_end.min(text_len);
    let mut kept = Vec::new();

    loop {
        let indentation_len = horizontal_whitespace_len(&text[cursor..]);
        match text[cursor + indentation_len..].chars().next() {
            // A formatting-only blank line keeps its newline and drops its indentation.
            Some('\n') => {
                let newline = cursor + indentation_len;
                kept.push((newline, newline + 1));
                cursor = newline + 1;
            }
            Some(_) => {
                kept.push((cursor, text_len));
                break;
            }
            None => break,
        }
    }

    kept
}

/// Returns the ranges retained by dropping up to `dedent_width` horizontal whitespace chars
/// after every newline.
fn dedent_keep_ranges(text: &str, dedent_width: usize) -> Vec<(usize, usize)> {
    let text_len = text.len();
    let mut kept = Vec::new();
    let mut keep_start = 0usize;
    let mut index = 0usize;

    while index < text_len {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        if ch != '\n' {
            index += ch.len_utf8();
            continue;
        }

        let newline_end = index + 1;
        kept.push((keep_start, newline_end));

        let mut skip_cursor = newline_end;
        let mut skipped_chars = 0usize;
        while skipped_chars < dedent_width {
            let Some(next) = text[skip_cursor..].chars().next() else {
                break;
            };
            if !next.is_non_newline_whitespace() {
                break;
            }
            skip_cursor += next.len_utf8();
            skipped_chars += 1;
        }

        keep_start = skip_cursor;
        index = skip_cursor;
    }

    kept.push((keep_start, text_len));
    kept
}

/// Returns the ranges retained by removing a final-newline suffix that holds only whitespace.
fn trailing_boundary_keep_ranges(text: &str) -> Vec<(usize, usize)> {
    let Some(last_newline) = text.rfind('\n') else {
        return vec![(0, text.len())];
    };

    if text[last_newline + 1..]
        .chars()
        .all(|ch| ch.is_non_newline_whitespace())
    {
        vec![(0, last_newline)]
    } else {
        vec![(0, text.len())]
    }
}

/// Resolves formatter input into whitespace pieces, optionally retaining authored provenance.
fn whitespace_pieces_from_input(
    input: FormatterInput,
    string_table: &StringTable,
    preserve_spans: bool,
) -> Vec<WhitespacePiece> {
    input
        .pieces
        .into_iter()
        .map(|piece| match piece {
            FormatterInputPiece::Text(text_piece) => {
                let text = string_table.resolve(text_piece.text).to_owned();
                let span = if preserve_spans {
                    text_piece.span
                } else {
                    None
                };
                let source_byte_len = u32::try_from(text.len()).unwrap_or(u32::MAX);
                let ranges = span.map(|_| {
                    vec![PreservedByteRange {
                        source_start: 0,
                        length: text.len(),
                    }]
                });
                WhitespacePiece::Text(WhitespaceTextPiece {
                    text,
                    span,
                    ranges,
                    source_byte_len,
                })
            }
            FormatterInputPiece::Opaque(anchor) => WhitespacePiece::Opaque(anchor),
        })
        .collect()
}

fn whitespace_pieces_into_output(pieces: Vec<WhitespacePiece>) -> FormatterOutput {
    FormatterOutput {
        pieces: pieces
            .into_iter()
            .map(|piece| match piece {
                WhitespacePiece::Text(text) => FormatterOutputPiece::Text(text.text),
                WhitespacePiece::Opaque(anchor) => FormatterOutputPiece::Opaque(anchor),
            })
            .collect(),
    }
}

/// Emits formatter input text pieces split at deleted gaps, each carrying its exact authored span.
fn whitespace_pieces_into_input(
    pieces: Vec<WhitespacePiece>,
    string_table: &mut StringTable,
) -> FormatterInput {
    let mut output = Vec::new();

    for piece in pieces {
        let text_piece = match piece {
            WhitespacePiece::Text(text_piece) => text_piece,
            WhitespacePiece::Opaque(anchor) => {
                output.push(FormatterInputPiece::Opaque(anchor));
                continue;
            }
        };

        let (Some(span), Some(ranges)) = (text_piece.span, text_piece.ranges) else {
            output.push(FormatterInputPiece::Text(FormatterTextPiece {
                text: string_table.intern(&text_piece.text),
                span: text_piece.span,
            }));
            continue;
        };

        let mut cursor = 0usize;
        for range in ranges {
            let end = cursor + range.length;
            let piece_span = u32::try_from(range.source_start)
                .ok()
                .and_then(|source_start| {
                    u32::try_from(range.length).ok().and_then(|length| {
                        span.narrow_inline(source_start, length, text_piece.source_byte_len)
                    })
                })
                .or(Some(span));

            output.push(FormatterInputPiece::Text(FormatterTextPiece {
                text: string_table.intern(&text_piece.text[cursor..end]),
                span: piece_span,
            }));
            cursor = end;
        }
        debug_assert_eq!(
            cursor,
            text_piece.text.len(),
            "provenance ranges must cover the normalized text"
        );
    }

    FormatterInput { pieces: output }
}

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
    let mut pieces = whitespace_pieces_from_input(input, string_table, false);

    // Run each pass sequentially over the resolved piece list.
    for pass in passes {
        pieces = apply_structured_whitespace_pass(pieces, *pass, run_position);
    }

    whitespace_pieces_into_output(pieces)
}

/// Runs the same whitespace passes while retaining each text piece's authored span.
///
/// The markdown formatter owns source diagnostics after preprocessing, so it needs exact
/// marker locations. Pieces are split at deleted byte gaps and every emitted piece carries
/// a span narrowed from its authored source range.
pub(crate) fn apply_whitespace_passes_to_input_preserving_spans(
    input: FormatterInput,
    passes: &[TemplateWhitespacePassProfile],
    run_position: TemplateBodyRunPosition,
    string_table: &mut StringTable,
) -> FormatterInput {
    let mut pieces = whitespace_pieces_from_input(input, string_table, true);

    for pass in passes {
        pieces = apply_structured_whitespace_pass(pieces, *pass, run_position);
    }

    whitespace_pieces_into_input(pieces, string_table)
}

/// Applies a single whitespace pass across a list of resolved pieces.
///
/// Computes dedent width from the first text piece, then normalizes each text piece
/// while keeping opaque anchors untouched.
fn apply_structured_whitespace_pass(
    pieces: Vec<WhitespacePiece>,
    pass: TemplateWhitespacePassProfile,
    run_position: TemplateBodyRunPosition,
) -> Vec<WhitespacePiece> {
    let trim_leading = pass.trim_leading_boundary && run_position.is_first();
    let trim_trailing = pass.trim_trailing_boundary && run_position.is_last();

    match pass.pass {
        TemplateWhitespacePass::DefaultTemplateBody => {
            normalize_structured_default_whitespace(pieces, trim_leading, trim_trailing)
        }
    }
}

/// Structured version of the default template dedent/trim behavior.
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
    pieces: Vec<WhitespacePiece>,
    trim_leading: bool,
    trim_trailing: bool,
) -> Vec<WhitespacePiece> {
    // Find the first text piece to compute dedent width.
    let first_text = pieces.iter().find_map(|piece| match piece {
        WhitespacePiece::Text(text) if !text.text.is_empty() => Some(text.text.as_str()),
        _ => None,
    });

    let (dedent_width, leading_trim_end) = match first_text {
        Some(text) => compute_leading_dedent_info(text),
        None => (0, 0),
    };

    // Find the index of the last non-empty text piece for trailing trim.
    let last_text_index = pieces
        .iter()
        .rposition(|piece| matches!(piece, WhitespacePiece::Text(text) if !text.text.is_empty()));

    // Find the index of the first non-empty text piece for leading trim.
    let first_text_index = pieces
        .iter()
        .position(|piece| matches!(piece, WhitespacePiece::Text(text) if !text.text.is_empty()));

    // Boundary trimming only applies to actual run boundaries. Opaque anchors
    // before the first text or after the last text represent already-emitted
    // child templates or expressions, so adjacent text is interior separator
    // whitespace and must not be trimmed as if it began or ended the template.
    let leading_text_is_run_boundary = first_text_index.is_some_and(|first_index| {
        pieces[..first_index]
            .iter()
            .all(|piece| matches!(piece, WhitespacePiece::Text(text) if text.text.is_empty()))
    });
    let trailing_text_is_run_boundary = last_text_index.is_some_and(|last_index| {
        pieces[last_index + 1..]
            .iter()
            .all(|piece| matches!(piece, WhitespacePiece::Text(text) if text.text.is_empty()))
    });

    let mut result = Vec::with_capacity(pieces.len());

    for (index, piece) in pieces.into_iter().enumerate() {
        match piece {
            WhitespacePiece::Opaque(anchor) => result.push(WhitespacePiece::Opaque(anchor)),
            WhitespacePiece::Text(mut text) => {
                if text.text.is_empty() {
                    result.push(WhitespacePiece::Text(text));
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
                    text.trim_leading_boundary(leading_trim_end);
                }

                // Dedent after every newline using the shared dedent width.
                if dedent_width > 0 {
                    text.dedent(dedent_width);
                }

                // Trailing boundary trim: remove whitespace-only content after the last
                // newline in the final text piece.
                if trim_trailing && is_last_text && trailing_text_is_run_boundary {
                    text.trim_trailing_boundary();
                }

                result.push(WhitespacePiece::Text(text));
            }
        }
    }

    result
}
