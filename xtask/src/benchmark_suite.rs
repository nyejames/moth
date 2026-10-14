//! Shared benchmark suite measurement, presentation and persistence.
//!
//! WHAT: one measured-iteration loop, one case-result builder, one previous-run
//! loader, one presentation path and one normal history/summary persistence
//! path, parameterised by `BenchmarkSuiteKind`.
//! WHY: CLI and frontend suites measure different work but must present,
//! compare and persist through exactly the same flow so no second comparison
//! or history implementation can drift.

use crate::bench_history::{
    append_local_run, find_latest_matching_run, read_local_runs, thread_identity_label,
    thread_identity_suffix, to_case_results, to_local_record,
};
use crate::bench_summary::update_monthly_summary;
use crate::bench_system::{SystemIdentityMode, load_or_create_system_at};
use crate::bench_time::BenchmarkTimestamp;
use crate::bench_types::{
    BENCHMARK_PROTOCOL_VERSION, BenchmarkCaseObservations, BenchmarkCaseResult,
    BenchmarkChangeKind, BenchmarkComparison, BenchmarkGroupStats, BenchmarkRecording,
    BenchmarkRun, BenchmarkRunPolicy, BenchmarkSelection, BenchmarkSuiteKind, BenchmarkSystem,
    BenchmarkThresholds, GitRevision, SuiteStats, calculate_group_stats, calculate_mean,
    calculate_median, calculate_stage_movement, calculate_stddev, format_stage_movement_line,
    format_top_current_stages,
};
use crate::benchmark_execution::{
    BenchmarkExecutionContext, average_case_observations, execute_case,
};
use crate::benchmark_manifest::BenchmarkCase;
use crate::benchmark_run::{BenchmarkPaths, PreparedBenchmarkRun};
use std::io::Write;
use std::num::NonZeroUsize;

const READ_ONLY_EVIDENCE_MARKER: &str = "MOTH_BENCH_EVIDENCE";

#[derive(serde::Serialize)]
struct ReadOnlyBenchmarkEvidence<'a> {
    evidence_schema: &'static str,
    benchmark_protocol_version: u32,
    timing_schema_version: u32,
    timestamp: String,
    month_key: String,
    commit: Option<&'a str>,
    git_dirty: Option<bool>,
    environment: ReadOnlyBenchmarkEnvironment<'a>,
    warmup_runs: usize,
    measured_iterations: usize,
    suite_kind: &'static str,
    primary_metric_name: &'static str,
    suite_average_ms: f64,
    suite_case_spread_ms: f64,
    thread_count: Option<u32>,
    thread_identity: String,
    groups: &'a [BenchmarkGroupStats],
    cases: &'a [BenchmarkCaseResult],
}

#[derive(serde::Serialize)]
struct ReadOnlyBenchmarkEnvironment<'a> {
    operating_system: &'static str,
    architecture: &'static str,
    system: Option<&'a BenchmarkSystem>,
}

fn write_read_only_evidence(
    writer: &mut impl Write,
    case_results: &[BenchmarkCaseResult],
    suite_kind: BenchmarkSuiteKind,
    thread_count: Option<u32>,
    measured_iterations: NonZeroUsize,
    git_revision: &GitRevision,
    presentation: &SuitePresentation,
) -> Result<(), String> {
    let timestamp = &presentation.timestamp;
    let evidence = ReadOnlyBenchmarkEvidence {
        evidence_schema: "moth_benchmark_read_only_v1",
        benchmark_protocol_version: BENCHMARK_PROTOCOL_VERSION,
        timing_schema_version: crate::bench_types::BENCHMARK_TIMING_SCHEMA_VERSION,
        timestamp: format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}",
            timestamp.year, timestamp.month, timestamp.day, timestamp.hour, timestamp.minute
        ),
        month_key: timestamp.month_key(),
        commit: git_revision.commit.as_deref(),
        git_dirty: git_revision.dirty,
        environment: ReadOnlyBenchmarkEnvironment {
            operating_system: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            system: presentation.system.as_ref(),
        },
        warmup_runs: 1,
        measured_iterations: measured_iterations.get(),
        suite_kind: suite_kind.persisted_name(),
        primary_metric_name: suite_kind.primary_metric_name(),
        suite_average_ms: presentation.suite.average_ms,
        suite_case_spread_ms: presentation.suite.case_spread_ms,
        thread_count,
        thread_identity: thread_identity_label(thread_count),
        groups: &presentation.groups,
        cases: case_results,
    };

    writer
        .write_all(READ_ONLY_EVIDENCE_MARKER.as_bytes())
        .map_err(|error| format!("failed to write read-only benchmark evidence marker: {error}"))?;
    writer.write_all(b" ").map_err(|error| {
        format!("failed to write read-only benchmark evidence separator: {error}")
    })?;
    serde_json::to_writer(&mut *writer, &evidence)
        .map_err(|error| format!("failed to write read-only benchmark evidence JSON: {error}"))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("failed to finish read-only benchmark evidence line: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("failed to flush read-only benchmark evidence: {error}"))
}

