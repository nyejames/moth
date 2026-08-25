//! Normalized borrow events, uses, loans and call effects.

use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::hir::ids::HirNodeId;

use super::ids::{CallId, EventId, LoanId, PlaceId, PointId, UseId, ValueOriginId};
use super::places::ProjectionElem;

/// Optional mapping retained for diagnostics and inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventSource {
    pub(crate) hir_node: Option<HirNodeId>,
    pub(crate) location: Option<SourceLocation>,
}

impl EventSource {
    pub(crate) const fn none() -> Self {
        Self {
            hir_node: None,
            location: None,
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
}

/// The source-semantic reason a place is observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UseKind {
    Read,
    Write,
    LoanObservation,
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
    pub(crate) place: PlaceId,
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
    Fresh(ValueOriginId),
    Alias(Box<[ValueOriginId]>),
}

/// The normalized semantic event vocabulary owned by BorrowProblem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EventKind {
    Fresh {
        destination: PlaceId,
        origin: ValueOriginId,
    },
    Alias {
        source: PlaceId,
        destination: PlaceId,
        origins: Box<[ValueOriginId]>,
    },
    ExclusiveAlias {
        source: PlaceId,
        destination: PlaceId,
        origins: Box<[ValueOriginId]>,
    },
    Copy {
        source: PlaceId,
        destination: PlaceId,
        origin: ValueOriginId,
    },
    Rebind {
        destination: PlaceId,
        value: RebindValue,
    },
    Aggregate {
        destination: PlaceId,
        origin: ValueOriginId,
        fields: Box<[AggregateField]>,
    },
    CallEffect(CallEffect),
    Access {
        use_id: UseId,
    },
    LoanIssue {
        loan: LoanId,
    },
    LoanKill {
        loan: LoanId,
        reason: KillReason,
    },
}
