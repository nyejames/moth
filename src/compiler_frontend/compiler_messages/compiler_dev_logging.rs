//! Developer-oriented logging and benchmark observation helpers.
//!
//! WHAT: provides feature-gated macros (`token_log!`, `timer_log!`, `hir_log!`) and benchmark
//!       snapshot types for debugging compiler internals without affecting release builds.
//! WHY: keeping developer instrumentation behind feature flags keeps normal builds deterministic,
//!      quiet, and free of debug output overhead.
//!
//! The benchmark observation collector (timings + counters) is owned by `crate::timing`.
//! This module re-exports the shared types and collection APIs under `timers` so both
//! timer-only and `detailed_timers` call sites compile. Counter-specific logging
//! (`log_benchmark_counter`) is gated by `benchmark_counters`; timer prose helpers
//! (`timer_log!`, `benchmark_timer_log!`, `log_aggregated_duration`) stay gated by
//! `detailed_timers`.

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
use crate::counter_observation;

#[cfg(feature = "detailed_timers")]
use std::time::Duration;

// Re-export the shared observation types and collection APIs from the central
// timing module so existing imports from `compiler_dev_logging` keep working.
// Gated by `timers` because the collection scope serves stage timings (available
// under `timers`) and counters (available under `benchmark_counters`); both
// in-process benchmark callers and test code import through this path.
//
// The types are re-exported for test code that references them via the
// `compiler_dev_logging` path. They appear unused during `cargo check`
// (which does not compile tests), so suppress the expected warning.
#[cfg(feature = "timers")]
#[allow(unused_imports)]
pub(crate) use crate::timing::{
    BenchmarkObservationMetric, BenchmarkObservationSnapshot, start_benchmark_collection,
    start_raw_benchmark_collection, stop_and_collect_benchmark_observations,
};

// TOKEN LOGGING MACROS
#[macro_export]
#[cfg(feature = "show_tokens")]
macro_rules! token_log {
    ($($arg:tt)*) => {
        saying::say!($($arg)*);
    };
}

#[macro_export]
#[cfg(not(feature = "show_tokens"))]
macro_rules! token_log {
    ($($arg:tt)*) => {
        // Nothing
    };
}

