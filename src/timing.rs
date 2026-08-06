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
pub(crate) use enabled::*;

// ---------------------------------------------------------------------------
//  Counter output mode (feature = "benchmark_counters")
// ---------------------------------------------------------------------------
//
// Counter mode parsing and stdout emission live on the facade because
// counter-only benchmark runs work without `timers`. Counter *storage* still
// requires `timers`, so `record_counter` above is the no-op path here.

/// Output mode controlling how high-volume benchmark counters reach the user.
///
/// Parsed from the `MOTH_COUNTERS` environment variable. Counters are always
/// collected into the central snapshot when `benchmark_counters` and `timers`
/// are both active; this mode only controls what reaches stdout.
///
/// - `Off` (default): collect counters but print nothing. Lets in-process
///   benchmark APIs read counters programmatically without flooding CLI output.
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
    /// Parse the output mode from the `MOTH_COUNTERS` environment variable.
    ///
    /// Unset or unrecognized values default to `Off` so regular benchmark
    /// builds do not flood stdout with counter prose.
    pub(crate) fn from_env() -> Self {
        match std::env::var("MOTH_COUNTERS").as_deref() {
            Ok("summary") => Self::Summary,
            Ok("full") => Self::Full,
            _ => Self::Off,
        }
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

/// The current counter output mode parsed from `MOTH_COUNTERS`.
///
/// Counters are collected regardless of this mode (when `timers` and
/// `benchmark_counters` are both active); this only governs stdout.
#[cfg(feature = "benchmark_counters")]
static CACHED_COUNTER_MODE: std::sync::Mutex<Option<CounterOutputMode>> =
    std::sync::Mutex::new(None);

#[cfg(feature = "benchmark_counters")]
pub(crate) fn current_counter_output_mode() -> CounterOutputMode {
    let mut guard = CACHED_COUNTER_MODE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard.get_or_insert_with(CounterOutputMode::from_env)
}

#[cfg(all(feature = "benchmark_counters", test))]
pub(crate) fn set_counter_output_mode_for_test(mode: CounterOutputMode) {
    *CACHED_COUNTER_MODE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(mode);
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

    let mode = current_counter_output_mode();

    if !output_suppressed && mode.emits_bench_counter_lines() {
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
//  Pipeline timer macros
// ---------------------------------------------------------------------------

/// Record a pipeline-stage timing with a stable metric name.
///
/// Usage:
/// ```ignore
/// let ast = pipeline_timer!("frontend.ast", build_ast()?);
/// ```
///
/// When `timers` is off, the macro expands to the wrapped expression, so the
/// metric name and `Instant` path are not evaluated or imported.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! pipeline_timer {
    ($metric:expr, $expression:expr) => {{
        let timing_start = std::time::Instant::now();
        let timing_result = $expression;
        $crate::timing::record_pipeline_timing($metric, timing_start.elapsed());
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! pipeline_timer {
    ($metric:expr, $expression:expr) => {{ $expression }};
}

/// Record a pipeline-stage timing with a stable metric name and a
/// human-readable label.
///
/// Usage:
/// ```ignore
/// let ast = labeled_pipeline_timer!("frontend.ast", "AST created in: ", build_ast()?);
/// ```
///
/// The human label is printed inline only in verbose mode. The stable
/// `MOTH_BENCH timing` line is emitted in bench or verbose mode.
///
/// When `timers` is off, the macro expands to the wrapped expression, so the
/// metric name, label, and `Instant` path are not evaluated or imported.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! labeled_pipeline_timer {
    ($metric:expr, $label:expr, $expression:expr) => {{
        let timing_start = std::time::Instant::now();
        let timing_result = $expression;
        $crate::timing::record_labeled_pipeline_timing($metric, timing_start.elapsed(), $label);
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! labeled_pipeline_timer {
    ($metric:expr, $label:expr, $expression:expr) => {{ $expression }};
}

/// Start a scope-guarded timing that records its metric when the current scope
/// ends.
///
/// Usage:
/// ```ignore
/// timing_guard!("backend.js.lower_hir");
/// ```
///
/// When `timers` is off, the expansion emits no statement and the metric
/// expression is not evaluated.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timing_guard {
    ($metric:expr) => {
        let _timing_guard = $crate::timing::PipelineTimingGuard::new($metric);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timing_guard {
    ($metric:expr) => {};
}

/// Finish a manually started pipeline stage.
///
/// The start token must be captured with a `#[cfg(feature = "timers")]` local,
/// for example:
/// ```ignore
/// #[cfg(feature = "timers")]
/// let timing_start = crate::timing::start_pipeline_timing();
/// ...
/// timed_manual_finish!("stage0.directory.total", timing_start);
/// ```
///
/// When `timers` is off, the expansion emits no statement and neither the
/// metric nor the start expression is evaluated.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_manual_finish {
    ($metric:expr, $start:expr $(,)?) => {
        $crate::timing::record_started_pipeline_timing($metric, $start);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_manual_finish {
    ($metric:expr, $start:expr $(,)?) => {};
}

/// Finish a manually started pipeline stage with explicit boundary/module
/// attribution context.
///
/// When `timers` is off, the expansion emits no statement and none of the
/// metric, start or context expressions are evaluated.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_manual_finish_attributed {
    ($metric:expr, $start:expr, $context:expr $(,)?) => {
        $crate::timing::record_started_pipeline_timing_attributed($metric, $start, $context);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! timed_manual_finish_attributed {
    ($metric:expr, $start:expr, $context:expr $(,)?) => {};
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
        let elapsed = timing_start.elapsed();
        #[allow(unused_variables)]
        let output_suppressed = $crate::timing::record_pipeline_timing_attributed(
            $metric,
            elapsed,
            $context,
        );

        // Human prose stays gated by detailed_timers for verbose developer output.
        #[cfg(feature = "detailed_timers")]
        {
            if $crate::timing::detailed_prose_enabled(output_suppressed) {
                saying::say!($prose_label, Green #elapsed);
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
        let elapsed = timing_start.elapsed();
        #[allow(unused_variables)]
        let output_suppressed = $crate::timing::record_pipeline_timing_multi(
            &[($metric, $context), ($child_metric, $context)],
            elapsed,
        );

        // Human prose stays gated by detailed_timers for verbose developer output.
        #[cfg(feature = "detailed_timers")]
        {
            if $crate::timing::detailed_prose_enabled(output_suppressed) {
                saying::say!($prose_label, Green #elapsed);
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

/// Start a scope-guarded AST aggregate timing that records on every exit.
///
/// The guard records the metric with its attribution context when the scope
/// ends, including early-return error paths. When `timers` is off the
/// expansion emits no statement and no guard type exists.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! timed_ast_stage_guard {
    ($binding:ident, $metric:expr, $context:expr, $prose_label:expr) => {
        #[allow(unused_variables)]
        let $binding = $crate::timing::AstStageTimingGuard::new($metric, $context, $prose_label);
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
/// With `detailed_timers` disabled the expansion is that expression itself;
/// no closure or timing wrapper survives in the build.
#[macro_export]
#[cfg(feature = "detailed_timers")]
macro_rules! timed_frontend_substep {
    ($metric:expr, $prose_label:expr, $substep:expr $(,)?) => {{
        let timing_start = std::time::Instant::now();
        let timing_result = $substep;
        $crate::benchmark_timer_log!(timing_start, $metric, $prose_label);
        timing_result
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
macro_rules! command_timing_start {
    ($binding:ident, $command:expr) => {
        let $binding = $crate::timing::start_command_session($command);
    };
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! command_timing_start {
    ($binding:ident, $command:expr) => {};
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

#[cfg(test)]
mod tests;
