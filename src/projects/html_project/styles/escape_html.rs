//! HTML-project-owned HTML text escaping.
//!
//! WHAT:
//! - `push_escaped_html_text` is the single allocation-free writer for the five
//!   HTML-sensitive bytes, shared by the `$code` highlighter and the `$escape_html`
//!   formatter.
//! - `EscapeHtmlTemplateFormatter` is the public `$escape_html` directive wrapper.
//! - Preserves opaque child anchors so frontend composition semantics remain unchanged.
//!
//! WHY:
//! - HTML escaping is output-policy behavior owned by the HTML project builder, not a core
//!   language directive. One writer keeps `$code` and `$escape_html` from duplicating the
//!   same five-byte escape loop.

use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterInputPiece, FormatterOutput, FormatterOutputPiece,
};
use crate::compiler_frontend::ast::templates::template::{
    Formatter, FormatterResult, TemplateFormatter,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::style_directives::StyleDirectiveArgumentValue;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;

/// Writes `text` with the five ASCII HTML-sensitive bytes escaped.
///
/// WHAT: copies safe UTF-8 slice batches between the escape bytes and replaces only
///       `& < > " '` with their named entities.
/// WHY: every replacement byte is ASCII, so the byte indexes between escapes stay
///      valid UTF-8 boundaries and plain text is copied without decoding every scalar.
pub(super) fn push_escaped_html_text(output: &mut String, text: &str) {
    let mut chunk_start = 0;

    for (index, byte) in text.bytes().enumerate() {
        let replacement: &str = match byte {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            b'\'' => "&#39;",
            _ => continue,
        };

        output.push_str(&text[chunk_start..index]);
        output.push_str(replacement);
        chunk_start = index + 1;
    }

    output.push_str(&text[chunk_start..]);
}

#[derive(Debug)]
struct EscapeHtmlTemplateFormatter;

impl TemplateFormatter for EscapeHtmlTemplateFormatter {
    fn format(
        &self,
        input: FormatterInput,
        string_table: &mut StringTable,
    ) -> Result<FormatterResult, CompilerMessages> {
        let pieces = input
            .pieces
            .into_iter()
            .map(|piece| match piece {
                FormatterInputPiece::Text(text_piece) => {
                    let text = string_table.resolve(text_piece.text);
                    let mut escaped = String::with_capacity(text.len());
                    push_escaped_html_text(&mut escaped, text);

                    FormatterOutputPiece::Text(escaped)
                }
                // Opaque anchors (child templates, dynamic expressions) pass through
                // without escaping — their content is sealed.
                FormatterInputPiece::Opaque(id) => FormatterOutputPiece::Opaque(id),
            })
            .collect();

        Ok(FormatterResult {
            output: FormatterOutput { pieces },
            warnings: Vec::new(),
        })
    }
}

pub(crate) fn escape_html_formatter() -> Formatter {
    Formatter {
        pre_format_whitespace_passes: Vec::new(),
        formatter: Arc::new(EscapeHtmlTemplateFormatter),
        post_format_whitespace_passes: Vec::new(),
    }
}

pub(crate) fn escape_html_formatter_factory(
    argument: Option<&StyleDirectiveArgumentValue>,
) -> Result<Formatter, String> {
    if argument.is_some() {
        return Err("'$escape_html' does not accept arguments.".to_string());
    }

    Ok(escape_html_formatter())
}
