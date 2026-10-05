//! Markdown formatter data shapes.
//!
//! WHAT: stores the small parsed markdown records shared by block and inline rendering.
//! WHY: keeping these shapes separate lets the formatter pipeline read as staged rendering
//! instead of a long prelude of local structs. Parsed records borrow their content from the
//! formatter's line stream so block detection never copies atoms.

use super::*;

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

#[derive(Clone, Debug, Default)]
pub(super) struct MarkdownLine {
    pub(super) atoms: Vec<MarkdownInlineAtom>,
}

#[derive(Debug)]
pub(super) struct ParsedMarkdownListItemLine<'a> {
    pub(super) indent_width: usize,
    pub(super) kind: MarkdownListKind,
    pub(super) content: &'a [MarkdownInlineAtom],
}

#[derive(Debug)]
pub(super) struct ParsedMarkdownHeadingLine<'a> {
    pub(super) level: usize,
    pub(super) content: &'a [MarkdownInlineAtom],
}

#[derive(Debug)]
pub(super) struct ParsedMarkdownLink {
    pub(super) target: String,
    pub(super) label: String,
    pub(super) consumed_atoms: usize,
}