// Extra timer logging
#[macro_export]
#[cfg(feature = "detailed_timers")]
macro_rules! timer_log {
    ($time:expr, $msg:expr) => {{
        if $crate::compiler_frontend::compiler_messages::compiler_dev_logging::detailed_timer_output_enabled() {
            saying::say!($msg, Green #$time.elapsed());
        }
    }};
}

#[macro_export]
#[cfg(not(feature = "detailed_timers"))]
macro_rules! timer_log {
    ($time:expr, $msg:expr) => {
        // Nothing
    };
}

/// Benchmark-aware timer macro: prints the existing human message, then emits a stable
/// machine-readable `MOTH_BENCH timing` line for benchmark observation parsing.
///
/// WHAT: wraps one stage/timer so it produces both developer-readable colored output and
/// a grep-friendly metric that survives prose refactors.
/// WHY: only benchmark-significant stages should use this; tiny substage timers should
/// keep using `timer_log!` so local history is not flooded with unstable micro-events.
#[macro_export]
#[cfg(feature = "detailed_timers")]
macro_rules! benchmark_timer_log {
    ($time:expr, $metric_name:expr, $human_msg:expr) => {{
        let elapsed = $time.elapsed();
        let output_suppressed = $crate::compiler_frontend::compiler_messages::compiler_dev_logging::log_benchmark_timing(
            $metric_name,
            elapsed,
        );
        if $crate::timing::detailed_prose_enabled(output_suppressed) {
            saying::say!($human_msg, Green #elapsed);
        }
    }};
}

#[macro_export]
#[cfg(not(feature = "detailed_timers"))]
macro_rules! benchmark_timer_log {
    ($time:expr, $metric_name:expr, $human_msg:expr) => {
        // Nothing
    };
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

#[cfg(feature = "detailed_timers")]
pub fn log_aggregated_duration(label: &str, duration: Duration) {
    if detailed_timer_output_enabled() {
        saying::say!(label, Green #duration);
    }
}

/// Emit one stable, machine-readable benchmark observation line.
///
/// WHAT: prints a plain `MOTH_BENCH timing <metric>=<millis>ms` line that the benchmark
/// observation parser can grep without depending on human prose.
/// WHY: separating the stable metric line from colored human output lets compiler
/// logging change its prose without silently breaking performance attribution.
#[cfg(feature = "detailed_timers")]
pub fn log_benchmark_timing(metric_name: &'static str, duration: Duration) -> bool {
    if metric_name.trim().is_empty() {
        return false;
    }

    // Record first so the suppression flag comes from the same collector lock
    // that stored the observation.
    let output_suppressed = crate::timing::record_timing(metric_name, duration);
    crate::timing::emit_bench_timing_line(metric_name, duration, output_suppressed);
    output_suppressed
}

/// Emit one stable, machine-readable benchmark counter observation.
///
/// WHAT: records `MOTH_BENCH counter <metric>=<value>` into the central collection
///       scope and prints the stable line when `MOTH_COUNTERS` requests stdout.
/// WHY: counters need a stable machine path for local benchmark history while
///      human counter prose remains optional display text. Gated by
///      `benchmark_counters` (independent of `detailed_timers`) so counter
///      benchmark runs do not have to enable verbose timer prose.
///
/// Counter storage reuses the `timers` collector, so observations are only
/// recorded when `timers` is also active. The stdout line is delegated to
/// `timing::emit_bench_counter_line`, which honors the `MOTH_COUNTERS` mode
/// and the in-process output-suppression flag.
#[cfg(feature = "benchmark_counters")]
pub fn log_benchmark_counter(metric_name: &'static str, value: f64) {
    if metric_name.trim().is_empty() || !value.is_finite() {
        return;
    }

    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let output_suppressed = counter_observation!(metric_name, value);
    #[cfg(not(all(feature = "timers", feature = "benchmark_counters")))]
    let output_suppressed = false;
    crate::timing::emit_bench_counter_line(metric_name, value, output_suppressed);
}

#[cfg(feature = "detailed_timers")]
pub fn detailed_timer_output_enabled() -> bool {
    crate::timing::output_enabled() && crate::timing::current_output_mode().emits_human_prose()
}

// Headers Logging
#[macro_export]
#[cfg(feature = "show_headers")]
macro_rules! header_log {
    ($($arg:tt)*) => {
        saying::say!($($arg)*);
    };
}

#[macro_export]
#[cfg(not(feature = "show_headers"))]
macro_rules! header_log {
    ($($arg:tt)*) => {
        // Nothing
    };
}

// AST LOGGING MACROS
#[macro_export]
#[cfg(feature = "show_ast")]
macro_rules! ast_log {
    ($($arg:tt)*) => {
        saying::say!($($arg)*);
    };
}

#[macro_export]
#[cfg(not(feature = "show_ast"))]
macro_rules! ast_log {
    ($($arg:tt)*) => {
        // Nothing
    };
}

// EVAL LOGGING MACROS
#[macro_export]
#[cfg(feature = "show_eval")]
macro_rules! eval_log {
    ($($arg:tt)*) => {
        saying::say!($($arg)*);
    };
}

#[macro_export]
#[cfg(not(feature = "show_eval"))]
macro_rules! eval_log {
    ($($arg:tt)*) => {
        // Nothing
    };
}

// CODEGEN LOGGING MACROS
#[macro_export]
#[cfg(feature = "show_codegen")]
macro_rules! codegen_log {
    ($($arg:tt)*) => {
        saying::say!($($arg)*);
    };
}

#[macro_export]
#[cfg(not(feature = "show_codegen"))]
macro_rules! codegen_log {
    ($($arg:tt)*) => {
        // Nothing
    };
}

// HIR LOGGING MACROS
#[macro_export]
#[cfg(feature = "show_hir")]
macro_rules! hir_log {
    ($($arg:tt)*) => {
        saying::say!($($arg)*);
    };
}

#[macro_export]
#[cfg(not(feature = "show_hir"))]
macro_rules! hir_log {
    ($($arg:tt)*) => {
        // Nothing
    };
}

// BORROW CHECKER LOGGING MACROS
#[macro_export]
#[cfg(feature = "show_borrow_checker")]
macro_rules! borrow_log {
    ($($arg:tt)*) => {
        saying::say!($($arg)*);
    };
}

#[macro_export]
#[cfg(not(feature = "show_borrow_checker"))]
macro_rules! borrow_log {
    ($($arg:tt)*) => {
        // Nothing
    };
}
