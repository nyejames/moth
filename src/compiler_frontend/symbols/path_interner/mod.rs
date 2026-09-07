//! Dense identities for build-local logical paths.
//!
//! WHAT: owns the mutable parent-linked trie and its lookup-only table for complete-path
//!       operations. `SourceDatabase` embeds the mutable builder so source slots and path nodes
//!       share one build-lifetime identity base.
//! WHY:  logical compiler identity needs compact shared prefixes, while filesystem `PathBuf` and
//!       source snapshot identity remain separate owners. Source consumers awaiting migration
//!       reconstruct transient `InternedPath` views through the source database.
//!
//! The implementation is split by data lifetime:
//!
//! - [`id`] defines the four-byte complete-path handle.
//! - [`builder`] interns one component at a time and freezes the dense node arrays.
//! - [`frozen`] resolves and renders paths without mutable interning.

mod builder;
mod frozen;
mod id;

#[cfg(test)]
mod tests;

pub(crate) use builder::PathInternerBuilder;
pub(crate) use frozen::PathTable;
pub(crate) use id::PathId;
