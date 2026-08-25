//! Binding-independent value-origin rows.

use super::ids::{CallId, ValueOriginId};
use super::places::ProjectionElem;

/// How one origin was introduced or derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OriginKind {
    /// Provenance is intentionally unresolved and must be refined by a later Boracle analysis.
    Unknown,
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

/// Preliminary provenance supplied for a call result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallResultProvenance {
    Fresh,
    Alias(Box<[ValueOriginId]>),
    AliasParams(Box<[usize]>),
    Unknown,
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
