//! In-memory timing and counter observation collector.
//!
//! WHAT: stores observations for exactly one active collection session and
//!      validates every attributed id against that session's generation.
//! WHY:  subprocess-free frontend benchmarks need programmatic access to the
//!       same metrics that CLI benchmarks extract from stable `MOTH_BENCH`
//!       lines, and command reports must never mix evidence from two sessions.
//!       Session ownership turns nested collection attempts into rejected
//!       tokens instead of silent replacement of an active report.

use super::attribution::{
    NO_TIMING_BOUNDARY, TimingBoundaryId, TimingBoundaryKind, TimingBoundaryRecord, TimingContext,
    TimingModuleKey, TimingModuleRecord,
};
use super::session::{TimingCollectionPurpose, TimingCommandKind, TimingSession, TimingSessionId};
use super::{BenchmarkObservationMetric, BenchmarkObservationSnapshot, TimingObservation};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

struct ActiveCollection {
    id: TimingSessionId,
    command: Option<TimingCommandKind>,
    purpose: TimingCollectionPurpose,
    suppress_output: bool,
    attribution: bool,
    timings: Vec<TimingObservation>,
    counters: Vec<BenchmarkObservationMetric>,
    boundaries: Vec<TimingBoundaryRecord>,
    modules: Vec<TimingModuleRecord>,
}

static ACTIVE_COLLECTOR: Mutex<Option<ActiveCollection>> = Mutex::new(None);

/// Recover the collector lock after poisoning instead of returning empty data.
///
/// The collector is pure bookkeeping: a previous panic must not silently
/// erase later observations. No code panics while holding this lock.
fn lock_collector() -> MutexGuard<'static, Option<ActiveCollection>> {
    ACTIVE_COLLECTOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Start a new collection session.
///
/// When another session is already active the new token is rejected and the
/// outer session is preserved untouched; the caller must treat a rejected
/// token as inactive and skip recording through it.
pub(crate) fn start_session(
    command: Option<TimingCommandKind>,
    purpose: TimingCollectionPurpose,
    suppress_output: bool,
    attribution: bool,
) -> TimingSession {
    let mut guard = lock_collector();
    if guard.is_some() {
        return rejected_session();
    }

    let id = super::session::next_session_id();
    *guard = Some(ActiveCollection {
        id,
        command,
        purpose,
        suppress_output,
        attribution,
        timings: Vec::new(),
        counters: Vec::new(),
        boundaries: Vec::new(),
        modules: Vec::new(),
    });
    active_session(id, command)
}

/// A rejected token for a nested or otherwise refused session start.
fn rejected_session() -> TimingSession {
    TimingSession::rejected()
}

/// An accepted token owning the given active scope.
fn active_session(id: TimingSessionId, command: Option<TimingCommandKind>) -> TimingSession {
    TimingSession::active(id, command)
}

/// Drain the active scope only when it belongs to the given session.
pub(crate) fn finish_session(id: TimingSessionId) -> BenchmarkObservationSnapshot {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return BenchmarkObservationSnapshot::default();
    };
    if collection.id != id {
        return BenchmarkObservationSnapshot::default();
    }

    let collection = guard.take().expect("active collection present");
    let mut snapshot = BenchmarkObservationSnapshot {
        timings: collection.timings,
        counters: collection.counters,
        boundaries: collection.boundaries,
        modules: collection.modules,
    };
    recompute_boundary_module_counts(&mut snapshot);
    snapshot
}

/// Drop an unfinished session's active scope without returning observations.
///
/// Called from `TimingSession::drop`; only the matching session is removed.
pub(crate) fn abandon_session(id: TimingSessionId) {
    let mut guard = lock_collector();
    if guard.as_ref().is_some_and(|collection| collection.id == id) {
        *guard = None;
    }
}

/// Record one timing observation if a collection session is active.
///
/// Returns whether stdout is suppressed by the active session.
pub(crate) fn record_timing(name: &'static str, duration: Duration) -> bool {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return false;
    };
    collection.timings.push(TimingObservation {
        name,
        duration,
        context: None,
    });
    collection.suppress_output
}

