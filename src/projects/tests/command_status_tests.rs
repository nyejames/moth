//! Tests for the command-status and benchmark diagnostic-count helpers.

use super::{benchmark_diagnostic_counts, benchmark_status_line};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

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

#[test]
fn infrastructure_event_counts_once_and_suppresses_benchmark_diagnostic_counts() {
    let mut messages = CompilerMessages::from_error(
        CompilerError::file_error(Path::new("missing.moth"), "Path does not exist"),
        StringTable::new(),
    );
    messages.set_infrastructure_error(CompilerError::compiler_error("later invariant failure"));

    assert!(messages.has_errors());
    assert_eq!(messages.error_count(), 1);
    assert_eq!(benchmark_diagnostic_counts(&messages), None);
}

#[test]
fn normal_diagnostics_remain_available_to_benchmark_counts() {
    let mut strings = StringTable::new();
    let name = strings.intern("unknown");
    let messages = CompilerMessages::from_diagnostic(
        CompilerDiagnostic::unknown_value_name(name, None),
        strings,
    );
    assert_eq!(benchmark_diagnostic_counts(&messages), Some((1, 0)));
}
