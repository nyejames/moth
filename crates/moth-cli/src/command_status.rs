//! Shared process status and benchmark diagnostic summary helpers.
//!
//! WHAT: keeps command success/failure semantics separate from user-facing rendering.
//! WHY: the CLI and benchmark subprocesses need one stable process-status contract while
//! diagnostics remain owned by their existing renderers.

use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandStatus {
    Success,
    Failure,
    CompilerBug,
}

impl From<CommandStatus> for ExitCode {
    fn from(status: CommandStatus) -> Self {
        match status {
            CommandStatus::Success => ExitCode::from(0),
            CommandStatus::Failure => ExitCode::from(1),
            CommandStatus::CompilerBug => ExitCode::from(2),
        }
    }
}

/// Format the optional stable benchmark diagnostic-count record.
///
/// WHAT: accepts the environment value explicitly so the emission policy can be tested without
///       mutating process-global environment state.
/// WHY: only the exact value `1` opts a caller into this machine-readable record; ordinary CLI
///      output remains unchanged for every other value.
pub(crate) fn benchmark_status_line(
    environment_value: Option<&str>,
    error_count: usize,
    warning_count: usize,
) -> Option<String> {
    (environment_value == Some("1"))
        .then(|| format!("MOTH_BENCH status errors={error_count} warnings={warning_count}"))
}

/// Emit one stable benchmark diagnostic-count record when explicitly enabled.
pub(crate) fn emit_benchmark_status(error_count: usize, warning_count: usize) {
    let environment_value = std::env::var("MOTH_BENCH_STATUS").ok();
    if let Some(line) =
        benchmark_status_line(environment_value.as_deref(), error_count, warning_count)
    {
        println!("{line}");
    }
}

#[cfg(test)]
#[path = "tests/command_status_tests.rs"]
mod tests;
