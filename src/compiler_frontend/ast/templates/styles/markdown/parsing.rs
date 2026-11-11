//! Markdown parsing helpers: list items, headings, links, markers and atom utilities.
//!
//! WHAT: parses markdown line structures and inline reference syntax from atom streams.
//! WHY: separating parsing from rendering keeps the grammar rules in one place
//!      and makes the renderer's control flow easier to follow. Link and marker
//!      candidates share one target/separator scan so the two spellings cannot drift.

use super::{
    JoinedMarkdownAtoms, MarkdownAtomsRef, MarkdownInlineAtom, MarkdownLine, MarkdownListKind,
    MarkdownMarkerOrigin, ParsedMarkdownHeadingLine, ParsedMarkdownLink,
    ParsedMarkdownListItemLine, ParsedMarkdownMarker, ParsedMarkdownReference, atom_char,
};

pub(super) fn parse_heading_line(line: &MarkdownLine) -> Option<ParsedMarkdownHeadingLine<'_>> {
    let start = MarkdownAtomsRef::whole(line).skip_leading_horizontal_whitespace();
    let mut index = start;
    let mut level = 0usize;

    while let Some('#') = atom_char(&line.atoms, index) {
        level += 1;
        index += 1;
    }

    if level == 0 {
        return None;
    }

    // The documented flavour stops at `h6`. A longer run is ordinary paragraph text,
    // never an `h7` element.
    if level > 6 {
        return None;
    }

    let separator = atom_char(&line.atoms, index)?;
    if !separator.is_whitespace() {
        return None;
    }

    Some(ParsedMarkdownHeadingLine {
        level,
        content: MarkdownAtomsRef::whole(line).slice(index + 1, line.atoms.len()),
    })
}

/// Parses a list item marker line. Blank lines never match because every marker is visible text.
pub(super) fn parse_list_item_line(line: &MarkdownLine) -> Option<ParsedMarkdownListItemLine<'_>> {
    let (indent_width, start_index) = consume_line_indentation(&line.atoms);

    if let Some(item) = parse_unordered_list_item(line, start_index, indent_width) {
        return Some(item);
    }

    parse_ordered_list_item(line, start_index, indent_width)
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

fn parse_unordered_list_item<'a>(
    line: &'a MarkdownLine,
    start_index: usize,
    indent_width: usize,
) -> Option<ParsedMarkdownListItemLine<'a>> {
    let marker = atom_char(&line.atoms, start_index)?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }

    let separator = atom_char(&line.atoms, start_index + 1)?;
    if !separator.is_whitespace() {
        return None;
    }

    Some(ParsedMarkdownListItemLine {
        indent_width,
        kind: MarkdownListKind::Unordered,
        content: MarkdownAtomsRef::whole(line)
            .slice(start_index + 2, line.atoms.len())
            .trimmed(),
    })
}

fn parse_ordered_list_item<'a>(
    line: &'a MarkdownLine,
    start_index: usize,
    indent_width: usize,
) -> Option<ParsedMarkdownListItemLine<'a>> {
    let mut index = start_index;

    while let Some(ch) = atom_char(&line.atoms, index) {
        if !ch.is_ascii_digit() {
            break;
        }
        index += 1;
    }

    if index == start_index {
        return None;
    }

    let marker = atom_char(&line.atoms, index)?;
    if !matches!(marker, '.' | ')') {
        return None;
    }

    let separator = atom_char(&line.atoms, index + 1)?;
    if !separator.is_whitespace() {
        return None;
    }

    Some(ParsedMarkdownListItemLine {
        indent_width,
        kind: MarkdownListKind::Ordered,
        content: MarkdownAtomsRef::whole(line)
            .slice(index + 2, line.atoms.len())
            .trimmed(),
    })
}

