//! Command timing session orchestration.
//!
//! WHAT: owns command-session startup and the summary render path that
//!      prints the structured report after diagnostics.
//! WHY:  command lifecycle policy stays separate from collector storage
//!       and counter presentation.

#[cfg(feature = "benchmark_counters")]
use super::counter_summary::render_counter_summary;
use super::summary;
use super::{
    BenchmarkObservationSnapshot, TimingCollectionPurpose, TimingCommandKind, TimingSession,
};
use super::{collector, current_output_mode, render};
pub(crate) fn start_command_session(command: TimingCommandKind) -> TimingSession {
    // Bench and Silent modes print stable lines or nothing; they never build a
    // command snapshot that no consumer will render.
    if !current_output_mode().collects_snapshot() {
        return TimingSession::rejected();
    }

    collector::start_session(
        Some(command),
        TimingCollectionPurpose::HumanSummary,
        false,
        true,
    )
}

/// Render a structured timing summary from an already-drained snapshot.
/// WHAT: prints the human summary when the output mode requests one, plus the
///      concise counter summary when `MOTH_COUNTERS` asks for it.
/// WHY:  the command kind is explicit so a malformed or incomplete snapshot can
///       never be mislabelled as another command.
pub(crate) fn render_command_timing_summary(
    snapshot: &BenchmarkObservationSnapshot,
    command: TimingCommandKind,
    succeeded: bool,
) {
    let mode = current_output_mode();

    if mode.emits_summary() {
        let report = summary::build_timing_summary(snapshot, command, succeeded);
        render::render_timing_summary_report(&report);
    }

    // Counter summary is owned by `benchmark_counters` and reuses the snapshot
    // just drained by the timing summary. It only prints when `MOTH_COUNTERS`
    // requests the concise summary view; the legacy full dump is printed inline
    // while counters are logged, not here.
    #[cfg(feature = "benchmark_counters")]
    {
        let counter_mode = crate::timing::current_counter_output_mode();
        if counter_mode.emits_counter_summary() {
            for line in render_counter_summary(snapshot) {
                saying::say!(line);
            }
        }
    }
}
