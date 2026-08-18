//! Enabled timer and benchmark-counter implementation.
//!
//! WHAT: owns collector state, output modes, typed snapshot types, summary
//!      rendering, command collection and RAII guards while the `timers`
//!      feature is active.
//! WHY:  compiling this module only when `timers` is selected keeps timer-only
//!       types, statics and renderer code out of no-timer binaries.
//!
//! Stage boundaries: this module owns timing and counter infrastructure only.
//! It must not import or depend on frontend, analysis, IR, or backend modules.
//! Compiler stages call into it through the `src/timing.rs` facade macros.

pub(crate) mod attribution;
pub(crate) mod collector;
pub(crate) mod command;
pub(crate) mod counter_summary;
pub(crate) mod guard;
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
pub(crate) use guard::{
    PipelineTimingGuard, PipelineTimingGuardAttributed, PipelineTimingStart,
    record_started_pipeline_timing, record_started_pipeline_timing_attributed,
    start_pipeline_timing,
};
pub(crate) use runtime::TimerOutputMode;
pub(crate) use schema::TimingMetric;
pub(crate) use session::{TimingCommandKind, TimingSession, TimingSessionStartError};
use std::time::Duration;

// ---------------------------------------------------------------------------
//  Snapshot types
// ---------------------------------------------------------------------------

/// One dense typed timing aggregate captured during a benchmark collection scope.
///
/// The collector stores these values in schema-indexed atomic slots and only
/// materialises the row values when a session finishes. Every global snapshot
/// contains one row for every schema metric, in `TimingMetric::ALL` order;
/// attributed snapshots contain only the rows allowed by that identity's
/// attribution kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimingMetricAggregate {
    pub(crate) metric: TimingMetric,
    pub(crate) total: Duration,
    pub(crate) samples: u64,
}

/// One named counter metric value captured during a benchmark collection scope.
///
/// Counters remain `f64` values; only timing observations carry `Duration`.
#[cfg(feature = "benchmark_counters")]
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
    /// The schema version used to materialise this snapshot.
    pub(crate) schema_version: u32,
    /// The command that owned the snapshot, when it was a command session.
    ///
    /// Summary rendering receives the command explicitly. The retained field
    /// remains useful to tests and in-process benchmark consumers.
    #[allow(dead_code)]
    pub(crate) command: Option<TimingCommandKind>,
    pub(crate) timings: Vec<TimingMetricAggregate>,
    #[cfg(feature = "benchmark_counters")]
    pub(crate) counters: Vec<BenchmarkObservationMetric>,
    /// Registered compilation boundaries in deterministic registration order.
    pub(crate) boundaries: Vec<TimingBoundaryRecord>,
    /// Registered modules with their logical identities and source facts.
    pub(crate) modules: Vec<TimingModuleRecord>,
}

// ---------------------------------------------------------------------------
//  Public collection API
// ---------------------------------------------------------------------------

/// Start a raw benchmark collection session for in-process timing tests.
///
/// When `suppress_output` is true, stdout is suppressed while observations
/// are still recorded in the collector. This is used by tests and in-process
/// tooling that read observations programmatically. A nested raw start
/// returns an error before the caller can enter compiler work.
#[cfg(test)]
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

/// Format the stable final aggregate records for one completed snapshot.
///
/// WHAT: emits the schema header once, then one line for each non-empty row in
///       the dense snapshot's schema order.
/// WHY: benchmark output must describe the completed collection rather than
///       perturbing measured stage bodies with per-event formatting.
pub(crate) fn format_bench_timing_snapshot(snapshot: &BenchmarkObservationSnapshot) -> Vec<String> {
    let mut lines = Vec::with_capacity(1 + snapshot.timings.len());
    lines.push(format!(
        "MOTH_BENCH timing-schema {}",
        snapshot.schema_version
    ));

    for aggregate in &snapshot.timings {
        if aggregate.samples == 0 {
            continue;
        }

        let millis = aggregate.total.as_secs_f64() * 1000.0;
        lines.push(format!(
            "MOTH_BENCH timing {}={}ms",
            aggregate.metric.descriptor().stable_name,
            millis
        ));
    }

    lines
}

