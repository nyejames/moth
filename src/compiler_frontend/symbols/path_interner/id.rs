//! Compact identities for complete logical paths.
//!
//! WHAT: `PathId` names one complete logical path in a build-lifetime path table without owning
//!       the path's components.
//! WHY:  a non-zero compact handle keeps path identity distinct from source snapshots and from
//!       filesystem paths while preserving an option niche for later source records.

use std::num::NonZeroU32;

/// A complete logical path in one build-lifetime path table.
///
/// `PathId` does not own its components. It is distinct from a source snapshot identity such as
/// `SourceId`, and is valid only with the path table that issued it.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathId(NonZeroU32);

impl PathId {
    /// The empty logical path at the root of the table.
    pub const ROOT: Self = Self(NonZeroU32::new(1).unwrap());

    /// Convert a zero-based path-node index into its non-zero identity when it fits.
    ///
    /// `None` reports authored table exhaustion: the build-lifetime path table cannot address
    /// another node, so the owning database surfaces a typed source-capacity failure.
    pub(crate) fn try_from_index(index: usize) -> Option<Self> {
        let index = u32::try_from(index).ok()?;
        let raw = index.checked_add(1)?;
        Some(Self(NonZeroU32::new(raw)?))
    }

    /// Return the zero-based node index addressed by this identity.
    pub(crate) fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}
