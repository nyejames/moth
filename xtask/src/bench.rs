//! CLI benchmark suite entry wrapper.
//!
//! WHAT: owns only CLI-specific setup: prepared run, recording eligibility,
//! release compiler construction, CLI case selection and the CLI execution
//! context.
//! WHY: measurement, comparison, presentation and persistence have one shared
//! owner in `benchmark_suite.rs`; this file stays a thin entry wrapper.

use crate::bench_history::effective_thread_count;
use crate::bench_types::{BenchmarkRecording, BenchmarkRunPolicy, BenchmarkSuiteKind};
use crate::benchmark_execution::{
    BenchmarkExecutionContext, format_case_failures, preflight_cases,
};
use crate::benchmark_manifest::BenchmarkCase;
use crate::benchmark_repository::verify_after_operation;
use crate::benchmark_run::PreparedBenchmarkRun;
use crate::benchmark_suite::{finish_suite_run, measure_cases};
use crate::benchmark_workspace::{BenchmarkExecutionWorkspace, finalise_workspace};
use crate::compiler_binary::build_release_compiler_with_timers;

/// Run the complete end-to-end CLI benchmark suite.
///
/// Preflight failures and measured iteration failures abort the run without
/// writing any data. Explicit workspace finalisation precedes repository
/// verification and persistence.
pub(crate) fn run_benchmarks(policy: BenchmarkRunPolicy) -> Result<(), String> {
    let prepared = PreparedBenchmarkRun::load(policy.recording())?;

    println!("Building release compiler...");
    let compiler = build_release_compiler_with_timers(&prepared.manifest.repository_root)?;
    let thread_count = effective_thread_count()?;
    let cases: Vec<BenchmarkCase> = prepared
        .manifest
        .cli_cases()
        .filter(|case| policy.selects_case(case.quick))
        .cloned()
        .collect();
    let workspace = BenchmarkExecutionWorkspace::create(&prepared.manifest.repository_root)?;
    let context =
        BenchmarkExecutionContext::new(&prepared.manifest, compiler.as_path(), &workspace);

    println!(
        "Running {} benchmark cases: 1 shared preflight + {} measured",
        cases.len(),
        policy.measured_iterations()
    );

    let git_revision = prepared.snapshot.git_revision();

    let measured = (|| {
        preflight_cases(&context, &cases)
            .map_err(|failures| format_case_failures("preflight", &failures))?;
        println!("Shared CLI preflight passed; starting measurements.");
        measure_cases(&context, &prepared, &cases, policy.measured_iterations())
    })();

    let result = match measured {
        Ok(case_results) => {
            // Explicit finalisation precedes verification and persistence.
            finalise_workspace(&workspace, Ok(()))?;
            if policy.recording() == BenchmarkRecording::Record {
                prepared.verify_unchanged()?;
            }
            finish_suite_run(
                case_results,
                BenchmarkSuiteKind::EndToEndCli,
                thread_count,
                policy,
                &git_revision,
                prepared.paths(),
            )
        }
        Err(operation) => finalise_workspace(&workspace, Err(operation)),
    };

    if policy.recording() == BenchmarkRecording::Record {
        result
    } else {
        verify_after_operation(
            &prepared.snapshot,
            &prepared.manifest.repository_root,
            result,
        )
    }
}
