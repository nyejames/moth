//! Ceilings for the diagnostic store's arenas.
//!
//! WHAT: one `usize` per arena. A compaction plan that would pass a ceiling fails, and the store latches full.
//! WHY: every arena offset and length is packed into `u32` fields. The ceilings keep each index inside that range,
//! and tests shrink them to reach the capacity path without allocating billions of rows.

/// Per-arena ceilings for one `DiagnosticStore`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiagnosticLimits {
    pub(super) records: usize,
    pub(super) extras: usize,
    pub(super) labels: usize,
    pub(super) label_data: usize,
    pub(super) string_ids: usize,
    pub(super) substitutions: usize,
    pub(super) foreign_sites: usize,
    pub(super) identity_contexts: usize,
}

impl DiagnosticLimits {
    /// The production ceilings. Every arena stays inside `u32`, and label data stays inside the 24-bit index that a
    /// secondary label packs.
    pub(super) const fn production() -> Self {
        Self {
            records: u32::MAX as usize,
            extras: u32::MAX as usize,
            labels: u32::MAX as usize,
            label_data: (1 << 24) - 1,
            string_ids: u32::MAX as usize,
            substitutions: u32::MAX as usize,
            foreign_sites: u32::MAX as usize,
            identity_contexts: u32::MAX as usize,
        }
    }
}
