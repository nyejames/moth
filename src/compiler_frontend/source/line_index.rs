//! Lazy line and column conversion over one retained UTF-8 snapshot.
//!
//! Scalar columns feed terminal and HTML carets; `utf16_column` stays reserved for the
//! deferred LSP-facing consumer.

use std::ops::Range;

/// Borrowed line and column conversion over retained source text.
///
/// The line-start slice inherits the 1C1 table shape: a non-empty snapshot starts with `0`,
/// every later entry is the byte immediately after a tokenizer line break, a final terminator
/// does not start a phantom line, and an empty snapshot has no entries. The line-break set is
/// the tokenizer's `newline_handling.rs` policy: `\n`, `\r\n` and a bare `\r`. This intentionally
/// differs from `str::lines()`, which splits on `\n` and leaves a bare `\r` in authored text.
/// This module decides how those exact byte ranges become source-facing positions. A line's
/// visible text strips one trailing `\r\n`, lone `\n` or lone `\r`.
///
/// An empty snapshot still resolves its zero-width EOF to line 0, column 0 even though its
/// inherited table is empty. In non-empty text, offsets inside a terminator and EOF after a
/// final terminator resolve to the end of that line's authored text; EOF without a final
/// terminator resolves to the end of the final line. Offsets beyond EOF are not addressable.
/// Columns count Unicode scalar starts before the offset, so an offset in the middle of a scalar
/// counts that scalar. UTF-16 conversion uses the same boundary rules and is reserved for an
/// LSP-facing caller. Long lines are not truncated or treated specially: lookup is binary search
/// and the one-line column scan is linear.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineIndex<'a> {
    text: &'a str,
    line_starts: &'a [u32],
}

/// A zero-based source position measured in Unicode scalar columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinePosition {
    pub(crate) line: u32,
    pub(crate) column: u32,
}

impl<'a> LineIndex<'a> {
    pub(crate) fn new(text: &'a str, line_starts: &'a [u32]) -> Self {
        Self { text, line_starts }
    }

    #[allow(dead_code)]
    pub(crate) fn line_count(self) -> u32 {
        self.line_starts.len() as u32
    }

    /// Return one line's exact byte range, including its authored LF, CRLF or bare-CR terminator.
    pub(crate) fn line_byte_range(self, line: u32) -> Option<Range<u32>> {
        let index = line as usize;
        let start = *self.line_starts.get(index)?;
        let end = self
            .line_starts
            .get(index + 1)
            .copied()
            .unwrap_or(self.text.len() as u32);

        Some(start..end)
    }

    /// Return one line's authored text without its LF, CRLF or bare-CR terminator.
    pub(crate) fn line_text(self, line: u32) -> Option<&'a str> {
        let range = self.line_byte_range(line)?;
        let line = self.text.get(range.start as usize..range.end as usize)?;
        let without_newline = line.strip_suffix('\n').unwrap_or(line);

        Some(
            without_newline
                .strip_suffix('\r')
                .unwrap_or(without_newline),
        )
    }

    /// Find the line containing an offset, preserving the table's final-newline rule.
    ///
    /// An offset inside a terminator still belongs to the line that terminator ends, and EOF
    /// belongs to the final authored line. The first table entry is always `0` for non-empty
    /// text, so the partition point is at least one and naming the line before it cannot
    /// underflow.
    pub(crate) fn line_of_offset(self, offset: u32) -> Option<u32> {
        if offset as usize > self.text.len() {
            return None;
        }
        if self.text.is_empty() {
            return Some(0);
        }

        let following_line = self.line_starts.partition_point(|start| *start <= offset);

        Some(following_line as u32 - 1)
    }

    pub(crate) fn position(self, offset: u32) -> Option<LinePosition> {
        let (line, line_text, authored_offset) = self.line_and_authored_offset(offset)?;
        let column = line_text
            .char_indices()
            .take_while(|(byte_offset, _)| (*byte_offset as u32) < authored_offset)
            .count() as u32;
        Some(LinePosition { line, column })
    }

    /// Return the UTF-16 column for an LSP-facing conversion only; terminal and source rendering
    /// must use [`Self::position`] and its Unicode-scalar column instead.
    #[allow(dead_code)]
    pub(crate) fn utf16_column(self, offset: u32) -> Option<u32> {
        let (_, line_text, authored_offset) = self.line_and_authored_offset(offset)?;
        let mut column = 0u32;
        for (byte_offset, scalar) in line_text.char_indices() {
            if (byte_offset as u32) >= authored_offset {
                break;
            }
            column += scalar.len_utf16() as u32;
        }
        Some(column)
    }

    /// Resolve an offset to its line, that line's visible text, and the offset within it.
    ///
    /// The returned offset is line-relative and may point past the visible text, into the
    /// terminator. Both column counters walk the visible text, so that is exactly what makes a
    /// terminator offset stop at the visible end rather than needing a separate clamp.
    fn line_and_authored_offset(self, offset: u32) -> Option<(u32, &'a str, u32)> {
        if self.text.is_empty() {
            return (offset == 0).then_some((0, self.text, 0));
        }

        let line = self.line_of_offset(offset)?;
        let range = self.line_byte_range(line)?;

        Some((line, self.line_text(line)?, offset - range.start))
    }
}

/// Build the line-start table for a snapshot that has just become owned.
///
/// Neither `\r` nor `\n` can occur inside a multi-byte UTF-8 sequence, so a byte scan is exact.
/// The first entry is always `0`. Each later entry is the byte immediately after a tokenizer
/// line break: `\n`, `\r\n` or a bare `\r`. A trailing terminator does not start another line,
/// so a start at `text.len()` is not recorded. An empty snapshot has no lines and so no entries,
/// which keeps the rule in the table rather than in every consumer that would otherwise
/// special-case it.
///
/// WHY two passes: counting sizes the table exactly, so the fill never reallocates, and both
/// passes are cheap byte reductions. Growing a `Vec` while scanning would copy the table roughly
/// once per doubling for no benefit.
pub(crate) fn line_start_offsets(text: &str) -> Box<[u32]> {
    let bytes = text.as_bytes();
    let Some((_, leading_bytes)) = bytes.split_last() else {
        return Box::default();
    };

    // A terminator in the final byte closes the last authored line without starting another, so
    // only the earlier bytes can open one. The `\r` of a CRLF pair is not a bare-CR boundary.
    let opens_next_line = |offset: usize, byte: u8| {
        byte == b'\n' || (byte == b'\r' && bytes.get(offset + 1) != Some(&b'\n'))
    };

    let line_count = 1 + leading_bytes
        .iter()
        .enumerate()
        .filter(|(offset, byte)| opens_next_line(*offset, **byte))
        .count();

    let mut line_starts = Vec::with_capacity(line_count);
    line_starts.push(0);

    for (offset, byte) in leading_bytes.iter().enumerate() {
        if opens_next_line(offset, *byte) {
            line_starts.push(offset as u32 + 1);
        }
    }

    debug_assert_eq!(
        line_starts.len(),
        line_count,
        "the counting pass must predict the fill exactly, or the fill reallocates"
    );
    line_starts.into_boxed_slice()
}
