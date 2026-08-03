use crate::bench_types::BenchmarkGroup;
use std::cell::{Cell, RefCell};

use super::*;
use crate::benchmark_manifest::{
    BenchmarkExpectation, CliBenchmarkCommand, FrontendBenchmarkProfile,
};

#[test]
fn policy_is_quick_read_only_with_exactly_three_iterations() {
    let policy = bench_ci_policy().expect("fixed bench-ci policy should be valid");

    assert_eq!(policy.measured_iterations().get(), 3);
    assert_eq!(policy.selection(), BenchmarkSelection::Quick);
    assert_eq!(policy.recording(), BenchmarkRecording::ReadOnly);
}

#[test]
fn all_cases_preflight_before_only_quick_sections_measure() {
    let cases = cases();
    let events = RefCell::new(Vec::new());

    run_bench_ci_pipeline(
        &cases,
        bench_ci_policy().expect("policy should be valid"),
        |preflight_cases| {
            events
                .borrow_mut()
                .push(format!("preflight:{}", case_ids(preflight_cases).join(",")));
            Ok(())
        },
        |section, measured_cases, policy| {
            events.borrow_mut().push(format!(
                "measure:{}:{}:{}",
                section.heading,
                case_ids(measured_cases).join(","),
                policy.measured_iterations()
            ));
            Ok(case_ids(measured_cases).join(","))
        },
        |section, measured_ids| {
            events
                .borrow_mut()
                .push(format!("present:{}:{}", section.heading, measured_ids));
            Ok(())
        },
    )
    .expect("bench-ci pipeline should succeed");

    assert_eq!(
        events.into_inner(),
        [
            "preflight:slow_cli,quick_frontend,quick_cli,slow_frontend",
            "measure:CLI results:quick_cli:3",
            "present:CLI results:quick_cli",
            "measure:Frontend results:quick_frontend:3",
            "present:Frontend results:quick_frontend",
        ]
    );
}

#[test]
fn failed_preflight_prevents_measurement_and_presentation() {
    let cases = cases();
    let measurement_called = Cell::new(false);
    let presentation_called = Cell::new(false);

    let result = run_bench_ci_pipeline(
        &cases,
        bench_ci_policy().expect("policy should be valid"),
        |preflight_cases| {
            assert_eq!(preflight_cases.len(), 4);
            Err("preflight failed".to_owned())
        },
        |_, _, _| {
            measurement_called.set(true);
            Ok(())
        },
        |_, _| {
            presentation_called.set(true);
            Ok(())
        },
    );

    assert_eq!(result, Err("preflight failed".to_owned()));
    assert!(!measurement_called.get());
    assert!(!presentation_called.get());
}

#[test]
fn cli_and_frontend_sections_have_separate_stable_inputs() {
    let (cli_cases, frontend_cases) = select_quick_sections(&cases());

    assert_eq!(CLI_SECTION.heading, "CLI results");
    assert_eq!(CLI_SECTION.suite_kind, BenchmarkSuiteKind::EndToEndCli);
    assert_eq!(case_ids(&cli_cases), ["quick_cli"]);

    assert_eq!(FRONTEND_SECTION.heading, "Frontend results");
    assert_eq!(
        FRONTEND_SECTION.suite_kind,
        BenchmarkSuiteKind::FrontendPhases
    );
    assert_eq!(case_ids(&frontend_cases), ["quick_frontend"]);
}

fn cases() -> Vec<BenchmarkCase> {
    vec![
        cli_case("slow_cli", false),
        frontend_case("quick_frontend", true),
        cli_case("quick_cli", true),
        frontend_case("slow_frontend", false),
    ]
}

fn cli_case(id: &str, quick: bool) -> BenchmarkCase {
    BenchmarkCase {
        id: id.to_owned(),
        case_index: 0,
        workload_index: 0,
        group_name: BenchmarkGroup::Core,
        quick,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Check,
            args: Vec::new(),
        },
    }
}

fn frontend_case(id: &str, quick: bool) -> BenchmarkCase {
    BenchmarkCase {
        id: id.to_owned(),
        case_index: 0,
        workload_index: 0,
        group_name: BenchmarkGroup::Core,
        quick,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Frontend {
            profile: FrontendBenchmarkProfile::Dev,
        },
    }
}

fn case_ids(cases: &[BenchmarkCase]) -> Vec<&str> {
    cases.iter().map(|case| case.id.as_str()).collect()
}
