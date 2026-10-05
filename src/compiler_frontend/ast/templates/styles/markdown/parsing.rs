//! Markdown parsing helpers: list items, headings, links, and atom utilities.
//!
//! WHAT: parses markdown line structures and inline link syntax from atom streams.
//! WHY: separating parsing from rendering keeps the grammar rules in one place
//!      and makes the renderer's control flow easier to follow.

use super::{
    MarkdownInlineAtom, MarkdownLine, MarkdownListKind, ParsedMarkdownHeadingLine,
    ParsedMarkdownLink, ParsedMarkdownListItemLine, atom_char,
};

pub(super) fn parse_heading_line(line: &MarkdownLine) -> Option<ParsedMarkdownHeadingLine<'_>> {
    let start = skip_leading_horizontal_whitespace(&line.atoms);
    let mut index = start;
    let mut level = 0usize;

    while let Some('#') = atom_char(&line.atoms, index) {
        level += 1;
        index += 1;
    }

    if level == 0 {
        return None;
    }

    let separator = atom_char(&line.atoms, index)?;
    if !separator.is_whitespace() {
        return None;
    }

    Some(ParsedMarkdownHeadingLine {
        level,
        content: &line.atoms[index + 1..],
    })
}

/// Parses a list item marker line. Blank lines never match because every marker is visible text.
pub(super) fn parse_list_item_line(line: &MarkdownLine) -> Option<ParsedMarkdownListItemLine<'_>> {
    let (indent_width, start_index) = consume_line_indentation(&line.atoms);

    if let Some(item) = parse_unordered_list_item(&line.atoms, start_index, indent_width) {
        return Some(item);
    }

    parse_ordered_list_item(&line.atoms, start_index, indent_width)
}

fn consume_line_indentation(atoms: &[MarkdownInlineAtom]) -> (usize, usize) {
    let mut indent_width = 0usize;
    let mut index = 0usize;

    while let Some(atom) = atoms.get(index) {
        match atom {
            MarkdownInlineAtom::Char(' ') => {
                indent_width += 1;
                index += 1;
            }
            MarkdownInlineAtom::Char('\t') => {
                // Tabs are treated as a single indentation chunk for nested list parsing.
                indent_width += 4;
                index += 1;
            }
            _ => break,
        }
    }

    (indent_width, index)
}

fn parse_unordered_list_item(
    atoms: &[MarkdownInlineAtom],
    start_index: usize,
    indent_width: usize,
) -> Option<ParsedMarkdownListItemLine<'_>> {
    let marker = atom_char(atoms, start_index)?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }

    let separator = atom_char(atoms, start_index + 1)?;
    if !separator.is_whitespace() {
        return None;
    }

    Some(ParsedMarkdownListItemLine {
        indent_width,
        kind: MarkdownListKind::Unordered,
        content: trim_atoms(&atoms[start_index + 2..]),
    })
}

fn parse_ordered_list_item(
    atoms: &[MarkdownInlineAtom],
    start_index: usize,
    indent_width: usize,
) -> Option<ParsedMarkdownListItemLine<'_>> {
    let mut index = start_index;

    while let Some(ch) = atom_char(atoms, index) {
        if !ch.is_ascii_digit() {
            break;
        }
        index += 1;
    }

    if index == start_index {
        return None;
    }

    let marker = atom_char(atoms, index)?;
    if !matches!(marker, '.' | ')') {
        return None;
    }

    let separator = atom_char(atoms, index + 1)?;
    if !separator.is_whitespace() {
        return None;
    }

    Some(ParsedMarkdownListItemLine {
        indent_width,
        kind: MarkdownListKind::Ordered,
        content: trim_atoms(&atoms[index + 2..]),
    })
}

/// Joins the lines of one paragraph into a single inline atom run.
///
/// Each line boundary stays a `'\n'` atom so every inline construct can enforce its own
/// newline rule. Inline code rejects it and a link separator accepts at most one. Inline
/// rendering prints each boundary as exactly one visible space.
pub(super) fn join_paragraph_lines(lines: &[&[MarkdownInlineAtom]]) -> Vec<MarkdownInlineAtom> {
    let mut joined = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            joined.push(MarkdownInlineAtom::Char('\n'));
        }
        joined.extend_from_slice(line);
    }

    joined
}

pub(super) fn trim_atoms(atoms: &[MarkdownInlineAtom]) -> &[MarkdownInlineAtom] {
    let mut start = 0usize;
    let mut end = atoms.len();

    while start < end && matches!(atoms[start], MarkdownInlineAtom::Char(ch) if ch.is_whitespace())
    {
        start += 1;
    }

    while end > start
        && matches!(atoms[end - 1], MarkdownInlineAtom::Char(ch) if ch.is_whitespace())
    {
        end -= 1;
    }

    &atoms[start..end]
}

