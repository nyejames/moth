//! Shared HTML-project formatter validation helpers.
//!
//! WHAT:
//! - Flattens structured formatter input into pass-through output while preserving opaque anchors.
//! - Maps validator byte ranges back to template source locations for warning emission.
//!
//! WHY:
//! - `$html` and `$css` both validate literal text without rewriting it, so they should share the
//!   source-span bookkeeping rather than duplicating offset-mapping logic.

use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterInputPiece, FormatterOutput, FormatterOutputPiece,
};
use crate::compiler_frontend::ast::templates::template::FormatterResult;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};

#[derive(Clone, Debug)]
pub(crate) struct SourceWarning {
    pub message: String,
    pub start_offset: usize,
    pub end_offset: usize, // exclusive
}

#[derive(Clone, Debug)]
struct BodySourceSpan {
    start_offset: usize,
    end_offset: usize, // exclusive
    text: String,
    location: SourceLocation,
}

pub(crate) struct PassThroughFormatterInput {
    pub output_pieces: Vec<FormatterOutputPiece>,
    pub flattened_source: String,
    spans: Vec<BodySourceSpan>,
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
                            start_offset: offset,
                            end_offset: offset + char_len,
                            text: text.clone(),
                            location: text_piece.location,
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

        Self {
            output_pieces,
            flattened_source,
            spans,
        }
    }

    pub(crate) fn into_formatter_result(
        self,
        warnings: Vec<CompilerDiagnostic>,
    ) -> FormatterResult {
        FormatterResult {
            output: FormatterOutput {
                pieces: self.output_pieces,
            },
            warnings,
        }
    }

    pub(crate) fn map_warnings(
        &self,
        warnings: Vec<SourceWarning>,
        make_diagnostic: impl Fn(
            crate::compiler_frontend::symbols::string_interning::StringId,
            SourceLocation,
        ) -> CompilerDiagnostic,
        string_table: &mut StringTable,
    ) -> Vec<CompilerDiagnostic> {
        if self.spans.is_empty() || self.flattened_source.trim().is_empty() {
            return Vec::new();
        }

        warnings
            .into_iter()
            .filter_map(|warning| {
                map_warning_span_to_text_location(&self.spans, &warning).map(|location| {
                    let msg_id = string_table.get_or_intern(warning.message);
                    make_diagnostic(msg_id, location)
                })
            })
            .collect()
    }
}

fn map_warning_span_to_text_location(
    spans: &[BodySourceSpan],
    warning: &SourceWarning,
) -> Option<SourceLocation> {
    let start = warning.start_offset;
    let end_inclusive = warning.end_offset.saturating_sub(1).max(start);

    let start_point = map_offset_to_point_location(spans, start)?;
    let end_point = map_offset_to_point_location(spans, end_inclusive)?;

    if start_point.scope != end_point.scope {
        return Some(start_point);
    }

    Some(SourceLocation {
        scope: start_point.scope,
        start_pos: start_point.start_pos,
        end_pos: end_point.end_pos,
        start_byte: start_point.start_byte,
        end_byte: end_point.end_byte,
    })
}

fn map_offset_to_point_location(spans: &[BodySourceSpan], offset: usize) -> Option<SourceLocation> {
    let total_chars = spans.last().map(|span| span.end_offset)?;
    if total_chars == 0 {
        return None;
    }

    let clamped_offset = offset.min(total_chars.saturating_sub(1));
    let span = spans
        .iter()
        .find(|span| clamped_offset >= span.start_offset && clamped_offset < span.end_offset)
        .or_else(|| spans.last())?;
    let local_offset = clamped_offset.saturating_sub(span.start_offset);
    let position = position_after_chars(&span.location.start_pos, &span.text, local_offset);

    Some(SourceLocation {
        scope: span.location.scope.to_owned(),
        start_pos: position,
        end_pos: position,
        start_byte: 0,
        end_byte: 0,
    })
}

fn position_after_chars(start: &CharPosition, text: &str, consumed_chars: usize) -> CharPosition {
    let mut position = *start;
    for ch in text.chars().take(consumed_chars) {
        if ch == '\n' {
            position.line_number += 1;
            position.char_column = 0;
        } else {
            position.char_column += 1;
        }
    }

    position
}
