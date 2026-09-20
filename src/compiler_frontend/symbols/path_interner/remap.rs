//! Compact remapping from worker-local path identities to merged identities.
//!
//! WHAT: rewrites `PathId` handles issued by one module-local fork into the destination builder
//!       after a deterministic delta merge.
//! WHY:  inherited prefix identities stay stable across workers, while locally interned suffix
//!       nodes may collide with paths merged from earlier workers.

use super::id::PathId;

/// Mapping from `PathId`s in one fork to `PathId`s in the merged destination table.
#[derive(Debug, Clone)]
pub struct PathIdRemap {
    /// IDs below this length are known to be identical in source and destination tables.
    identity_prefix_len: usize,

    /// Remapped IDs for the source suffix after `identity_prefix_len`.
    mapped_suffix: Vec<PathId>,

    is_identity: bool,
}

impl PathIdRemap {
    pub(super) fn new(
        identity_prefix_len: usize,
        mapped_suffix: Vec<PathId>,
        is_identity: bool,
    ) -> Self {
        Self {
            identity_prefix_len,
            mapped_suffix,
            is_identity,
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
            identity_prefix_len: 0,
            mapped_suffix: mapped,
            is_identity,
        }
    }

    pub fn get(&self, old: PathId) -> PathId {
        let old_index = old.index();

        if old_index < self.identity_prefix_len {
            return old;
        }

        self.mapped_suffix[old_index - self.identity_prefix_len]
    }

    /// Return the inherited prefix length that maps to itself.
    pub fn identity_prefix_len(&self) -> usize {
        self.identity_prefix_len
    }

    /// Return the number of remapped worker-local nodes.
    pub fn mapped_len(&self) -> usize {
        self.mapped_suffix.len()
    }

    /// Return bytes occupied by the mapped suffix rows.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn mapped_bytes(&self) -> usize {
        self.mapped_suffix
            .len()
            .saturating_mul(std::mem::size_of::<PathId>())
    }

    /// Return bytes reserved by the mapped suffix backing vector.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn mapped_capacity_bytes(&self) -> usize {
        self.mapped_suffix
            .capacity()
            .saturating_mul(std::mem::size_of::<PathId>())
    }

    /// Return whether every source ID maps to the same numeric ID in the destination.
    pub fn is_identity(&self) -> bool {
        self.is_identity
    }
}
