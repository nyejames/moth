//! Repetition and thread-count stress lanes for the stateful test owners.
//!
//! WHAT: runs the Rust unit suite and the canonical integration suite repeatedly under one
//!      thread, default parallelism and a higher bounded thread count, then reports every lane's
//!      outcome.
//! WHY:  the process-global owners — the timing collector, the frontend counter stores, output
//!      writes, current-directory guards and the Node render harness — only misbehave under a
//!      thread schedule or a repeat that a single default-parallelism run never produces. A
//!      developer shell loop cannot be run by CI on every platform, so the repeat lanes have an
//!      owned command.
//!
//! # What this module owns
//! - The lane matrix (thread counts, repeat count, which suite each lane runs)
//! - Subprocess execution of `cargo test` and the integration runner for each lane
//! - Continuing through every lane and reporting the complete outcome table
//!
//! # What this module does NOT own
//! - The suites themselves, or their pass criteria
//! - Feature-lane coverage (see the feature matrix in the validation guide)

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Repeats used when the caller does not choose one.
pub const DEFAULT_STRESS_REPEATS: u32 = 3;

/// The higher bounded thread count CI uses.
const HIGH_THREAD_COUNT: u32 = 16;

/// Thread counts every lane family runs, in order.
///
/// `None` is default parallelism: the schedule a developer actually sees, kept beside the two
/// bounds so a failure that only appears there is still caught.
const THREAD_COUNTS: &[Option<u32>] = &[Some(1), None, Some(HIGH_THREAD_COUNT)];

/// Which suite a lane runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StressSuite {
    /// The workspace Rust unit and subsystem tests.
    Unit,
    /// The canonical integration cases run through `moth tests`.
    Integration,
}

impl StressSuite {
    const fn label(self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Integration => "integration",
        }
    }
}

/// One scheduled execution of one suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StressLane {
    suite: StressSuite,
    threads: Option<u32>,
    iteration: u32,
    repeats: u32,
}

impl fmt::Display for StressLane {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let threads = match self.threads {
            Some(count) => count.to_string(),
            None => "default".to_string(),
        };
        write!(
            formatter,
            "{} threads={} run {}/{}",
            self.suite.label(),
            threads,
            self.iteration,
            self.repeats
        )
    }
}

/// Why one lane failed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StressFailure {
    /// The lane's command could not be started.
    Launch(String),
    /// The lane ran and reported failure.
    Exit(Option<i32>),
}

impl fmt::Display for StressFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(error) => write!(formatter, "could not start: {error}"),
            Self::Exit(Some(code)) => write!(formatter, "exit code {code}"),
            Self::Exit(None) => formatter.write_str("terminated without an exit code"),
        }
    }
}

/// Run every stress lane and report the complete outcome table.
///
/// Lanes keep running after a failure: a stress run exists to show which schedules are unstable,
/// and stopping at the first one hides the rest of the matrix.
pub fn run_stress_matrix(repeats: u32) -> Result<(), String> {
    if repeats == 0 {
        return Err("stress repeats must be greater than 0".to_string());
    }

    let workspace_root = workspace_root()?;
    let mut failures: Vec<(StressLane, StressFailure)> = Vec::new();
    let mut executed = 0_u32;

    for lane in stress_lanes(repeats) {
        println!("\n=== stress lane: {lane} ===");
        executed += 1;
        if let Err(failure) = run_stress_lane(&workspace_root, lane) {
            println!("lane failed: {failure}");
            failures.push((lane, failure));
        }
    }

    println!("\n=== stress summary ===");
    println!("lanes run: {executed}");
    println!("lanes failed: {}", failures.len());
    if failures.is_empty() {
        return Ok(());
    }

    for (lane, failure) in &failures {
        println!("  {lane}: {failure}");
    }
    Err(format!(
        "{} of {executed} stress lanes failed",
        failures.len()
    ))
}

/// The complete lane matrix, in execution order.
fn stress_lanes(repeats: u32) -> Vec<StressLane> {
    let mut lanes = Vec::new();
    for suite in [StressSuite::Unit, StressSuite::Integration] {
        for threads in THREAD_COUNTS {
            for iteration in 1..=repeats {
                lanes.push(StressLane {
                    suite,
                    threads: *threads,
                    iteration,
                    repeats,
                });
            }
        }
    }
    lanes
}

/// Execute one lane, inheriting stdio so a failing lane shows its own output.
fn run_stress_lane(workspace_root: &Path, lane: StressLane) -> Result<(), StressFailure> {
    let mut command = match lane.suite {
        StressSuite::Unit => unit_suite_command(lane.threads),
        StressSuite::Integration => integration_suite_command(lane.threads),
    };

    let status = command
        .current_dir(workspace_root)
        .status()
        .map_err(|error| StressFailure::Launch(error.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(StressFailure::Exit(status.code()))
    }
}

/// `cargo test` for the whole workspace at one thread count.
fn unit_suite_command(threads: Option<u32>) -> Command {
    let mut command = Command::new("cargo");
    command
        .arg("test")
        .arg("--workspace")
        .arg("--quiet")
        .arg("--")
        .arg("--format")
        .arg("terse");
    if let Some(count) = threads {
        command.arg(format!("--test-threads={count}"));
    }
    command
}

/// The canonical integration suite at one runner thread count.
fn integration_suite_command(threads: Option<u32>) -> Command {
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--quiet")
        .arg("--")
        .arg("tests")
        .arg("--terse");
    match threads {
        // The runner reads its own thread count, so an unset variable means default parallelism.
        Some(count) => command.env("MOTH_TEST_THREADS", count.to_string()),
        None => command.env_remove("MOTH_TEST_THREADS"),
    };
    command
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask manifest has no parent directory".to_string())
}

#[cfg(test)]
mod tests;
