//! In-memory timing and counter observation collector.
//!
//! WHAT: captures stable benchmark metric values during an active collection
//!      scope so that in-process benchmark APIs can read timings and counters
//!      directly instead of parsing stdout.
//! WHY:  subprocess-free frontend benchmarks need programmatic access to the
//!       same metrics that CLI benchmarks extract from stable `MOTH_BENCH`
//!       lines.

use super::{BenchmarkObservationMetric, BenchmarkObservationSnapshot, TimingObservation};
use std::sync::Mutex;
use std::time::Duration;

struct ActiveCollection {
    timings: Vec<TimingObservation>,
    counters: Vec<BenchmarkObservationMetric>,
    suppress_output: bool,
}

static ACTIVE_COLLECTOR: Mutex<Option<ActiveCollection>> = Mutex::new(None);

/// Start a new collection scope, discarding any previous in-flight data.
///
/// When `suppress_output` is true, all stdout output is suppressed while
/// observations are still recorded. This is used by in-process benchmarks
/// that read observations programmatically instead of parsing stdout.
pub(crate) fn start_collection(suppress_output: bool) {
    if let Ok(mut guard) = ACTIVE_COLLECTOR.lock() {
        *guard = Some(ActiveCollection {
            timings: Vec::new(),
            counters: Vec::new(),
            suppress_output,
        });
    }
}

/// Record one timing observation if a collection scope is active.
pub(crate) fn record_timing(name: &'static str, duration: Duration) {
    if let Ok(mut guard) = ACTIVE_COLLECTOR.lock()
        && let Some(collection) = guard.as_mut()
    {
        collection.timings.push(TimingObservation {
            name,
            duration,
            label: None,
        });
    }
}

/// Record one timing observation with an attribution label.
///
/// The label is stored for summary max display only; it never appears in
/// stable `MOTH_BENCH timing` lines so benchmark parsing is unaffected.
pub(crate) fn record_labeled_timing(name: &'static str, duration: Duration, label: &str) {
    if let Ok(mut guard) = ACTIVE_COLLECTOR.lock()
        && let Some(collection) = guard.as_mut()
    {
        collection.timings.push(TimingObservation {
            name,
            duration,
            label: Some(label.to_owned()),
        });
    }
}

/// Record one counter observation if a collection scope is active.
///
/// The public `record_counter` wrapper is gated behind `benchmark_counters`,
/// so this is only reached when both `timers` (the collector) and
/// `benchmark_counters` are active. `detailed_timers` alone no longer
/// routes counters here.
pub(crate) fn record_counter(name: &'static str, value: f64) {
    if let Ok(mut guard) = ACTIVE_COLLECTOR.lock()
        && let Some(collection) = guard.as_mut()
    {
        collection.counters.push(BenchmarkObservationMetric {
            name: name.to_owned(),
            value,
            label: None,
        });
    }
}

/// Whether stdout output is currently allowed.
///
/// Returns false when an in-process collection scope has suppressed output.
/// Returns true when no scope is active (normal CLI compilation).
pub(crate) fn output_enabled() -> bool {
    match ACTIVE_COLLECTOR.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(collection) => !collection.suppress_output,
            None => true,
        },
        Err(_) => true,
    }
}

/// Stop the current collection scope and return all captured observations.
///
/// Returns an empty snapshot if no scope was active or if the lock was poisoned.
pub(crate) fn stop_and_collect() -> BenchmarkObservationSnapshot {
    if let Ok(mut guard) = ACTIVE_COLLECTOR.lock() {
        guard
            .take()
            .map_or_else(BenchmarkObservationSnapshot::default, |collection| {
                BenchmarkObservationSnapshot {
                    timings: collection.timings,
                    counters: collection.counters,
                }
            })
    } else {
        BenchmarkObservationSnapshot::default()
    }
}
