//! Durable type-display ids for compact diagnostics.
//!
//! WHAT: `TypeDisplayId` names one rendered type display in the type-display store.
//! WHY: compact diagnostic records store display ids, never raw `TypeId`s. Ids are one-based, so
//! zero is never a valid id and a zeroed fact word cannot name a display.

use std::num::NonZeroU32;

/// Returned when a dense store index cannot be represented as a one-based `u32` id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TypeDisplayCapacityError;

/// One-based id of a rendered type display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TypeDisplayId(NonZeroU32);

impl TypeDisplayId {
    /// Converts a dense zero-based store index into its one-based id.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "P4.3 render adapter builds type-display snapshots"
        )
    )]
    pub(crate) fn from_index(index: usize) -> Result<Self, TypeDisplayCapacityError> {
        u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .and_then(NonZeroU32::new)
            .map(Self)
            .ok_or(TypeDisplayCapacityError)
    }

    /// Returns the dense zero-based store index this id names.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.3 render adapter reads type-display snapshots")
    )]
    pub(crate) fn index(self) -> usize {
        // `from_index` guarantees the stored value is at least one, so this cannot underflow.
        (self.0.get() - 1) as usize
    }
}
