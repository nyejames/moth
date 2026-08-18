//! Developer-oriented logging, compiler dumps and benchmark counters.
//!
//! WHAT: provides feature-gated compiler dump macros and the optional
//!       benchmark-counter logging entry point without affecting release builds.
//! WHY: keeping developer instrumentation behind feature flags keeps normal
//!      builds deterministic, quiet, and free of debug output overhead.
//!
//! Timing snapshots, timing collection APIs and detailed timer prose belong
//! exclusively to `crate::timing`; this module does not provide a compatibility
//! surface for them. Counter-specific logging (`log_benchmark_counter`) is
//! gated by `benchmark_counters` and delegates storage/output policy to the
//! timing owner when timers are also active.

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
use crate::counter_observation;

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
/// recorded when `timers` is also active. With `timers`, counter call sites
/// record only — stable `MOTH_BENCH counter` lines are emitted from the
/// drained snapshot after the command total to avoid perturbing measured
/// stages. Without `timers` (counter-only builds), the stable line is
/// emitted directly here since there is no timer boundary to contaminate.
#[cfg(feature = "benchmark_counters")]
pub fn log_benchmark_counter(metric_name: &'static str, value: f64) {
    if metric_name.trim().is_empty() || !value.is_finite() {
        return;
    }

    // With timers, record only. Stable bench counter lines are emitted
    // from the drained BenchmarkObservationSnapshot after the session ends.
    #[cfg(feature = "timers")]
    {
        let _ = counter_observation!(metric_name, value);
    }

    // Without timers, emit directly — no timer boundary to perturb.
    #[cfg(not(feature = "timers"))]
    {
        crate::timing::emit_bench_counter_line(metric_name, value, false);
    }
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
