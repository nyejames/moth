//! Tests for the command-status and benchmark diagnostic-count helpers.

use super::benchmark_status_line;

#[test]
fn benchmark_status_line_formats_clean_counts() {
    assert_eq!(
        benchmark_status_line(Some("1"), 0, 0),
        Some(String::from("MOTH_BENCH status errors=0 warnings=0"))
    );
}

#[test]
fn benchmark_status_line_formats_warning_counts() {
    assert_eq!(
        benchmark_status_line(Some("1"), 0, 3),
        Some(String::from("MOTH_BENCH status errors=0 warnings=3"))
    );
}

#[test]
fn benchmark_status_line_formats_error_counts() {
    assert_eq!(
        benchmark_status_line(Some("1"), 2, 1),
        Some(String::from("MOTH_BENCH status errors=2 warnings=1"))
    );
}

#[test]
fn benchmark_status_line_requires_exact_opt_in_value() {
    for environment_value in [None, Some("0"), Some("true"), Some("01"), Some(" 1")] {
        assert_eq!(
            benchmark_status_line(environment_value, 1, 2),
            None,
            "unexpected benchmark status emission for {environment_value:?}"
        );
    }
}
