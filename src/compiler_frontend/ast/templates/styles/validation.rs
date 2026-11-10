//! Shared frontend formatter validation helpers.
//!
//! WHAT:
//! - Flattens structured formatter input into pass-through output while preserving opaque anchors.
//! - Maps validator character ranges back to exact template source byte spans.
//!
//! WHY:
//! - `$html` and `$css` both validate literal text without rewriting it, so they should share the
//!   source-span bookkeeping rather than duplicating offset-mapping logic.

use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterInputPiece, FormatterOutput, FormatterOutputPiece,
};
use crate::compiler_frontend::ast::templates::template::FormatterResult;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, MalformedTemplateReason};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SourceWarning {
    pub reason: MalformedTemplateReason,
    /// Start character index in the flattened validator source.
    pub start_char: usize,
    /// Exclusive end character index in the flattened validator source.
    pub end_char: usize,
}

#[derive(Clone, Debug)]
struct BodySourceSpan {
    start_char: usize,
    end_char: usize, // exclusive
    /// Byte length of the piece text the source span covers exactly.
    text_byte_len: u32,
    span: Option<SourceSpan>,
}

pub(crate) struct PassThroughFormatterInput {
    pub output_pieces: Vec<FormatterOutputPiece>,
    pub flattened_source: String,
    spans: Vec<BodySourceSpan>,
    /// Flattened character index to flattened byte offset, plus one past the final character.
    char_to_byte: Vec<usize>,
}

impl PassThroughFormatterInput {
    /// Flattens formatter input while preserving opaque anchors in the output stream.
    ///
    /// WHAT:
    /// - Collects the literal text seen by a validator into one flattened string.
    /// - Preserves the original text pieces as pass-through formatter output.
    /// - Records source spans so later warnings can map flattened offsets back to template
    ///   locations.
    ///
    /// WHY:
    /// - HTML/CSS validators need a contiguous string view, but the frontend render plan still
    ///   needs opaque child anchors to survive untouched.
    pub(crate) fn from_input(input: FormatterInput, string_table: &StringTable) -> Self {
        let mut output_pieces = Vec::with_capacity(input.pieces.len());
        let mut spans = Vec::new();
        let mut flattened_source = String::new();
        let mut offset = 0usize;

        for piece in input.pieces {
            match piece {
                FormatterInputPiece::Text(text_piece) => {
                    let text = string_table.resolve(text_piece.text).to_owned();
                    let char_len = text.chars().count();
                    if char_len > 0 {
                        spans.push(BodySourceSpan {
                            start_char: offset,
                            end_char: offset + char_len,
                            text_byte_len: u32::try_from(text.len()).unwrap_or(u32::MAX),
                            span: text_piece.span,
                        });
                        offset += char_len;
                    }
                    flattened_source.push_str(&text);
                    output_pieces.push(FormatterOutputPiece::Text(text));
                }

                FormatterInputPiece::Opaque(anchor) => {
                    output_pieces.push(FormatterOutputPiece::Opaque(anchor));
                }
            }
        }

        // Validators scan characters, while authored spans are byte ranges. One table keeps the
        // conversion in this owner and makes non-ASCII text map correctly.
        let mut char_to_byte = Vec::with_capacity(flattened_source.chars().count() + 1);
        char_to_byte.extend(flattened_source.char_indices().map(|(byte, _)| byte));
        char_to_byte.push(flattened_source.len());

        Self {
            output_pieces,
            flattened_source,
            spans,
            char_to_byte,
        }
    }

    pub(crate) fn into_formatter_result(self, warnings: DiagnosticBag) -> FormatterResult {
        FormatterResult {
            output: FormatterOutput {
                pieces: self.output_pieces,
            },
            warnings,
        }
    }

    /// Turn validator warnings into malformed-template diagnostics using their typed reasons.
    pub(crate) fn map_warnings(&self, warnings: Vec<SourceWarning>) -> DiagnosticBag {
        if self.spans.is_empty() || self.flattened_source.trim().is_empty() {
            return DiagnosticBag::new();
        }

        DiagnosticBag::from_diagnostics(
            warnings
                .into_iter()
                .filter_map(|warning| {
                    let span =
                        map_warning_span_to_source_span(&self.spans, &self.char_to_byte, &warning)?;
                    Some(templates::malformed_template(warning.reason, span))
                })
                .collect(),
        )
    }
}

fn map_warning_span_to_source_span(
    spans: &[BodySourceSpan],
    char_to_byte: &[usize],
    warning: &SourceWarning,
) -> Option<Option<SourceSpan>> {
    let total_chars = spans.last().map(|span| span.end_char)?;
    if total_chars == 0 {
        return None;
    }

    let start_char = warning.start_char.min(total_chars.saturating_sub(1));
    let piece = spans
        .iter()
        .find(|span| start_char >= span.start_char && start_char < span.end_char)
        .or_else(|| spans.last())?;
    let Some(piece_span) = piece.span else {
        return Some(None);
    };

    let piece_start_byte = *char_to_byte.get(piece.start_char)?;
    let start_byte = *char_to_byte.get(start_char)?;
    let offset_in_piece = u32::try_from(start_byte - piece_start_byte).ok()?;

    // A literal that runs past this piece cannot name one authored range, so the piece span keeps
    // the warning at the authored segment instead of inventing a joined location.
    let end_char = warning.end_char.min(piece.end_char).max(start_char + 1);
    let end_byte = *char_to_byte.get(end_char)?;
    let byte_length = u32::try_from(end_byte.saturating_sub(start_byte).max(1)).ok()?;

    // Formatter pieces arrive with their exact authored span. Narrowing needs no resolver or
    // builder while that span is stored inline; an extended piece span keeps the broader exact
    // authored segment because this stage cannot read its start offset.
    let span = piece_span
        .narrow_inline(offset_in_piece, byte_length, piece.text_byte_len)
        .unwrap_or(piece_span);

    Some(Some(span))
}
