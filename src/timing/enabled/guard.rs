//! Typed timing start tokens and scope guards.
//!
//! WHAT: owns the enabled implementation of expression-adjacent timing spans,
//!      including exact-once finish and drop behaviour.
//! WHY: guards need timer-only state and clock reads, while the facade macros
//!      remain compile-erasing at call sites.

use super::schema::TimingMetric;
use super::{
    TimingContext, record_pipeline_timing_attributed_with_admission,
    record_pipeline_timing_with_admission,
};
use std::time::Instant;

/// Opaque start token for manually timed pipeline stages.
///
/// The token carries a clock and its metric-aware admission while the metrics
/// channel is active. Holding the admission through the expression keeps the
/// session drain from clearing or replacing the policy before completion.
#[derive(Debug)]
pub(crate) struct PipelineTimingStart {
    started_at: Option<Instant>,
    admission: Option<super::runtime::TimingRecordAdmission>,
}

impl PipelineTimingStart {
    fn inactive() -> Self {
        Self {
            started_at: None,
            admission: None,
        }
    }

    /// Whether this start retained attribution metadata for its session.
    pub(crate) fn attribution_active(&self) -> bool {
        self.admission
            .as_ref()
            .is_some_and(|admission| admission.attribution_active())
    }

    fn take(&mut self) -> Option<(Instant, super::runtime::TimingRecordAdmission)> {
        match (self.started_at.take(), self.admission.take()) {
            (Some(started_at), Some(admission)) => Some((started_at, admission)),
            _ => None,
        }
    }
}

/// Start a manually recorded pipeline stage only when this metric is active.
pub(crate) fn start_pipeline_timing(metric: TimingMetric) -> PipelineTimingStart {
    let Some(admission) = super::runtime::begin_metric_record(metric) else {
        return PipelineTimingStart::inactive();
    };

    #[cfg(test)]
    super::runtime::record_timing_clock_read_for_test();

    PipelineTimingStart {
        started_at: Some(Instant::now()),
        admission: Some(admission),
    }
}

/// Record a manually timed pipeline stage from a previously captured start.
pub(crate) fn record_started_pipeline_timing(
    metric: TimingMetric,
    start: &mut PipelineTimingStart,
) -> bool {
    let Some((started_at, admission)) = start.take() else {
        return false;
    };

    record_pipeline_timing_with_admission(metric, started_at.elapsed(), admission)
}

/// Record a manually timed pipeline stage with attribution context.
pub(crate) fn record_started_pipeline_timing_attributed(
    metric: TimingMetric,
    start: &mut PipelineTimingStart,
    context: Option<TimingContext>,
) -> bool {
    let Some((started_at, admission)) = start.take() else {
        return false;
    };

    record_pipeline_timing_attributed_with_admission(
        metric,
        started_at.elapsed(),
        context,
        admission,
    )
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
            start: start_pipeline_timing(metric),
            finished: false,
        }
    }

    /// Record the stage now and suppress the drop record.
    pub(crate) fn finish(mut self) {
        record_started_pipeline_timing(self.metric, &mut self.start);
        self.finished = true;
    }
}

impl Drop for PipelineTimingGuard {
    fn drop(&mut self) {
        if !self.finished {
            record_started_pipeline_timing(self.metric, &mut self.start);
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
    pub(crate) fn new(
        metric: TimingMetric,
        context: impl FnOnce() -> Option<TimingContext>,
    ) -> Self {
        let start = start_pipeline_timing(metric);
        let context = if start.attribution_active() {
            context()
        } else {
            None
        };

        Self {
            metric,
            start,
            context,
            finished: false,
        }
    }

    /// Record the stage now and suppress the drop record.
    pub(crate) fn finish(mut self) {
        record_started_pipeline_timing_attributed(self.metric, &mut self.start, self.context);
        self.finished = true;
    }
}

impl Drop for PipelineTimingGuardAttributed {
    fn drop(&mut self) {
        if !self.finished {
            record_started_pipeline_timing_attributed(self.metric, &mut self.start, self.context);
        }
    }
}
