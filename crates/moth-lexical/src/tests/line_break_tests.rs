//! Tests for the shared logical line-break boundary.

use super::is_line_break;

#[test]
fn only_lf_and_cr_are_logical_line_breaks() {
    for line_break in ['\n', '\r'] {
        assert!(
            is_line_break(line_break),
            "{line_break:?} should break a line"
        );
    }

    for horizontal_whitespace in [
        ' ', '\t', '\u{000b}', '\u{000c}', '\u{0085}', '\u{00a0}', '\u{2028}', '\u{2029}',
    ] {
        assert!(
            !is_line_break(horizontal_whitespace),
            "{horizontal_whitespace:?} should remain horizontal whitespace"
        );
    }
}