/// Emit the stable final aggregate records for one completed snapshot.
pub(crate) fn emit_bench_timing_snapshot(snapshot: &BenchmarkObservationSnapshot) {
    for line in format_bench_timing_snapshot(snapshot) {
        saying::say!(line);
    }
}

/// Record a pipeline-stage timing.
///
/// Used by the already-captured-duration facade. Stable benchmark output is
/// emitted only after the owning session finishes; detailed prose still uses
/// the supplied duration at the semantic recording endpoint.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn record_pipeline_timing(metric: TimingMetric, duration: Duration) -> bool {
    let outcome = collector::record_timing(metric, duration);
    finish_recorded_timing(outcome)
}

/// Record one command-total duration using an already-captured stopwatch reading.
///
/// WHAT: records the duration under the supplied command-total metric in the active session.
/// WHY:  ensures human-facing command durations and structured metrics share one clock read,
///       rejecting non-command-total metrics so this cannot become a generic escape hatch.
pub(crate) fn record_command_total_timing(metric: TimingMetric, duration: Duration) -> bool {
    assert!(
        metric.is_command_total(),
        "record_command_total_timing only accepts command-total metrics, got {metric:?}"
    );
    let outcome = collector::record_timing(metric, duration);
    finish_recorded_timing(outcome)
}

/// Record a span using admission retained from before its clock started.
pub(crate) fn record_pipeline_timing_with_admission(
    metric: TimingMetric,
    duration: Duration,
    admission: runtime::TimingRecordAdmission,
) -> bool {
    let outcome = collector::record_timing_with_admission(metric, duration, admission);
    finish_recorded_timing(outcome)
}

fn finish_recorded_timing(outcome: collector::TimingRecordOutcome) -> bool {
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

/// Register a module before Stage 0 has collected its source facts.
pub(crate) fn register_timing_module_for_preparation(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_module_path: &str,
) -> TimingModuleKey {
    if !runtime::attribution_active() {
        return TimingModuleKey::new(boundary, module_index);
    }

    collector::register_module_for_preparation(boundary, module_index, logical_module_path)
}

/// Finalize source facts for a module registered before Stage 0 preparation.
pub(crate) fn finalize_timing_module_source_facts(
    key: TimingModuleKey,
    source_file_count: u64,
    source_byte_count: u64,
) {
    // A key without an accumulator came from the zero-cost fallback path.
    // Registered keys may finish after the runtime channel drops, so the
    // collector owns the final session/key lookup and stale-callback policy.
    if key.accumulator().is_some() {
        collector::finalize_module_source_facts(key, source_file_count, source_byte_count);
    }
}

/// Record a pipeline-stage timing with attribution context.
///
/// WHAT: records the global typed slot and, when requested, the registered
///      boundary/module accumulator named by the compact context.
/// WHY:  the basic summary reads deterministic metadata records after the
///       session drains; identities never appear in stable benchmark lines.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn record_pipeline_timing_attributed(
    metric: TimingMetric,
    duration: Duration,
    context: Option<TimingContext>,
) -> bool {
    let outcome = collector::record_attributed_timing(metric, duration, context);
    finish_recorded_timing(outcome)
}

/// Record an attributed span using admission retained from before its clock
/// started.
pub(crate) fn record_pipeline_timing_attributed_with_admission(
    metric: TimingMetric,
    duration: Duration,
    context: Option<TimingContext>,
    admission: runtime::TimingRecordAdmission,
) -> bool {
    let outcome =
        collector::record_attributed_timing_with_admission(metric, duration, context, admission);
    finish_recorded_timing(outcome)
}

/// Record an already-captured duration with lazily constructed attribution.
///
/// WHAT: acquires the generation-stable admission before evaluating the
///      context expression, then records through that exact admission.
/// WHY: direct-duration macros cannot let a mutable session bit decide
///      attribution before admission because a drain may begin between those
///      operations.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn record_pipeline_timing_attributed_lazy(
    metric: TimingMetric,
    duration: Duration,
    context: impl FnOnce() -> Option<TimingContext>,
) -> bool {
    let Some(admission) = runtime::begin_metric_record(metric) else {
        return false;
    };
    let context = if admission.attribution_active() {
        context()
    } else {
        None
    };
    let outcome =
        collector::record_attributed_timing_with_admission(metric, duration, context, admission);
    finish_recorded_timing(outcome)
}
