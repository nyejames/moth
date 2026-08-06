//! Owned timing collection sessions.
//!
//! WHAT: owns the session token, command kind and collection purpose that
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

/// Which command owns a human-summary session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingCommandKind {
    Build,
    Check,
    Dev,
}

/// Why a session collects observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingCollectionPurpose {
    /// A CLI command or dev cycle that renders a human summary.
    HumanSummary,
    /// An explicit in-process benchmark that reads raw observations.
    RawBenchmark,
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

    /// The raw generation value.
    pub(crate) fn raw(self) -> u64 {
        self.0
    }
}
/// One owned collection session.
///
/// Only the session returned by a successful start is active. A rejected
/// nested start returns an inactive session whose finish produces an empty
/// snapshot and whose drop touches no collector state.
pub(crate) struct TimingSession {
    id: TimingSessionId,
    command: Option<TimingCommandKind>,
    active: bool,
}

impl TimingSession {
    /// An accepted token owning the given active scope.
    pub(crate) fn active(id: TimingSessionId, command: Option<TimingCommandKind>) -> Self {
        Self {
            id,
            command,
            active: true,
        }
    }

    /// A rejected token returned for a nested start.
    pub(crate) fn rejected() -> Self {
        Self {
            id: TimingSessionId(u64::MAX),
            command: None,
            active: false,
        }
    }

    /// Whether this token owns the active collector scope.
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    /// The command that owns this session, when it is a command session.
    pub(crate) fn command(&self) -> Option<TimingCommandKind> {
        self.command
    }

    /// Finish the session and drain only its matching active scope.
    ///
    /// A rejected or already-finished session returns an empty snapshot and
    /// never drains another session's observations.
    pub(crate) fn finish(mut self) -> BenchmarkObservationSnapshot {
        self.active = false;
        collector::finish_session(self.id)
    }

    /// Finish the session and render its human summary.
    ///
    /// Used by the command macros after diagnostics have already been
    /// printed. Benchmark sessions never render.
    pub(crate) fn render_summary(self, succeeded: bool) {
        let command = self.command;
        let snapshot = self.finish();
        if let Some(command) = command {
            super::render_command_timing_summary(&snapshot, command, succeeded);
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

/// Drain a session that was handed to an API which no longer owns it.
///
/// Kept only for the benchmark observation facade; callers normally call
/// `TimingSession::finish` directly.
pub(crate) fn stop_session(session: TimingSession) -> BenchmarkObservationSnapshot {
    session.finish()
}
