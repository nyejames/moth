//! Typed timing start tokens and scope guards.
//!
//! WHAT: owns the enabled implementation of expression-adjacent timing spans,
//!      including exact-once finish and drop behaviour.
//! WHY: guards need timer-only state and clock reads, while the facade macros
//!      remain compile-erasing at call sites.

use super::schema::TimingMetric;
use super::{
    TimingContext, record_pipeline_timing, record_pipeline_timing_attributed,
    record_pipeline_timing_multi,
};
use std::time::{Duration, Instant};

/// Opaque start token for manually timed pipeline stages.
///
/// The token carries an `Instant` only while the metrics channel is active.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PipelineTimingStart(Option<Instant>);

impl PipelineTimingStart {
    /// Return the captured duration when this session enabled metric timing.
    pub(crate) fn elapsed(&self) -> Option<Duration> {
        self.0.map(|start| start.elapsed())
    }
}

/// Start a manually recorded pipeline stage only when metrics are active.
pub(crate) fn start_pipeline_timing() -> PipelineTimingStart {
    if !super::runtime::metrics_active() {
        return PipelineTimingStart(None);
    }

    #[cfg(test)]
    super::runtime::record_timing_clock_read_for_test();

    PipelineTimingStart(Some(Instant::now()))
}

/// Record a manually timed pipeline stage from a previously captured start.
pub(crate) fn record_started_pipeline_timing(
    metric: TimingMetric,
    start: PipelineTimingStart,
) -> bool {
    start
        .elapsed()
        .is_some_and(|duration| record_pipeline_timing(metric, duration))
}

/// Record a manually timed pipeline stage with attribution context.
pub(crate) fn record_started_pipeline_timing_attributed(
    metric: TimingMetric,
    start: PipelineTimingStart,
    context: Option<TimingContext>,
) -> bool {
    start
        .elapsed()
        .is_some_and(|duration| record_pipeline_timing_attributed(metric, duration, context))
}

/// RAII guard that records one metric when dropped.
pub(crate) struct PipelineTimingGuard {
    metric: TimingMetric,
    start: PipelineTimingStart,
    finished: bool,
}

impl PipelineTimingGuard {
    /// Start a stage that records when the guard drops.
    pub(crate) fn new(metric: TimingMetric) -> Self {
        Self {
            metric,
            start: start_pipeline_timing(),
            finished: false,
        }
    }

    /// Record the stage now and suppress the drop record.
    pub(crate) fn finish(mut self) {
        record_started_pipeline_timing(self.metric, self.start);
        self.finished = true;
    }
}

impl Drop for PipelineTimingGuard {
    fn drop(&mut self) {
        if !self.finished {
            record_started_pipeline_timing(self.metric, self.start);
        }
    }
}

/// RAII guard that records one attributed metric when dropped.
pub(crate) struct PipelineTimingGuardAttributed {
    metric: TimingMetric,
    start: PipelineTimingStart,
    context: Option<TimingContext>,
    finished: bool,
}

impl PipelineTimingGuardAttributed {
    /// Start an attributed stage that records when the guard drops.
    pub(crate) fn new(metric: TimingMetric, context: Option<TimingContext>) -> Self {
        Self {
            metric,
            start: start_pipeline_timing(),
            context,
            finished: false,
        }
    }

    /// Record the stage now and suppress the drop record.
    pub(crate) fn finish(mut self) {
        record_started_pipeline_timing_attributed(self.metric, self.start, self.context);
        self.finished = true;
    }
}

impl Drop for PipelineTimingGuardAttributed {
    fn drop(&mut self) {
        if !self.finished {
            record_started_pipeline_timing_attributed(self.metric, self.start, self.context);
        }
    }
}

/// RAII guard that records several metrics from one captured duration.
//
// This guard is instantiated through the exported `timing_scope_multi!` macro.
// The library target does not expand that macro itself, so the implementation
// appears unused to Clippy even though it remains part of the facade contract.
#[allow(dead_code)]
pub(crate) struct PipelineTimingGuardMulti<'entries> {
    entries: &'entries [(TimingMetric, Option<TimingContext>)],
    start: PipelineTimingStart,
    finished: bool,
}

#[allow(dead_code)]
impl<'entries> PipelineTimingGuardMulti<'entries> {
    /// Start a shared measurement boundary for several typed metrics.
    pub(crate) fn new(entries: &'entries [(TimingMetric, Option<TimingContext>)]) -> Self {
        Self {
            entries,
            start: start_pipeline_timing(),
            finished: false,
        }
    }

    /// Record the shared duration now and suppress the drop record.
    pub(crate) fn finish(mut self) {
        if let Some(duration) = self.start.elapsed() {
            record_pipeline_timing_multi(self.entries, duration);
        }
        self.finished = true;
    }
}

impl Drop for PipelineTimingGuardMulti<'_> {
    fn drop(&mut self) {
        if !self.finished
            && let Some(duration) = self.start.elapsed()
        {
            record_pipeline_timing_multi(self.entries, duration);
        }
    }
}
