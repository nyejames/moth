//! Dense identities for build-local logical paths.
//!
//! WHAT: owns the mutable parent-linked trie used during one build-base identity pass and its
//!       lookup-only frozen table for complete-path operations.
//! WHY:  logical compiler identity needs compact shared prefixes, while filesystem `PathBuf` and
//!       source snapshot identity remain separate owners. This slice intentionally does not migrate
//!       the compiler's existing `InternedPath` representation or add parallel merge machinery.
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
