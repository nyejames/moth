//! In-memory timing and counter observation collector.
//!
//! WHAT: owns the lifecycle mutex and dense atomic timing storage for exactly
//! one active collection session.
//! WHY: stage recording must not allocate, format or enter a collector mutex;
//! lifecycle and metadata registration remain serialized while global and
//! attributed totals use schema-indexed atomic slots.

use super::attribution::{
    NO_TIMING_BOUNDARY, TimingBoundaryId, TimingBoundaryKind, TimingBoundaryRecord, TimingContext,
    TimingMetricAccumulator, TimingModuleKey, TimingModuleRecord, acquire_boundary_accumulator,
    acquire_module_accumulator,
};
use super::runtime::{self, TimingSessionConfiguration};
use super::schema::{TIMING_METRIC_COUNT, TIMING_SCHEMA_VERSION, TimingAttributionKind};
use super::session::{TimingCommandKind, TimingSession, TimingSessionId, TimingSessionStartError};
use super::{
    BenchmarkObservationMetric, BenchmarkObservationSnapshot, TimingMetric, TimingMetricAggregate,
};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

struct ActiveCollection {
    id: TimingSessionId,
    configuration: TimingSessionConfiguration,
    command: Option<TimingCommandKind>,
    counters: Vec<BenchmarkObservationMetric>,
    boundaries: Vec<TimingBoundaryRecord>,
    modules: Vec<TimingModuleRecord>,
}

static ACTIVE_COLLECTOR: Mutex<Option<ActiveCollection>> = Mutex::new(None);
static GLOBAL_METRICS: [TimingMetricAccumulator; TIMING_METRIC_COUNT] =
    [const { TimingMetricAccumulator::new() }; TIMING_METRIC_COUNT];

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

/// The result of recording several metrics under one admission window.
///
/// The facade uses the captured generation to reject stale contexts without
/// allocating a per-entry outcome buffer. Stable benchmark lines are emitted
/// from the completed session snapshot rather than this record path.
// This result is consumed by the multi-metric facade path, whose call sites
// come from macro expansion rather than this library target.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TimingMultiRecordOutcome {
    session: Option<TimingSessionId>,
    attribution_enabled: bool,
    pub(crate) output_suppressed: bool,
}

#[allow(dead_code)]
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
    pub(crate) fn recorded_entry(
        self,
        metric: TimingMetric,
        context: Option<TimingContext>,
    ) -> bool {
        let Some(session) = self.session else {
            return false;
        };

        accepts_attribution(metric, context, session.raw(), self.attribution_enabled)
    }
}

/// Recover the collector lock after poisoning instead of returning empty data.
///
/// The collector is pure bookkeeping: a previous panic must not silently
/// erase later observations. Internal invariant checks may panic while this
/// lock is held, so recovery preserves later bookkeeping after the mutex is
/// poisoned instead of silently discarding it.
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
        command,
        counters: Vec::new(),
        boundaries: Vec::new(),
        modules: Vec::new(),
    });
    reset_global_metrics();
    runtime::activate_session(id.raw(), configuration);

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

    // Clear the fast-path bits before another session can acquire the
    // lifecycle lock, then wait for every already-admitted record before
    // extracting the dense slots. The lifecycle lock stays held throughout
    // this handoff, so another session cannot reset the slots early.
    runtime::deactivate_session();
    runtime::wait_for_records();
    let collection = guard.take().expect("active collection present");

    let mut snapshot = BenchmarkObservationSnapshot {
        schema_version: TIMING_SCHEMA_VERSION,
        command: collection.command,
        timings: snapshot_global_metrics(),
        counters: collection.counters,
        boundaries: collection.boundaries,
        modules: collection.modules,
    };
    for boundary in &mut snapshot.boundaries {
        if let Some(accumulator) = boundary.id.accumulator() {
            boundary.timings = accumulator.snapshot(TimingAttributionKind::Boundary);
        }
    }
    for module in &mut snapshot.modules {
        if let Some(accumulator) = module.key.accumulator() {
            module.timings = accumulator.snapshot(TimingAttributionKind::Module);
        }
    }
    recompute_boundary_module_counts(&mut snapshot);
    snapshot
}

/// Drop an unfinished session's active scope without returning observations.
///
/// Called from `TimingSession::drop`; only the matching session is removed.
pub(crate) fn abandon_session(id: TimingSessionId) {
    let mut guard = lock_collector();
    if guard.as_ref().is_some_and(|collection| collection.id == id) {
        // This runs under the lifecycle lock for the same reason as finish.
        runtime::deactivate_session();
        runtime::wait_for_records();
        *guard = None;
    }
}

/// Record one un-attributed timing aggregate without entering the lifecycle
/// collector mutex.
pub(crate) fn record_timing(metric: TimingMetric, duration: Duration) -> TimingRecordOutcome {
    let Some(_session) = runtime::begin_record() else {
        return TimingRecordOutcome::default();
    };
    if !runtime::metrics_active() {
        runtime::end_record();
        return TimingRecordOutcome::default();
    }

    GLOBAL_METRICS[metric.index()].record(duration);
    let output_suppressed = runtime::output_suppressed();
    runtime::end_record();
    TimingRecordOutcome::recorded(output_suppressed)
}

