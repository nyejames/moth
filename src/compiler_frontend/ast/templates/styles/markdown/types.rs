//! Markdown formatter data shapes.
//!
//! WHAT: stores the small parsed markdown records shared by block and inline rendering.
//! WHY: keeping these shapes separate lets the formatter pipeline read as staged rendering
//! instead of a long prelude of local structs. Parsed records borrow their content from the
//! formatter's line stream so block detection never copies atoms.
//!
//! Marker origins stay sparse: only authored `@` characters carry a source location. The atom
//! view adjusts its base offset when headings or list markers are stripped, so origin indices
//! remain correct without a per-character span record.

use super::*;
use crate::compiler_frontend::source::SourceSpan;
use std::ops::Deref;

/// Returns the character at the given atom index if it is a plain text atom.
pub(super) fn atom_char(atoms: &[MarkdownInlineAtom], index: usize) -> Option<char> {
    match atoms.get(index)? {
        MarkdownInlineAtom::Char(ch) => Some(*ch),
        MarkdownInlineAtom::Opaque(_) => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarkdownListKind {
    Unordered,
    Ordered,
}

impl MarkdownListKind {
    pub(super) fn open_tag(self) -> &'static str {
        match self {
            Self::Unordered => "<ul>",
            Self::Ordered => "<ol>",
        }
    }

    pub(super) fn close_tag(self) -> &'static str {
        match self {
            Self::Unordered => "</ul>",
            Self::Ordered => "</ol>",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarkdownInlineAtom {
    Char(char),
    Opaque(FormatterOpaquePiece),
}

/// Authored location of one `@` atom inside a formatter text piece.
///
/// WHAT: maps a run of atoms from one authored text piece back to its exact source bytes.
/// WHY: marker diagnostics must point at the marker after dedent/trim, but the formatter
/// sees rewritten text. `atom_end` rejects a candidate that crosses into another piece, so
/// narrowing never claims bytes from an unrelated authored range.
#[derive(Clone, Copy, Debug)]
pub(super) struct MarkdownMarkerOrigin {
    /// `@` atom index in the owning line or joined run.
    pub(super) atom_index: usize,
    /// Exclusive end of the atoms that came from the same authored text piece.
    pub(super) atom_end: usize,
    /// Exact authored span of that text piece.
    pub(super) span: SourceSpan,
    /// Byte offset of the `@` inside the authored text piece.
    pub(super) byte_offset: usize,
    /// Byte length of the authored text piece.
    pub(super) text_byte_len: u32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct MarkdownLine {
    pub(super) atoms: Vec<MarkdownInlineAtom>,
    pub(super) marker_origins: Vec<MarkdownMarkerOrigin>,
}

/// Borrowed atom content with its sparse marker origins and slice base.
#[derive(Clone, Copy, Debug)]
pub(super) struct MarkdownAtomsRef<'a> {
    pub(super) atoms: &'a [MarkdownInlineAtom],
    pub(super) marker_origins: &'a [MarkdownMarkerOrigin],
    /// Index of `atoms[0]` in the origin indices.
    pub(super) atom_base: usize,
}

impl<'a> MarkdownAtomsRef<'a> {
    pub(super) fn whole(line: &'a MarkdownLine) -> Self {
        Self {
            atoms: &line.atoms,
            marker_origins: &line.marker_origins,
            atom_base: 0,
        }
    }

    pub(super) fn slice(self, start: usize, end: usize) -> Self {
        Self {
            atoms: &self.atoms[start..end],
            marker_origins: self.marker_origins,
            atom_base: self.atom_base + start,
        }
    }

    pub(super) fn trimmed(self) -> Self {
        let mut start = 0usize;
        let mut end = self.atoms.len();

        while start < end && matches!(atom_char(self.atoms, start), Some(ch) if ch.is_whitespace())
        {
            start += 1;
        }
        while end > start
            && matches!(atom_char(self.atoms, end - 1), Some(ch) if ch.is_whitespace())
        {
            end -= 1;
        }

        self.slice(start, end)
    }

    pub(super) fn trim_leading_horizontal_whitespace(self) -> Self {
        let mut start = 0usize;
        while let Some(MarkdownInlineAtom::Char(' ' | '\t')) = self.atoms.get(start) {
            start += 1;
        }
        self.slice(start, self.atoms.len())
    }

    pub(super) fn skip_leading_horizontal_whitespace(self) -> usize {
        let mut index = 0usize;
        while let Some(MarkdownInlineAtom::Char(' ' | '\t')) = self.atoms.get(index) {
            index += 1;
        }
        index
    }

    /// Returns the exact authored span for `atom_count` atoms starting at `start`.
    ///
    /// Falls back to the containing authored piece when the atoms cross a piece boundary or
    /// the span cannot narrow in place. A synthetic piece with no authored span returns `None`.
    pub(super) fn source_span_for_atoms(
        self,
        start: usize,
        atom_count: usize,
    ) -> Option<SourceSpan> {
        let absolute_start = self.atom_base + start;
        let origin = self
            .marker_origins
            .iter()
            .find(|origin| origin.atom_index == absolute_start)?;
        let absolute_end = absolute_start + atom_count;
        if absolute_end > origin.atom_end {
            return Some(origin.span);
        }

        let mut byte_length = 0u32;
        for atom in &self.atoms[start..start + atom_count] {
            let MarkdownInlineAtom::Char(ch) = atom else {
                return Some(origin.span);
            };
            byte_length = byte_length.checked_add(u32::try_from(ch.len_utf8()).ok()?)?;
        }

        let source_start = u32::try_from(origin.byte_offset).ok()?;
        origin
            .span
            .narrow_inline(source_start, byte_length, origin.text_byte_len)
            .or(Some(origin.span))
    }
}

impl Deref for MarkdownAtomsRef<'_> {
    type Target = [MarkdownInlineAtom];

    fn deref(&self) -> &Self::Target {
        self.atoms
    }
}

/// Owned atom run built from several paragraph lines.
#[derive(Debug, Default)]
pub(super) struct JoinedMarkdownAtoms {
    pub(super) atoms: Vec<MarkdownInlineAtom>,
    pub(super) marker_origins: Vec<MarkdownMarkerOrigin>,
}

impl JoinedMarkdownAtoms {
    pub(super) fn view(&self) -> MarkdownAtomsRef<'_> {
        MarkdownAtomsRef {
            atoms: &self.atoms,
            marker_origins: &self.marker_origins,
            atom_base: 0,
        }
    }
}

#[derive(Debug)]
pub(super) struct ParsedMarkdownListItemLine<'a> {
    pub(super) indent_width: usize,
    pub(super) kind: MarkdownListKind,
    pub(super) content: MarkdownAtomsRef<'a>,
}

#[derive(Debug)]
pub(super) struct ParsedMarkdownHeadingLine<'a> {
    pub(super) level: usize,
    pub(super) content: MarkdownAtomsRef<'a>,
}

#[derive(Debug)]
pub(super) struct ParsedMarkdownLink {
    pub(super) target: String,
    pub(super) label: String,
    pub(super) consumed_atoms: usize,
}

/// One bare `@#id` marker, with its exact authored span when available.
#[derive(Debug)]
pub(super) struct ParsedMarkdownMarker {
    pub(super) id: String,
    pub(super) span: Option<SourceSpan>,
    pub(super) consumed_atoms: usize,
}

/// Decision for one `@` reference candidate.
#[derive(Debug)]
pub(super) enum ParsedMarkdownReference {
    Link(ParsedMarkdownLink),
    Marker(ParsedMarkdownMarker),
    /// A marker-shaped candidate with an empty, malformed or unterminated ID.
    InvalidMarker {
        span: Option<SourceSpan>,
    },
    /// Existing literal fallback: only the `@` is consumed.
    Literal,
}
