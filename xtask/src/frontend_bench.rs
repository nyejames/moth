//! Frontend benchmark suite entry wrapper.
//!
//! WHAT: owns only frontend-specific setup: prepared run, recording
//! eligibility, frontend case selection, the frontend execution context and
//! the public compiler-facing frontend adapter.
//! WHY: measurement, comparison, presentation and persistence have one shared
//! owner in `benchmark_suite.rs`; this file stays a thin entry wrapper.

use moth::benchmarking::{
    FrontendBenchmarkBuildProfile, FrontendBenchmarkOptions, FrontendBenchmarkReport,
    run_frontend_benchmark,
};

use crate::bench_history::effective_thread_count;
use crate::bench_observations::{BenchmarkObservationError, validate_frontend_observations};
use crate::bench_types::{BenchmarkRecording, BenchmarkRunPolicy, BenchmarkSuiteKind};
use crate::benchmark_execution::{
    BenchmarkExecutionContext, format_case_failures, preflight_cases,
};
use crate::benchmark_manifest::{BenchmarkCase, BenchmarkManifest, FrontendBenchmarkProfile};
use crate::benchmark_repository::verify_after_operation;
use crate::benchmark_run::PreparedBenchmarkRun;
use crate::benchmark_suite::{finish_suite_run, measure_cases};
use crate::benchmark_workspace::{BenchmarkExecutionWorkspace, finalise_workspace};

/// Run the complete in-process frontend benchmark suite.
///
/// Preflight failures and measured iteration failures abort the run without
/// writing any data. Explicit workspace finalisation precedes repository
/// verification and persistence.
pub(crate) fn run_frontend_benchmarks(policy: BenchmarkRunPolicy) -> Result<(), String> {
    let prepared = PreparedBenchmarkRun::load()?;

    // Recording requires an exactly clean, committed repository before any
    // fingerprint traversal, compiler construction or history read/write.
    prepared.require_recording_eligible(policy.recording())?;

    let cases: Vec<BenchmarkCase> = prepared
        .manifest
        .frontend_cases()
        .filter(|case| policy.selects_case(case.quick))
        .cloned()
        .collect();
    let workspace = BenchmarkExecutionWorkspace::create(&prepared.manifest.repository_root)?;
    let context = BenchmarkExecutionContext::frontend(&prepared.manifest, &workspace);

    println!(
        "Running {} frontend benchmark cases: 1 shared preflight + {} measured",
        cases.len(),
        policy.measured_iterations()
    );

    let thread_count = effective_thread_count()?;
    let git_revision = prepared.snapshot.git_revision();

    let measured = (|| {
        preflight_cases(&context, &cases)
            .map_err(|failures| format_case_failures("preflight", &failures))?;
        println!("Shared frontend preflight passed; starting measurements.");
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
                BenchmarkSuiteKind::FrontendPhases,
                thread_count,
                policy,
                &git_revision,
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

/// Run one frontend benchmark case through the public compiler API.
pub(crate) fn run_one_frontend_case(
    manifest: &BenchmarkManifest,
    case: &BenchmarkCase,
) -> Result<FrontendBenchmarkReport, String> {
    let invocation = manifest
        .frontend_invocation(case)
        .map_err(|error| error.to_string())?;
    let build_profile = match invocation.profile {
        FrontendBenchmarkProfile::Dev => FrontendBenchmarkBuildProfile::Dev,
    };
    let options = FrontendBenchmarkOptions {
        entry_path: invocation.entry,
        build_profile,
    };

    match run_frontend_benchmark(options) {
        Ok(report) => Ok(report),
        Err(error) => Err(format!(
            "Frontend benchmark failed for '{}': {}",
            case.id, error
        )),
    }
}

/// Convert one frontend report into the shared observation shape.
pub(crate) fn report_to_observations(
    report: &FrontendBenchmarkReport,
) -> Result<crate::bench_types::BenchmarkCaseObservations, BenchmarkObservationError> {
    validate_frontend_observations(crate::bench_types::BenchmarkCaseObservations {
        stage_timings: report
            .stages
            .iter()
            .map(|stage| crate::bench_types::BenchmarkMetric {
                name: stage.name.clone(),
                value: stage.duration_ms,
            })
            .collect(),
        counters: report
            .counters
            .iter()
            .map(|counter| crate::bench_types::BenchmarkMetric {
                name: counter.name.clone(),
                value: counter.value,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests;
