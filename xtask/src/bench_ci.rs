//! Bounded non-recording benchmark gate for normal development validation.
//!
//! WHAT: preflights the complete typed manifest once, then measures the quick
//! CLI and frontend subsets through their existing measurement owners.
//! WHY: validation needs complete correctness coverage without running or
//! recording both full ten-iteration performance suites.

use crate::bench::{present_read_only_benchmark_run, run_benchmark_cases};
use crate::bench_history::effective_thread_count;
use crate::bench_types::{
    BenchmarkCaseResult, BenchmarkRecording, BenchmarkRunPolicy, BenchmarkSelection,
    BenchmarkSuiteKind,
};
use crate::benchmark_execution::{
    BenchmarkExecutionContext, format_case_failures, preflight_cases,
};
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkManifest, BenchmarkRunner, load_benchmark_manifest,
};
use crate::benchmark_repository::{BenchmarkRepositorySnapshot, verify_after_operation};
use crate::benchmark_workspace::BenchmarkExecutionWorkspace;
use crate::compiler_binary::build_release_compiler_with_timers;
use crate::frontend_bench::{present_read_only_frontend_run, run_frontend_cases};
use crate::workload_fingerprint::{WorkloadFingerprint, compute_workload_fingerprints};

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
    let manifest = load_benchmark_manifest().map_err(|error| error.to_string())?;

    // Capture repository state before any compiler construction or preflight.
    let snapshot = BenchmarkRepositorySnapshot::capture(&manifest.repository_root)
        .map_err(|error| error.to_string())?;

    println!("Building release compiler...");
    let compiler = build_release_compiler_with_timers(&manifest.repository_root)?;
    let workload_fingerprints =
        compute_workload_fingerprints(&manifest).map_err(|error| error.to_string())?;
    let workspace = BenchmarkExecutionWorkspace::create(&manifest.repository_root)?;
    let context = BenchmarkExecutionContext::new(&manifest, compiler.as_path(), &workspace);
    let thread_count = effective_thread_count()?;
    let policy = bench_ci_policy()?;

    println!(
        "Preflighting all {} benchmark cases before quick selection...",
        manifest.cases.len()
    );

    let result = run_bench_ci_pipeline(
        &manifest.cases,
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

            measure_section(
                section,
                &context,
                &manifest,
                &workload_fingerprints,
                cases,
                policy,
            )
        },
        |section, case_results| {
            present_section(
                section,
                &case_results,
                thread_count,
                BenchmarkSelection::Quick,
            )
        },
    );

    verify_after_operation(&snapshot, &manifest.repository_root, result)
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
    section: BenchCiSection,
    context: &BenchmarkExecutionContext<'_>,
    manifest: &BenchmarkManifest,
    workload_fingerprints: &[WorkloadFingerprint],
    cases: &[BenchmarkCase],
    policy: BenchmarkRunPolicy,
) -> Result<Vec<BenchmarkCaseResult>, String> {
    match section.suite_kind {
        BenchmarkSuiteKind::EndToEndCli => run_benchmark_cases(
            context,
            manifest,
            workload_fingerprints,
            cases,
            policy.measured_iterations(),
        ),
        BenchmarkSuiteKind::FrontendPhases => run_frontend_cases(
            context,
            manifest,
            workload_fingerprints,
            cases,
            policy.measured_iterations(),
        ),
    }
}

fn present_section(
    section: BenchCiSection,
    case_results: &[BenchmarkCaseResult],
    thread_count: Option<u32>,
    selection: BenchmarkSelection,
) -> Result<(), String> {
    match section.suite_kind {
        BenchmarkSuiteKind::EndToEndCli => {
            present_read_only_benchmark_run(case_results, thread_count, selection)
        }
        BenchmarkSuiteKind::FrontendPhases => {
            present_read_only_frontend_run(case_results, thread_count, selection)
        }
    }
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
