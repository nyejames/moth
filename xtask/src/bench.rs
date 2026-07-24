//! Benchmark orchestration module - Coordinates benchmark execution
//!
//! This module orchestrates the benchmark workflow: building the compiler,
//! parsing benchmark cases, executing warmup and measured runs, and
//! calculating statistics through named domain types.
//!
//! # Ownership boundaries
//!
//! This file owns orchestration only. Other concerns belong in their
//! respective modules:
//! - Subprocess execution belongs in the process runner module (`process_runner.rs`).
//! - Compiler binary building belongs in `compiler_binary.rs`.
//! - Old result migration belongs in `bench_migration.rs`.
//! - Comparison logic belongs in benchmark domain/comparison modules (`bench_types.rs`).
//! - Summary rendering belongs in summary modules (`bench_summary.rs`).

use crate::bench_history::{
    RUNS_JSONL_PATH, append_local_run, effective_thread_count, find_latest_matching_run,
    get_commit_hash, read_local_runs, thread_identity_suffix, to_case_results, to_local_record,
};
use crate::bench_migration::migrate_old_results;
use crate::bench_observations::{average_observations, parse_stdout_observations};
use crate::bench_summary::update_monthly_summary;
use crate::bench_system::{SystemIdentityMode, load_or_create_system};
use crate::bench_time::BenchmarkTimestamp;
use crate::bench_types::{
    BenchmarkCaseObservations, BenchmarkCaseResult, BenchmarkChangeKind, BenchmarkComparison,
    BenchmarkRun, BenchmarkSuiteKind, BenchmarkThresholds, SuiteStats, calculate_group_stats,
    calculate_mean, calculate_median, calculate_stage_movement, calculate_stddev,
    format_stage_movement_line, format_top_current_stages,
};
use crate::case_parser::{BenchmarkCase, parse_cases};
use crate::compiler_binary::build_release_compiler_with_timers;
use crate::process_runner::run_moth_command;
use std::path::{Path, PathBuf};

const BENCHMARK_CASES_PATH: &str = "benchmarks/cases.txt";
const OLD_RESULTS_PATH: &str = "benchmarks/results";
const OLD_BENCHMARKS_DIR: &str = "benchmarks/old-benchmarks";

/// Distinguishes whether a benchmark run should record results or run read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchMode {
    /// Record results to local history and tracked summaries.
    Record,
    /// Run read-only without writing benchmark history.
    Check,
}

/// Benchmark execution options
#[derive(Debug, Clone)]
pub struct BenchOptions {
    /// Number of warmup runs (not recorded)
    pub warmup_runs: usize,
    /// Number of measured iterations (recorded)
    pub measured_iterations: usize,
    /// Whether this run should record results or stay read-only
    pub mode: BenchMode,
}

/// Run the complete benchmark suite
///
/// Orchestrates the benchmark workflow:
/// 1. Build the compiler
/// 2. Parse benchmark cases
/// 3. Execute benchmarks with warmup and measurement
/// 4. Calculate statistics through named domain types
/// 5. Print result line with suite stats and comparison
///
/// Warmup failures and measured iteration failures are treated as hard
/// failures that abort the entire run without writing any data.
///
/// # Arguments
///
/// * `options` - Benchmark execution options (warmup, iterations, mode)
///
/// # Returns
///
/// Ok(()) on success, or an error message on failure.
pub fn run_benchmarks(options: BenchOptions) -> Result<(), String> {
    println!("Building release compiler...");
    let compiler = build_release_compiler_with_timers()?;
    let moth_path = compiler.as_path();

    let thread_count = effective_thread_count()?;

    let cases = load_benchmark_cases()?;

    println!(
        "Running {} benchmark cases: {} warmup + {} measured",
        cases.len(),
        options.warmup_runs,
        options.measured_iterations
    );

    let case_results = run_benchmark_cases(moth_path, &cases, &options)?;

    let groups = calculate_group_stats(&case_results);
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

    let suite = SuiteStats::from_case_results(&case_results);
    let timestamp = BenchmarkTimestamp::now();

    let system = match load_or_create_system(system_identity_mode(options.mode))? {
        Some(sys) => sys,
        None => {
            println!(
                "Result: avg ~{:.0}ms, case spread ~{:.0}ms{}",
                suite.average_ms,
                suite.case_spread_ms,
                thread_identity_suffix(thread_count)
            );
            if let Some(top_stages) = format_top_current_stages(&case_results) {
                println!("{}", top_stages);
            }
            println!("No local baseline found. Run 'just bench' to create one.");
            return Ok(());
        }
    };

    let previous_cases = load_previous_cases_for_system(
        &system.system_uuid,
        BenchmarkSuiteKind::EndToEndCli,
        thread_count,
    )?;

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

    if options.mode == BenchMode::Record {
        let run = BenchmarkRun {
            timestamp,
            commit: get_commit_hash(),
            system: system.clone(),
            suite_kind: BenchmarkSuiteKind::EndToEndCli,
            cases: case_results,
            groups,
            suite,
            warmup_runs: options.warmup_runs,
            measured_iterations: options.measured_iterations,
            thread_count,
        };
        record_benchmark_run(&run, &comparison)?;
    }

    Ok(())
}

