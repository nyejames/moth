//! Compact command-local boundary and module attribution identities.
//!
//! WHAT: owns the session-scoped dense ids, registered boundary/module
//!      records and the typed attribution context passed through build
//!      orchestration while `timers` is active.
//! WHY:  attribution must be deterministic, keyed by graph order and scoped
//!       to one collection session. Every id carries its session generation,
//!       so a stale context from an older command can never attach to a newer
//!       report. These types exist only when `timers` is selected, so no
//!       timing-only field or ABI argument survives in a no-timer build.
//!
//! Boundary records are registered by the build system in deterministic
//! package order, then the main project; module records are registered inside
//! each boundary's compile call in deterministic wave order. Timing
//! observations carry only the compact ids, never paths or labels.

use super::TimingMetricAggregate;
use super::schema::{TIMING_METRIC_COUNT, TimingAttributionKind, TimingMetric};
use super::session::TimingSessionId;
use std::cmp::Ordering as CmpOrdering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Dense handle for one compilation boundary inside one command session.
///
/// The session generation makes the handle unrepresentable outside its owning
/// collection; the index is a registration-table slot.
#[derive(Clone, Copy)]
pub(crate) struct TimingBoundaryId {
    session: TimingSessionId,
    index: u32,
    accumulator: Option<&'static TimingAttributionAccumulator>,
}

impl fmt::Debug for TimingBoundaryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimingBoundaryId")
            .field("session", &self.session)
            .field("index", &self.index)
            .finish()
    }
}

impl PartialEq for TimingBoundaryId {
    fn eq(&self, other: &Self) -> bool {
        self.session == other.session && self.index == other.index
    }
}

impl Eq for TimingBoundaryId {}

impl Hash for TimingBoundaryId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.session.hash(state);
        self.index.hash(state);
    }
}

impl Ord for TimingBoundaryId {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.session
            .cmp(&other.session)
            .then(self.index.cmp(&other.index))
    }
}

impl PartialOrd for TimingBoundaryId {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

/// Sentinel id returned when no collection session is active.
///
/// A compile that starts before a session can later finish inside one; the
/// sentinel's session generation never matches a live session, so its late
/// observations and registrations are dropped instead of polluting the first
/// active session.
pub(crate) const NO_TIMING_BOUNDARY: TimingBoundaryId = TimingBoundaryId {
    session: TimingSessionId::from_raw(u64::MAX),
    index: u32::MAX,
    accumulator: None,
};

impl TimingBoundaryId {
    /// Build a boundary id from its owning session and table slot.
    #[cfg(test)]
    pub(crate) fn from_session(session: TimingSessionId, index: u32) -> Self {
        Self {
            session,
            index,
            accumulator: None,
        }
    }

    /// Build a registered boundary id with its lock-free attribution storage.
    pub(crate) fn with_accumulator(
        session: TimingSessionId,
        index: u32,
        accumulator: &'static TimingAttributionAccumulator,
    ) -> Self {
        Self {
            session,
            index,
            accumulator: Some(accumulator),
        }
    }

    /// The registration-table slot for this boundary.
    pub(crate) fn index(self) -> usize {
        self.index as usize
    }

    /// The session generation that owns this boundary.
    pub(crate) fn session(self) -> TimingSessionId {
        self.session
    }

    /// The dense attributed storage owned by this registered boundary.
    pub(crate) fn accumulator(self) -> Option<&'static TimingAttributionAccumulator> {
        self.accumulator
    }
}

/// Which compilation boundary kind a record represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingBoundaryKind {
    /// A source-backed package boundary, for example `@html`.
    SourcePackage,
    /// The main project boundary, named from `Config.project_name`.
    MainProject,
}

/// One registered compilation boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimingBoundaryRecord {
    pub(crate) id: TimingBoundaryId,
    pub(crate) kind: TimingBoundaryKind,
    /// Display name such as `@html` or `moth_docs`.
    pub(crate) display_name: String,
    /// Number of modules registered in this boundary.
    pub(crate) module_count: u64,
    /// Dense boundary-attributed metrics in schema order for this kind.
    pub(crate) timings: Vec<TimingMetricAggregate>,
}

/// Dense module key inside one boundary.
///
/// `module_index` is the boundary's graph-owned dense `ModuleId`, so two
/// boundaries never collide even when both contain a module with index zero.
#[derive(Clone, Copy)]
pub(crate) struct TimingModuleKey {
    boundary: TimingBoundaryId,
    module_index: u32,
    accumulator: Option<&'static TimingAttributionAccumulator>,
}

impl fmt::Debug for TimingModuleKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimingModuleKey")
            .field("boundary", &self.boundary)
            .field("module_index", &self.module_index)
            .finish()
    }
}

impl PartialEq for TimingModuleKey {
    fn eq(&self, other: &Self) -> bool {
        self.boundary == other.boundary && self.module_index == other.module_index
    }
}

impl Eq for TimingModuleKey {}

impl Hash for TimingModuleKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.boundary.hash(state);
        self.module_index.hash(state);
    }
}

impl Ord for TimingModuleKey {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.boundary
            .cmp(&other.boundary)
            .then(self.module_index.cmp(&other.module_index))
    }
}

impl PartialOrd for TimingModuleKey {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl TimingModuleKey {
    /// Build a module key inside a registered boundary.
    pub(crate) fn new(boundary: TimingBoundaryId, module_index: u32) -> Self {
        Self {
            boundary,
            module_index,
            accumulator: None,
        }
    }

    /// Build a registered module key with its lock-free attribution storage.
    pub(crate) fn with_accumulator(
        boundary: TimingBoundaryId,
        module_index: u32,
        accumulator: &'static TimingAttributionAccumulator,
    ) -> Self {
        Self {
            boundary,
            module_index,
            accumulator: Some(accumulator),
        }
    }

