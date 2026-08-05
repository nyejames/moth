//! In-memory timing and counter observation collector.
//!
//! WHAT: captures stable benchmark metric values during an active collection
//!      scope so that in-process benchmark APIs can read timings and counters
//!      directly instead of parsing stdout.
//! WHY:  subprocess-free frontend benchmarks need programmatic access to the
//!       same metrics that CLI benchmarks extract from stable `MOTH_BENCH`
//!       lines.

use super::{
    BenchmarkObservationMetric, BenchmarkObservationSnapshot, NO_TIMING_BOUNDARY, TimingBoundaryId,
    TimingBoundaryKind, TimingBoundaryRecord, TimingModuleContext, TimingModuleKey,
    TimingModuleRecord, TimingObservation,
};
use std::sync::Mutex;
use std::time::Duration;

struct ActiveCollection {
    timings: Vec<TimingObservation>,
    counters: Vec<BenchmarkObservationMetric>,
    boundaries: Vec<TimingBoundaryRecord>,
    modules: Vec<TimingModuleRecord>,
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
            boundaries: Vec::new(),
            modules: Vec::new(),
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
            boundary: None,
            module: None,
        });
    }
}

/// Record one timing observation with an optional label and compact context.
///
/// The label is preserved on the raw observation for detailed tooling and the
/// ids drive structured summary attribution. Neither appears in stable
/// `MOTH_BENCH timing` lines so benchmark parsing is unaffected.
pub(crate) fn record_attributed_timing(
    name: &'static str,
    duration: Duration,
    label: Option<&str>,
    context: TimingModuleContext,
) {
    if let Ok(mut guard) = ACTIVE_COLLECTOR.lock()
        && let Some(collection) = guard.as_mut()
    {
        collection.timings.push(TimingObservation {
            name,
            duration,
            label: label.map(str::to_owned),
            boundary: context.boundary,
            module: context.module,
        });
    }
}

/// Register one compilation boundary and return its dense id.
///
/// Returns a sentinel id when no collection scope is active. A compile that
/// starts before a scope can finish inside one; the sentinel never matches a
/// real registration slot, so its late attributed work is dropped instead of
/// polluting the first active scope's boundary zero.
pub(crate) fn register_boundary(
    kind: TimingBoundaryKind,
    display_name: String,
) -> TimingBoundaryId {
    if let Ok(mut guard) = ACTIVE_COLLECTOR.lock()
        && let Some(collection) = guard.as_mut()
    {
        let id = TimingBoundaryId::from_index(collection.boundaries.len() as u32);
        collection.boundaries.push(TimingBoundaryRecord {
            id,
            kind,
            display_name,
            module_count: 0,
        });
        return id;
    }

    NO_TIMING_BOUNDARY
}

/// Register one module inside a boundary and return its dense key.
///
/// The module index is the boundary's graph-owned dense `ModuleId`, so the
/// same index in two boundaries stays distinct. The logical identity combines
/// the boundary display name with the portable logical module path.
pub(crate) fn register_module(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_module_path: &str,
    source_file_count: u64,
    source_byte_count: u64,
) -> TimingModuleKey {
    let key = TimingModuleKey {
        boundary,
        module_index,
    };

    if let Ok(mut guard) = ACTIVE_COLLECTOR.lock()
        && let Some(collection) = guard.as_mut()
        && let Some(boundary_record) = collection.boundaries.get(boundary.index())
    {
        let logical_identity = if logical_module_path.is_empty() {
            boundary_record.display_name.clone()
        } else {
            format!("{}/{}", boundary_record.display_name, logical_module_path)
        };
        collection.modules.push(TimingModuleRecord {
            key,
            logical_identity,
            source_file_count,
            source_byte_count,
        });
        collection.boundaries[boundary.index()].module_count += 1;
    }

    key
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
                    boundaries: collection.boundaries,
                    modules: collection.modules,
                }
            })
    } else {
        BenchmarkObservationSnapshot::default()
    }
}
