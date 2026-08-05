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

/// Shared serialization lock for tests that reset/read process-global counter stores.
///
/// WHY: frontend counters are intentionally process-global atomics so Rayon module
/// compilation can update them cheaply. Any test that resets and reads those counters
/// must share one lock across modules, otherwise parallel test execution can contaminate
/// counter snapshots.
#[cfg(test)]
pub(crate) static COUNTER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_counter_test() -> std::sync::MutexGuard<'static, ()> {
    COUNTER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
