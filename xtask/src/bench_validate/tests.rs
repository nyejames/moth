use std::path::PathBuf;

use crate::benchmark_execution::format_case_failures;
use crate::benchmark_execution::{BenchmarkCaseFailure, BenchmarkFailureKind};
use crate::benchmark_manifest::{BenchmarkRunner, CliBenchmarkCommand, FrontendBenchmarkProfile};

#[test]
fn preflight_failure_report_prints_every_failure_in_manifest_order() {
    let failures = vec![
        failure(
            "first_case",
            BenchmarkRunner::Cli {
                command: CliBenchmarkCommand::Check,
                args: Vec::new(),
            },
        ),
        failure(
            "second_case",
            BenchmarkRunner::Frontend {
                profile: FrontendBenchmarkProfile::Dev,
            },
        ),
    ];

    let report = format_case_failures("preflight", &failures);
    let first_position = report.find("first_case").expect("first case should render");
    let second_position = report
        .find("second_case")
        .expect("second case should render");

    assert!(report.starts_with("2 benchmark case(s) failed preflight:"));
    assert!(first_position < second_position);
    assert_eq!(report.matches("failure:").count(), 2);
}

fn failure(case_id: &str, runner: BenchmarkRunner) -> BenchmarkCaseFailure {
    BenchmarkCaseFailure {
        case_id: case_id.to_owned(),
        workload_id: format!("{case_id}_workload"),
        runner,
        entry: PathBuf::from(format!("benchmarks/{case_id}.moth")),
        kind: BenchmarkFailureKind::NonZeroProcessStatus,
        exit_code: Some(1),
        benchmark_status: None,
        stdout_evidence: None,
        stderr_evidence: None,
    }
}
