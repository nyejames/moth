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
    get_commit_hash, read_local_runs, thread_identity_suffix, to_case_results, to_local_record,
};
use crate::bench_observations::average_observations;
use crate::bench_summary::update_monthly_summary;
use crate::bench_system::{SystemIdentityMode, load_or_create_system};
use crate::bench_time::BenchmarkTimestamp;
use crate::bench_types::{
    BenchmarkCaseObservations, BenchmarkCaseResult, BenchmarkChangeKind, BenchmarkComparison,
    BenchmarkMetric, BenchmarkRun, BenchmarkSuiteKind, BenchmarkThresholds, SuiteStats,
    calculate_group_stats, calculate_mean, calculate_median, calculate_stage_movement,
    calculate_stddev, format_stage_movement_line, format_top_current_stages,
};
use crate::case_parser::{BenchmarkCase, parse_cases};

const FRONTEND_CASES_PATH: &str = "benchmarks/frontend-cases.txt";

/// Distinguishes whether a frontend benchmark run should record results or run read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendBenchMode {
    /// Record results to local history and tracked summaries.
    Record,
    /// Run read-only without writing benchmark history.
    Check,
}

/// Frontend benchmark execution options.
#[derive(Debug, Clone)]
pub struct FrontendBenchOptions {
    /// Number of warmup runs (not recorded).
    pub warmup_runs: usize,
    /// Number of measured iterations (recorded).
    pub measured_iterations: usize,
    /// Whether this run should record results or stay read-only.
    pub mode: FrontendBenchMode,
}

