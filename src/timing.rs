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
//! - `MOTH_TIMERS=bench` emits stable `MOTH_BENCH timing` lines only.
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
//! - `benchmark_counters` alone never enables the collector; counter storage
//!   still requires `timers`.
//!
//! Stage boundaries: this facade owns timing and counter infrastructure only.
//! It must not import or depend on frontend, analysis, IR, or backend modules.

// Counter summary helpers are only called from the command timing summary
// (gated by `timers`); suppress dead-code warnings for `benchmark_counters`-only
// builds where no command summary runs.
#![cfg_attr(
    all(feature = "benchmark_counters", not(feature = "timers")),
    allow(dead_code)
)]

#[cfg(feature = "timers")]
mod enabled;

#[cfg(feature = "timers")]
/// Re-exported for facade macros and timing tests; many names are unused in
/// lib-only builds where the collector paths are exercised only by tests.
#[allow(unused_imports)]
pub(crate) use enabled::{
    AstStageTimingGuard, BenchmarkObservationMetric, BenchmarkObservationSnapshot,
    NO_TIMING_BOUNDARY, PipelineTimingGuard, PipelineTimingGuardAttributed,
    PipelineTimingGuardMulti, PipelineTimingStart, TimerOutputMode, TimingBoundaryId,
    TimingBoundaryKind, TimingBoundaryRecord, TimingCommandKind, TimingContext,
    TimingMetricSummary, TimingModuleKey, TimingModuleRecord, TimingObservation, TimingSession,
    TimingSessionStartError, detailed_prose_enabled, detailed_timer_output_enabled,
    emit_bench_timing_line, output_enabled, record_pipeline_timing,
    record_pipeline_timing_attributed, record_pipeline_timing_multi,
    record_started_pipeline_timing, record_started_pipeline_timing_attributed,
    register_timing_boundary, register_timing_module, render_command_timing_summary,
    start_benchmark_collection, start_command_session, start_detailed_timing,
    start_pipeline_timing, start_raw_benchmark_collection, stop_and_collect_benchmark_observations,
    timing_attribution_active,
};