/// Record one timing observation with compact boundary/module context.
///
/// Attribution-off sessions keep the metric but deliberately discard its
/// context. A stale or wrong-kind context rejects the entire observation,
/// including its eventual final benchmark aggregate.
pub(crate) fn record_attributed_timing(
    metric: TimingMetric,
    duration: Duration,
    context: Option<TimingContext>,
) -> TimingRecordOutcome {
    let Some(session) = runtime::begin_record() else {
        return TimingRecordOutcome::default();
    };
    if !runtime::metrics_active() {
        runtime::end_record();
        return TimingRecordOutcome::default();
    }

    let attribution_enabled = runtime::attribution_active();
    if !accepts_attribution(metric, context, session, attribution_enabled) {
        runtime::end_record();
        return TimingRecordOutcome::default();
    }

    GLOBAL_METRICS[metric.index()].record(duration);
    if attribution_enabled && let Some(context) = context {
        context
            .accumulator()
            .expect("accepted attributed context has storage")
            .record(metric, duration);
    }
    let output_suppressed = runtime::output_suppressed();
    runtime::end_record();
    TimingRecordOutcome::recorded(output_suppressed)
}

/// Record several timing aggregates under one lock-free admission window.
///
/// Entries with stale contexts are skipped, matching single-record behaviour
/// without adding a temporary allocation.
#[allow(dead_code)]
pub(crate) fn record_attributed_timing_multi(
    entries: &[(TimingMetric, Option<TimingContext>)],
    duration: Duration,
) -> TimingMultiRecordOutcome {
    let Some(session_raw) = runtime::begin_record() else {
        return TimingMultiRecordOutcome::default();
    };
    if !runtime::metrics_active() {
        runtime::end_record();
        return TimingMultiRecordOutcome::default();
    }

    let attribution_enabled = runtime::attribution_active();
    let outcome = TimingMultiRecordOutcome::recorded(
        TimingSessionId::from_raw(session_raw),
        attribution_enabled,
        runtime::output_suppressed(),
    );

    for &(metric, context) in entries {
        if !accepts_attribution(metric, context, session_raw, attribution_enabled) {
            continue;
        }
        GLOBAL_METRICS[metric.index()].record(duration);
        if attribution_enabled && let Some(context) = context {
            context
                .accumulator()
                .expect("accepted attributed context has storage")
                .record(metric, duration);
        }
    }

    runtime::end_record();
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

    let index = collection.boundaries.len();
    let accumulator = acquire_boundary_accumulator(index);
    let id = TimingBoundaryId::with_accumulator(collection.id, index as u32, accumulator);
    collection.boundaries.push(TimingBoundaryRecord {
        id,
        kind,
        display_name: display_name(),
        module_count: 0,
        timings: Vec::new(),
    });
    id
}

/// Register one fully described module inside a boundary and return its dense
/// key.
///
/// The module index is the boundary's graph-owned dense `ModuleId`, so the
/// same index in two boundaries stays distinct. Exact duplicate registrations
/// return the existing key. Conflicting logical identity or source metadata
/// is an internal invariant failure and never mutates the existing record.
pub(crate) fn register_module(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_module_path: &str,
    source_file_count: u64,
    source_byte_count: u64,
) -> TimingModuleKey {
    register_module_with_metadata(
        boundary,
        module_index,
        logical_module_path,
        source_file_count,
        source_byte_count,
        true,
    )
}

/// Register a module before Stage 0 has finished collecting its source facts.
///
/// The preparation lifecycle is explicit so it cannot be mistaken for a
/// conflicting duplicate of a complete registration. Call
/// [`finalize_module_source_facts`] exactly once the prepared module is known.
pub(crate) fn register_module_for_preparation(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_module_path: &str,
) -> TimingModuleKey {
    register_module_with_metadata(boundary, module_index, logical_module_path, 0, 0, false)
}

