//! Enabled timer and benchmark-counter implementation.
//!
//! WHAT: owns collector state, output modes, observation types, summary
//!      rendering, command collection and RAII guards while the `timers`
//!      feature is active.
//! WHY:  compiling this module only when `timers` is selected keeps timer-only
//!       types, statics and renderer code out of no-timer binaries.
//!
//! Stage boundaries: this module owns timing and counter infrastructure only.
//! It must not import or depend on frontend, analysis, IR, or backend modules.
//! Compiler stages call into it through the `src/timing.rs` facade macros.

// Some collector-backed APIs can look unused when `timers` is active but no
// caller records a particular observation. This targeted allowance suppresses
// those expected dead-code warnings so `cargo check --features timers` stays
// quiet.
#![cfg_attr(feature = "timers", allow(dead_code))]

pub(crate) mod attribution;
pub(crate) mod collector;
pub(crate) mod command;
pub(crate) mod counter_summary;
pub(crate) mod render;
pub(crate) mod runtime;
pub(crate) mod schema;
pub(crate) mod session;
pub(crate) mod summary;

#[allow(unused_imports)]
/// Re-exported for tests that construct sentinel contexts directly.
pub(crate) use attribution::{
    NO_TIMING_BOUNDARY, TimingBoundaryId, TimingBoundaryKind, TimingBoundaryRecord, TimingContext,
    TimingModuleKey, TimingModuleRecord,
};
#[cfg(test)]
pub(crate) use command::start_command_session_with_configuration;
pub(crate) use command::{render_command_timing_summary, start_command_session};
pub(crate) use runtime::TimerOutputMode;
pub(crate) use session::{TimingCommandKind, TimingSession, TimingSessionStartError};
use std::time::Duration;

// ---------------------------------------------------------------------------
//  Observation types
// ---------------------------------------------------------------------------

/// One named timing observation captured during a benchmark collection scope.
#[derive(Debug, Clone)]
pub(crate) struct TimingObservation {
    pub(crate) name: &'static str,
    pub(crate) duration: Duration,
    /// Compact attribution context for this observation, when attributed.
    pub(crate) context: Option<TimingContext>,
}

/// One named counter metric value captured during a benchmark collection scope.
///
/// Counters remain `f64` values; only timing observations carry `Duration`.
#[derive(Debug, Clone)]
pub(crate) struct BenchmarkObservationMetric {
    pub(crate) name: &'static str,
    pub(crate) value: f64,
}

/// Snapshot of all observations captured in one collection scope.
///
/// `timings` is populated only while the owning session enables metrics.
/// `counters` is populated only while a `benchmark_counters` session enables
/// its counter channel. Both reuse this collector; `detailed_timers` alone
/// never enables counters.
#[derive(Debug, Clone, Default)]
pub(crate) struct BenchmarkObservationSnapshot {
    pub(crate) timings: Vec<TimingObservation>,
    pub(crate) counters: Vec<BenchmarkObservationMetric>,
    /// Registered compilation boundaries in deterministic registration order.
    pub(crate) boundaries: Vec<TimingBoundaryRecord>,
    /// Registered modules with their logical identities and source facts.
    pub(crate) modules: Vec<TimingModuleRecord>,
}

/// Aggregated view of repeated timing observations for summary output.
///
/// WHAT: combines multiple observations with the same stable metric name into
/// one total duration.
/// WHY: project-level timing summaries stay short even when later phases
/// record per-module metrics; raw labels and sample counts remain available
/// on the observations themselves.
#[derive(Debug, Clone, Default)]
pub(crate) struct TimingMetricSummary {
    pub(crate) total: Duration,
}

impl TimingMetricSummary {
    fn record(&mut self, duration: Duration) {
        self.total += duration;
    }
}

// ---------------------------------------------------------------------------
//  Public collection API
// ---------------------------------------------------------------------------

/// Start a raw benchmark collection session.
///
/// When `suppress_output` is true, stdout is suppressed while observations
/// are still recorded in the collector. This is used by tests and in-process
/// tooling that read observations programmatically. A nested raw start
/// returns an error before the caller can enter compiler work.
pub(crate) fn start_benchmark_collection(
    suppress_output: bool,
) -> Result<TimingSession, TimingSessionStartError> {
    collector::try_start_session(
        None,
        runtime::TimingSessionConfiguration::raw_benchmark(suppress_output, true),
    )
}

