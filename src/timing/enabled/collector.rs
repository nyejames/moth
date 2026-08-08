//! In-memory timing and counter observation collector.
//!
//! WHAT: stores raw observations for exactly one active collection session and
//! validates attributed ids against that session's generation.
//! WHY: Phase 2 keeps the existing event snapshot while moving lifecycle and
//! channel policy into one explicit owner. The later dense collector replaces
//! this storage only after every recording call site is typed.

use super::attribution::{
    NO_TIMING_BOUNDARY, TimingBoundaryId, TimingBoundaryKind, TimingBoundaryRecord, TimingContext,
    TimingModuleKey, TimingModuleRecord,
};
use super::runtime::{self, TimingSessionConfiguration};
use super::session::{TimingCommandKind, TimingSession, TimingSessionId, TimingSessionStartError};
use super::{BenchmarkObservationMetric, BenchmarkObservationSnapshot, TimingObservation};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

struct ActiveCollection {
    id: TimingSessionId,
    configuration: TimingSessionConfiguration,
    timings: Vec<TimingObservation>,
    counters: Vec<BenchmarkObservationMetric>,
    boundaries: Vec<TimingBoundaryRecord>,
    modules: Vec<TimingModuleRecord>,
}

static ACTIVE_COLLECTOR: Mutex<Option<ActiveCollection>> = Mutex::new(None);

/// The result of attempting to retain one timing observation.
///
/// Keeping this structured inside timing avoids emitting benchmark output for
/// a stale attributed context, while ordinary facade callers still receive
/// the compact suppression flag they need for human prose.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TimingRecordOutcome {
    pub(crate) recorded: bool,
    pub(crate) output_suppressed: bool,
}

impl TimingRecordOutcome {
    const fn recorded(output_suppressed: bool) -> Self {
        Self {
            recorded: true,
            output_suppressed,
        }
    }
}

/// The result of recording several metrics under one collector lock.
///
/// The facade uses the captured generation after releasing the lock to emit
/// benchmark lines only for entries the collector accepted. This keeps stale
/// contexts out of both the snapshot and terminal output without allocating a
/// per-entry outcome buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TimingMultiRecordOutcome {
    session: Option<TimingSessionId>,
    attribution_enabled: bool,
    pub(crate) output_suppressed: bool,
}

impl TimingMultiRecordOutcome {
    const fn recorded(
        session: TimingSessionId,
        attribution_enabled: bool,
        output_suppressed: bool,
    ) -> Self {
        Self {
            session: Some(session),
            attribution_enabled,
            output_suppressed,
        }
    }

    /// Whether this outcome retained the entry with its supplied context.
    pub(crate) fn recorded_entry(self, context: Option<TimingContext>) -> bool {
        let Some(session) = self.session else {
            return false;
        };

        if !self.attribution_enabled {
            return true;
        }

        match context {
            Some(context) => context.session() == session,
            None => true,
        }
    }
}

/// Recover the collector lock after poisoning instead of returning empty data.
///
/// The collector is pure bookkeeping: a previous panic must not silently
/// erase later observations. No code panics while holding this lock.
fn lock_collector() -> MutexGuard<'static, Option<ActiveCollection>> {
    #[cfg(test)]
    COLLECTOR_LOCK_ACQUISITIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    ACTIVE_COLLECTOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Try to start a new collection session without replacing an existing one.
///
/// Command startup may deliberately turn this error into an inactive session
/// because a test can own a surrounding raw collector. Explicit raw benchmark
/// APIs instead surface the error before entering compiler work.
pub(crate) fn try_start_session(
    command: Option<TimingCommandKind>,
    configuration: TimingSessionConfiguration,
) -> Result<TimingSession, TimingSessionStartError> {
    debug_assert!(configuration.has_collection());

    let mut guard = lock_collector();
    if guard.is_some() {
        return Err(TimingSessionStartError::CollectorBusy);
    }

    let id = super::session::next_session_id();
    *guard = Some(ActiveCollection {
        id,
        configuration,
        timings: Vec::new(),
        counters: Vec::new(),
        boundaries: Vec::new(),
        modules: Vec::new(),
    });
    runtime::activate_session(configuration);

    Ok(TimingSession::active(id, command, configuration))
}

/// Drain the active scope only when it belongs to the given session.
pub(crate) fn finish_session(id: TimingSessionId) -> BenchmarkObservationSnapshot {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_ref() else {
        return BenchmarkObservationSnapshot::default();
    };
    if collection.id != id {
        return BenchmarkObservationSnapshot::default();
    }

    let collection = guard.take().expect("active collection present");
    // Clear the fast-path bits before another session can acquire the
    // lifecycle lock, so an old finish can never disable a newer session.
    runtime::deactivate_session();
    drop(guard);

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
        // This runs under the lifecycle lock for the same reason as finish.
        runtime::deactivate_session();
    }
}

