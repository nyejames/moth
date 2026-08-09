//! Central timing and benchmark-counter collector facade.
//!
//! WHAT: owns the compile-time facade for named pipeline-stage durations and
//!      high-volume benchmark counters during compilation.
//! WHY:  lets concise timing runs happen behind the `timers` feature without
//!      pulling in the high-volume counters that flood output for large
//!      projects, and lets counter-only benchmark runs happen behind
//!      `benchmark_counters` without enabling verbose timer prose.
//!
//! Three output products share one collector:
//! - `MOTH_TIMERS=summary` prints the curated human report (pipeline,
//!   compilation boundaries, frontend, backend, slowest module).
//! - `MOTH_TIMERS=bench` emits one `MOTH_BENCH timing-schema 1` header and one
//!   final aggregate set of stable `MOTH_BENCH timing` lines in schema order.
//! - `MOTH_TIMERS=verbose` keeps detailed inline prose and ends with the
//!   curated report.
//!
//! The dev server starts one fresh collection per build cycle and renders
//! the same report after its one-line status.
//!
//! Zero-cost rule:
//! - With `timers` selected, `timing::enabled` owns the collector, output
//!   modes, observation types, guards and renderers, and the macros below
//!   expand to real timing calls.
//! - Without `timers`, expression macros expand to the wrapped production
//!   expression only and guard/command macros expand to nothing. No timer
//!   type, collector or renderer exists in that build, and no timer-only
//!   function ABI or field survives at a call site.
//! - `benchmark_counters` alone never enables the collector; collector-backed
//!   counter storage still requires `timers`, while direct counter logging can
//!   emit stable lines in a counter-only build.
//!
//! Stage boundaries: this facade owns timing and counter infrastructure only.
//! It must not import or depend on frontend, analysis, IR, or backend modules.

/// Start one detailed-only prose timer from the timing implementation owner.
///
/// Detailed AST microstage logging predates the typed pipeline guards and is
/// intentionally kept as human evidence rather than a schema metric. Keeping
/// its clock creation here prevents frontend modules from owning timer types or
/// direct clock calls.
#[cfg(feature = "detailed_timers")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DetailedTimerStart(Option<std::time::Instant>);

#[cfg(feature = "detailed_timers")]
impl DetailedTimerStart {
    /// Return elapsed time only when the detailed channel captured a start.
    pub(crate) fn elapsed(&self) -> Option<std::time::Duration> {
        self.0.map(|start| start.elapsed())
    }
}

#[cfg(feature = "detailed_timers")]
pub(crate) fn start_detailed_timer() -> DetailedTimerStart {
    if !enabled::runtime::timer_human_prose_active() {
        return DetailedTimerStart(None);
    }

    #[cfg(test)]
    enabled::runtime::record_timing_clock_read_for_test();

    DetailedTimerStart(Some(std::time::Instant::now()))
}

#[cfg(feature = "timers")]
mod enabled;

#[cfg(feature = "timers")]
/// Re-exported for facade macros and timing tests; many names are unused in
/// lib-only builds where the collector paths are exercised only by tests.
#[allow(unused_imports)]
pub(crate) use enabled::{
    BenchmarkObservationSnapshot, NO_TIMING_BOUNDARY, PipelineTimingGuard,
    PipelineTimingGuardAttributed, PipelineTimingStart, TimerOutputMode, TimingBoundaryId,
    TimingBoundaryKind, TimingBoundaryRecord, TimingCommandKind, TimingContext, TimingMetric,
    TimingMetricAggregate, TimingModuleKey, TimingModuleRecord, TimingSession,
    TimingSessionStartError, detailed_prose_enabled, emit_detailed_metric_prose,
    finalize_timing_module_source_facts, record_pipeline_timing, record_pipeline_timing_attributed,
    record_pipeline_timing_attributed_lazy, record_started_pipeline_timing,
    record_started_pipeline_timing_attributed, register_timing_boundary, register_timing_module,
    register_timing_module_for_preparation, render_command_timing_summary, start_command_session,
    start_pipeline_timing, start_raw_benchmark_collection,
};

#[cfg(all(feature = "timers", feature = "detailed_timers"))]
pub(crate) use enabled::detailed_timer_output_enabled;

