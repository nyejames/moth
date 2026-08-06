//! Frontend performance instrumentation.
//!
//! WHAT: exposes counters for clone-heavy, cache-sensitive, and remap-heavy frontend paths.
//! WHY: benchmark runs built with `benchmark_counters` need enough local evidence to
//! interpret small end-to-end timing changes, while normal compiler builds must not
//! pay for or print this diagnostic data.

pub(crate) mod ast_counters;
pub(crate) mod frontend_counters;

pub(crate) use ast_counters::*;
pub(crate) use frontend_counters::*;

/// Shared serialization lock for tests that reset or read the process-global
/// compiler-instrumentation stores.
///
/// WHY: frontend counters and the timing collector are one process-global
/// scope. Any test that resets/reads counters or starts a collection session
/// must serialize against every other such test, otherwise parallel test
/// execution can interleave sessions. This delegates to the single
/// facade-owned lock so timing and frontend suites share one fence.
#[cfg(test)]
pub(crate) fn lock_counter_test() -> std::sync::MutexGuard<'static, ()> {
    crate::timing::lock_instrumentation_tests()
}

#[cfg(test)]
mod tests;