/// Record one timing observation with compact boundary/module context.
///
/// The ids drive structured summary attribution and never appear in stable
/// `MOTH_BENCH timing` lines, so benchmark parsing is unaffected. Context
/// from another session or the sentinel is dropped rather than stored.
/// Returns whether stdout is suppressed by the active session.
pub(crate) fn record_attributed_timing(
    name: &'static str,
    duration: Duration,
    context: Option<TimingContext>,
) -> bool {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return false;
    };

    if let Some(context) = context
        && context.session() != collection.id
    {
        return false;
    }

    collection.timings.push(TimingObservation {
        name,
        duration,
        context,
    });
    collection.suppress_output
}

/// Record several timing observations from one captured duration.
///
/// Each entry carries its own attribution context; entries from another
/// session are skipped. Returns whether stdout is suppressed.
pub(crate) fn record_attributed_timing_multi(
    entries: &[(&'static str, Option<TimingContext>)],
    duration: Duration,
) -> bool {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return false;
    };

    for (name, context) in entries {
        if let Some(context) = context
            && context.session() != collection.id
        {
            continue;
        }
        collection.timings.push(TimingObservation {
            name,
            duration,
            context: *context,
        });
    }
    collection.suppress_output
}

/// Register one compilation boundary inside the active session.
///
/// Returns the sentinel id when no collection session is active; attributed
/// observations carrying that sentinel are dropped because its session
/// generation never matches a live session.
pub(crate) fn register_boundary(
    kind: TimingBoundaryKind,
    display_name: impl FnOnce() -> String,
) -> TimingBoundaryId {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return NO_TIMING_BOUNDARY;
    };

    let id = TimingBoundaryId::from_session(collection.id, collection.boundaries.len() as u32);
    if !collection.attribution {
        return id;
    }
    collection.boundaries.push(TimingBoundaryRecord {
        id,
        kind,
        display_name: display_name(),
        module_count: 0,
    });
    id
}

/// Register one module inside a boundary and return its dense key.
///
/// The module index is the boundary's graph-owned dense `ModuleId`, so the
/// same index in two boundaries stays distinct. Duplicate registrations are
/// ignored and return the existing key, keeping module records and boundary
/// counts deterministic.
pub(crate) fn register_module(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_module_path: &str,
    source_file_count: u64,
    source_byte_count: u64,
) -> TimingModuleKey {
    let key = TimingModuleKey::new(boundary, module_index);

    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return key;
    };
    if boundary.session() != collection.id {
        return key;
    }
    if !collection.attribution {
        return key;
    }
    let Some(boundary_record) = collection.boundaries.get(boundary.index()) else {
        return key;
    };
    if boundary_record.id != boundary {
        return key;
    }
    if collection.modules.iter().any(|record| record.key == key) {
        return key;
    }

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
    key
}

/// Record one counter observation if a collection session is active.
///
/// The public `record_counter` wrapper is gated behind `benchmark_counters`,
/// so this is only reached when both `timers` (the collector) and
/// `benchmark_counters` are active. `detailed_timers` alone no longer routes
/// counters here.
pub(crate) fn record_counter(name: &'static str, value: f64) -> bool {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return false;
    };
    collection
        .counters
        .push(BenchmarkObservationMetric { name, value });
    collection.suppress_output
}

/// Whether stdout output is currently allowed.
///
/// Returns false when an in-process collection session has suppressed output.
/// Returns true when no session is active (normal CLI compilation).
pub(crate) fn output_enabled() -> bool {
    match lock_collector().as_ref() {
        Some(collection) => !collection.suppress_output,
        None => true,
    }
}

/// Keep boundary module counts derived from the registered module records.
///
/// The retained counter is validated here so a duplicate registration or a
/// future re-registration path cannot drift from the actual record table.
fn recompute_boundary_module_counts(snapshot: &mut BenchmarkObservationSnapshot) {
    let mut counts = std::collections::BTreeMap::<TimingBoundaryId, u64>::new();
    for record in &snapshot.modules {
        *counts.entry(record.key.boundary()).or_default() += 1;
    }
    for boundary in &mut snapshot.boundaries {
        boundary.module_count = counts.get(&boundary.id).copied().unwrap_or(0);
    }
}
