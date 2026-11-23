//! Compact remapping from worker-local path identities to merged identities.
//!
//! WHAT: rewrites `PathId` handles issued by one module-local fork into the destination builder
//!       after a deterministic delta merge.
//! WHY:  inherited prefix identities stay stable across workers, while locally interned suffix
//!       nodes may collide with paths merged from earlier workers.
//!
//! Two further forms serve foreign closures. A sparse remap names only the paths one retained
//! closure references, so a provider chain does not re-intern every inherited domain in each
//! requester. A recording remap observes that same path set while mapping it to itself, which
//! keeps the selection probe and the row visitor in one implementation.

use super::id::PathId;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use rustc_hash::FxHashMap;
use std::cell::RefCell;

/// Mapping from `PathId`s in one fork to `PathId`s in the merged destination table.
#[derive(Debug, Clone)]
pub struct PathIdRemap {
    rows: PathIdRemapRows,
}

#[derive(Debug, Clone)]
enum PathIdRemapRows {
    /// Worker-delta or complete-table remap with an inherited identity prefix.
    Dense {
        /// IDs below this length are known to be identical in source and destination tables.
        identity_prefix_len: usize,
        /// Remapped IDs for the source suffix after `identity_prefix_len`.
        mapped_suffix: Vec<PathId>,
        is_identity: bool,
    },
    /// Selected-path remap for a foreign closure that names only a bounded subset of its issuing
    /// table. A requested path outside the selected subset is an internal invariant break.
    Sparse {
        mapped: FxHashMap<PathId, PathId>,
        is_identity: bool,
    },
    /// Probe remap that records every requested source path and maps it to itself.
    ///
    /// WHAT: supports collecting the exact path fields one retained row traverses without a
    ///       second parallel visitor implementation.
    /// WHY:  selected foreign closures must intern only the paths their rows actually reference.
    /// It reports identity, so it is only valid over a field tree whose remap visits every path
    /// rather than short-circuiting on `is_identity`.
    Recording { recorded: RefCell<Vec<PathId>> },
}

impl PathIdRemap {
    pub(super) fn new(
        identity_prefix_len: usize,
        mapped_suffix: Vec<PathId>,
        is_identity: bool,
    ) -> Self {
        Self {
            rows: PathIdRemapRows::Dense {
                identity_prefix_len,
                mapped_suffix,
                is_identity,
            },
        }
    }

    /// Build a remap for a complete source table whose IDs are all potentially foreign.
    ///
    /// Unlike a worker-delta remap, this form has no identity prefix: every source path is
    /// explicitly mapped into the destination fork. It is used when a generated template crosses
    /// a project/package boundary and therefore cannot assume that the donor's numeric `PathId`
    /// domain is the requester's domain.
    pub(crate) fn from_full(mapped: Vec<PathId>) -> Self {
        let is_identity = mapped
            .iter()
            .enumerate()
            .all(|(index, path)| path.index() == index);
        Self {
            rows: PathIdRemapRows::Dense {
                identity_prefix_len: 0,
                mapped_suffix: mapped,
                is_identity,
            },
        }
    }

    /// Build a remap for only the selected source paths a foreign closure references.
    pub(crate) fn from_sparse(mapped: FxHashMap<PathId, PathId>) -> Self {
        let is_identity = mapped
            .iter()
            .all(|(source, destination)| source == destination);
        Self {
            rows: PathIdRemapRows::Sparse {
                mapped,
                is_identity,
            },
        }
    }

    /// Build a probe remap that records every requested source path.
    pub(crate) fn recording() -> Self {
        Self {
            rows: PathIdRemapRows::Recording {
                recorded: RefCell::new(Vec::new()),
            },
        }
    }

    /// Take the source paths one recording probe observed.
    pub(crate) fn take_recorded(self) -> Vec<PathId> {
        match self.rows {
            PathIdRemapRows::Recording { recorded } => recorded.into_inner(),
            _ => crate::compiler_bug!(
                CompilerStage::GenericMaterialisation,
                "only a recording path remap owns recorded paths; this is a compiler bug"
            ),
        }
    }

    pub fn get(&self, old: PathId) -> PathId {
        match &self.rows {
            PathIdRemapRows::Dense {
                identity_prefix_len,
                mapped_suffix,
                ..
            } => {
                let old_index = old.index();

                if old_index < *identity_prefix_len {
                    return old;
                }

                mapped_suffix[old_index - *identity_prefix_len]
            }
            PathIdRemapRows::Sparse { mapped, .. } => *mapped.get(&old).unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::GenericMaterialisation,
                    "selected path remap was asked for an unrecorded path; this is a compiler bug"
                )
            }),
            PathIdRemapRows::Recording { recorded } => {
                recorded.borrow_mut().push(old);
                old
            }
        }
    }

    /// Return the inherited prefix length that maps to itself.
    pub fn identity_prefix_len(&self) -> usize {
        match &self.rows {
            PathIdRemapRows::Dense {
                identity_prefix_len,
                ..
            } => *identity_prefix_len,
            PathIdRemapRows::Sparse { .. } | PathIdRemapRows::Recording { .. } => 0,
        }
    }

    /// Return the number of remapped worker-local nodes.
    pub fn mapped_len(&self) -> usize {
        match &self.rows {
            PathIdRemapRows::Dense { mapped_suffix, .. } => mapped_suffix.len(),
            PathIdRemapRows::Sparse { mapped, .. } => mapped.len(),
            PathIdRemapRows::Recording { .. } => 0,
        }
    }

    /// Return bytes occupied by the mapped rows.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn mapped_bytes(&self) -> usize {
        let row_words = match &self.rows {
            PathIdRemapRows::Dense { mapped_suffix, .. } => mapped_suffix.len(),
            // A sparse row stores its source and destination path.
            PathIdRemapRows::Sparse { mapped, .. } => mapped.len().saturating_mul(2),
            PathIdRemapRows::Recording { recorded } => recorded.borrow().len(),
        };
        row_words.saturating_mul(std::mem::size_of::<PathId>())
    }

    /// Return bytes reserved by the mapped row backing stores.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn mapped_capacity_bytes(&self) -> usize {
        let row_words = match &self.rows {
            PathIdRemapRows::Dense { mapped_suffix, .. } => mapped_suffix.capacity(),
            PathIdRemapRows::Sparse { mapped, .. } => mapped.capacity().saturating_mul(2),
            PathIdRemapRows::Recording { recorded } => recorded.borrow().capacity(),
        };
        row_words.saturating_mul(std::mem::size_of::<PathId>())
    }

    /// Return whether every source ID maps to the same numeric ID in the destination.
    pub fn is_identity(&self) -> bool {
        match &self.rows {
            PathIdRemapRows::Dense { is_identity, .. }
            | PathIdRemapRows::Sparse { is_identity, .. } => *is_identity,
            PathIdRemapRows::Recording { .. } => true,
        }
    }
}
