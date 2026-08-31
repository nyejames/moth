//! Buffered markdown formatter output assembly.
//!
//! WHAT: batches text while preserving opaque formatter anchors as separate pieces.
//! WHY: inline and block renderers need one flush point so anchors never get flattened
//! into escaped text.

use super::*;

#[derive(Debug, Default)]
pub(super) struct MarkdownOutputBuilder {
    pieces: Vec<FormatterOutputPiece>,
    text_buffer: String,
}

impl MarkdownOutputBuilder {
    pub(super) fn push_raw(&mut self, text: &str) {
        self.text_buffer.push_str(text);
    }

    pub(super) fn push_escaped_char(&mut self, ch: char) {
        match ch {
            '<' => self.text_buffer.push_str("&lt;"),
            '>' => self.text_buffer.push_str("&gt;"),
            '&' => self.text_buffer.push_str("&amp;"),
            '"' => self.text_buffer.push_str("&quot;"),
            '\'' => self.text_buffer.push_str("&#39;"),
            _ => self.text_buffer.push(ch),
        }
    }

    pub(super) fn push_escaped_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.push_escaped_char(ch);
        }
    }

    pub(super) fn push_opaque(&mut self, anchor: FormatterOpaquePiece) {
        self.flush_text();
        self.pieces.push(FormatterOutputPiece::Opaque(anchor));
    }
    /// Preserves a formatter-generated site-root anchor through whitespace and TIR adaptation.
    pub(super) fn push_site_root(&mut self) {
        self.flush_text();
        self.pieces
            .push(FormatterOutputPiece::Opaque(FormatterOpaquePiece {
                id: FormatterAnchorId(0),
                kind: FormatterOpaqueKind::SiteRoot,
            }));
    }

    pub(super) fn append_pieces(&mut self, pieces: Vec<FormatterOutputPiece>) {
        for piece in pieces {
            match piece {
                FormatterOutputPiece::Text(text) => self.text_buffer.push_str(&text),
                FormatterOutputPiece::Opaque(anchor) => self.push_opaque(anchor),
            }
        }
    }

    pub(super) fn finish(mut self) -> Vec<FormatterOutputPiece> {
        self.flush_text();
        self.pieces
    }

    pub(super) fn flush_text(&mut self) {
        if self.text_buffer.is_empty() {
            return;
        }

        self.pieces.push(FormatterOutputPiece::Text(std::mem::take(
            &mut self.text_buffer,
        )));
    }
}
