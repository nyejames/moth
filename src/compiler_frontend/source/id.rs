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

    /// Convert a proven-in-range index into its non-zero identity.
    ///
    /// Callers must have proven the index comes from a table the compiler sized. Authored
    /// registration goes through [`Self::try_from_index`], which reports exhaustion instead of
    /// panicking.
    #[cfg(test)]
    pub(crate) fn from_index(index: usize) -> Self {
        Self::try_from_index(index).expect("source index is inside the compact identity domain")
    }

    /// Convert a zero-based source-record index into its non-zero identity when it fits.
    ///
    /// `None` reports authored table exhaustion: the compact identity table cannot address
    /// another entry, so the owning database surfaces a typed source-capacity failure.
    pub(crate) fn try_from_index(index: usize) -> Option<Self> {
        let index = u32::try_from(index).ok()?;
        let raw = index.checked_add(1)?;
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
