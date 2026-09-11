//! Normalized places and conservative projection overlap.

use super::ids::{BindingId, PlaceId};

/// A source-semantic projection step in a normalized place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ProjectionElem {
    Field(u32),
    FixedIndex(u32),
    DynamicIndex,
    // Boracle plan: docs/roadmap/plans/boracle-next-research-plans/ (aggregate-copy provenance).
    #[allow(dead_code)]
    CollectionElement,
    MapEntry,
}

/// One interned place: a binding cell plus its ordered projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    pub(crate) id: PlaceId,
    pub(crate) root: BindingId,
    pub(crate) projections: Box<[ProjectionElem]>,
}

impl Place {
    pub(crate) fn new(id: PlaceId, root: BindingId, projections: Vec<ProjectionElem>) -> Self {
        Self {
            id,
            root,
            projections: projections.into_boxed_slice(),
        }
    }

    /// Classify initial overlap without consulting origins or lifetime families.
    pub(crate) fn overlap(&self, other: &Self) -> PlaceOverlap {
        if self.root != other.root {
            return PlaceOverlap::Disjoint;
        }

        let common_len = self.projections.len().min(other.projections.len());
        for index in 0..common_len {
            let left = self.projections[index];
            let right = other.projections[index];
            if left == right {
                continue;
            }

            return match (left, right) {
                (ProjectionElem::Field(left), ProjectionElem::Field(right)) if left != right => {
                    PlaceOverlap::Disjoint
                }
                _ => PlaceOverlap::Conservative,
            };
        }

        PlaceOverlap::Overlap
    }
}

// LEAVE-LOCAL: this shared structural trichotomy remains an input fact; decision lanes are boracle/relations.rs:237 OriginOverlapDecision and boracle/oracle/conflicts.rs:23 DynamicOverlap.
// Independence contract: boracle/oracle/mod.rs:5-6 and docs/src/developer-docs/memory-management/boracle/boracle-operational-oracle.mtf:22; differential evidence: boracle/tests/differential.rs:21-32.
/// The initial place-overlap relation used by Boracle fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlaceOverlap {
    Disjoint,
    Overlap,
    Conservative,
}
