use super::line_index::{LineIndex, LinePosition, line_start_offsets};

#[test]
fn line_index_resolves_newline_shapes_and_zero_width_eof() {
    let crlf = "ab\r\n";
    let crlf_starts = line_start_offsets(crlf);
    let crlf_index = LineIndex::new(crlf, &crlf_starts);
    assert_eq!(crlf_index.line_count(), 1);
    assert_eq!(crlf_index.line_text(0), Some("ab"));
    for offset in [2, 3, 4] {
        assert_eq!(
            crlf_index.position(offset),
            Some(LinePosition { line: 0, column: 2 }),
            "CRLF offset {offset} should render at the visible line end"
        );
        assert_eq!(
            crlf_index.utf16_column(offset),
            Some(2),
            "UTF-16 conversion must use the same visible-line end"
        );
    }
    assert_eq!(crlf_index.line_of_offset(4), Some(0));

    let bare_cr = "ab\rc";
    let bare_cr_starts = line_start_offsets(bare_cr);
    let bare_cr_index = LineIndex::new(bare_cr, &bare_cr_starts);
    assert_eq!(bare_cr_index.line_count(), 2);
    assert_eq!(bare_cr_index.line_text(0), Some("ab"));
    assert_eq!(bare_cr_index.line_text(1), Some("c"));
    assert_eq!(
        bare_cr_index.position(2),
        Some(LinePosition { line: 0, column: 2 })
    );
    assert_eq!(
        bare_cr_index.position(3),
        Some(LinePosition { line: 1, column: 0 })
    );

    let trailing_bare_cr = "ab\r";
    let trailing_bare_cr_starts = line_start_offsets(trailing_bare_cr);
    let trailing_bare_cr_index = LineIndex::new(trailing_bare_cr, &trailing_bare_cr_starts);
    assert_eq!(trailing_bare_cr_index.line_count(), 1);
    assert_eq!(trailing_bare_cr_index.line_text(0), Some("ab"));
    assert_eq!(
        trailing_bare_cr_index.position(3),
        Some(LinePosition { line: 0, column: 2 })
    );

    let final_newline = "ab\n";
    let final_newline_starts = line_start_offsets(final_newline);
    let final_newline_index = LineIndex::new(final_newline, &final_newline_starts);
    assert_eq!(final_newline_index.line_count(), 1);
    assert_eq!(final_newline_index.line_text(0), Some("ab"));
    assert_eq!(
        final_newline_index.position(3),
        Some(LinePosition { line: 0, column: 2 })
    );
    assert_eq!(final_newline_index.line_of_offset(3), Some(0));

    let no_final_newline = "ab\ncd";
    let no_final_newline_starts = line_start_offsets(no_final_newline);
    let no_final_newline_index = LineIndex::new(no_final_newline, &no_final_newline_starts);
    assert_eq!(no_final_newline_index.line_count(), 2);
    assert_eq!(no_final_newline_index.line_text(1), Some("cd"));
    assert_eq!(
        no_final_newline_index.position(5),
        Some(LinePosition { line: 1, column: 2 })
    );

    let empty = "";
    let empty_starts = line_start_offsets(empty);
    let empty_index = LineIndex::new(empty, &empty_starts);
    assert_eq!(empty_index.line_count(), 0);
    assert_eq!(empty_index.line_text(0), None);
    assert_eq!(empty_index.line_of_offset(0), Some(0));
    assert_eq!(
        empty_index.position(0),
        Some(LinePosition { line: 0, column: 0 })
    );
    assert_eq!(empty_index.utf16_column(0), Some(0));
}

/// `str::lines()` agrees only for the LF and CRLF subset; bare CR is a tokenizer line break.
#[test]
fn line_text_matches_str_lines_for_lf_and_crlf() {
    let text = "lf\ncrlf\r\nempty\n\r\nlast";
    let line_starts = line_start_offsets(text);
    let line_index = LineIndex::new(text, &line_starts);
    let expected_lines: Vec<&str> = text.lines().collect();

    let actual_lines: Vec<&str> = (0..line_index.line_count())
        .map(|line| {
            line_index
                .line_text(line)
                .expect("counted line should have text")
        })
        .collect();
    assert_eq!(actual_lines, expected_lines);
}

