//! xtask - Benchmark orchestration tool for Moth compiler
//!
//! This crate provides build automation and benchmark tooling for the Moth
//! compiler project. It is a workspace member that runs benchmarks and generates
//! timing reports.
//!
//! # Usage
//!
//! ```text
//! cargo run --package xtask --bin xtask -- <mode> [options]
//! ```
//!
//! Modes:
//! - `bench`                - Run the full benchmark suite and update local/public summaries
//! - `bench-check`          - Run the full benchmark suite without writing benchmark history
//! - `bench-ci`             - Preflight all cases, then measure the quick subset read-only
//! - `bench-report`         - Print a local-only benchmark drilldown report
//! - `bench-frontend-check` - Run the focused frontend benchmark suite without writing history
//! - `bench-frontend`       - Run the focused frontend benchmark suite and record
//! - `bench-validate`       - Preflight every benchmark case without measurements
//! - `bench-profile`        - Run Samply-backed profiling on benchmark cases
//! - `stress`               - Repeat the unit and integration suites across thread counts

mod bench;
mod bench_ci;
mod bench_history;
mod bench_observations;
mod bench_report;
mod bench_summary;
mod bench_system;
mod bench_time;
mod bench_types;
mod bench_validate;
mod benchmark_execution;
mod benchmark_fingerprint;
mod benchmark_manifest;
mod benchmark_repository;
mod benchmark_run;
mod benchmark_status;
mod benchmark_suite;
mod benchmark_workspace;
mod compiler_binary;
mod frontend_bench;
mod mode;
mod process_runner;
mod profile;
mod stress;
#[cfg(test)]
mod test_fs;
mod timers_erasure_check;

use bench::run_benchmarks;
use bench_ci::run_bench_ci;
use bench_report::run_benchmark_report;
use bench_types::{BenchmarkRecording, BenchmarkRunPolicy, BenchmarkSelection};
use bench_validate::validate_all_benchmarks;
use frontend_bench::run_frontend_benchmarks;
use mode::{BenchmarkMode, ModeParseResult, TOP_LEVEL_USAGE};
use std::env;
use std::process;
use stress::run_stress_matrix;
use timers_erasure_check::run_timers_erasure_check;

fn main() {
    let args: Vec<String> = env::args().collect();

    // The binary name is args[0]; mode and options follow.
    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    // parse_args receives everything after the binary name.
    let mode = match BenchmarkMode::parse_args(&args[1..]) {
        ModeParseResult::Mode(mode) => mode,
        ModeParseResult::ProfileHelp(help) => {
            println!("{}", help);
            process::exit(0);
        }
        ModeParseResult::Error(error) => {
            eprintln!("Error: {}", error);
            eprintln!();
            print_usage();
            process::exit(1);
        }
    };

    match mode {
        BenchmarkMode::Bench => {
            exit_with_result(full_run_policy(BenchmarkRecording::Record).and_then(run_benchmarks));
        }
        BenchmarkMode::BenchCheck => {
            exit_with_result(
                full_run_policy(BenchmarkRecording::ReadOnly).and_then(run_benchmarks),
            );
        }
        BenchmarkMode::BenchCi => {
            exit_with_result(run_bench_ci());
        }
        BenchmarkMode::BenchReport => {
            exit_with_result(run_benchmark_report());
        }
        BenchmarkMode::BenchFrontendCheck => {
            exit_with_result(
                full_run_policy(BenchmarkRecording::ReadOnly).and_then(run_frontend_benchmarks),
            );
        }
        BenchmarkMode::BenchFrontend => {
            exit_with_result(
                full_run_policy(BenchmarkRecording::Record).and_then(run_frontend_benchmarks),
            );
        }
        BenchmarkMode::BenchProfile(options) => {
            exit_with_result(profile::run_profile_benchmarks(options));
        }
        BenchmarkMode::BenchValidate => {
            exit_with_result(validate_all_benchmarks());
        }
        BenchmarkMode::Stress { repeats } => {
            exit_with_result(run_stress_matrix(repeats));
        }
        BenchmarkMode::TimersErasureCheck => {
            exit_with_result(run_timers_erasure_check());
        }
    }
}

/// Print the top-level usage message listing all supported modes.
fn print_usage() {
    eprintln!("{TOP_LEVEL_USAGE}");
}

fn full_run_policy(recording: BenchmarkRecording) -> Result<BenchmarkRunPolicy, String> {
    BenchmarkRunPolicy::new(10, BenchmarkSelection::Full, recording)
        .map_err(|error| error.to_string())
}

fn exit_with_result(result: Result<(), String>) -> ! {
    match result {
        Ok(()) => process::exit(0),
        Err(error) => {
            eprintln!("Error: {}", error);
            process::exit(1);
        }
    }
}
