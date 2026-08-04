//! Bounded non-recording benchmark gate for normal development validation.
//!
//! WHAT: preflights the complete typed manifest once, then measures the quick
//! CLI and frontend subsets through their existing measurement owners.
//! WHY: validation needs complete correctness coverage without running or
//! recording both full ten-iteration performance suites.

use crate::bench_history::effective_thread_count;
use crate::bench_types::{
    BenchmarkCaseResult, BenchmarkRecording, BenchmarkRunPolicy, BenchmarkSelection,
    BenchmarkSuiteKind,
};
use crate::benchmark_execution::{
    BenchmarkExecutionContext, format_case_failures, preflight_cases,
};
use crate::benchmark_manifest::{BenchmarkCase, BenchmarkRunner};
use crate::benchmark_repository::verify_after_operation;
use crate::benchmark_run::PreparedBenchmarkRun;
use crate::benchmark_suite::{measure_cases, present_read_only};
use crate::benchmark_workspace::{BenchmarkExecutionWorkspace, finalise_workspace};
use crate::compiler_binary::build_release_compiler_with_timers;

const BENCH_CI_MEASURED_ITERATIONS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BenchCiSection {
    suite_kind: BenchmarkSuiteKind,
    heading: &'static str,
}

const CLI_SECTION: BenchCiSection = BenchCiSection {
    suite_kind: BenchmarkSuiteKind::EndToEndCli,
    heading: "CLI results",
};

const FRONTEND_SECTION: BenchCiSection = BenchCiSection {
    suite_kind: BenchmarkSuiteKind::FrontendPhases,
    heading: "Frontend results",
};

/// Run the complete bounded validation benchmark gate.
pub(crate) fn run_bench_ci() -> Result<(), String> {
    let prepared = PreparedBenchmarkRun::load(BenchmarkRecording::ReadOnly)?;

    println!("Building release compiler...");
    let compiler = build_release_compiler_with_timers(&prepared.manifest.repository_root)?;
    let workspace = BenchmarkExecutionWorkspace::create(&prepared.manifest.repository_root)?;
    let context =
        BenchmarkExecutionContext::new(&prepared.manifest, compiler.as_path(), &workspace);
    let thread_count = effective_thread_count()?;
    let policy = bench_ci_policy()?;

    println!(
        "Preflighting all {} benchmark cases before quick selection...",
        prepared.manifest.cases.len()
    );

    let result = run_bench_ci_pipeline(
        &prepared.manifest.cases,
        policy,
        |cases| {
            let executions = preflight_cases(&context, cases)
                .map_err(|failures| format_case_failures("preflight", &failures))?;
            println!(
                "All {} benchmark cases passed shared preflight.",
                executions.len()
            );
            Ok(())
        },
        |section, cases, policy| {
            println!("\n{}:", section.heading);
            println!(
                "Measuring {} quick cases with {} iterations each.",
                cases.len(),
                policy.measured_iterations()
            );

            measure_section(section, &context, &prepared, cases, policy)
        },
        |section, case_results| {
            present_section(
                section,
                &case_results,
                thread_count,
                BenchmarkSelection::Quick,
                prepared.paths(),
            )
        },
    );

    let result = finalise_workspace(&workspace, result);
    verify_after_operation(
        &prepared.snapshot,
        &prepared.manifest.repository_root,
        result,
    )
}

fn bench_ci_policy() -> Result<BenchmarkRunPolicy, String> {
    BenchmarkRunPolicy::new(
        BENCH_CI_MEASURED_ITERATIONS,
        BenchmarkSelection::Quick,
        BenchmarkRecording::ReadOnly,
    )
    .map_err(|error| error.to_string())
}

fn measure_section(
    _section: BenchCiSection,
    context: &BenchmarkExecutionContext<'_>,
    prepared: &PreparedBenchmarkRun,
    cases: &[BenchmarkCase],
    policy: BenchmarkRunPolicy,
) -> Result<Vec<BenchmarkCaseResult>, String> {
    measure_cases(context, prepared, cases, policy.measured_iterations())
}

fn present_section(
    section: BenchCiSection,
    case_results: &[BenchmarkCaseResult],
    thread_count: Option<u32>,
    selection: BenchmarkSelection,
    paths: &crate::benchmark_run::BenchmarkPaths,
) -> Result<(), String> {
    present_read_only(
        case_results,
        section.suite_kind,
        thread_count,
        selection,
        paths,
    )
}

/// Preserve the gate ordering: complete preflight first, then derive and run
/// the two quick result sections. The completion callback only presents data;
/// recording is absent from this orchestration boundary.
fn run_bench_ci_pipeline<T>(
    all_cases: &[BenchmarkCase],
    policy: BenchmarkRunPolicy,
    preflight: impl FnOnce(&[BenchmarkCase]) -> Result<(), String>,
    mut measure: impl FnMut(BenchCiSection, &[BenchmarkCase], BenchmarkRunPolicy) -> Result<T, String>,
    mut present: impl FnMut(BenchCiSection, T) -> Result<(), String>,
) -> Result<(), String> {
    preflight(all_cases)?;

    let (cli_cases, frontend_cases) = select_quick_sections(all_cases);
    let sections = [(CLI_SECTION, cli_cases), (FRONTEND_SECTION, frontend_cases)];

    for (section, cases) in sections {
        if cases.is_empty() {
            return Err(format!(
                "bench-ci requires at least one quick {} case",
                section.suite_kind.display_label()
            ));
        }

        let measurements = measure(section, &cases, policy)?;
        present(section, measurements)?;
    }

    Ok(())
}

fn select_quick_sections(all_cases: &[BenchmarkCase]) -> (Vec<BenchmarkCase>, Vec<BenchmarkCase>) {
    let mut cli_cases = Vec::new();
    let mut frontend_cases = Vec::new();

    for case in all_cases.iter().filter(|case| case.quick) {
        match case.runner {
            BenchmarkRunner::Cli { .. } => cli_cases.push(case.clone()),
            BenchmarkRunner::Frontend { .. } => frontend_cases.push(case.clone()),
        }
    }

    (cli_cases, frontend_cases)
}

#[cfg(test)]
mod tests;