/// Measured case results plus the presentation facts needed to persist a run.
struct SuitePresentation {
    timestamp: BenchmarkTimestamp,
    system: Option<BenchmarkSystem>,
    groups: Vec<BenchmarkGroupStats>,
    suite: SuiteStats,
    comparison: Option<BenchmarkComparison>,
}

/// Measure every selected case with the shared loop and result builder.
///
/// Preflight must already have succeeded once; this runs the measured
/// iterations and constructs one `BenchmarkCaseResult` per case through the
/// shared identity helper.
pub(crate) fn measure_cases(
    context: &BenchmarkExecutionContext<'_>,
    prepared: &PreparedBenchmarkRun,
    cases: &[BenchmarkCase],
    measured_iterations: NonZeroUsize,
) -> Result<Vec<BenchmarkCaseResult>, String> {
    let mut case_results = Vec::new();

    for case in cases {
        print!("{} ", case.id);

        let (durations, observations) = measure_one_case(context, case, measured_iterations)?;

        println!();

        let result = build_case_result(context, prepared, case, &durations, &observations)?;
        case_results.push(result);
    }

    Ok(case_results)
}

/// Execute measured iterations for a single case, failing fast on error.
///
/// Returns the collected durations and raw observations.
fn measure_one_case(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    measured_iterations: NonZeroUsize,
) -> Result<(Vec<f64>, Vec<BenchmarkCaseObservations>), String> {
    let mut durations = Vec::new();
    let mut detailed_observations = Vec::new();

    for _ in 0..measured_iterations.get() {
        let execution = match execute_case(context, case) {
            Ok(execution) => execution,
            Err(failure) => {
                println!();
                return Err(format!("Measured iteration failed:\n{failure}"));
            }
        };

        durations.push(execution.total_duration_ms);
        detailed_observations.push(execution.observations);
        print!(".");
    }

    Ok((durations, detailed_observations))
}

/// Build a single `BenchmarkCaseResult` from durations and observations.
fn build_case_result(
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
        group_name: case.group_name.persistence_spelling().to_string(),
        runner: case.runner.clone(),
        mean_ms: mean,
        median_ms: median,
        stddev_ms: stddev,
        observations,
    })
}

/// Present and persist one completed suite run.
///
/// Full read-only runs verify the repository snapshot at successful
/// finalisation before emitting evidence. Recorded runs present with a created
/// system identity, then append local history and update the tracked summary.
pub(crate) fn finish_suite_run(
    case_results: Vec<BenchmarkCaseResult>,
    suite_kind: BenchmarkSuiteKind,
    thread_count: Option<u32>,
    policy: BenchmarkRunPolicy,
    git_revision: &GitRevision,
    prepared: &PreparedBenchmarkRun,
) -> Result<(), String> {
    if policy.recording() == BenchmarkRecording::ReadOnly {
        let presentation = present_run(
            &case_results,
            suite_kind,
            thread_count,
            policy.selection(),
            SystemIdentityMode::ReadOnly,
            prepared.paths(),
        )?;

        if policy.selection() == BenchmarkSelection::Full {
            prepared.verify_unchanged()?;

            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            write_read_only_evidence(
                &mut stdout,
                &case_results,
                suite_kind,
                thread_count,
                policy.measured_iterations(),
                git_revision,
                &presentation,
            )?;
        }

        return Ok(());
    }

    let presentation = present_run(
        &case_results,
        suite_kind,
        thread_count,
        policy.selection(),
        SystemIdentityMode::CreateIfMissing,
        prepared.paths(),
    )?;
    let SuitePresentation {
        timestamp,
        system,
        groups,
        suite,
        comparison,
    } = presentation;
    let system =
        system.ok_or_else(|| "recording benchmark run has no system identity".to_owned())?;
    let comparison =
        comparison.ok_or_else(|| "recording benchmark run has no comparison".to_owned())?;

    let run = BenchmarkRun {
        timestamp,
        benchmark_protocol_version: BENCHMARK_PROTOCOL_VERSION,
        git_revision: git_revision.clone(),
        system,
        suite_kind,
        cases: case_results,
        groups,
        suite,
        warmup_runs: 1,
        measured_iterations: policy.measured_iterations().get(),
        thread_count,
    };
    record_run(&run, &comparison, prepared.paths())
}

