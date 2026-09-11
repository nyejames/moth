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
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, MalformedTemplateReason, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SourceWarning {
    pub reason: MalformedTemplateReason,
    pub start_offset: usize,
    pub end_offset: usize, // exclusive
}

#[derive(Clone, Debug)]
struct BodySourceSpan {
    start_offset: usize,
    end_offset: usize, // exclusive
    span: Option<SourceSpan>,
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

    /// Turn validator warnings into malformed-template diagnostics of one syntax kind.
    ///
    /// WHY: both callers report the same payload shape and differ only in which template kind
    /// was malformed, so the kind travels as data rather than as a per-caller constructor.
    pub(crate) fn map_warnings(
        &self,
        warnings: Vec<SourceWarning>,
        kind: SyntaxDiagnosticKind,
    ) -> Vec<CompilerDiagnostic> {
        if self.spans.is_empty() || self.flattened_source.trim().is_empty() {
            return Vec::new();
        }

        warnings
            .into_iter()
            .filter_map(|warning| {
                let span = map_warning_span_to_source_span(&self.spans, &warning)?;
                Some(CompilerDiagnostic::malformed_template(
                    kind,
                    warning.reason,
                    span,
                ))
            })
            .collect()
    }
}

fn map_warning_span_to_source_span(
    spans: &[BodySourceSpan],
    warning: &SourceWarning,
) -> Option<Option<SourceSpan>> {
    let start = warning.start_offset;
    let total_chars = spans.last().map(|span| span.end_offset)?;
    if total_chars == 0 {
        return None;
    }

    let clamped_offset = start.min(total_chars.saturating_sub(1));
    let span = spans
        .iter()
        .find(|span| clamped_offset >= span.start_offset && clamped_offset < span.end_offset)
        .or_else(|| spans.last())?;
    Some(span.span)
}
