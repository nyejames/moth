//! In-memory timing and counter observation collector.
//!
//! WHAT: owns the lifecycle mutex and dense atomic timing storage for exactly
//! one active collection session.
//! WHY: stage recording must not allocate, format or enter a collector mutex;
//! lifecycle and metadata registration remain serialized while global and
//! attributed totals use schema-indexed atomic slots.

#[cfg(feature = "benchmark_counters")]
use super::BenchmarkObservationMetric;
use super::attribution::{
    NO_TIMING_BOUNDARY, TimingBoundaryId, TimingBoundaryKind, TimingBoundaryRecord, TimingContext,
    TimingMetricAccumulator, TimingModuleKey, TimingModuleRecord, acquire_boundary_accumulator,
    acquire_module_accumulator,
};
use super::runtime::{self, TimingRecordAdmission, TimingSessionConfiguration};
use super::schema::{TIMING_METRIC_COUNT, TIMING_SCHEMA_VERSION, TimingAttributionKind};
use super::session::{TimingCommandKind, TimingSession, TimingSessionId, TimingSessionStartError};
use super::{BenchmarkObservationSnapshot, TimingMetric, TimingMetricAggregate};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

struct ActiveCollection {
    id: TimingSessionId,
    configuration: TimingSessionConfiguration,
    command: Option<TimingCommandKind>,
    #[cfg(feature = "benchmark_counters")]
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
/// the compact suppression flag they need.
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
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: Vec::new(),
        modules: Vec::new(),
    });
    reset_global_metrics();
    runtime::activate_session(id.raw(), command, configuration);

    Ok(TimingSession::active(id, command, configuration))
}

/// Drain the active scope only when it belongs to the given session.
pub(crate) fn finish_session(id: TimingSessionId) -> BenchmarkObservationSnapshot {
    let guard = lock_collector();
    let Some(collection) = guard.as_ref() else {
        return BenchmarkObservationSnapshot::default();
    };
    if collection.id != id {
        return BenchmarkObservationSnapshot::default();
    }

    // Clear the fast-path bits while this session still owns the collector. Keep the active
    // collection installed so a competing start remains busy, but release the mutex before
    // waiting: an admitted timed expression may still need collector metadata during its body.
    // Holding the mutex here would deadlock that recorder against the drain.
    runtime::deactivate_session();
    drop(guard);
    runtime::wait_for_records();

    let mut guard = lock_collector();
    if guard.as_ref().is_none_or(|collection| collection.id != id) {
        return BenchmarkObservationSnapshot::default();
    }
    let collection = guard.take().expect("active collection present");
    let command = collection.command;
    let configuration = collection.configuration;

    let mut snapshot = BenchmarkObservationSnapshot {
        schema_version: TIMING_SCHEMA_VERSION,
        command,
        timings: snapshot_global_metrics(),
        #[cfg(feature = "benchmark_counters")]
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
    validate_snapshot_invariants(&snapshot, command, configuration);
    snapshot
}

/// Drop an unfinished session's active scope without returning observations.
///
/// Called from `TimingSession::drop`; only the matching session is removed.
pub(crate) fn abandon_session(id: TimingSessionId) {
    let mut guard = lock_collector();
    if guard.as_ref().is_some_and(|collection| collection.id == id) {
        runtime::deactivate_session();
        drop(guard);
        runtime::wait_for_records();

        guard = lock_collector();
        if guard.as_ref().is_some_and(|collection| collection.id == id) {
            *guard = None;
        }
    }
}

/// Record one un-attributed timing aggregate without entering the lifecycle
/// collector mutex.
///
/// This is the already-captured-duration macro endpoint; its current
/// production consumers are macro expansions, which rustc does not count as
/// reachability for this private helper.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn record_timing(metric: TimingMetric, duration: Duration) -> TimingRecordOutcome {
    let Some(admission) = runtime::begin_metric_record(metric) else {
        return TimingRecordOutcome::default();
    };
    record_timing_with_admission(metric, duration, admission)
}

/// Record one timing observation using admission retained from before its
/// clock started. The admission remains live until this function returns.
pub(crate) fn record_timing_with_admission(
    metric: TimingMetric,
    duration: Duration,
    admission: TimingRecordAdmission,
) -> TimingRecordOutcome {
    if !admission.metric_active(metric)
        || !accepts_attribution(
            metric,
            None,
            admission.session(),
            admission.attribution_active(),
        )
    {
        return TimingRecordOutcome::default();
    }

    GLOBAL_METRICS[metric.index()].record(duration);
    let output_suppressed = admission.output_suppressed();
    TimingRecordOutcome::recorded(output_suppressed)
}

