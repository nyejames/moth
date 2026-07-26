use super::{BenchmarkDiagnosticStatus, BenchmarkStatusError};

fn parse_benchmark_status(output: &str) -> Result<BenchmarkDiagnosticStatus, BenchmarkStatusError> {
    BenchmarkDiagnosticStatus::try_from(output)
}

#[test]
fn parses_clean_record() {
    assert_eq!(
        parse_benchmark_status("MOTH_BENCH status errors=0 warnings=0"),
        Ok(BenchmarkDiagnosticStatus {
            error_count: 0,
            warning_count: 0,
        })
    );
}

#[test]
fn parses_warning_record() {
    assert_eq!(
        parse_benchmark_status("MOTH_BENCH status errors=0 warnings=3"),
        Ok(BenchmarkDiagnosticStatus {
            error_count: 0,
            warning_count: 3,
        })
    );
}

#[test]
fn parses_error_record() {
    assert_eq!(
        parse_benchmark_status("MOTH_BENCH status errors=2 warnings=1"),
        Ok(BenchmarkDiagnosticStatus {
            error_count: 2,
            warning_count: 1,
        })
    );
}

#[test]
fn rejects_missing_record() {
    assert_eq!(
        parse_benchmark_status("ordinary compiler output"),
        Err(BenchmarkStatusError::Missing)
    );
}

#[test]
fn rejects_duplicate_record() {
    let output = "\
MOTH_BENCH status errors=0 warnings=0
MOTH_BENCH status errors=0 warnings=0
";

    assert_eq!(
        parse_benchmark_status(output),
        Err(BenchmarkStatusError::Duplicate { count: 2 })
    );
}

#[test]
fn rejects_malformed_prefix_record() {
    let line = "MOTH_BENCH statuserrors=0 warnings=0";

    assert_eq!(
        parse_benchmark_status(line),
        Err(BenchmarkStatusError::Malformed {
            line: line.to_owned(),
        })
    );
}

#[test]
fn rejects_unknown_field() {
    let line = "MOTH_BENCH status errors=0 notes=0";

    assert_eq!(
        parse_benchmark_status(line),
        Err(BenchmarkStatusError::Malformed {
            line: line.to_owned(),
        })
    );
}

#[test]
fn rejects_negative_value() {
    let line = "MOTH_BENCH status errors=-1 warnings=0";

    assert_eq!(
        parse_benchmark_status(line),
        Err(BenchmarkStatusError::Malformed {
            line: line.to_owned(),
        })
    );
}

#[test]
fn rejects_explicit_positive_sign() {
    let line = "MOTH_BENCH status errors=+1 warnings=0";

    assert_eq!(
        parse_benchmark_status(line),
        Err(BenchmarkStatusError::Malformed {
            line: line.to_owned(),
        })
    );
}

#[test]
fn rejects_overflow() {
    let overflow = format!("{}0", usize::MAX);
    let line = format!("MOTH_BENCH status errors={overflow} warnings=0");

    assert_eq!(
        parse_benchmark_status(&line),
        Err(BenchmarkStatusError::Malformed { line })
    );
}

#[test]
fn rejects_trailing_prose() {
    let line = "MOTH_BENCH status errors=0 warnings=0 complete";

    assert_eq!(
        parse_benchmark_status(line),
        Err(BenchmarkStatusError::Malformed {
            line: line.to_owned(),
        })
    );
}

#[test]
fn accepts_unrelated_surrounding_output() {
    let output = "\
Checking project
MOTH_BENCH timing command.check.total=12.5ms
MOTH_BENCH status errors=0 warnings=2
Finished
";

    assert_eq!(
        parse_benchmark_status(output),
        Ok(BenchmarkDiagnosticStatus {
            error_count: 0,
            warning_count: 2,
        })
    );
}
