//! Process runner - Executes the Moth compiler binary as a subprocess
//!
//! This module owns subprocess execution of the built `moth` binary.
//! It measures wall-clock time and captures stdout/stderr.
//!
//! # What this module owns
//! - Spawning `std::process::Command` for the moth binary
//! - Measuring subprocess wall-clock duration
//! - Capturing stdout and stderr output
//!
//! # What this module does NOT own
//! - Building the compiler binary (see `compiler_binary.rs`)
//! - Parsing benchmark protocol output (see `benchmark_status.rs` and `bench_observations.rs`)
//! - Orchestration of preflight and measured iterations (see `bench.rs`)

use std::path::Path;
use std::process::Command;
use std::time::Instant;

/// Process outcome retained for benchmark failure reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessStatus {
    /// Whether the command completed successfully.
    pub success: bool,
    /// Platform exit code, or `None` when the process ended without one.
    pub code: Option<i32>,
}

/// Result of executing a single subprocess run.
///
/// Contains timing data, success status, and captured output.
#[derive(Debug, Clone)]
pub struct ProcessRun {
    /// Duration in milliseconds
    pub duration_ms: f64,
    /// Typed subprocess outcome.
    pub status: ProcessStatus,
    /// Captured stderr for diagnostic output on failure
    pub stderr: String,
    /// Captured stdout for detailed timer parsing
    pub stdout: String,
}

/// Run a moth compiler command as a timed subprocess
///
/// Spawns the moth binary, measures wall-clock time, and captures output.
///
/// # Arguments
///
/// * `moth_path` - Path to the moth binary (e.g., target/release/moth)
/// * `current_directory` - Canonical repository root for resolving manifest paths
/// * `command` - The subcommand to execute (e.g., "check", "build")
/// * `args` - Arguments to pass to the subcommand
///
/// # Returns
///
/// A `ProcessRun` with timing and output data, or an error message.
pub fn run_moth_command(
    moth_path: &Path,
    current_directory: &Path,
    command: &str,
    args: &[String],
) -> Result<ProcessRun, String> {
    let start = Instant::now();

    let output = Command::new(moth_path)
        .current_dir(current_directory)
        .arg(command)
        .args(args)
        // Stable machine-readable timing lines for benchmark parsing.
        // The subprocess is built with the concise `timers` feature, so
        // MOTH_TIMERS=bench emits MOTH_BENCH timing lines without verbose
        // human prose. MOTH_COUNTERS=off suppresses counter output so
        // normal benchmark runs stay low-noise. MOTH_BENCH_STATUS=1 requests
        // the exact diagnostic-count record checked by benchmark execution.
        .env("MOTH_TIMERS", "bench")
        .env("MOTH_COUNTERS", "off")
        .env("MOTH_BENCH_STATUS", "1")
        .output()
        .map_err(|e| {
            format!(
                "Failed to execute moth binary at '{}' from '{}': {}",
                moth_path.display(),
                current_directory.display(),
                e
            )
        })?;

    let elapsed = start.elapsed();
    let duration_ms = elapsed.as_secs_f64() * 1000.0;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let status = ProcessStatus {
        success: output.status.success(),
        code: output.status.code(),
    };

    Ok(ProcessRun {
        duration_ms,
        status,
        stderr,
        stdout,
    })
}

#[cfg(test)]
mod tests;