fn register_module_with_metadata(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_module_path: &str,
    source_file_count: u64,
    source_byte_count: u64,
    source_facts_finalized: bool,
) -> TimingModuleKey {
    let fallback_key = TimingModuleKey::new(boundary, module_index);

    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        return fallback_key;
    };
    if !collection.configuration.channels().attribution() || boundary.session() != collection.id {
        return fallback_key;
    }
    let Some(boundary_record) = collection.boundaries.get(boundary.index()) else {
        return fallback_key;
    };
    if boundary_record.id != boundary
        || !same_accumulator(boundary_record.id.accumulator(), boundary.accumulator())
    {
        return fallback_key;
    }
    let logical_identity = if logical_module_path.is_empty() {
        boundary_record.display_name.clone()
    } else {
        format!("{}/{}", boundary_record.display_name, logical_module_path)
    };
    if let Some(index) = collection.modules.iter().position(|record| {
        record.key.boundary() == boundary && record.key.module_index() == module_index
    }) {
        let record = &collection.modules[index];
        assert_eq!(
            record.logical_identity, logical_identity,
            "timing module registration changed logical identity"
        );
        assert_eq!(
            record.source_file_count, source_file_count,
            "timing module registration changed source file count"
        );
        assert_eq!(
            record.source_byte_count, source_byte_count,
            "timing module registration changed source byte count"
        );
        assert_eq!(
            record.source_facts_finalized, source_facts_finalized,
            "timing module registration changed source-fact lifecycle"
        );
        return record.key;
    }

    let accumulator = acquire_module_accumulator(collection.modules.len());
    let key = TimingModuleKey::with_accumulator(boundary, module_index, accumulator);
    collection.modules.push(TimingModuleRecord {
        key,
        logical_identity,
        source_file_count,
        source_byte_count,
        source_facts_finalized,
        timings: Vec::new(),
    });
    collection.boundaries[boundary.index()].module_count += 1;
    key
}

/// Finalize the source facts for a module registered before preparation.
///
/// Exact repeated finalization is idempotent. A conflicting finalization is an
/// internal invariant failure and is checked before any record mutation.
pub(crate) fn finalize_module_source_facts(
    key: TimingModuleKey,
    source_file_count: u64,
    source_byte_count: u64,
) {
    let mut guard = lock_collector();
    let Some(collection) = guard.as_mut() else {
        // Preparation can finish after the owning collection has drained.
        // The callback belongs to that completed session and must not panic
        // or affect a later collection.
        return;
    };
    // Preparation can outlive the collection that registered the key. A new
    // collection may already be active when that stale callback arrives, so
    // its old session generation must not mutate or panic in the new report.
    let Some(record) = collection
        .modules
        .iter_mut()
        .find(|record| record.key == key)
    else {
        return;
    };

    if record.source_facts_finalized {
        assert_eq!(
            record.source_file_count, source_file_count,
            "timing module finalization changed source file count"
        );
        assert_eq!(
            record.source_byte_count, source_byte_count,
            "timing module finalization changed source byte count"
        );
        return;
    }

    assert_eq!(
        record.source_file_count, 0,
        "timing module placeholder has non-zero source file count"
    );
    assert_eq!(
        record.source_byte_count, 0,
        "timing module placeholder has non-zero source byte count"
    );
    record.source_file_count = source_file_count;
    record.source_byte_count = source_byte_count;
    record.source_facts_finalized = true;
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
    let mut counts = std::collections::BTreeMap::<(TimingSessionId, usize), u64>::new();
    for record in &snapshot.modules {
        let boundary = record.key.boundary();
        *counts
            .entry((boundary.session(), boundary.index()))
            .or_default() += 1;
    }
    for boundary in &mut snapshot.boundaries {
        boundary.module_count = counts
            .get(&(boundary.id.session(), boundary.id.index()))
            .copied()
            .unwrap_or(0);
    }
}

/// Validate the typed context against the metric's schema attribution kind.
///
/// When attribution is disabled the caller intentionally discards context and
/// records only the global slot. When it is enabled, a context must name the
/// current session, carry registered storage and match the metric's declared
/// boundary/module kind; this keeps invalid attribution out of both global and
/// attributed aggregates.
fn accepts_attribution(
    metric: TimingMetric,
    context: Option<TimingContext>,
    session_raw: u64,
    attribution_enabled: bool,
) -> bool {
    if !attribution_enabled {
        return true;
    }

    // A caller may intentionally record a typed metric without requesting an
    // attribution row. Keep that global evidence while reserving attributed
    // slots for a registered context of the metric's declared kind.
    if context.is_none() {
        return true;
    }

    let session = TimingSessionId::from_raw(session_raw);
    match metric.descriptor().attribution {
        TimingAttributionKind::None => context.is_none(),
        TimingAttributionKind::Boundary => matches!(
            context,
            Some(TimingContext::Boundary(boundary))
                if boundary.session() == session
                    && boundary.accumulator().is_some()
        ),
        TimingAttributionKind::Module => matches!(
            context,
            Some(TimingContext::Module(module))
                if module.boundary().session() == session
                    && module.accumulator().is_some()
        ),
    }
}

fn same_accumulator(
    left: Option<&'static super::attribution::TimingAttributionAccumulator>,
    right: Option<&'static super::attribution::TimingAttributionAccumulator>,
) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => std::ptr::eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn reset_global_metrics() {
    for metric in &GLOBAL_METRICS {
        metric.reset();
    }
}

fn snapshot_global_metrics() -> Vec<TimingMetricAggregate> {
    TimingMetric::ALL
        .iter()
        .copied()
        .map(|metric| GLOBAL_METRICS[metric.index()].snapshot(metric))
        .collect()
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
