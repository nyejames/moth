//! Normalized borrow events, uses, loans and call effects.

use super::ids::{
    BindingId, BlockId, CallId, EventId, LoanId, PlaceId, PointId, UseId, ValueOriginId,
};
use super::places::ProjectionElem;
use crate::compiler_frontend::hir::ids::HirNodeId;
use crate::compiler_frontend::source::SourceSpan;

/// Optional exact span mapping retained for diagnostics and inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EventSource {
    pub(crate) hir_node: Option<HirNodeId>,
    pub(crate) span: Option<SourceSpan>,
}

/// The control-flow meaning attached to a terminator event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminatorEventKind {
    Jump {
        target: BlockId,
        arguments: Box<[JumpArgument]>,
    },
    Branch {
        targets: Box<[BlockId]>,
    },
    Return,
    ReturnSuccess,
    ReturnError,
    Break {
        target: BlockId,
    },
    Continue {
        target: BlockId,
    },
    RuntimeFailure,
    AssertFailure,
}

/// One value captured from the predecessor and installed as a new edge definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JumpArgument {
    pub(crate) source: PlaceId,
    pub(crate) destination: BindingDestination,
}

/// Whether a normalized write creates a binding generation or updates the current place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingDestination {
    Define(PlaceId),
    Update(PlaceId),
}

impl BindingDestination {
    pub(crate) const fn place(self) -> PlaceId {
        match self {
            Self::Define(place) | Self::Update(place) => place,
        }
    }

    pub(crate) const fn defines(self) -> bool {
        matches!(self, Self::Define(_))
    }
}

impl EventSource {
    pub(crate) const fn none() -> Self {
        Self {
            hir_node: None,
            span: None,
        }
    }
}

/// One normalized event anchored at a semantic program point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Event {
    pub(crate) id: EventId,
    pub(crate) point: PointId,
    pub(crate) source: EventSource,
    pub(crate) kind: EventKind,
}

impl Event {
    pub(crate) fn new(id: EventId, point: PointId, kind: EventKind, source: EventSource) -> Self {
        Self {
            id,
            point,
            source,
            kind,
        }
    }
}

/// An access or observation recorded independently from the event that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Use {
    pub(crate) id: UseId,
    pub(crate) point: PointId,
    pub(crate) place: PlaceId,
    pub(crate) kind: UseKind,
    /// True when the access replaces the value generation at this place.
    pub(crate) definition: bool,
}

/// The source-semantic reason a place is observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UseKind {
    Read,
    /// A write access paired with an explicit HIR local definition or place update.
    BindingWrite(BindingDestination),
    Write,
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (loop-generation epochs and edge last use).
    #[allow(dead_code)]
    LoanObservation,
}

impl UseKind {
    pub(crate) const fn access_kind(self) -> AccessKind {
        match self {
            Self::Read => AccessKind::Shared,
            Self::LoanObservation => AccessKind::Shared,
            Self::Write | Self::BindingWrite(_) => AccessKind::Exclusive,
        }
    }

    pub(crate) const fn is_write(self) -> bool {
        matches!(self, Self::Write | Self::BindingWrite(_))
    }

    pub(crate) const fn binding_destination(self) -> Option<BindingDestination> {
        match self {
            Self::BindingWrite(destination) => Some(destination),
            Self::Read | Self::Write | Self::LoanObservation => None,
        }
    }
}

/// Shared and mutation-capable access kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessKind {
    Shared,
    Exclusive,
}

/// One source-semantic loan declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Loan {
    pub(crate) id: LoanId,
    pub(crate) kind: AccessKind,
    pub(crate) issued_at: PointId,
    pub(crate) place: PlaceId,
    pub(crate) origins: Box<[ValueOriginId]>,
    pub(crate) holders: Box<[PlaceId]>,
    pub(crate) uses: Box<[UseId]>,
    pub(crate) kills: Box<[PointId]>,
}

/// Why a normalized loan stops being usable.
// Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (loop-generation epochs and edge last use).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KillReason {
    FinalUse,
    Rebind,
    ScopeExit,
    UnreachableContinuation,
    Explicit,
}

/// A stored child relationship inside an aggregate value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AggregateField {
    pub(crate) projection: ProjectionElem,
    pub(crate) source: PlaceId,
}

/// One ordered call argument access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallArgument {
    pub(crate) place: PlaceId,
    pub(crate) access: AccessKind,
    pub(crate) use_id: UseId,
}

/// One result place and its preliminary origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CallResult {
    pub(crate) destination: BindingDestination,
    pub(crate) origin: ValueOriginId,
}

/// Resolved call effects consumed by the reference model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallEffect {
    pub(crate) call: CallId,
    pub(crate) arguments: Box<[CallArgument]>,
    pub(crate) result: Option<CallResult>,
}

/// A stable call-summary handle. The label is opaque to Phase 2 and deterministic in fixtures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    pub(crate) id: CallId,
    pub(crate) label: String,
}

/// A rebinding preserves an explicit value meaning instead of mutating an old origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RebindValue {
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (conflict-directed relational refinement).
    #[allow(dead_code)]
    Fresh(ValueOriginId),
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (conflict-directed relational refinement).
    #[allow(dead_code)]
    Alias(Box<[ValueOriginId]>),
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (conflict-directed relational refinement).
    #[allow(dead_code)]
    AliasFromPlace(PlaceId),
}

/// The normalized semantic event vocabulary owned by BorrowProblem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EventKind {
    Fresh {
        destination: BindingDestination,
        origin: ValueOriginId,
    },
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (conflict-directed relational refinement).
    #[allow(dead_code)]
    Alias {
        source: PlaceId,
        destination: BindingDestination,
        origins: Box<[ValueOriginId]>,
    },
    AliasFromPlace {
        source: PlaceId,
        destination: BindingDestination,
    },
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (conflict-directed relational refinement).
    #[allow(dead_code)]
    ExclusiveAlias {
        source: PlaceId,
        destination: BindingDestination,
        origins: Box<[ValueOriginId]>,
    },
    ExclusiveAliasFromPlace {
        source: PlaceId,
        destination: BindingDestination,
    },
    Copy {
        source: PlaceId,
        destination: BindingDestination,
        origin: ValueOriginId,
    },
    Projection {
        source: PlaceId,
        destination: BindingDestination,
        origin: ValueOriginId,
    },
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (conflict-directed relational refinement).
    #[allow(dead_code)]
    Rebind {
        destination: BindingDestination,
        value: RebindValue,
    },
    Aggregate {
        destination: BindingDestination,
        origin: ValueOriginId,
        fields: Box<[AggregateField]>,
    },
    ScopeExit {
        bindings: Box<[BindingId]>,
    },
    /// One ordered argument access belonging to a call effect.
    ///
    /// The complete [`CallEffect`] remains the result-provenance boundary. Keeping each
    /// argument as its own event gives last-use and conflict witnesses an exact evaluation
    /// boundary without duplicating the call's result metadata.
    CallArgument {
        call: CallId,
        index: u32,
        argument: CallArgument,
    },
    Terminator {
        kind: TerminatorEventKind,
    },
    CallEffect(CallEffect),
    Access {
        use_id: UseId,
    },
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (conflict-directed relational refinement).
    #[allow(dead_code)]
    LoanIssue {
        loan: LoanId,
    },
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (conflict-directed relational refinement).
    #[allow(dead_code)]
    LoanKill {
        loan: LoanId,
        reason: KillReason,
    },
}