#[test]
fn line_index_counts_scalar_and_utf16_columns_at_arbitrary_offsets() {
    let text = "aé😀z";
    let line_starts = line_start_offsets(text);
    let line_index = LineIndex::new(text, &line_starts);

    // `é` begins at byte 1 and occupies bytes 1..3; byte 2 is not a UTF-8 boundary.
    assert_eq!(
        line_index.position(2),
        Some(LinePosition { line: 0, column: 2 })
    );
    assert_eq!(
        line_index.position(5),
        Some(LinePosition { line: 0, column: 3 })
    );
    assert_eq!(
        line_index.utf16_column(5),
        Some(4),
        "the astral scalar occupies two UTF-16 code units"
    );
    assert_eq!(
        line_index.position(text.len() as u32),
        Some(LinePosition { line: 0, column: 4 })
    );
    assert_eq!(line_index.utf16_column(text.len() as u32), Some(5));
    assert_eq!(line_index.position(text.len() as u32 + 1), None);
    assert_eq!(line_index.utf16_column(text.len() as u32 + 1), None);
    assert_eq!(line_index.line_of_offset(text.len() as u32 + 1), None);
}

#[test]
fn line_index_keeps_long_lines_addressable_without_truncation() {
    let text = format!("{}\nend", "x".repeat(4096));
    let line_starts = line_start_offsets(&text);
    let line_index = LineIndex::new(&text, &line_starts);

    assert_eq!(line_index.line_count(), 2);
    assert_eq!(line_index.line_text(0).map(str::len), Some(4096));
    assert_eq!(
        line_index.position(4096),
        Some(LinePosition {
            line: 0,
            column: 4096
        })
    );
    assert_eq!(
        line_index.position(4097),
        Some(LinePosition { line: 1, column: 0 })
    );
    assert_eq!(
        line_index.position(text.len() as u32),
        Some(LinePosition { line: 1, column: 3 })
    );
}

#[derive(Clone, Copy)]
struct ReferenceLineRange {
    start: usize,
    visible_end: usize,
    end: usize,
}

fn reference_line_ranges(source: &str) -> Vec<ReferenceLineRange> {
    let mut ranges = Vec::new();
    let mut line_start = 0usize;
    let mut char_indices = source.char_indices().peekable();

    while let Some((offset, scalar)) = char_indices.next() {
        let Some(end) = (match scalar {
            '\n' => Some(offset + 1),
            '\r' => {
                if let Some(&(next_offset, '\n')) = char_indices.peek() {
                    char_indices.next();
                    Some(next_offset + 1)
                } else {
                    Some(offset + 1)
                }
            }
            _ => None,
        }) else {
            continue;
        };

        ranges.push(ReferenceLineRange {
            start: line_start,
            visible_end: offset,
            end,
        });
        line_start = end;
    }

    if ranges.is_empty() || line_start < source.len() {
        ranges.push(ReferenceLineRange {
            start: line_start,
            visible_end: source.len(),
            end: source.len(),
        });
    }

    ranges
}

fn reference_position<'a>(
    source: &'a str,
    ranges: &[ReferenceLineRange],
    offset: usize,
) -> (u32, u32, u32, &'a str) {
    let (line_number, line_range) = ranges
        .iter()
        .enumerate()
        .find(|(index, line_range)| {
            offset < line_range.end || (*index + 1 == ranges.len() && offset == line_range.end)
        })
        .expect("every source boundary must belong to one authored line");
    let line_text = &source[line_range.start..line_range.visible_end];
    let authored_offset = offset.saturating_sub(line_range.start).min(line_text.len());
    let mut scalar_column = 0u32;
    let mut utf16_column = 0u32;

    for (byte_offset, scalar) in line_text.char_indices() {
        if byte_offset >= authored_offset {
            break;
        }
        scalar_column += 1;
        utf16_column += scalar.len_utf16() as u32;
    }

    (line_number as u32, scalar_column, utf16_column, line_text)
}

#[test]
fn line_index_matches_an_independent_unicode_and_newline_oracle_at_every_boundary() {
    let source = "ascii é€😀\n\r\n\r\n\nx\rfinal é€😀";
    let reference_ranges = reference_line_ranges(source);
    let line_starts = line_start_offsets(source);
    let line_index = LineIndex::new(source, &line_starts);

    assert_eq!(
        reference_ranges.len(),
        6,
        "the authored fixture must retain its three empty terminator-only lines"
    );
    assert_eq!(line_index.line_count() as usize, reference_ranges.len());

    let offsets = source
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(source.len()));
    for offset in offsets {
        let (line, scalar_column, utf16_column, line_text) =
            reference_position(source, &reference_ranges, offset);

        assert_eq!(
            line_index.position(offset as u32),
            Some(LinePosition {
                line,
                column: scalar_column,
            }),
            "scalar position disagreed at source byte {offset}"
        );
        assert_eq!(
            line_index.line_of_offset(offset as u32),
            Some(line),
            "line lookup disagreed at source byte {offset}"
        );
        assert_eq!(
            line_index.utf16_column(offset as u32),
            Some(utf16_column),
            "UTF-16 position disagreed at source byte {offset}"
        );
        assert_eq!(
            line_index.line_text(line),
            Some(line_text),
            "visible line text disagreed at source byte {offset}"
        );
    }
}