/// Joins the lines of one paragraph into a single inline atom run.
///
/// Each line boundary stays a `'\n'` atom so every inline construct can enforce its own
/// newline rule. Inline code rejects it and a link separator accepts at most one. Inline
/// rendering prints each boundary as exactly one visible space. Marker origins move with
/// their atoms so a marker keeps its authored piece and byte offset.
pub(super) fn join_paragraph_lines(lines: &[MarkdownAtomsRef<'_>]) -> JoinedMarkdownAtoms {
    let mut joined = JoinedMarkdownAtoms::default();

    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            joined.atoms.push(MarkdownInlineAtom::Char('\n'));
        }

        let atom_base = joined.atoms.len();
        joined.atoms.extend_from_slice(line);
        for origin in line.marker_origins {
            let Some(relative_index) = origin.atom_index.checked_sub(line.atom_base) else {
                continue;
            };
            if relative_index >= line.len() {
                continue;
            }

            joined.marker_origins.push(MarkdownMarkerOrigin {
                atom_index: atom_base + relative_index,
                atom_end: atom_base + (origin.atom_end - line.atom_base),
                ..*origin
            });
        }
    }

    joined
}

/// Returns the char atom at `index` if it is plain text.
fn target_char(atoms: &[MarkdownInlineAtom], index: usize) -> Option<char> {
    atom_char(atoms, index)
}

/// One scanned `@` candidate before deciding whether it is a link, marker or literal.
struct ReferenceCandidate {
    target_start: usize,
    target_end: usize,
    /// Present only when at least one separator and then `(` follow the target.
    label_open: Option<usize>,
}

/// Parses `@target (label)` starting at the `@` atom.
///
/// Returns `None` without consuming anything when the candidate is malformed, so the
/// renderer prints it as literal text and keeps scanning for later constructs.
#[cfg(test)]
pub(super) fn try_parse_link_at_atoms(
    atoms: &[MarkdownInlineAtom],
    at_index: usize,
) -> Option<ParsedMarkdownLink> {
    let candidate = scan_reference_candidate(atoms, at_index)?;
    parse_link_label(atoms, at_index, &candidate)
}

/// Decides one `@` candidate as a labelled link, a bare section marker or literal text.
///
/// Labelled links take precedence. A recognised label introducer that fails to parse keeps
/// the existing literal fallback for the whole candidate, even when the target looks like a
/// marker. A `#` target without an introducer is a marker candidate; its ID grammar is
/// independent of the greedy URL target scan.
pub(super) fn try_parse_reference_at_atoms(
    atoms: MarkdownAtomsRef<'_>,
    at_index: usize,
    closing_emphasis_strength: Option<usize>,
) -> ParsedMarkdownReference {
    let Some(candidate) = scan_reference_candidate(&atoms, at_index) else {
        return ParsedMarkdownReference::Literal;
    };

    if candidate.label_open.is_some() {
        return match parse_link_label(&atoms, at_index, &candidate) {
            Some(link) => ParsedMarkdownReference::Link(link),
            None => ParsedMarkdownReference::Literal,
        };
    }

    let target = &atoms[candidate.target_start..candidate.target_end];
    if target.first() != Some(&MarkdownInlineAtom::Char('#')) {
        return ParsedMarkdownReference::Literal;
    }

    match parse_marker_id(target, closing_emphasis_strength) {
        Ok((id, consumed_id_chars)) => {
            let consumed_atoms = 2 + consumed_id_chars;
            ParsedMarkdownReference::Marker(ParsedMarkdownMarker {
                id,
                span: atoms.source_span_for_atoms(at_index, consumed_atoms),
                consumed_atoms,
            })
        }
        Err(consumed_atoms) => ParsedMarkdownReference::InvalidMarker {
            span: atoms.source_span_for_atoms(at_index, consumed_atoms),
        },
    }
}

/// Scans the target and optional label introducer shared by links and markers.
///
/// The target scan keeps the existing URL-prefix and non-whitespace rules. The separator
/// scan stops at an invalid character instead of rejecting the whole candidate, which lets
/// the caller distinguish a label introducer from ordinary following text.
fn scan_reference_candidate(
    atoms: &[MarkdownInlineAtom],
    at_index: usize,
) -> Option<ReferenceCandidate> {
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

    let separator_start = cursor;
    let mut crossed_line_boundary = false;
    while let Some(atom) = atoms.get(cursor) {
        match atom {
            MarkdownInlineAtom::Char(' ' | '\t') => cursor += 1,

            MarkdownInlineAtom::Char('\r' | '\n') if !crossed_line_boundary => {
                crossed_line_boundary = true;
                cursor = consume_soft_newline(atoms, cursor);
            }

            _ => break,
        }
    }

    let label_open = if cursor > separator_start && atom_char(atoms, cursor) == Some('(') {
        Some(cursor)
    } else {
        None
    };

    Some(ReferenceCandidate {
        target_start,
        target_end,
        label_open,
    })
}