/// Run the complete frontend benchmark suite.
///
/// Orchestrates the benchmark workflow:
/// 1. Parse frontend benchmark cases
/// 2. Execute benchmarks with warmup and measurement
/// 3. Calculate statistics through named domain types
/// 4. Print result line with suite stats and comparison
///
/// Warmup failures and measured iteration failures are treated as hard
/// failures that abort the entire run without writing any data.
///
/// # Arguments
///
/// * `options` - Frontend benchmark execution options (warmup, iterations, mode)
///
/// # Returns
///
/// Ok(()) on success, or an error message on failure.
pub fn run_frontend_benchmarks(options: FrontendBenchOptions) -> Result<(), String> {
    let cases = load_frontend_cases()?;

    println!(
        "Running {} frontend benchmark cases: {} warmup + {} measured",
        cases.len(),
        options.warmup_runs,
        options.measured_iterations
    );

    let thread_count = effective_thread_count()?;

    let case_results = run_frontend_cases(&cases, &options)?;

    let groups = calculate_group_stats(&case_results);
    let suite = SuiteStats::from_case_results(&case_results);
    let timestamp = BenchmarkTimestamp::now();

    let system = match load_or_create_system(system_identity_mode(options.mode))? {
        Some(sys) => sys,
        None => {
            println!(
                "Result: frontend avg ~{:.0}ms, case spread ~{:.0}ms{}",
                suite.average_ms,
                suite.case_spread_ms,
                thread_identity_suffix(thread_count)
            );
            if let Some(top_stages) = format_top_current_stages(&case_results) {
                println!("{}", top_stages);
            }
            println!("No local baseline found. Run 'just bench-frontend' to create one.");
            return Ok(());
        }
    };

    let previous_cases = load_previous_cases_for_system(&system.system_uuid, thread_count)?;

    let comparison = match &previous_cases {
        Some(cases) => BenchmarkComparison::new(&case_results, Some(cases)),
        None => BenchmarkComparison::new(&case_results, None),
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
            if let Some(top_stages) = format_top_current_stages(&case_results) {
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

    // Verify group count consistency before moving values to the record path.
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

    if options.mode == FrontendBenchMode::Record {
        let run = BenchmarkRun {
            timestamp,
            commit: get_commit_hash(),
            system: system.clone(),
            suite_kind: BenchmarkSuiteKind::FrontendPhases,
            cases: case_results,
            groups,
            suite,
            warmup_runs: options.warmup_runs,
            measured_iterations: options.measured_iterations,
            thread_count,
        };
        record_frontend_run(&run, &comparison)?;
    }

    Ok(())
}

fn system_identity_mode(mode: FrontendBenchMode) -> SystemIdentityMode {
    match mode {
        FrontendBenchMode::Record => SystemIdentityMode::CreateIfMissing,
        FrontendBenchMode::Check => SystemIdentityMode::ReadOnly,
    }
}

fn load_frontend_cases() -> Result<Vec<BenchmarkCase>, String> {
    let cases_path = PathBuf::from(FRONTEND_CASES_PATH);
    parse_cases(&cases_path)
}

fn run_frontend_cases(
    cases: &[BenchmarkCase],
    options: &FrontendBenchOptions,
) -> Result<Vec<BenchmarkCaseResult>, String> {
    let mut case_results = Vec::new();

    for case in cases {
        print!("{} ", case.name);

        run_frontend_warmups(case, options.warmup_runs)?;

        let (durations, observations) =
            run_frontend_measurements(case, options.measured_iterations)?;

        println!();

        let result = build_frontend_case_result(case, &durations, &observations);
        case_results.push(result);
    }

    Ok(case_results)
}

fn run_frontend_warmups(case: &BenchmarkCase, warmup_runs: usize) -> Result<(), String> {
    for _ in 0..warmup_runs {
        let report = run_one_frontend_case(case)?;

        if report.total_ms <= 0.0 {
            return Err(format!(
                "Warmup produced invalid timing for case '{}'",
                case.name
            ));
        }
    }

    Ok(())
}

fn run_frontend_measurements(
    case: &BenchmarkCase,
    measured_iterations: usize,
) -> Result<(Vec<f64>, Vec<BenchmarkCaseObservations>), String> {
    let mut durations = Vec::new();
    let mut observations = Vec::new();

    for _ in 0..measured_iterations {
        let report = run_one_frontend_case(case)?;
        durations.push(report.total_ms);
        observations.push(report_to_observations(&report));
        print!(".");
    }

    Ok((durations, observations))
}

fn run_one_frontend_case(case: &BenchmarkCase) -> Result<FrontendBenchmarkReport, String> {
    if case.args.len() != 1 {
        return Err(format!(
            "Frontend case '{}' must have exactly one path argument",
            case.name
        ));
    }

    let path = &case.args[0];

    let build_profile = match case.command.as_str() {
        "frontend" => FrontendBenchmarkBuildProfile::Dev,
        _ => {
            return Err(format!(
                "Unknown frontend benchmark command '{}' for case '{}'",
                case.command, case.name
            ));
        }
    };

    let options = FrontendBenchmarkOptions {
        entry_path: PathBuf::from(path),
        build_profile,
    };

    match run_frontend_benchmark(options) {
        Ok(report) => Ok(report),
        Err(error) => Err(format!(
            "Frontend benchmark failed for '{}': {}",
            case.name, error
        )),
    }
}

fn report_to_observations(report: &FrontendBenchmarkReport) -> BenchmarkCaseObservations {
    BenchmarkCaseObservations {
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
    }
}

fn build_frontend_case_result(
    case: &BenchmarkCase,
    durations: &[f64],
    observations: &[BenchmarkCaseObservations],
) -> BenchmarkCaseResult {
    let mean = calculate_mean(durations);
    let median = calculate_median(durations);
    let stddev = calculate_stddev(durations, mean);

    BenchmarkCaseResult {
        case_name: case.name.clone(),
        group_name: case.group_name.clone(),
        command: case.command.clone(),
        args: case.args.clone(),
        mean_ms: mean,
        median_ms: median,
        stddev_ms: stddev,
        observations: average_observations(observations),
    }
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
    let record = to_local_record(run, run.commit.clone());
    append_local_run(&runs_path, &record)?;

    update_monthly_summary(run, comparison)?;

    Ok(())
}

#[cfg(test)]
mod tests;
