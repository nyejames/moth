//! Binding-independent value-origin rows.

use super::ids::{CallId, ValueOriginId};
use super::places::ProjectionElem;

/// How one origin was introduced or derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OriginKind {
    /// Provenance is intentionally unresolved and must be refined by a later Boracle analysis.
    Unknown,
    /// The value entered the function through one source parameter.
    Parameter {
        index: u32,
    },
    Fresh,
    Alias(Box<[ValueOriginId]>),
    ExclusiveAlias(Box<[ValueOriginId]>),
    Copy(Box<[ValueOriginId]>),
    Projection {
        source: ValueOriginId,
        projection: ProjectionElem,
    },
    Join(Box<[ValueOriginId]>),
    CallResult {
        call: CallId,
        provenance: CallResultProvenance,
    },
}

/// Why a call result's provenance is unresolved at the problem boundary.
///
/// This deliberately lives in the normalized-problem layer rather than importing Boracle's
/// relation vocabulary. The solver maps each boundary fact to its own precision-loss reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallResultUnknownReason {
    /// The callee summary explicitly says that the returned value may alias an unknown source.
    SummaryUnknown,
    /// No local, generated or module-private summary was available.
    MissingSummary,
    /// The call crossed an opaque external boundary, including an opaque builtin operation.
    OpaqueExternal,
}

/// Preliminary provenance supplied for a call result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallResultProvenance {
    Fresh,
    Alias(Box<[ValueOriginId]>),
    AliasParams(Box<[usize]>),
    Unknown(CallResultUnknownReason),
}

/// One explicit source-semantic origin lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValueOrigin {
    pub(crate) id: ValueOriginId,
    pub(crate) kind: OriginKind,
}

impl ValueOrigin {
    pub(crate) const fn unknown(id: ValueOriginId) -> Self {
        Self {
            id,
            kind: OriginKind::Unknown,
        }
    }

    pub(crate) const fn fresh(id: ValueOriginId) -> Self {
        Self {
            id,
            kind: OriginKind::Fresh,
        }
    }

    pub(crate) fn new(id: ValueOriginId, kind: OriginKind) -> Self {
        Self { id, kind }
    }
}