/// Start a raw benchmark collection session without attribution metadata.
///
/// Records every raw metric while skipping boundary/module record tables;
/// used by in-process frontend benchmarks that export only metric names and
/// durations. A busy collector is reported to the caller instead of creating
/// a rejected token that could silently record into an outer session.
pub(crate) fn start_raw_benchmark_collection(
    suppress_output: bool,
) -> Result<TimingSession, TimingSessionStartError> {
    collector::try_start_session(
        None,
        runtime::TimingSessionConfiguration::raw_benchmark(suppress_output, false),
    )
}

/// Stop a benchmark collection session and return all captured observations.
///
/// Returns an empty snapshot for a rejected or already-finished session.
pub(crate) fn stop_and_collect_benchmark_observations(
    session: TimingSession,
) -> BenchmarkObservationSnapshot {
    session::stop_session(session)
}

/// Record one counter observation in the active collection scope.
///
/// Called by `compiler_dev_logging::log_benchmark_counter` and by the
/// Stage 0 discovery paths. Counter storage reuses the `timers` collector and
/// records only while the active session enables its counter channel.
/// `detailed_timers` alone never routes counters here.
#[cfg(feature = "benchmark_counters")]
pub(crate) fn record_counter(name: &'static str, value: f64) -> bool {
    if !runtime::counters_active() {
        return false;
    }

    collector::record_counter(name, value).output_suppressed
}

/// Whether stdout output is currently allowed (not suppressed by an
/// in-process collection scope).
pub(crate) fn output_enabled() -> bool {
    collector::output_enabled()
}

/// Whether verbose human prose should print for one recorded event.
///
/// Takes the suppression flag captured while recording so callers never
/// take a second collector lock just to decide whether to print.
pub(crate) fn detailed_prose_enabled(output_suppressed: bool) -> bool {
    !output_suppressed && runtime::timer_human_prose_active()
}

/// Whether verbose human prose is enabled for prose-only call sites.
///
/// Used by developer logging macros that print without recording; the record
/// paths use `detailed_prose_enabled` with the captured suppression flag.
pub(crate) fn detailed_timer_output_enabled() -> bool {
    runtime::timer_human_prose_active() && output_enabled()
}

