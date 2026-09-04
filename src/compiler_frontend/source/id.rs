//! Compact identities for retained frontend source records.
//!
//! `SourceId` is a non-zero table handle. The zero-based index used by source databases remains
//! available at the boundary, while the stored representation reserves zero for `Option`'s niche.

use std::num::NonZeroU32;

/// Identity of one source record in a frontend source database.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(NonZeroU32);

impl SourceId {
    /// Convert a zero-based source-record index into its non-zero identity.
    pub(crate) fn from_index(index: usize) -> Self {
        let index = u32::try_from(index)
            .expect("source database cannot contain more than u32::MAX records");
        let raw = index
            .checked_add(1)
            .expect("source database index must leave room for the non-zero identity");
        Self(NonZeroU32::new(raw).expect("source identities are always non-zero"))
    }
    /// Convert a zero-based physical-source row into its compiler identity.
    ///
    /// The compilation-root row occupies the preceding storage slot. Keeping this conversion here
    /// prevents Stage 0 callers from encoding the root offset themselves.
    pub(crate) fn from_physical_index(physical_index: usize) -> Option<Self> {
        let record_index = physical_index.checked_add(1)?;
        let record_index = u32::try_from(record_index).ok()?;
        let raw = record_index.checked_add(1)?;
        Some(Self(NonZeroU32::new(raw)?))
    }

    /// Return the zero-based source-record index addressed by this identity.
    pub(crate) fn index(self) -> usize {
        self.0.get() as usize - 1
    }

    /// Return the dense index of a physical source, excluding the compilation root.
    ///
    /// `None` identifies the reserved compilation-root record rather than a physical source.
    pub(crate) fn physical_index(self) -> Option<usize> {
        self.index().checked_sub(1)
    }
}