fn system_identity_mode(mode: BenchMode) -> SystemIdentityMode {
    match mode {
        BenchMode::Record => SystemIdentityMode::CreateIfMissing,
        BenchMode::Check => SystemIdentityMode::ReadOnly,
    }
}

/// Parse the benchmark cases file.
fn load_benchmark_cases() -> Result<Vec<BenchmarkCase>, String> {
    let cases_path = PathBuf::from(BENCHMARK_CASES_PATH);
    parse_cases(&cases_path)
}

/// Run all benchmark cases, returning per-case results.
fn run_benchmark_cases(
    moth_path: &Path,
    cases: &[BenchmarkCase],
    options: &BenchOptions,
) -> Result<Vec<BenchmarkCaseResult>, String> {
    let mut case_results = Vec::new();

    for case in cases {
        print!("{} ", case.name);

        run_case_warmups(moth_path, case, options.warmup_runs)?;

        let (durations, observations) =
            run_case_measurements(moth_path, case, options.measured_iterations)?;

        println!();

        let result = build_case_result(case, &durations, &observations);
        case_results.push(result);
    }

    Ok(case_results)
}

/// Execute warmup runs for a single case, failing fast on error.
fn run_case_warmups(
    moth_path: &Path,
    case: &BenchmarkCase,
    warmup_runs: usize,
) -> Result<(), String> {
    for _ in 0..warmup_runs {
        let run = run_moth_command(moth_path, &case.command, &case.args)?;
        if !run.success {
            println!();
            return Err(format!(
                "Warmup failed for case '{}': {}",
                case.name, run.stderr
            ));
        }
    }

    Ok(())
}

/// Execute measured iterations for a single case, failing fast on error.
///
/// Returns the collected durations and raw observations.
fn run_case_measurements(
    moth_path: &Path,
    case: &BenchmarkCase,
    measured_iterations: usize,
) -> Result<(Vec<f64>, Vec<BenchmarkCaseObservations>), String> {
    let mut durations = Vec::new();
    let mut detailed_observations = Vec::new();

    for _ in 0..measured_iterations {
        let run = run_moth_command(moth_path, &case.command, &case.args)?;
        if !run.success {
            println!();
            return Err(format!(
                "Measured iteration failed for case '{}': {}",
                case.name, run.stderr
            ));
        }
        durations.push(run.duration_ms);
        detailed_observations.push(parse_stdout_observations(&run.stdout));
        print!(".");
    }

    Ok((durations, detailed_observations))
}

/// Build a single `BenchmarkCaseResult` from durations and observations.
fn build_case_result(
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

/// Load the most recent previous case results for the given system UUID and thread identity.
fn load_previous_cases_for_system(
    system_uuid: &str,
    suite_kind: BenchmarkSuiteKind,
    thread_count: Option<u32>,
) -> Result<Option<Vec<BenchmarkCaseResult>>, String> {
    let runs_path = PathBuf::from(RUNS_JSONL_PATH);
    if !runs_path.exists() {
        return Ok(None);
    }

    let runs = read_local_runs(&runs_path)?;
    Ok(find_latest_matching_run(&runs, system_uuid, suite_kind, thread_count).map(to_case_results))
}

/// Persist a completed benchmark run to local history and update the tracked summary.
///
/// Appends the run to local raw history, then delegates tracked-summary
/// updates to `update_monthly_summary`, which owns the default-thread policy
/// and safely no-ops for fixed-thread runs.
fn record_benchmark_run(
    run: &BenchmarkRun,
    comparison: &BenchmarkComparison,
) -> Result<(), String> {
    migrate_old_results(Path::new(OLD_RESULTS_PATH), Path::new(OLD_BENCHMARKS_DIR));

    let runs_path = PathBuf::from(RUNS_JSONL_PATH);
    let record = to_local_record(run, run.commit.clone());
    append_local_run(&runs_path, &record)?;

    update_monthly_summary(run, comparison)?;

    Ok(())
}