/// Emit one stable `MOTH_BENCH timing` line to stdout if the output mode
/// permits and output is not suppressed.
///
/// WHAT: prints a plain `MOTH_BENCH timing <metric>=<millis>ms` line that the
/// benchmark observation parser can grep without depending on human prose.
/// WHY: separating the stable metric line from colored human output lets
/// compiler logging change its prose without silently breaking attribution.
pub(crate) fn emit_bench_timing_line(
    name: &'static str,
    duration: Duration,
    output_suppressed: bool,
) {
    if name.trim().is_empty() {
        return;
    }

    if !output_suppressed && runtime::timing_bench_output_active() {
        let millis = duration.as_secs_f64() * 1000.0;
        saying::say!("MOTH_BENCH timing ", name, "=", #millis, "ms");
    }
}

/// Record a pipeline-stage timing and emit the stable bench line when
/// appropriate.
///
/// Used by the `timed_stage!` macro. The timing is always recorded in the
/// collector (when a scope is active); the stdout line depends on the output
/// mode and suppression flag.
pub(crate) fn record_pipeline_timing(metric: &'static str, duration: Duration) -> bool {
    if !runtime::metrics_active() {
        return false;
    }

    let outcome = collector::record_timing(metric, duration);
    if outcome.recorded {
        emit_bench_timing_line(metric, duration, outcome.output_suppressed);
    }
    outcome.output_suppressed
}

/// Register one compilation boundary in deterministic registration order.
///
/// Returns the dense boundary id for later observations and module
/// registration. With no active collection scope the call is a no-op that
/// returns a sentinel id; every subsequent attributed call is also dropped.
pub(crate) fn register_timing_boundary(
    kind: TimingBoundaryKind,
    display_name: impl FnOnce() -> String,
) -> TimingBoundaryId {
    if !runtime::attribution_active() {
        return NO_TIMING_BOUNDARY;
    }

    collector::register_boundary(kind, display_name)
}

/// Register one module inside a boundary with its logical identity and source facts.
///
/// `module_index` is the boundary's dense graph `ModuleId`, so registration is
/// deterministic and independent of worker completion order. The logical
/// identity is composed from the boundary display name and the module's
/// portable logical path, never from an absolute filesystem path.
pub(crate) fn register_timing_module(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_module_path: &str,
    source_file_count: u64,
    source_byte_count: u64,
) -> TimingModuleKey {
    if !runtime::attribution_active() {
        return TimingModuleKey::new(boundary, module_index);
    }

    collector::register_module(
        boundary,
        module_index,
        logical_module_path,
        source_file_count,
        source_byte_count,
    )
}

/// Record a pipeline-stage timing with attribution context.
///
/// WHAT: like `record_pipeline_timing` but stores the compact boundary/module
///      ids on the observation for the structured summary.
/// WHY:  the basic summary needs registered metadata, never labels or paths,
///       to attribute boundary and slowest-module rows. The ids never appear
///       in stable `MOTH_BENCH timing` lines.
pub(crate) fn record_pipeline_timing_attributed(
    metric: &'static str,
    duration: Duration,
    context: Option<TimingContext>,
) -> bool {
    if !runtime::metrics_active() {
        return false;
    }

    let outcome = collector::record_attributed_timing(metric, duration, context);
    if outcome.recorded {
        emit_bench_timing_line(metric, duration, outcome.output_suppressed);
    }
    outcome.output_suppressed
}

/// Record several metrics from one captured duration.
///
/// Used when two stable metrics intentionally share one measurement boundary,
/// so the second metric never includes the first record's overhead.
pub(crate) fn record_pipeline_timing_multi(
    entries: &[(&'static str, Option<TimingContext>)],
    duration: Duration,
) -> bool {
    if !runtime::metrics_active() {
        return false;
    }

    let outcome = collector::record_attributed_timing_multi(entries, duration);
    for &(metric, context) in entries {
        if outcome.recorded_entry(context) {
            emit_bench_timing_line(metric, duration, outcome.output_suppressed);
        }
    }
    outcome.output_suppressed
}

/// Opaque start token for manually timed pipeline stages.
///
/// WHAT: carries an `Instant` only while the metrics channel is active.
/// WHY: command/build orchestration sometimes needs a start token across
/// branching error paths, but inactive timer modes must not read a clock.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PipelineTimingStart(Option<std::time::Instant>);

impl PipelineTimingStart {
    /// Return the captured duration when this session enabled metric timing.
    pub(crate) fn elapsed(&self) -> Option<Duration> {
        self.0.map(|start| start.elapsed())
    }
}

/// Start a manually recorded pipeline stage only when metrics are active.
pub(crate) fn start_pipeline_timing() -> PipelineTimingStart {
    if !runtime::metrics_active() {
        return PipelineTimingStart(None);
    }

    #[cfg(test)]
    runtime::record_timing_clock_read_for_test();

    PipelineTimingStart(Some(std::time::Instant::now()))
}

/// Start one detailed-only measurement when its session channel is active.
///
/// Detailed frontend substeps do not belong to the ordinary pipeline clock
/// path, but they follow the same inactive-channel rule.
pub(crate) fn start_detailed_timing() -> Option<std::time::Instant> {
    if !runtime::detailed_active() {
        return None;
    }

    #[cfg(test)]
    runtime::record_timing_clock_read_for_test();

    Some(std::time::Instant::now())
}

/// Whether the active session retains timing attribution metadata.
///
/// The facade uses this before evaluating context expressions so raw benchmark
/// callers that requested metric-only collection avoid building unused module
/// or boundary context.
pub(crate) fn timing_attribution_active() -> bool {
    runtime::attribution_active()
}

/// Record a manually timed pipeline stage from a previously captured start token.
pub(crate) fn record_started_pipeline_timing(
    metric: &'static str,
    start: PipelineTimingStart,
) -> bool {
    start
        .elapsed()
        .is_some_and(|duration| record_pipeline_timing(metric, duration))
}

/// Record a manually timed pipeline stage with attribution context.
pub(crate) fn record_started_pipeline_timing_attributed(
    metric: &'static str,
    start: PipelineTimingStart,
    context: Option<TimingContext>,
) -> bool {
    start
        .elapsed()
        .is_some_and(|duration| record_pipeline_timing_attributed(metric, duration, context))
}

/// RAII guard that records a pipeline-stage timing when dropped.
///
/// WHAT: captures a start instant on construction and records the elapsed
///      duration under the given metric name when the guard goes out of scope.
/// WHY:  backend orchestration has many early-return error paths; a Drop guard
///       ensures every stage is timed without scattering explicit record calls
///       before every `return Err`.
pub(crate) struct PipelineTimingGuard {
    metric: &'static str,
    start: PipelineTimingStart,
    finished: bool,
}

impl PipelineTimingGuard {
    /// Start timing a stage that will be recorded when the guard drops.
    pub(crate) fn new(metric: &'static str) -> Self {
        Self {
            metric,
            start: start_pipeline_timing(),
            finished: false,
        }
    }

    /// Record the stage now and suppress the drop record.
    ///
    /// Used at the original finish point of a manual start/finish pair so the
    /// measured boundary stays identical; error paths still record on drop.
    pub(crate) fn finish(mut self) {
        record_started_pipeline_timing(self.metric, self.start);
        self.finished = true;
    }
}

/// RAII guard that records an attributed pipeline-stage timing when dropped.
///
/// WHAT: captures a start instant and records the elapsed duration under the
///      given metric with its attribution context when the guard drops.
/// WHY:  scopes with many early returns need one record path that covers
///       every exit without scattering explicit record calls.
pub(crate) struct PipelineTimingGuardAttributed {
    metric: &'static str,
    start: PipelineTimingStart,
    context: Option<TimingContext>,
    finished: bool,
}

impl PipelineTimingGuardAttributed {
    /// Start timing a stage that will be recorded when the guard drops.
    pub(crate) fn new(metric: &'static str, context: Option<TimingContext>) -> Self {
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

/// RAII guard that records several metrics from one captured duration.
///
/// WHAT: captures a start instant and records every entry with the same
///      elapsed duration when the guard drops.
/// WHY:  shared measurement boundaries must never let the second metric
///       include the first record's overhead.
pub(crate) struct PipelineTimingGuardMulti<'a> {
    entries: &'a [(&'static str, Option<TimingContext>)],
    start: PipelineTimingStart,
    finished: bool,
}

impl<'a> PipelineTimingGuardMulti<'a> {
    /// Start timing a stage that records several metrics when the guard drops.
    pub(crate) fn new(entries: &'a [(&'static str, Option<TimingContext>)]) -> Self {
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

impl<'a> Drop for PipelineTimingGuardMulti<'a> {
    fn drop(&mut self) {
        if !self.finished
            && let Some(duration) = self.start.elapsed()
        {
            record_pipeline_timing_multi(self.entries, duration);
        }
    }
}
impl Drop for PipelineTimingGuardAttributed {
    fn drop(&mut self) {
        if !self.finished {
            record_started_pipeline_timing_attributed(self.metric, self.start, self.context);
        }
    }
}
impl Drop for PipelineTimingGuard {
    fn drop(&mut self) {
        if !self.finished {
            record_started_pipeline_timing(self.metric, self.start);
        }
    }
}

/// RAII guard that records one AST aggregate stage on every exit.
///
/// WHAT: captures a start instant and records the metric with its attribution
///      context when the guard drops, including error paths that return early.
/// WHY:  AST aggregate stages previously recorded only after success, so a
///       failed environment, emission or finalization pass left no child
///       evidence in the basic report.
pub(crate) struct AstStageTimingGuard {
    metric: &'static str,
    start: PipelineTimingStart,
    context: Option<TimingContext>,
    prose_label: &'static str,
}

impl AstStageTimingGuard {
    /// Start timing one AST aggregate stage.
    pub(crate) fn new(
        metric: &'static str,
        context: Option<TimingContext>,
        prose_label: &'static str,
    ) -> Self {
        Self {
            metric,
            start: start_pipeline_timing(),
            context,
            prose_label,
        }
    }
}

impl Drop for AstStageTimingGuard {
    fn drop(&mut self) {
        if let Some(elapsed) = self.start.elapsed() {
            let output_suppressed =
                record_pipeline_timing_attributed(self.metric, elapsed, self.context);
            if detailed_prose_enabled(output_suppressed) {
                saying::say!(self.prose_label, Green #elapsed);
            }
        }
    }
}