/// Present one completed suite without entering any persistence path.
pub(crate) fn present_read_only(
    case_results: &[BenchmarkCaseResult],
    suite_kind: BenchmarkSuiteKind,
    thread_count: Option<u32>,
    selection: BenchmarkSelection,
    paths: &BenchmarkPaths,
) -> Result<(), String> {
    present_run(
        case_results,
        suite_kind,
        thread_count,
        selection,
        SystemIdentityMode::ReadOnly,
        paths,
    )
    .map(|_| ())
}

fn present_run(
    case_results: &[BenchmarkCaseResult],
    suite_kind: BenchmarkSuiteKind,
    thread_count: Option<u32>,
    selection: BenchmarkSelection,
    identity_mode: SystemIdentityMode,
    paths: &BenchmarkPaths,
) -> Result<SuitePresentation, String> {
    let groups = calculate_group_stats(case_results);
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

    let suite = SuiteStats::from_case_results(case_results);
    let timestamp = BenchmarkTimestamp::now();

    let system = match load_or_create_system_at(&paths.system_toml, identity_mode)? {
        Some(system) => system,
        None => {
            println!(
                "Result: {} ~{:.0}ms, case spread ~{:.0}ms{}",
                suite_kind.read_only_avg_label(),
                suite.average_ms,
                suite.case_spread_ms,
                thread_identity_suffix(thread_count)
            );
            if let Some(top_stages) = format_top_current_stages(case_results) {
                println!("{}", top_stages);
            }
            println!(
                "No local baseline found. Run '{}' to create one.",
                suite_kind.record_command_hint()
            );
            return Ok(SuitePresentation {
                timestamp,
                system: None,
                groups,
                suite,
                comparison: None,
            });
        }
    };

    let previous_cases =
        load_previous_cases_for_system(&system.system_uuid, suite_kind, thread_count, paths)?;

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

    Ok(SuitePresentation {
        timestamp,
        system: Some(system),
        groups,
        suite,
        comparison: Some(comparison),
    })
}

/// Load the most recent previous case results for the given system and suite.
fn load_previous_cases_for_system(
    system_uuid: &str,
    suite_kind: BenchmarkSuiteKind,
    thread_count: Option<u32>,
    paths: &BenchmarkPaths,
) -> Result<Option<Vec<BenchmarkCaseResult>>, String> {
    if !paths.runs_jsonl.exists() {
        return Ok(None);
    }

    let runs = read_local_runs(&paths.runs_jsonl)?;
    Ok(find_latest_matching_run(&runs, system_uuid, suite_kind, thread_count).map(to_case_results))
}

/// Persist a completed benchmark run to local history and the tracked summary.
///
/// Appends the run to local raw history, then delegates tracked-summary
/// updates to `update_monthly_summary`, which owns the default-thread policy
/// and safely no-ops for fixed-thread runs.
fn record_run(
    run: &BenchmarkRun,
    comparison: &BenchmarkComparison,
    paths: &BenchmarkPaths,
) -> Result<(), String> {
    let record = to_local_record(run);
    append_local_run(&paths.runs_jsonl, &record)?;

    update_monthly_summary(run, comparison, paths)?;

    Ok(())
}

#[cfg(test)]
#[path = "benchmark_suite/tests.rs"]
mod tests;
