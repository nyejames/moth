//! Dense identities for build-local logical paths.
//!
//! WHAT: owns the mutable parent-linked trie, its module-local forks and its lookup-only table
//!       for complete-path operations. `SourceDatabase` embeds the mutable builder so source
//!       slots and path nodes share one build-lifetime identity base.
//! WHY:  logical compiler identity needs compact shared prefixes, while filesystem `PathBuf` and
//!       source snapshot identity remain separate owners. Source consumers resolve logical paths
//!       through the source database.
//!
//! Path domains:
//!
//! - Compiler logical and semantic component paths share this table as `PathId`. Their component
//!   semantics are compatible, so one dense trie serves both without a second interner.
//! - Filesystem paths remain cold `Path` and `PathBuf` values owned by source records and IO.
//!   Interning a filesystem path converts its components into logical IDs; the table never stores
//!   a `PathBuf` identity.
//! - Rendered free text is never interned as a path. Rendering resolves components through a
//!   string table into portable or native spellings without creating new identities.
//!
//! The implementation is split by data lifetime:
//!
//! - [`id`] defines the four-byte complete-path handle.
//! - [`builder`] interns one component at a time, merges worker deltas and freezes the dense
//!   node arrays.
//! - [`fork`] snapshots an immutable base for module-local deltas that merge deterministically.
//! - [`remap`] rewrites worker-local identities after a merge.
//! - [`frozen`] resolves and renders paths without mutable interning.

use std::path::PathBuf;

/// A filesystem path containing a component that cannot be represented as UTF-8.
///
/// Filesystem identity is exact or rejected; lossy conversion could collapse distinct names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonUtf8PathComponent {
    pub(crate) path: PathBuf,
}

mod builder;
mod fork;
mod frozen;
mod id;
mod remap;

#[cfg(test)]
mod tests;

pub(crate) use builder::{PathInternError, PathInternerBuilder};
#[allow(unused_imports)] // Slice 2B wires path forks into module compilation.
pub(crate) use fork::{PathInternerFork, PathInternerForkSource};
pub(crate) use frozen::PathTable;
pub(crate) use id::PathId;
#[allow(unused_imports)] // Slice 2B wires path remaps into module compilation.
pub(crate) use remap::PathIdRemap;
