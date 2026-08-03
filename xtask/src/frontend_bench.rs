//! Frontend benchmark orchestration for xtask.
//!
//! WHAT: runs in-process frontend benchmarks through the compiler's public API.
//! WHY: avoids subprocess noise for focused compiler-stage checks.

use moth::benchmarking::{
    FrontendBenchmarkBuildProfile, FrontendBenchmarkOptions, FrontendBenchmarkReport,
    run_frontend_benchmark,
};
use std::path::PathBuf;

use crate::bench_history::{
    RUNS_JSONL_PATH, append_local_run, effective_thread_count, find_latest_matching_run,
    read_local_runs, thread_identity_suffix, to_case_results, to_local_record,
};
use crate::bench_observations::{BenchmarkObservationError, validate_frontend_observations};
use crate::bench_summary::update_monthly_summary;
use crate::bench_system::{SystemIdentityMode, load_or_create_system};
use crate::bench_time::BenchmarkTimestamp;
use crate::bench_types::{
    BENCHMARK_PROTOCOL_VERSION, BenchmarkCaseObservations, BenchmarkCaseResult,
    BenchmarkChangeKind, BenchmarkComparison, BenchmarkMetric, BenchmarkRecording, BenchmarkRun,
    BenchmarkRunPolicy, BenchmarkSelection, BenchmarkSuiteKind, BenchmarkThresholds, GitRevision,
    SuiteStats, calculate_group_stats, calculate_mean, calculate_median, calculate_stage_movement,
    calculate_stddev, format_stage_movement_line, format_top_current_stages,
};
use crate::benchmark_execution::{
    BenchmarkExecutionContext, average_case_observations, execute_case, run_preflighted_suite,
};
use crate::benchmark_manifest::{BenchmarkCase, BenchmarkManifest, FrontendBenchmarkProfile};
use crate::benchmark_repository::verify_after_operation;
use crate::benchmark_run::PreparedBenchmarkRun;
use crate::benchmark_workspace::BenchmarkExecutionWorkspace;
use std::num::NonZeroUsize;

/// Run the complete frontend benchmark suite.
///
/// Orchestrates the benchmark workflow:
/// 1. Parse frontend benchmark cases
/// 2. Preflight the full frontend selection once, then execute measurements
/// 3. Calculate statistics through named domain types
/// 4. Print result line with suite stats and comparison
///
/// Preflight failures and measured iteration failures are treated as hard
/// failures that abort the entire run without writing any data.
///
/// # Arguments
///
/// * `policy` - Typed measured-run selection, iteration and recording policy
///
/// # Returns
///
/// Ok(()) on success, or an error message on failure.
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

    let result = run_preflighted_suite(
        &context,
        &cases,
        || {
            println!("Shared frontend preflight passed; starting measurements.");
            run_frontend_cases(&context, &prepared, &cases, policy.measured_iterations())
        },
        |case_results| {
            if policy.recording() == BenchmarkRecording::Record {
                prepared.verify_unchanged()?;
            }
            complete_frontend_run(case_results, thread_count, policy, &git_revision)
        },
    );

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

fn complete_frontend_run(
    case_results: Vec<BenchmarkCaseResult>,
    thread_count: Option<u32>,
    policy: BenchmarkRunPolicy,
    git_revision: &GitRevision,
) -> Result<(), String> {
    if policy.recording() == BenchmarkRecording::ReadOnly {
        return present_read_only_frontend_run(&case_results, thread_count, policy.selection());
    }

    let presentation = present_frontend_run(
        &case_results,
        thread_count,
        policy.selection(),
        SystemIdentityMode::CreateIfMissing,
    )?
    .ok_or_else(|| "recording frontend benchmark run has no system identity".to_owned())?;

    let run = BenchmarkRun {
        timestamp: presentation.timestamp,
        benchmark_protocol_version: BENCHMARK_PROTOCOL_VERSION,
        git_revision: git_revision.clone(),
        system: presentation.system,
        suite_kind: BenchmarkSuiteKind::FrontendPhases,
        cases: case_results,
        groups: presentation.groups,
        suite: presentation.suite,
        warmup_runs: 1,
        measured_iterations: policy.measured_iterations().get(),
        thread_count,
    };
    record_frontend_run(&run, &presentation.comparison)
}

