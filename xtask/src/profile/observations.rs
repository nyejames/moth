//! Profile observation logging
//!
//! WHAT: Runs warmup and observation passes for each benchmark case,
//! parses stable `MOTH_BENCH` stdout into `BenchmarkCaseObservations`,
//! and writes per-case artifacts (stdout/stderr logs, observations JSON,
//! summary markdown).
//!
//! WHY: The observation pass gives timer data, plus counters when explicitly
//! enabled by the profiling build and environment, without profiler
//! overhead. Separating observation from profiling (Samply) keeps each
//! concern independently testable and lets Phase 3 add Samply recording
//! beside the observation artifacts without changing this module.
//!
//! # What this module owns
//! - `ProfileObservation` struct wrapping per-case run data
//! - Warmup execution via `run_moth_command`
//! - Observation execution via `run_moth_command`
//! - Parsing observation stdout with `bench_observations::parse_stdout_observations`
//!
//! # What this module does NOT own
//! - Artifact directory layout (see `artifacts.rs`)
//! - Samply runner integration (see `runner.rs`)
//! - Profile JSON parsing or hotspot extraction (see `parse.rs`, `hotspots.rs`)
//! - Agent summaries and enriched per-case summaries (see `summary.rs`)

use crate::bench_observations::parse_stdout_observations;
use crate::bench_types::BenchmarkCaseObservations;
use crate::case_parser::BenchmarkCase;
use crate::process_runner::run_moth_command;
use std::path::Path;

/// Observation data collected from one benchmark case run.
///
/// WHAT: Wraps the case identity, command, wall time, parsed
/// observations, raw output, and output paths for a single case execution.
/// WHY: A named struct avoids tuple-heavy returns and makes the
/// data flow from observation to artifact writing explicit.
pub(crate) struct ProfileObservation {
    /// Case name from `BenchmarkCase.name`.
    pub(crate) case_name: String,
    /// Group name from `BenchmarkCase.group_name`.
    pub(crate) group_name: String,
    /// The command executed (e.g., "check", "build").
    pub(crate) command: String,
    /// Arguments passed to the command.
    pub(crate) command_args: Vec<String>,
    /// Wall-clock time in milliseconds for the observation pass.
    pub(crate) wall_ms: f64,
    /// Parsed stage timings and counters from compiler stdout.
    pub(crate) observations: BenchmarkCaseObservations,
    /// Raw stdout captured from the observation pass.
    pub(crate) stdout: String,
    /// Raw stderr captured from the observation pass.
    pub(crate) stderr: String,
}

/// Run one warmup pass for a case to prime caches and stabilize timing.
///
/// WHAT: Executes `run_moth_command` once and checks for success.
/// WHY: The first run often has cold-cache effects; warming up gives
/// the observation pass more stable measurements.
pub(crate) fn run_warmup(moth_path: &Path, case: &BenchmarkCase) -> Result<(), String> {
    let run = run_moth_command(moth_path, &case.command, &case.args)?;
    if !run.success {
        return Err(format!(
            "Warmup failed for case '{}': {}",
            case.name, run.stderr
        ));
    }
    Ok(())
}

/// Run one observation pass for a case, parse stdout, and collect artifacts.
///
/// WHAT: Executes `run_moth_command`, parses `MOTH_BENCH timing` and
/// `MOTH_BENCH counter` lines from stdout, and returns a `ProfileObservation`.
/// WHY: This is the measured pass that provides the timer/counter data
/// written beside Samply profiles. The wall time here is used for
/// hotspot estimation in later phases.
pub(crate) fn run_observation(
    moth_path: &Path,
    case: &BenchmarkCase,
) -> Result<ProfileObservation, String> {
    let run = run_moth_command(moth_path, &case.command, &case.args)?;
    if !run.success {
        return Err(format!(
            "Observation pass failed for case '{}': {}",
            case.name, run.stderr
        ));
    }

    let observations = parse_stdout_observations(&run.stdout);

    Ok(ProfileObservation {
        case_name: case.name.clone(),
        group_name: case.group_name.clone(),
        command: case.command.clone(),
        command_args: case.args.clone(),
        wall_ms: run.duration_ms,
        observations,
        stdout: run.stdout,
        stderr: run.stderr,
    })
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "observations_tests.rs"]
mod tests;