#[cfg(all(feature = "timers", feature = "benchmark_counters", test))]
pub(crate) use enabled::BenchmarkObservationMetric;

#[cfg(all(feature = "timers", test))]
pub(crate) use enabled::start_benchmark_collection;

#[cfg(feature = "timers")]
pub(crate) use enabled::schema::{
    TIMING_BUILD_PIPELINE_METRIC_NAMES, TIMING_CHECK_PIPELINE_METRIC_NAMES,
    TIMING_COMMAND_BUILD_TOTAL_NAME, TIMING_COMMAND_CHECK_TOTAL_NAME,
    TIMING_FRONTEND_AST_EMIT_NAME, TIMING_FRONTEND_AST_ENVIRONMENT_NAME,
    TIMING_FRONTEND_AST_FINALISE_NAME, TIMING_FRONTEND_AST_TOTAL_NAME,
    TIMING_FRONTEND_BORROW_CONVERGE_NAME, TIMING_FRONTEND_BORROW_INITIAL_NAME,
    TIMING_FRONTEND_HIR_NAME, TIMING_FRONTEND_ORDER_DECLARATIONS_NAME,
    TIMING_FRONTEND_PREPARE_NAME, TIMING_SCHEMA_METRIC_NAMES, benchmark_label_for_name,
};

#[cfg(feature = "timers")]
pub use enabled::schema::TIMING_SCHEMA_VERSION;

#[cfg(all(feature = "timers", test))]
pub(crate) use enabled::start_command_session_with_configuration;

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
pub(crate) use enabled::record_counter;

// ---------------------------------------------------------------------------
//  Detailed human prose
// ---------------------------------------------------------------------------