/// Present completed frontend measurements without entering persistence.
pub(crate) fn present_read_only_frontend_run(
    case_results: &[BenchmarkCaseResult],
    thread_count: Option<u32>,
    selection: BenchmarkSelection,
) -> Result<(), String> {
    present_frontend_run(
        case_results,
        thread_count,
        selection,
        SystemIdentityMode::ReadOnly,
    )
    .map(|_| ())
}

struct FrontendBenchmarkPresentation {
    timestamp: BenchmarkTimestamp,
    system: crate::bench_types::BenchmarkSystem,
    groups: Vec<crate::bench_types::BenchmarkGroupStats>,
    suite: SuiteStats,
    comparison: BenchmarkComparison,
}

fn present_frontend_run(
    case_results: &[BenchmarkCaseResult],
    thread_count: Option<u32>,
    selection: BenchmarkSelection,
    identity_mode: SystemIdentityMode,
) -> Result<Option<FrontendBenchmarkPresentation>, String> {
    let groups = calculate_group_stats(case_results);
    let suite = SuiteStats::from_case_results(case_results);
    let timestamp = BenchmarkTimestamp::now();

    let system = match load_or_create_system(identity_mode)? {
        Some(sys) => sys,
        None => {
            println!(
                "Result: frontend avg ~{:.0}ms, case spread ~{:.0}ms{}",
                suite.average_ms,
                suite.case_spread_ms,
                thread_identity_suffix(thread_count)
            );
            if let Some(top_stages) = format_top_current_stages(case_results) {
                println!("{}", top_stages);
            }
            println!("No local baseline found. Run 'just bench-frontend' to create one.");
            return Ok(None);
        }
    };

    let previous_cases = load_previous_cases_for_system(&system.system_uuid, thread_count)?;

    let comparison = match &previous_cases {
        Some(cases) if selection == BenchmarkSelection::Quick => {
            BenchmarkComparison::for_quick_subset(case_results, Some(cases))
        }
        Some(cases) => BenchmarkComparison::new(case_results, Some(cases)),
        None => BenchmarkComparison::new(case_results, None),
    };

    println!(
        "Result: {} ({}): {}{}",
        system.display_name,
        system.public_system_id,
        timestamp.format_run_header(),
        thread_identity_suffix(thread_count)
    );
    println!("{}", comparison.format_run_change_line());

    match comparison.change_kind {
        BenchmarkChangeKind::Baseline => {
            if let Some(top_stages) = format_top_current_stages(case_results) {
                println!("{}", top_stages);
            }
        }
        _ => {
            let movements = calculate_stage_movement(&comparison);
            if let Some(stage_line) =
                format_stage_movement_line(&movements, &BenchmarkThresholds::DEFAULT)
            {
                println!("{}", stage_line);
            }
        }
    }

    // Verify group count consistency before any recording caller consumes it.
    debug_assert_eq!(
        groups
            .iter()
            .map(|group_stats| group_stats.case_count)
            .sum::<usize>(),
        case_results.len()
    );
    debug_assert!(
        groups
            .iter()
            .all(|group_stats| group_stats.average_ms.is_finite())
    );
    debug_assert!(case_results.iter().all(|case| case.median_ms.is_finite()));

    Ok(Some(FrontendBenchmarkPresentation {
        timestamp,
        system,
        groups,
        suite,
        comparison,
    }))
}

