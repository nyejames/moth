//! Owned timing collection sessions.
//!
//! WHAT: owns the session token, command kind and immutable channels that
//!      scope one timing collection, and the start/finish lifecycle that
//!      protects an outer session from nested collection attempts.
//! WHY:  a process-global collector without ownership can silently replace an
//!       active report. A session token makes nesting a rejected state instead
//!       of a destructive one, and lets finish/drop drain only the matching
//!       active scope.
//!
//! Sessions exist only while `timers` is selected, so no session type, field
//! or argument survives in a no-timer build.

use super::BenchmarkObservationSnapshot;
use super::collector;
use super::runtime::TimingSessionConfiguration;
use std::fmt;

/// Which command owns a human-summary session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingCommandKind {
    Build,
    Check,
    Dev,
}

/// Process-local session generation carried by boundary and module ids.
///
/// The numeric value is opaque and command-local; it is never persisted and
/// never appears in stable benchmark output.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TimingSessionId(u64);

impl TimingSessionId {
    /// Build a session id from its raw generation value.
    ///
    /// Used by tests and by the collector when constructing boundary ids;
    /// production sessions always receive ids from `next_session_id`.
    pub(crate) const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Return the opaque generation for the runtime record-admission gate.
    pub(crate) const fn raw(self) -> u64 {
        self.0
    }
}

/// A raw collection could not acquire the process-global session owner.
///
/// Raw benchmark callers surface this error before running compiler work, so
/// they never append observations to an unrelated command or benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingSessionStartError {
    /// Another timing session owns the collector.
    CollectorBusy,
}

impl fmt::Display for TimingSessionStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CollectorBusy => formatter.write_str("another timing session is already active"),
        }
    }
}

impl std::error::Error for TimingSessionStartError {}
/// One owned collection session.
///
/// A successful raw start always owns the collector. Command startup may
/// instead return an inactive token when timing is silent or another test
/// intentionally owns a surrounding raw collection; its finish is empty and
/// its drop touches no collector state.
pub(crate) struct TimingSession {
    id: TimingSessionId,
    command: Option<TimingCommandKind>,
    configuration: Option<TimingSessionConfiguration>,
    active: bool,
}

impl TimingSession {
    /// An accepted token owning the given active scope.
    pub(crate) fn active(
        id: TimingSessionId,
        command: Option<TimingCommandKind>,
        configuration: TimingSessionConfiguration,
    ) -> Self {
        Self {
            id,
            command,
            configuration: Some(configuration),
            active: true,
        }
    }

    /// A rejected token returned for a nested start.
    pub(crate) fn rejected() -> Self {
        Self {
            id: TimingSessionId(u64::MAX),
            command: None,
            configuration: None,
            active: false,
        }
    }

    /// Whether this token owns the active collector scope.
    #[cfg(test)]
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    /// The command that owns this session, when it is a command session.
    #[cfg(test)]
    pub(crate) fn command(&self) -> Option<TimingCommandKind> {
        self.command
    }

    /// The channels that configured this session, when it was accepted.
    #[cfg(test)]
    pub(crate) fn configuration(&self) -> Option<TimingSessionConfiguration> {
        self.configuration
    }

    /// Finish the session and drain only its matching active scope.
    ///
    /// A rejected or already-finished session returns an empty snapshot and
    /// never drains another session's observations.
    pub(crate) fn finish(mut self) -> BenchmarkObservationSnapshot {
        let emit_bench_output = self.configuration.is_some_and(|configuration| {
            configuration.channels().bench_output() && !configuration.suppress_output()
        });
        self.active = false;
        let snapshot = collector::finish_session(self.id);
        if emit_bench_output {
            super::emit_bench_timing_snapshot(&snapshot);
        }
        snapshot
    }

    /// Finish the session and render its human summary.
    ///
    /// Used by the command macros after diagnostics have already been
    /// printed. Benchmark sessions never render.
    pub(crate) fn render_summary(self, succeeded: bool) {
        let command = self.command;
        let configuration = self.configuration;
        let snapshot = self.finish();
        if let (Some(command), Some(configuration)) = (command, configuration) {
            super::command::render_command_timing_summary_with_configuration(
                &snapshot,
                command,
                configuration,
                succeeded,
            );
        }
    }
}

impl Drop for TimingSession {
    fn drop(&mut self) {
        if self.active {
            collector::abandon_session(self.id);
            self.active = false;
        }
    }
}

/// Build a fresh session id.
pub(crate) fn next_session_id() -> TimingSessionId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
    TimingSessionId(NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed))
}
