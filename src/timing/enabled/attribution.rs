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

use super::session::TimingSessionId;

/// Dense handle for one compilation boundary inside one command session.
///
/// The session generation makes the handle unrepresentable outside its owning
/// collection; the index is a registration-table slot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TimingBoundaryId {
    session: TimingSessionId,
    index: u32,
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
};

impl TimingBoundaryId {
    /// Build a boundary id from its owning session and table slot.
    pub(crate) fn from_session(session: TimingSessionId, index: u32) -> Self {
        Self { session, index }
    }

    /// The registration-table slot for this boundary.
    pub(crate) fn index(self) -> usize {
        self.index as usize
    }

    /// The session generation that owns this boundary.
    pub(crate) fn session(self) -> TimingSessionId {
        self.session
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
    boundary: TimingBoundaryId,
    module_index: u32,
}

impl TimingModuleKey {
    /// Build a module key inside a registered boundary.
    pub(crate) fn new(boundary: TimingBoundaryId, module_index: u32) -> Self {
        Self {
            boundary,
            module_index,
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

    /// The session generation required by this context.
    pub(crate) fn session(self) -> TimingSessionId {
        match self {
            Self::Boundary(boundary) => boundary.session(),
            Self::Module(key) => key.boundary().session(),
        }
    }

    /// The boundary named by this context, when it names one.
    pub(crate) fn boundary(self) -> Option<TimingBoundaryId> {
        match self {
            Self::Boundary(boundary) => Some(boundary),
            Self::Module(key) => Some(key.boundary()),
        }
    }

    /// The module named by this context, when it names one.
    pub(crate) fn module(self) -> Option<TimingModuleKey> {
        match self {
            Self::Boundary(_) => None,
            Self::Module(key) => Some(key),
        }
    }
}