/// Consumes one LF, CRLF or CR separator and returns the atom after it.
pub(super) fn consume_soft_newline(atoms: &[MarkdownInlineAtom], index: usize) -> usize {
    if atom_char(atoms, index) == Some('\r') && atom_char(atoms, index + 1) == Some('\n') {
        index + 2
    } else {
        index + 1
    }
}

fn parse_link_label(
    atoms: &[MarkdownInlineAtom],
    at_index: usize,
    candidate: &ReferenceCandidate,
) -> Option<ParsedMarkdownLink> {
    let mut cursor = candidate.label_open? + 1;
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

    let target = collect_plain_chars(&atoms[candidate.target_start..candidate.target_end]);

    Some(ParsedMarkdownLink {
        target,
        label,
        consumed_atoms: cursor + 1 - at_index,
    })
}

/// Parses a `#id` marker target.
///
/// Returns the ID and the number of body atoms it consumed, or the number of atoms in the
/// malformed marker candidate to locate in a diagnostic. A trailing star run is accepted
/// only when it is long enough to close the emphasis state already active at the marker.
fn parse_marker_id(
    target: &[MarkdownInlineAtom],
    closing_emphasis_strength: Option<usize>,
) -> Result<(String, usize), usize> {
    if target.first() != Some(&MarkdownInlineAtom::Char('#')) {
        return Err(2);
    }

    let body = &target[1..];
    if body.is_empty() {
        return Err(2);
    }

    let mut id = String::new();
    let mut consumed_chars = 0usize;

    for (index, atom) in body.iter().enumerate() {
        let MarkdownInlineAtom::Char(ch) = atom else {
            return Err(2 + index);
        };

        let is_valid = if index == 0 {
            ch.is_alphabetic()
        } else {
            ch.is_alphanumeric() || matches!(ch, '_' | '-')
        };
        if !is_valid {
            break;
        }

        id.push(*ch);
        consumed_chars += 1;
    }

    if consumed_chars == body.len() {
        return Ok((id, consumed_chars));
    }

    // A marker must carry an ID, even when a following star could close emphasis.
    if consumed_chars == 0 {
        return Err(2 + 1.min(body.len()));
    }

    // A closing emphasis delimiter is a valid terminator when it can close the active state.
    let remaining = &body[consumed_chars..];
    let remaining_chars_are_stars = remaining
        .iter()
        .all(|atom| matches!(atom, MarkdownInlineAtom::Char('*')));
    if remaining_chars_are_stars
        && closing_emphasis_strength.is_some_and(|strength| remaining.len() >= strength)
    {
        return Ok((id, consumed_chars));
    }

    Err(2 + consumed_chars + 1)
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

    match target_char(atoms, start)? {
        '/' | '#' | '?' => Some(1),
        ch if ch.is_ascii_alphabetic() => scheme_prefix_length(atoms, start),
        _ => None,
    }
}

/// Returns the length of a `scheme:` prefix whose first letter is already known to be alphabetic.
fn scheme_prefix_length(atoms: &[MarkdownInlineAtom], start: usize) -> Option<usize> {
    let mut cursor = start + 1;
    while let Some(ch) = target_char(atoms, cursor) {
        if !is_scheme_char(ch) {
            break;
        }
        cursor += 1;
    }

    if target_char(atoms, cursor)? != ':' {
        return None;
    }

    Some(cursor + 1 - start)
}

fn starts_with_chars(atoms: &[MarkdownInlineAtom], start: usize, prefix: &str) -> bool {
    prefix
        .chars()
        .enumerate()
        .all(|(offset, expected)| target_char(atoms, start + offset) == Some(expected))
}

fn is_scheme_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-')
}