    /// The owning boundary.
    pub(crate) fn boundary(self) -> TimingBoundaryId {
        self.boundary
    }

    /// The boundary's dense module index.
    pub(crate) fn module_index(self) -> u32 {
        self.module_index
    }

    /// The dense attributed storage owned by this registered module.
    pub(crate) fn accumulator(self) -> Option<&'static TimingAttributionAccumulator> {
        self.accumulator
    }
}

/// One registered module with its logical display identity and source facts.
///
/// The identity is derived from the stable module origin (portable logical
/// module path), never from absolute filesystem paths, so it cannot leak a
/// checkout-specific prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimingModuleRecord {
    pub(crate) key: TimingModuleKey,
    pub(crate) logical_identity: String,
    pub(crate) source_file_count: u64,
    pub(crate) source_byte_count: u64,
    /// Whether the source facts were finalized after Stage 0 preparation.
    pub(crate) source_facts_finalized: bool,
    /// Dense module-attributed metrics in schema order for this kind.
    pub(crate) timings: Vec<TimingMetricAggregate>,
}

/// Explicit attribution context passed through compilation.
///
/// The boundary variant names boundary-level work; the module variant names
/// per-module frontend work. Passing the ids explicitly keeps attribution
/// independent of thread scheduling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimingContext {
    /// Work owned by one compilation boundary.
    Boundary(TimingBoundaryId),
    /// Work owned by one module inside a boundary.
    Module(TimingModuleKey),
}

impl TimingContext {
    /// Context for a boundary-level observation.
    pub(crate) fn for_boundary(boundary: TimingBoundaryId) -> Self {
        Self::Boundary(boundary)
    }

    /// Context for one module observation.
    pub(crate) fn for_module(key: TimingModuleKey) -> Self {
        Self::Module(key)
    }

    /// The lock-free attribution storage named by this context.
    pub(crate) fn accumulator(self) -> Option<&'static TimingAttributionAccumulator> {
        match self {
            Self::Boundary(boundary) => boundary.accumulator(),
            Self::Module(module) => module.accumulator(),
        }
    }
}

/// Dense atomic timing slots owned by one boundary or module identity.
///
/// Registration allocates or reuses these tables. Recording only follows the
/// pointer carried by the typed context and performs relaxed atomic updates;
/// it never formats, allocates or enters the lifecycle collector mutex.
pub(crate) struct TimingAttributionAccumulator {
    metrics: [TimingMetricAccumulator; TIMING_METRIC_COUNT],
}

impl TimingAttributionAccumulator {
    pub(crate) const fn new() -> Self {
        Self {
            metrics: [const { TimingMetricAccumulator::new() }; TIMING_METRIC_COUNT],
        }
    }

    pub(crate) fn reset(&self) {
        for metric in &self.metrics {
            metric.reset();
        }
    }

    pub(crate) fn record(&self, metric: TimingMetric, duration: Duration) {
        self.metrics[metric.index()].record(duration);
    }

    pub(crate) fn snapshot(&self, kind: TimingAttributionKind) -> Vec<TimingMetricAggregate> {
        TimingMetric::ALL
            .iter()
            .copied()
            .filter(|metric| metric.descriptor().attribution == kind)
            .map(|metric| self.metrics[metric.index()].snapshot(metric))
            .collect()
    }
}

/// One dense atomic total/sample pair.
pub(crate) struct TimingMetricAccumulator {
    total_nanos: AtomicU64,
    samples: AtomicU64,
}

impl TimingMetricAccumulator {
    pub(crate) const fn new() -> Self {
        Self {
            total_nanos: AtomicU64::new(0),
            samples: AtomicU64::new(0),
        }
    }

    pub(crate) fn reset(&self) {
        self.total_nanos.store(0, Ordering::Relaxed);
        self.samples.store(0, Ordering::Relaxed);
    }

    pub(crate) fn record(&self, duration: Duration) {
        let nanos = duration.as_nanos().min(u64::MAX as u128) as u64;
        self.total_nanos
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(nanos))
            })
            .expect("the timing total update always returns a value");
        self.samples.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self, metric: TimingMetric) -> TimingMetricAggregate {
        TimingMetricAggregate {
            metric,
            total: Duration::from_nanos(self.total_nanos.load(Ordering::Relaxed)),
            samples: self.samples.load(Ordering::Relaxed),
        }
    }
}

static BOUNDARY_ACCUMULATORS: Mutex<Vec<&'static TimingAttributionAccumulator>> =
    Mutex::new(Vec::new());
static MODULE_ACCUMULATORS: Mutex<Vec<&'static TimingAttributionAccumulator>> =
    Mutex::new(Vec::new());

fn acquire_accumulator(
    pool: &Mutex<Vec<&'static TimingAttributionAccumulator>>,
    slot: usize,
) -> &'static TimingAttributionAccumulator {
    let mut pool = pool.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    while pool.len() <= slot {
        pool.push(Box::leak(Box::new(TimingAttributionAccumulator::new())));
    }
    let accumulator = pool[slot];
    accumulator.reset();
    accumulator
}

/// Acquire the reusable accumulator for one boundary registration slot.
pub(crate) fn acquire_boundary_accumulator(slot: usize) -> &'static TimingAttributionAccumulator {
    acquire_accumulator(&BOUNDARY_ACCUMULATORS, slot)
}

/// Acquire the reusable accumulator for one module registration slot.
pub(crate) fn acquire_module_accumulator(slot: usize) -> &'static TimingAttributionAccumulator {
    acquire_accumulator(&MODULE_ACCUMULATORS, slot)
}
