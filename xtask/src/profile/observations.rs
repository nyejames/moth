//! Profile observation logging
//!
//! WHAT: Runs the non-profiled observation pass for each benchmark case
//! through the shared benchmark executor and retains its validated timing
//! observations plus raw process channels.
//!
//! WHY: The observation pass gives timer data, plus counters when explicitly
//! enabled by the profiling build and environment, without profiler
//! overhead. Separating observation from profiling (Samply) keeps each
//! concern independently testable and lets Samply recording sit beside the
//! observation artifacts without changing this module.
//!
//! # What this module owns
//! - `ProfileObservation` struct wrapping per-case run data
//! - Adapting shared execution results into profiling observation data
//!
//! # What this module does NOT own
//! - Artifact directory layout (see `artifacts.rs`)
//! - Samply runner integration (see `runner.rs`)
//! - Profile JSON parsing or hotspot extraction (see `parse.rs`, `hotspots.rs`)
//! - Agent summaries and enriched per-case summaries (see `summary.rs`)

use crate::bench_types::BenchmarkCaseObservations;
use crate::command_line::{RecordedWorkingDirectory, posix_shell_command_line};
use crate::process_runner::BENCHMARK_ENVIRONMENT;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// Reject non-finite profile-owned numeric data before serialization.
///
/// Profile JSON writers must fail on NaN or infinite values instead of
/// emitting invalid machine-readable artifacts.
pub(crate) fn require_finite(value: f64, subject: &str) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("{subject} must be finite, got {value}"))
    }
}
use crate::benchmark_execution::{BenchmarkCaseExecution, BenchmarkExecutionContext, execute_case};
use crate::benchmark_manifest::BenchmarkCase;

/// Observation data collected from one benchmark case run.
///
/// WHAT: Wraps the case identity, command, wall time, parsed
/// observations, raw output, and output paths for a single case execution.
/// WHY: A named struct avoids tuple-heavy returns and makes the
/// data flow from observation to artifact writing explicit.
pub(crate) struct ProfileObservation {
    /// Authored case ID from the typed benchmark manifest.
    pub(crate) case_id: String,
    /// Group name from the typed benchmark manifest.
    pub(crate) group_name: String,
    /// The command executed (e.g., "check", "build").
    pub(crate) command: String,
    /// Native arguments passed to the command.
    pub(crate) command_args: Vec<OsString>,
    /// Path to the profiling compiler binary that executed the case.
    pub(crate) moth_path: PathBuf,
    /// Working directory the case ran from.
    pub(crate) current_directory: PathBuf,
    /// Wall-clock time in milliseconds for the observation pass.
    pub(crate) wall_ms: f64,
    /// Parsed stage timings and counters from compiler stdout.
    pub(crate) observations: BenchmarkCaseObservations,
    /// Raw stdout captured from the observation pass.
    pub(crate) stdout: String,
    /// Raw stderr captured from the observation pass.
    pub(crate) stderr: String,
}

impl ProfileObservation {
    /// Render the executed command as a POSIX shell command for reports and
    /// artifact records.
    ///
    /// File-entry cases run from a generated case directory that is also the CLI output root and
    /// is removed when the run finishes. The recorded line therefore recreates that directory
    /// before entering it, even when the entry argument is absolute. The benchmark environment is
    /// recorded from the same definition the executor applies, so replaying the line uses the
    /// configuration the run actually had. `mkdir -p` is idempotent, so directory-entry cases
    /// whose working directory is the repository root still replay successfully.
    pub(crate) fn posix_shell_command_line(&self) -> String {
        posix_shell_command_line(
            Some(RecordedWorkingDirectory::Ephemeral(
                self.current_directory.as_os_str(),
            )),
            BENCHMARK_ENVIRONMENT,
            self.moth_path.as_os_str(),
            std::iter::once(OsStr::new(&self.command))
                .chain(self.command_args.iter().map(OsString::as_os_str)),
        )
    }
}

/// Run one observation pass for a case and collect its validated artifacts.
///
/// WHAT: Executes one already-preflighted case through the shared validation
/// authority and returns a `ProfileObservation`.
/// WHY: This is the measured pass that provides the timer/counter data
/// The wall time here is used for hotspot estimation.
pub(crate) fn run_observation(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
) -> Result<ProfileObservation, String> {
    let invocation = context
        .resolve_cli_invocation(case)
        .map_err(|error| error.to_string())?;
    let moth_path = context
        .compiler_binary()
        .ok_or_else(|| {
            format!(
                "Observation pass for CLI case '{}' requires a compiler binary.",
                case.id
            )
        })?
        .to_path_buf();
    let execution = execute_case(context, case)
        .map_err(|failure| format!("Observation pass failed:\n{failure}"))?;
    let BenchmarkCaseExecution {
        total_duration_ms,
        observations,
        stdout,
        stderr,
        ..
    } = execution;
    let stdout = stdout.ok_or_else(|| {
        format!(
            "Observation pass for CLI case '{}' returned no process stdout.",
            case.id
        )
    })?;
    let stderr = stderr.unwrap_or_default();

    Ok(ProfileObservation {
        case_id: case.id.clone(),
        group_name: case.group_name.persistence_spelling().to_string(),
        command: invocation.command.as_str().to_owned(),
        command_args: invocation.args,
        moth_path,
        current_directory: invocation.current_directory,
        wall_ms: total_duration_ms,
        observations,
        stdout,
        stderr,
    })
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "observations_tests.rs"]
mod tests;
