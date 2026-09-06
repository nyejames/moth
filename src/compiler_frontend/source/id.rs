//! Compact identities for retained frontend source records.
//!
//! `SourceId` is a non-zero table handle. Its storage index remains zero-based internally, while
//! the stored representation reserves zero for `Option`'s niche.

use std::num::NonZeroU32;

/// Identity of one source record in a frontend source database.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(NonZeroU32);

impl SourceId {
    /// Identity every database reserves at index 0, for a token stream whose text belongs to the
    /// whole compilation rather than to any registered file. The record itself owns no path and
    /// no snapshot, so a lookup for physical source text still finds nothing.
    pub const COMPILATION_ROOT: Self = Self(NonZeroU32::new(1).unwrap());

    /// Convert a zero-based source-record index into its non-zero identity.
    pub(crate) fn from_index(index: usize) -> Self {
        let index = u32::try_from(index)
            .expect("source database cannot contain more than u32::MAX records");
        let raw = index
            .checked_add(1)
            .expect("source database index must leave room for the non-zero identity");
        Self(NonZeroU32::new(raw).expect("source identities are always non-zero"))
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
