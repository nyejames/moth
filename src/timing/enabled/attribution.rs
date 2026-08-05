//! Compact command-local boundary and module attribution identities.
//!
//! WHAT: owns the dense ids, registered boundary/module records and the
//!      explicit context value passed through build orchestration while
//!      `timers` is active.
//! WHY:  attribution must be deterministic and keyed by graph order, never by
//!       worker completion or event insertion order. These types exist only
//!       when `timers` is selected, so no timing-only field or ABI argument
//!       survives in a no-timer build.
//!
//! Boundary records are registered by the build system in deterministic
//! package order, then the main project; module records are registered inside
//! each boundary's compile call in deterministic wave order. Timing
//! observations carry only the compact ids, never paths or labels.

/// Dense handle for one compilation boundary inside one command run.
///
/// The numeric value is a registration-table slot, never a persistent or
/// cross-command identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TimingBoundaryId(u32);

/// Sentinel id returned when no collection scope is active.
///
/// A compile that starts before a collection scope can later finish inside
/// one; this sentinel never matches a real registration slot, so its late
/// observations and registrations are dropped instead of polluting boundary
/// zero of the first active scope.
pub(crate) const NO_TIMING_BOUNDARY: TimingBoundaryId = TimingBoundaryId(u32::MAX);

impl TimingBoundaryId {
    /// Build a boundary id from its registration-table slot.
    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }

    /// The registration-table slot for this boundary.
    pub(crate) fn index(self) -> usize {
        self.0 as usize
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
}

/// Dense module key inside one boundary.
///
/// `module_index` is the boundary's graph-owned dense `ModuleId`, so two
/// boundaries never collide even when both contain a module with index zero.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TimingModuleKey {
    pub(crate) boundary: TimingBoundaryId,
    pub(crate) module_index: u32,
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
}

/// Explicit attribution context passed through compilation.
///
/// The boundary is always present for build-orchestration observations; the
/// module is present for per-module frontend observations. Passing the ids
/// explicitly keeps attribution independent of thread scheduling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TimingModuleContext {
    pub(crate) boundary: Option<TimingBoundaryId>,
    pub(crate) module: Option<TimingModuleKey>,
}

impl TimingModuleContext {
    /// Context for a boundary-level observation with no module attribution.
    pub(crate) fn for_boundary(boundary: TimingBoundaryId) -> Self {
        Self {
            boundary: Some(boundary),
            module: None,
        }
    }

    /// Context for one module observation.
    pub(crate) fn for_module(key: TimingModuleKey) -> Self {
        Self {
            boundary: Some(key.boundary),
            module: Some(key),
        }
    }
}

/// One module's timing label and compact context, bundled so frontend
/// signatures stay small.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TimingModuleAttribution<'a> {
    pub(crate) label: Option<&'a str>,
    pub(crate) context: TimingModuleContext,
}

impl<'a> TimingModuleAttribution<'a> {
    pub(crate) fn new(label: Option<&'a str>, context: TimingModuleContext) -> Self {
        Self { label, context }
    }
}
