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
//! - `bench-report`         - Print a local-only benchmark drilldown report
//! - `bench-frontend-check` - Run the focused frontend benchmark suite without writing history
//! - `bench-frontend`       - Run the focused frontend benchmark suite and record
//! - `bench-profile`        - Run Samply-backed profiling on benchmark cases

mod bench;
mod bench_history;
mod bench_migration;
mod bench_observations;
mod bench_report;
mod bench_summary;
mod bench_system;
mod bench_time;
mod bench_types;
mod bench_validate;
mod case_parser;
mod compiler_binary;
mod frontend_bench;
mod mode;
mod process_runner;
mod profile;

use bench::{BenchMode, BenchOptions, run_benchmarks};
use bench_report::run_benchmark_report;
use bench_validate::validate_all_benchmarks;
use frontend_bench::{FrontendBenchMode, FrontendBenchOptions, run_frontend_benchmarks};
use mode::{BenchmarkMode, ModeParseResult};
use std::env;
use std::process;

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
            let options = BenchOptions {
                warmup_runs: 1,
                measured_iterations: 10,
                mode: BenchMode::Record,
            };

            exit_with_result(run_benchmarks(options));
        }
        BenchmarkMode::BenchCheck => {
            let options = BenchOptions {
                warmup_runs: 1,
                measured_iterations: 10,
                mode: BenchMode::Check,
            };

            exit_with_result(run_benchmarks(options));
        }
        BenchmarkMode::BenchReport => {
            exit_with_result(run_benchmark_report());
        }
        BenchmarkMode::BenchFrontendCheck => {
            let options = FrontendBenchOptions {
                warmup_runs: 1,
                measured_iterations: 10,
                mode: FrontendBenchMode::Check,
            };

            exit_with_result(run_frontend_benchmarks(options));
        }
        BenchmarkMode::BenchFrontend => {
            let options = FrontendBenchOptions {
                warmup_runs: 1,
                measured_iterations: 10,
                mode: FrontendBenchMode::Record,
            };

            exit_with_result(run_frontend_benchmarks(options));
        }
        BenchmarkMode::BenchProfile(options) => {
            exit_with_result(profile::run_profile_benchmarks(options));
        }
        BenchmarkMode::BenchValidate => {
            exit_with_result(validate_all_benchmarks());
        }
    }
}

/// Print the top-level usage message listing all supported modes.
fn print_usage() {
    eprintln!("Usage: xtask <mode> [options]");
    eprintln!();
    eprintln!("Modes:");
    eprintln!(
        "  bench                Run the full benchmark suite and update local/public summaries"
    );
    eprintln!(
        "  bench-check          Run the full benchmark suite without writing benchmark history"
    );
    eprintln!("  bench-report         Print a local-only benchmark drilldown report");
    eprintln!(
        "  bench-frontend-check Run the focused frontend benchmark suite without writing history"
    );
    eprintln!("  bench-frontend       Run the focused frontend benchmark suite and record");
    eprintln!("  bench-validate       Validate all benchmark cases compile without errors");
    eprintln!("  bench-profile        Run Samply-backed profiling (use --help for options)");
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