/// Print one detailed-only microstage using the timer's captured start.
#[macro_export]
#[cfg(feature = "detailed_timers")]
macro_rules! timer_log {
    ($time:expr, $message:expr) => {{
        if let Some(elapsed) = $time.elapsed() {
            if $crate::timing::detailed_timer_output_enabled() {
                saying::say!($message, Green #elapsed);
            }
        }
    }};
}

#[macro_export]
#[cfg(not(feature = "detailed_timers"))]
macro_rules! timer_log {
    ($time:expr, $message:expr) => {};
}

/// Print a captured detailed aggregate duration without taking a second clock.
#[cfg(feature = "detailed_timers")]
pub(crate) fn log_aggregated_duration(label: &str, duration: std::time::Duration) {
    if detailed_timer_output_enabled() {
        saying::say!(label, Green #duration);
    }
}

// ---------------------------------------------------------------------------
//  Counter output mode (feature = "benchmark_counters")
// ---------------------------------------------------------------------------
//
// Counter mode parsing and stdout emission live on the facade because
// counter-only benchmark runs work without `timers`. Counter *storage* still
// requires `timers`, so `record_counter` above is the no-op path here.

/// Output mode controlling how high-volume benchmark counters reach the user.
///
/// Parsed from the `MOTH_COUNTERS` environment variable. In a timer build it
/// selects the counter channel and its presentation policy; a counters-only
/// build still uses it solely for direct stdout output.
///
/// - `Off` (default): do not collect command counters and print nothing.
///   Explicit raw benchmark sessions may still collect counters programmatically.
/// - `Summary`: emit stable `MOTH_BENCH counter` lines and print a concise
///   grouped counter summary after compilation.
/// - `Full`: emit stable `MOTH_BENCH counter` lines and print the legacy
///   per-counter human dump (the old `detailed_timers` behavior).
#[cfg(feature = "benchmark_counters")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CounterOutputMode {
    /// Collect counters but print no counter stdout.
    Off,
    /// Stable counter lines plus a concise grouped summary.
    Summary,
    /// Stable counter lines plus the legacy per-counter human dump.
    Full,
}

#[cfg(feature = "benchmark_counters")]
impl CounterOutputMode {
    /// Parse one optional `MOTH_COUNTERS` value without reading process state.
    ///
    /// Unset or unrecognized values default to `Off` so regular benchmark
    /// builds do not flood stdout with counter prose.
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("summary") => Self::Summary,
            Some("full") => Self::Full,
            _ => Self::Off,
        }
    }

    /// Read the process setting while runtime configuration initialises.
    pub(crate) fn from_environment() -> Self {
        let value = std::env::var("MOTH_COUNTERS").ok();
        Self::parse(value.as_deref())
    }

    /// Whether stable `MOTH_BENCH counter` lines should be printed.
    pub(crate) fn emits_bench_counter_lines(self) -> bool {
        matches!(self, Self::Summary | Self::Full)
    }

    /// Whether the concise grouped counter summary should be printed after
    /// compilation.
    #[cfg(feature = "timers")]
    pub(crate) fn emits_counter_summary(self) -> bool {
        matches!(self, Self::Summary)
    }

    /// Whether the legacy per-counter human dump should be printed while
    /// counters are logged.
    pub(crate) fn emits_human_counter_prose(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// The immutable counter output policy for counters-only process builds.
///
/// Timer builds keep this policy inside their shared runtime configuration;
/// they use the active session channel directly instead of reading it while
/// recording a counter.
#[cfg(all(feature = "benchmark_counters", not(feature = "timers")))]
static CACHED_COUNTER_MODE: std::sync::OnceLock<CounterOutputMode> = std::sync::OnceLock::new();

#[cfg(all(feature = "benchmark_counters", not(feature = "timers")))]
pub(crate) fn current_counter_output_mode() -> CounterOutputMode {
    *CACHED_COUNTER_MODE.get_or_init(CounterOutputMode::from_environment)
}

/// Whether legacy per-counter human prose is enabled for the active session.
#[cfg(feature = "benchmark_counters")]
pub(crate) fn counter_human_prose_enabled() -> bool {
    #[cfg(feature = "timers")]
    {
        enabled::runtime::counter_human_prose_active()
    }

    #[cfg(not(feature = "timers"))]
    {
        current_counter_output_mode().emits_human_counter_prose()
    }
}

/// Emit one stable `MOTH_BENCH counter` line to stdout if the counter output
/// mode permits and output is not suppressed.
///
/// WHAT: prints a plain `MOTH_BENCH counter <metric>=<value>` line that the
///      benchmark observation parser can grep without depending on human prose.
/// WHY:  like the timing line, separating the stable counter metric from
///       human prose lets counter logging change its display without breaking
///       benchmark attribution. The line is only emitted for `MOTH_COUNTERS`
///       modes that request stdout (`summary` or `full`).
#[cfg(feature = "benchmark_counters")]
pub(crate) fn emit_bench_counter_line(name: &'static str, value: f64, output_suppressed: bool) {
    if name.trim().is_empty() {
        return;
    }

    #[cfg(feature = "timers")]
    let emits_bench_line = enabled::runtime::counter_bench_output_active();

    #[cfg(not(feature = "timers"))]
    let emits_bench_line = current_counter_output_mode().emits_bench_counter_lines();

    if !output_suppressed && emits_bench_line {
        saying::say!("MOTH_BENCH counter ", name, "=", #value);
    }
}

/// Record one benchmark counter observation at a production call site.
///
/// Counter storage requires both `timers` and `benchmark_counters`. In every
/// other build the expansion emits no statement and neither argument is
/// evaluated, so direct counter call sites never touch a no-op stub.
#[macro_export]
#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
macro_rules! counter_observation {
    ($name:expr, $value:expr $(,)?) => {
        $crate::timing::record_counter($name, $value)
    };
}

#[macro_export]
#[cfg(not(all(feature = "timers", feature = "benchmark_counters")))]
macro_rules! counter_observation {
    ($name:expr, $value:expr $(,)?) => {};
}

// ---------------------------------------------------------------------------
//  Timing facade macros
// ---------------------------------------------------------------------------

/// Time one expression and record a stable metric.
///
/// The expression is evaluated directly; the disabled expansion is the
/// expression itself, so no timing wrapper survives in a no-timer build.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_stage {
    ($metric:expr, $expression:expr $(,)?) => {{
        let timing_metric = $metric;
        let mut timing_start = $crate::timing::start_pipeline_timing(timing_metric);
        let timing_result = $expression;
        $crate::timing::record_started_pipeline_timing(timing_metric, &mut timing_start);
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_stage {
    ($metric:expr, $expression:expr $(,)?) => {{ $expression }};
}

/// Time one expression and record a stable metric with attribution context.
///
/// The context expression is only evaluated when the active session retains
/// attribution metadata.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_stage_attributed {
    ($metric:expr, $context:expr, $expression:expr $(,)?) => {{
        let timing_metric = $metric;
        let mut timing_start = $crate::timing::start_pipeline_timing(timing_metric);
        let timing_result = $expression;
        let timing_context = if timing_start.attribution_active() {
            $context
        } else {
            None
        };
        $crate::timing::record_started_pipeline_timing_attributed(
            timing_metric,
            &mut timing_start,
            timing_context,
        );
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_stage_attributed {
    ($metric:expr, $context:expr, $expression:expr $(,)?) => {{ $expression }};
}

/// Start a named scope guard that records its metric when the scope ends.
///
/// Callers must name the guard; the disabled expansion emits no statement.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timing_scope {
    ($binding:ident, $metric:expr $(,)?) => {
        #[allow(unused_variables)]
        let $binding = $crate::timing::PipelineTimingGuard::new($metric);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timing_scope {
    ($binding:ident, $metric:expr $(,)?) => {};
}

/// Start a named scope guard that records an attributed metric when the scope
/// ends.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timing_scope_attributed {
    ($binding:ident, $metric:expr, $context:expr $(,)?) => {
        #[allow(unused_variables)]
        let $binding = $crate::timing::PipelineTimingGuardAttributed::new($metric, || $context);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timing_scope_attributed {
    ($binding:ident, $metric:expr, $context:expr $(,)?) => {};
}

/// Record one already-captured duration under a stable metric.
///
/// The duration expression is only evaluated while `timers` is active.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! record_timing_duration {
    ($metric:expr, $duration:expr) => {
        $crate::timing::record_pipeline_timing($metric, $duration)
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! record_timing_duration {
    ($metric:expr, $duration:expr) => {};
}

/// Record one already-captured duration with attribution context.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! record_attributed_duration {
    ($metric:expr, $duration:expr, $context:expr) => {
        $crate::timing::record_pipeline_timing_attributed_lazy($metric, $duration, || $context)
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! record_attributed_duration {
    ($metric:expr, $duration:expr, $context:expr) => {};
}

/// Finish a named timing scope exactly once.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! finish_timing_scope {
    ($binding:expr $(,)?) => {
        $binding.finish();
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! finish_timing_scope {
    ($binding:expr $(,)?) => {};
}

/// Start the command-level timing collection scope with an explicit command kind.
///
/// The bound session token owns the active scope; finishing it drains only
/// that session. When `timers` is off, the expansion emits no statement and
/// neither binding nor command expression is evaluated.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! command_timing_scope {
    ($binding:ident, $command:expr $(,)?) => {
        let $binding = $crate::timing::start_command_session($command);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! command_timing_scope {
    ($binding:ident, $command:expr $(,)?) => {};
}

/// Finish a command timing session and print the configured summary.
///
/// The `succeeded` expression only changes the human title; stable metrics are
/// unchanged. When `timers` is off, the expansion emits no statement and
/// neither the session nor the expression is evaluated.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! finish_command_timing {
    ($session:expr, $succeeded:expr) => {
        $session.render_summary($succeeded);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! finish_command_timing {
    ($session:expr, $succeeded:expr) => {};
}

// ---------------------------------------------------------------------------
//  Shared test-serialization lock
// ---------------------------------------------------------------------------
//
// The timing collector and the frontend counter stores are one process-global
// compiler-instrumentation scope. Every test that starts a collection session
// or resets those stores must share a single lock, otherwise parallel test
// execution interleaves unrelated sessions. The frontend counter-test lock
// delegates here (matching the shared active-counter channel).

/// Serialize every compiler-instrumentation test behind one process-global
/// lock.
#[cfg(test)]
pub(crate) fn lock_instrumentation_tests() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::Mutex;
    static INSTRUMENTATION_TEST_LOCK: Mutex<()> = Mutex::new(());
    INSTRUMENTATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
