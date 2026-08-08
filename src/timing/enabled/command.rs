//! Command timing session orchestration.
//!
//! WHAT: owns command-session startup and summary rendering after a command
//! has drained its raw event snapshot.
//! WHY: command lifecycle policy stays separate from collector storage, and
//! the immutable session configuration rather than a global mode decides what
//! each completed command may render.

#[cfg(feature = "benchmark_counters")]
use super::counter_summary::render_counter_summary;
use super::runtime::{self, TimingSessionConfiguration};
use super::summary;
use super::{BenchmarkObservationSnapshot, TimingCommandKind, TimingSession};
use super::{collector, render};

/// Start one command-owned timing session for the process configuration.
pub(crate) fn start_command_session(command: TimingCommandKind) -> TimingSession {
    start_command_session_with_config(command, runtime::command_session_configuration())
}

/// Start a command session from explicit channels.
///
/// A silent command with no counter channel intentionally owns no collector.
/// A nested command stays an inactive token rather than replacing an outer
/// raw collector; instrumentation tests use that outer session to inspect
/// command spans without rendering a competing report.
fn start_command_session_with_config(
    command: TimingCommandKind,
    configuration: TimingSessionConfiguration,
) -> TimingSession {
    if !configuration.has_collection() {
        return TimingSession::rejected();
    }

    collector::try_start_session(Some(command), configuration)
        .unwrap_or_else(|_| TimingSession::rejected())
}

/// Start a command session from test-injected immutable channels.
#[cfg(test)]
pub(crate) fn start_command_session_with_configuration(
    command: TimingCommandKind,
    configuration: TimingSessionConfiguration,
) -> TimingSession {
    start_command_session_with_config(command, configuration)
}

/// Render a structured timing and/or counter summary from an already-drained
/// snapshot. The session's immutable configuration ensures a later command
/// cannot change this command's presentation policy.
pub(crate) fn render_command_timing_summary(
    snapshot: &BenchmarkObservationSnapshot,
    command: TimingCommandKind,
    succeeded: bool,
) {
    render_command_timing_summary_with_configuration(
        snapshot,
        command,
        runtime::command_session_configuration(),
        succeeded,
    );
}

/// Render a command summary with the configuration captured by its session.
pub(crate) fn render_command_timing_summary_with_configuration(
    snapshot: &BenchmarkObservationSnapshot,
    command: TimingCommandKind,
    configuration: TimingSessionConfiguration,
    succeeded: bool,
) {
    if configuration.channels().human_summary() && configuration.timer_mode().emits_summary() {
        let report = summary::build_timing_summary(snapshot, command, succeeded);
        render::render_timing_summary_report(&report);
    }

    // Counter summary is owned by `benchmark_counters` and reuses the raw
    // event snapshot until Phase 5 replaces it with dense counter aggregates.
    #[cfg(feature = "benchmark_counters")]
    if configuration.channels().human_summary()
        && configuration.counter_mode().emits_counter_summary()
    {
        for line in render_counter_summary(snapshot) {
            saying::say!(line);
        }
    }
}