pub(crate) fn run_frontend_cases(
    context: &BenchmarkExecutionContext<'_>,
    prepared: &PreparedBenchmarkRun,
    cases: &[BenchmarkCase],
    measured_iterations: NonZeroUsize,
) -> Result<Vec<BenchmarkCaseResult>, String> {
    let mut case_results = Vec::new();

    for case in cases {
        print!("{} ", case.id);

        let (durations, observations) =
            run_frontend_measurements(context, case, measured_iterations)?;

        println!();

        let result =
            build_frontend_case_result(context, prepared, case, &durations, &observations)?;
        case_results.push(result);
    }

    Ok(case_results)
}

fn run_frontend_measurements(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    measured_iterations: NonZeroUsize,
) -> Result<(Vec<f64>, Vec<BenchmarkCaseObservations>), String> {
    let mut durations = Vec::new();
    let mut observations = Vec::new();

    for _ in 0..measured_iterations.get() {
        let execution = match execute_case(context, case) {
            Ok(execution) => execution,
            Err(failure) => {
                println!();
                return Err(format!("Measured iteration failed:\n{failure}"));
            }
        };

        durations.push(execution.total_duration_ms);
        observations.push(execution.observations);
        print!(".");
    }

    Ok((durations, observations))
}

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

pub(crate) fn report_to_observations(
    report: &FrontendBenchmarkReport,
) -> Result<BenchmarkCaseObservations, BenchmarkObservationError> {
    validate_frontend_observations(BenchmarkCaseObservations {
        stage_timings: report
            .stages
            .iter()
            .map(|stage| BenchmarkMetric {
                name: stage.name.clone(),
                value: stage.duration_ms,
            })
            .collect(),
        counters: report
            .counters
            .iter()
            .map(|counter| BenchmarkMetric {
                name: counter.name.clone(),
                value: counter.value,
            })
            .collect(),
    })
}

fn build_frontend_case_result(
    context: &BenchmarkExecutionContext<'_>,
    prepared: &PreparedBenchmarkRun,
    case: &BenchmarkCase,
    durations: &[f64],
    observations: &[BenchmarkCaseObservations],
) -> Result<BenchmarkCaseResult, String> {
    let mean = calculate_mean(durations);
    let median = calculate_median(durations);
    let stddev = calculate_stddev(durations, mean);
    let observations = average_case_observations(context, case, observations)
        .map_err(|failure| format!("Measured observations failed:\n{failure}"))?;
    let identity = prepared
        .fingerprints
        .identity_for(&prepared.manifest, case)
        .map_err(|error| error.to_string())?;

    Ok(BenchmarkCaseResult {
        case_id: case.id.clone(),
        identity: Some(identity),
        group_name: case.group_name.clone(),
        runner: case.runner.clone(),
        mean_ms: mean,
        median_ms: median,
        stddev_ms: stddev,
        observations,
    })
}

/// Load the most recent previous frontend case results for the given system UUID and thread identity.
fn load_previous_cases_for_system(
    system_uuid: &str,
    thread_count: Option<u32>,
) -> Result<Option<Vec<BenchmarkCaseResult>>, String> {
    let runs_path = PathBuf::from(RUNS_JSONL_PATH);
    if !runs_path.exists() {
        return Ok(None);
    }

    let runs = read_local_runs(&runs_path)?;
    Ok(find_latest_matching_run(
        &runs,
        system_uuid,
        BenchmarkSuiteKind::FrontendPhases,
        thread_count,
    )
    .map(to_case_results))
}

/// Persist a completed frontend benchmark run to local history and update the tracked summary.
///
/// Appends the run to local raw history, then delegates tracked-summary
/// updates to `update_monthly_summary`, which owns the default-thread policy
/// and safely no-ops for fixed-thread runs.
fn record_frontend_run(run: &BenchmarkRun, comparison: &BenchmarkComparison) -> Result<(), String> {
    let runs_path = PathBuf::from(RUNS_JSONL_PATH);
    let record = to_local_record(run);
    append_local_run(&runs_path, &record)?;

    update_monthly_summary(run, comparison)?;

    Ok(())
}

#[cfg(test)]
mod tests;