pub(super) fn trim_leading_horizontal_whitespace(
    atoms: &[MarkdownInlineAtom],
) -> &[MarkdownInlineAtom] {
    &atoms[skip_leading_horizontal_whitespace(atoms)..]
}

pub(super) fn skip_leading_horizontal_whitespace(atoms: &[MarkdownInlineAtom]) -> usize {
    let mut index = 0usize;
    while let Some(MarkdownInlineAtom::Char(' ' | '\t')) = atoms.get(index) {
        index += 1;
    }
    index
}

/// Parses `@target (label)` starting at the `@` atom.
///
/// Returns `None` without consuming anything when the candidate is malformed, so the
/// renderer prints it as literal text and keeps scanning for later constructs.
pub(super) fn try_parse_link_at_atoms(
    atoms: &[MarkdownInlineAtom],
    at_index: usize,
) -> Option<ParsedMarkdownLink> {
    if atom_char(atoms, at_index)? != '@' {
        return None;
    }

    let target_start = at_index + 1;
    let mut cursor = target_start + target_prefix_length(atoms, target_start)?;

    while let Some(atom) = atoms.get(cursor) {
        match atom {
            MarkdownInlineAtom::Char(ch) if !ch.is_whitespace() => cursor += 1,
            MarkdownInlineAtom::Char(_) => break,
            MarkdownInlineAtom::Opaque(_) => return None,
        }
    }
    let target_end = cursor;

    // The separator is spaces or tabs with at most one soft line boundary, so a wrapped
    // paragraph can put the label on the next line. A second boundary would mean a blank
    // line, and an opaque anchor could render as anything, so both reject the candidate.
    let separator_start = cursor;
    let mut crossed_line_boundary = false;
    while let Some(atom) = atoms.get(cursor) {
        match atom {
            MarkdownInlineAtom::Char(' ' | '\t') => cursor += 1,

            MarkdownInlineAtom::Char('\n') if !crossed_line_boundary => {
                crossed_line_boundary = true;
                cursor += 1;
            }

            MarkdownInlineAtom::Char('\n') | MarkdownInlineAtom::Opaque(_) => return None,

            MarkdownInlineAtom::Char(_) => break,
        }
    }
    if separator_start == cursor {
        return None;
    }

    if atom_char(atoms, cursor)? != '(' {
        return None;
    }
    cursor += 1;

    let label_start = cursor;
    while let Some(atom) = atoms.get(cursor) {
        match atom {
            MarkdownInlineAtom::Char(')') => break,
            MarkdownInlineAtom::Char(_) => cursor += 1,
            MarkdownInlineAtom::Opaque(_) => return None,
        }
    }

    if atom_char(atoms, cursor)? != ')' {
        return None;
    }

    let label = collect_plain_chars(&atoms[label_start..cursor]);
    if label.chars().all(char::is_whitespace) {
        return None;
    }

    let target = collect_plain_chars(&atoms[target_start..target_end]);

    Some(ParsedMarkdownLink {
        target,
        label,
        consumed_atoms: cursor + 1 - at_index,
    })
}

fn collect_plain_chars(atoms: &[MarkdownInlineAtom]) -> String {
    atoms
        .iter()
        .filter_map(|atom| match atom {
            MarkdownInlineAtom::Char(ch) => Some(*ch),
            MarkdownInlineAtom::Opaque(_) => None,
        })
        .collect()
}

/// Returns the atom length of the scheme or path prefix that must open a link target.
fn target_prefix_length(atoms: &[MarkdownInlineAtom], start: usize) -> Option<usize> {
    // Multi-character path prefixes are checked first so `//` never reads as a site route.
    for prefix in ["//", "./", "../"] {
        if starts_with_chars(atoms, start, prefix) {
            return Some(prefix.len());
        }
    }

    match atom_char(atoms, start)? {
        '/' | '#' | '?' => Some(1),
        ch if ch.is_ascii_alphabetic() => scheme_prefix_length(atoms, start),
        _ => None,
    }
}

/// Returns the length of a `scheme:` prefix whose first letter is already known to be alphabetic.
fn scheme_prefix_length(atoms: &[MarkdownInlineAtom], start: usize) -> Option<usize> {
    let mut cursor = start + 1;
    while let Some(ch) = atom_char(atoms, cursor) {
        if !is_scheme_char(ch) {
            break;
        }
        cursor += 1;
    }

    if atom_char(atoms, cursor)? != ':' {
        return None;
    }

    Some(cursor + 1 - start)
}

fn starts_with_chars(atoms: &[MarkdownInlineAtom], start: usize, prefix: &str) -> bool {
    prefix
        .chars()
        .enumerate()
        .all(|(offset, expected)| atom_char(atoms, start + offset) == Some(expected))
}

fn is_scheme_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-')
}