/// Record one un-attributed timing observation.
pub(crate) fn record_timing(name: &'static str, duration: Duration) -> TimingRecordOutcome {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return TimingRecordOutcome::default();
    };
    if !collection.configuration.channels().metrics() {
        return TimingRecordOutcome::default();
    }

    collection.timings.push(TimingObservation {
        name,
        duration,
        context: None,
    });
    TimingRecordOutcome::recorded(collection.configuration.suppress_output())
}

/// Record one timing observation with compact boundary/module context.
///
/// Attribution-off sessions keep the metric but deliberately discard its
/// context. A stale context rejects the entire observation, including any
/// later benchmark-line emission by the facade.
pub(crate) fn record_attributed_timing(
    name: &'static str,
    duration: Duration,
    context: Option<TimingContext>,
) -> TimingRecordOutcome {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return TimingRecordOutcome::default();
    };
    if !collection.configuration.channels().metrics() {
        return TimingRecordOutcome::default();
    }

    let context = if collection.configuration.channels().attribution() {
        match context {
            Some(context) if context.session() != collection.id => {
                return TimingRecordOutcome::default();
            }
            context => context,
        }
    } else {
        None
    };

    collection.timings.push(TimingObservation {
        name,
        duration,
        context,
    });
    TimingRecordOutcome::recorded(collection.configuration.suppress_output())
}

/// Record several timing observations while holding the collector lock once.
///
/// The caller receives the active generation needed to emit stable benchmark
/// lines after releasing the lock. Entries with stale contexts are skipped,
/// matching single-record behaviour without adding a temporary allocation.
pub(crate) fn record_attributed_timing_multi(
    entries: &[(&'static str, Option<TimingContext>)],
    duration: Duration,
) -> TimingMultiRecordOutcome {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return TimingMultiRecordOutcome::default();
    };
    if !collection.configuration.channels().metrics() {
        return TimingMultiRecordOutcome::default();
    }

    let attribution_enabled = collection.configuration.channels().attribution();
    let outcome = TimingMultiRecordOutcome::recorded(
        collection.id,
        attribution_enabled,
        collection.configuration.suppress_output(),
    );

    for &(name, context) in entries {
        let context = if attribution_enabled {
            match context {
                Some(context) if context.session() != collection.id => continue,
                context => context,
            }
        } else {
            None
        };

        collection.timings.push(TimingObservation {
            name,
            duration,
            context,
        });
    }

    outcome
}

/// Register one compilation boundary inside the active attributed session.
///
/// The facade checks the atomic attribution bit before entering here, but the
/// configuration check keeps the lifecycle owner correct if a finish races a
/// registration call.
pub(crate) fn register_boundary(
    kind: TimingBoundaryKind,
    display_name: impl FnOnce() -> String,
) -> TimingBoundaryId {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return NO_TIMING_BOUNDARY;
    };
    if !collection.configuration.channels().attribution() {
        return NO_TIMING_BOUNDARY;
    }

    let id = TimingBoundaryId::from_session(collection.id, collection.boundaries.len() as u32);
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
/// ignored and return the existing key, keeping records deterministic.
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
    if !collection.configuration.channels().attribution() || boundary.session() != collection.id {
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

/// Record one counter observation when the counter channel is active.
pub(crate) fn record_counter(name: &'static str, value: f64) -> TimingRecordOutcome {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return TimingRecordOutcome::default();
    };
    if !collection.configuration.channels().counters() {
        return TimingRecordOutcome::default();
    }

    collection
        .counters
        .push(BenchmarkObservationMetric { name, value });
    TimingRecordOutcome::recorded(collection.configuration.suppress_output())
}

/// Whether stdout output is currently allowed.
///
/// Returns false when an in-process collection session has suppressed output.
/// It first checks the active-channel bit so ordinary inactive call sites do
/// not take the collector mutex.
pub(crate) fn output_enabled() -> bool {
    if !runtime::collection_active() {
        return true;
    }

    match lock_collector().as_ref() {
        Some(collection) => !collection.configuration.suppress_output(),
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

#[cfg(test)]
use std::sync::atomic::AtomicUsize;

#[cfg(test)]
static COLLECTOR_LOCK_ACQUISITIONS: AtomicUsize = AtomicUsize::new(0);

/// Reset the test-only collector lock counter.
#[cfg(test)]
pub(crate) fn reset_lock_acquisitions_for_test() {
    COLLECTOR_LOCK_ACQUISITIONS.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Return the test-only collector lock acquisition count.
#[cfg(test)]
pub(crate) fn lock_acquisitions_for_test() -> usize {
    COLLECTOR_LOCK_ACQUISITIONS.load(std::sync::atomic::Ordering::Relaxed)
}