#[cfg(all(feature = "timers", test))]
pub(crate) use enabled::start_command_session_with_configuration;

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
pub(crate) use enabled::record_counter;

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
    ($metric:expr, $expression:expr) => {{
        let timing_start = $crate::timing::start_pipeline_timing();
        let timing_result = $expression;
        if let Some(elapsed) = timing_start.elapsed() {
            $crate::timing::record_pipeline_timing($metric, elapsed);
        }
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_stage {
    ($metric:expr, $expression:expr) => {{ $expression }};
}

/// Time one expression and record a stable metric with attribution context.
///
/// The context expression is only evaluated when the active session retains
/// attribution metadata.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_stage_attributed {
    ($metric:expr, $context:expr, $expression:expr) => {{
        let timing_start = $crate::timing::start_pipeline_timing();
        let timing_result = $expression;
        if let Some(elapsed) = timing_start.elapsed() {
            let timing_context = if $crate::timing::timing_attribution_active() {
                $context
            } else {
                None
            };
            $crate::timing::record_pipeline_timing_attributed($metric, elapsed, timing_context);
        }
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_stage_attributed {
    ($metric:expr, $context:expr, $expression:expr) => {{ $expression }};
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

/// Start a named scope guard that records several metrics from one duration.
///
/// The entries slice is only evaluated while `timers` is active.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timing_scope_multi {
    ($binding:ident, $entries:expr $(,)?) => {
        #[allow(unused_variables)]
        let $binding = $crate::timing::PipelineTimingGuardMulti::new($entries);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timing_scope_multi {
    ($binding:ident, $entries:expr $(,)?) => {};
}

/// Start a named scope guard that records an attributed metric when the scope
/// ends.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timing_scope_attributed {
    ($binding:ident, $metric:expr, $context:expr $(,)?) => {
        let timing_context = if $crate::timing::timing_attribution_active() {
            $context
        } else {
            None
        };
        #[allow(unused_variables)]
        let $binding = $crate::timing::PipelineTimingGuardAttributed::new($metric, timing_context);
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
        if $crate::timing::timing_attribution_active() {
            $crate::timing::record_pipeline_timing_attributed($metric, $duration, $context)
        } else {
            $crate::timing::record_pipeline_timing_attributed($metric, $duration, None)
        }
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! record_attributed_duration {
    ($metric:expr, $duration:expr, $context:expr) => {};
}

/// Time one frontend stage through an erasing wrapper.
///
/// The stage argument is a direct production expression, not a closure. The
/// metric, prose label and attribution context are only evaluated while
/// `timers` is active; the disabled expansion is the production expression
/// itself, so no timing wrapper survives in that build.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_frontend_stage {
    ($metric:expr, $prose_label:expr, $context:expr, $stage:expr $(,)?) => {{
        let timing_start = $crate::timing::start_pipeline_timing();
        let timing_result = $stage;
        if let Some(elapsed) = timing_start.elapsed() {
            let timing_context = if $crate::timing::timing_attribution_active() {
                $context
            } else {
                None
            };
            #[allow(unused_variables)]
            let output_suppressed = $crate::timing::record_pipeline_timing_attributed(
                $metric,
                elapsed,
                timing_context,
            );

            // Human prose stays gated by detailed_timers for verbose developer output.
            #[cfg(feature = "detailed_timers")]
            {
                if $crate::timing::detailed_prose_enabled(output_suppressed) {
                    saying::say!($prose_label, Green #elapsed);
                }
            }
        }
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_frontend_stage {
    ($metric:expr, $prose_label:expr, $context:expr, $stage:expr $(,)?) => {{ $stage }};
}

/// Time one frontend stage and record a child metric from the same duration.
///
/// The aggregate metric keeps its existing name and boundary; the child metric
/// is recorded with the same captured duration so the human report can show
/// projection or finalization evidence without redefining the aggregate.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_frontend_stage_with_child {
    ($metric:expr, $child_metric:expr, $prose_label:expr, $context:expr, $stage:expr $(,)?) => {{
        let timing_start = $crate::timing::start_pipeline_timing();
        let timing_result = $stage;
        if let Some(elapsed) = timing_start.elapsed() {
            let timing_context = if $crate::timing::timing_attribution_active() {
                $context
            } else {
                None
            };
            #[allow(unused_variables)]
            let output_suppressed = $crate::timing::record_pipeline_timing_multi(
                &[($metric, timing_context), ($child_metric, timing_context)],
                elapsed,
            );

            // Human prose stays gated by detailed_timers for verbose developer output.
            #[cfg(feature = "detailed_timers")]
            {
                if $crate::timing::detailed_prose_enabled(output_suppressed) {
                    saying::say!($prose_label, Green #elapsed);
                }
            }
        }
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_frontend_stage_with_child {
    ($metric:expr, $child_metric:expr, $prose_label:expr, $context:expr, $stage:expr $(,)?) => {{ $stage }};
}

/// Record one AST aggregate stage whenever `timers` is active, keeping the
/// human prose detailed-only.
///
/// WHAT: records the stable metric and emits the `MOTH_BENCH timing` line
///      under `timers`; prints the existing human message only under
///      `detailed_timers`.
/// WHY:  the basic summary promotes these existing detailed-only metrics
///       without double recording when both features are active.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_ast_stage {
    ($time:expr, $metric_name:expr, $human_msg:expr) => {{
        let elapsed = $time.elapsed();
        #[allow(unused_variables)]
        let output_suppressed = $crate::timing::record_pipeline_timing($metric_name, elapsed);
        #[cfg(feature = "detailed_timers")]
        {
            if $crate::timing::detailed_prose_enabled(output_suppressed) {
                saying::say!($human_msg, Green #elapsed);
            }
        }
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_ast_stage {
    ($time:expr, $metric_name:expr, $human_msg:expr) => {
        // Nothing
    };
}

/// Start a scope-guarded AST aggregate timing that records on every exit.
///
/// The guard records the metric with its attribution context when the scope
/// ends, including early-return error paths. When `timers` is off the
/// expansion emits no statement and no guard type exists.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_ast_stage_guard {
    ($binding:ident, $metric:expr, $context:expr, $prose_label:expr) => {
        let timing_context = if $crate::timing::timing_attribution_active() {
            $context
        } else {
            None
        };
        #[allow(unused_variables)]
        let $binding =
            $crate::timing::AstStageTimingGuard::new($metric, timing_context, $prose_label);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_ast_stage_guard {
    ($binding:ident, $metric:expr, $context:expr, $prose_label:expr) => {};
}

/// Time one detailed frontend substep through an erasing macro.
///
/// The substep argument is a direct production expression, not a closure.
/// With `detailed_timers` disabled, or with no active detailed channel, the
/// expansion executes that expression without a clock read.
#[macro_export]
#[cfg(feature = "detailed_timers")]
macro_rules! timed_frontend_substep {
    ($metric:expr, $prose_label:expr, $substep:expr $(,)?) => {{
        if let Some(timing_start) = $crate::timing::start_detailed_timing() {
            let timing_result = $substep;
            $crate::benchmark_timer_log!(timing_start, $metric, $prose_label);
            timing_result
        } else {
            $substep
        }
    }};
}

#[macro_export]
#[cfg(not(feature = "detailed_timers"))]
macro_rules! timed_frontend_substep {
    ($metric:expr, $prose_label:expr, $substep:expr $(,)?) => {{ $substep }};
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
macro_rules! command_timing_finish {
    ($session:expr, $succeeded:expr) => {
        $session.render_summary($succeeded);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! command_timing_finish {
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