/// Record one timing observation with compact boundary/module context.
///
/// Attribution-off sessions keep the metric but deliberately discard its
/// context. A stale or wrong-kind context rejects the entire observation,
/// including its eventual final benchmark aggregate.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn record_attributed_timing(
    metric: TimingMetric,
    duration: Duration,
    context: Option<TimingContext>,
) -> TimingRecordOutcome {
    let Some(admission) = runtime::begin_metric_record(metric) else {
        return TimingRecordOutcome::default();
    };
    record_attributed_timing_with_admission(metric, duration, context, admission)
}

/// Record an attributed timing observation using admission retained from
/// before its clock started.
pub(crate) fn record_attributed_timing_with_admission(
    metric: TimingMetric,
    duration: Duration,
    context: Option<TimingContext>,
    admission: TimingRecordAdmission,
) -> TimingRecordOutcome {
    if !admission.metric_active(metric)
        || !accepts_attribution(
            metric,
            context,
            admission.session(),
            admission.attribution_active(),
        )
    {
        return TimingRecordOutcome::default();
    }

    GLOBAL_METRICS[metric.index()].record(duration);
    if admission.attribution_active()
        && let Some(context) = context
    {
        context
            .accumulator()
            .expect("accepted attributed context has storage")
            .record(metric, duration);
    }
    let output_suppressed = admission.output_suppressed();
    TimingRecordOutcome::recorded(output_suppressed)
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
#[cfg(feature = "benchmark_counters")]
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

    let session = TimingSessionId::from_raw(session_raw);
    match (metric.descriptor().attribution, context) {
        (TimingAttributionKind::None, None) => true,
        (TimingAttributionKind::Boundary, Some(TimingContext::Boundary(boundary))) => {
            boundary.session() == session && boundary.accumulator().is_some()
        }
        (TimingAttributionKind::Module, Some(TimingContext::Module(module))) => {
            module.boundary().session() == session && module.accumulator().is_some()
        }
        _ => false,
    }
}

/// Validate the facts that make a typed snapshot safe to consume.
///
/// Admission rejects command-inapplicable metrics before they mutate any
/// aggregate. This final check remains a defensive invariant for unexpected
/// collector misuse. Attributed sessions must also preserve the accounting
/// identity: every global boundary/module aggregate is exactly the sum of its
/// corresponding registered slots.
fn validate_snapshot_invariants(
    snapshot: &BenchmarkObservationSnapshot,
    command: Option<TimingCommandKind>,
    configuration: TimingSessionConfiguration,
) {
    if let Some(command) = command {
        for aggregate in &snapshot.timings {
            if aggregate.samples > 0 {
                assert!(
                    aggregate.metric.applies_to(command),
                    "timing metric {} does not apply to command {:?}",
                    aggregate.metric.descriptor().stable_name,
                    command
                );
            }
        }
    }

    if !configuration.channels().attribution() {
        return;
    }

    for aggregate in snapshot.timings.iter().filter(|aggregate| {
        aggregate.metric.descriptor().attribution != TimingAttributionKind::None
    }) {
        let (total, samples) = match aggregate.metric.descriptor().attribution {
            TimingAttributionKind::None => unreachable!("filtered un-attributed metric"),
            TimingAttributionKind::Boundary => sum_attributed_rows(
                snapshot
                    .boundaries
                    .iter()
                    .flat_map(|boundary| boundary.timings.iter()),
                aggregate.metric,
            ),
            TimingAttributionKind::Module => sum_attributed_rows(
                snapshot
                    .modules
                    .iter()
                    .flat_map(|module| module.timings.iter()),
                aggregate.metric,
            ),
        };

        assert_eq!(
            aggregate.total,
            total,
            "global timing total for {} differs from attributed slots",
            aggregate.metric.descriptor().stable_name
        );
        assert_eq!(
            aggregate.samples,
            samples,
            "global timing samples for {} differ from attributed slots",
            aggregate.metric.descriptor().stable_name
        );
    }
}

fn sum_attributed_rows<'rows>(
    rows: impl Iterator<Item = &'rows TimingMetricAggregate>,
    metric: TimingMetric,
) -> (Duration, u64) {
    rows.filter(|aggregate| aggregate.metric == metric).fold(
        (Duration::ZERO, 0),
        |(total, samples), aggregate| {
            (
                total.saturating_add(aggregate.total),
                samples.saturating_add(aggregate.samples),
            )
        },
    )
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
