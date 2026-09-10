//! Lookup-only source, path and string identity after the mutable build boundary.
//!
//! WHAT: owns the finalized source slots, snapshots, and path trie beside the merged immutable
//!       string table used by logical-path components.
//! WHY:  source slots, source snapshots, line starts, extended spans and path nodes must cross one
//!       consuming boundary without a second source database or copied source text.
//!
//! [`FrozenIdentityContext::from_parts`] consumes the merged root [`StringTable`] and a finalized
//! [`SourceDatabase`]. The source database moves its existing arrays and path table into a
//! [`FrozenSourceDatabase`], while the string table moves its existing string allocations into a
//! [`FrozenStringTable`] held behind a shared owner. Every `PathId` therefore remains paired
//! with the exact `StringId` table that issued its components.
//!
//! One merged root freeze can be shared across project/package identity domains: each additional
//! domain moves only its own finalized [`SourceDatabase`] through
//! [`FrozenIdentityContext::from_shared_strings`] with a clone of the root's shared string
//! allocation. Sharing the owner shares the one frozen string allocation; no source text or
//! string allocation is copied.

use super::line_index::LineIndex;
use super::{FrozenSourceDatabase, SourceDatabase, SourceId, SourceRecord};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringId, StringTable,
};
use std::sync::Arc;

/// Late-bound owner for one generic body or scope chain's final frozen identity.
///
/// The handle is created before semantic work can finish its source span builders, then installed
/// exactly once at the final render boundary. Retained tokens always carry this handle alongside
/// their `SourceId`; they never expose a donor ID without an owner that can resolve it after the
/// mutable source builder is dropped.
#[derive(Clone, Debug)]
pub(crate) struct FrozenIdentityHandle {
    identity: Arc<std::sync::OnceLock<Arc<FrozenIdentityContext>>>,
    /// Stable package domain for the source IDs carried by this handle.
    ///
    /// Source IDs are only unique inside one project/package database, so a donor handle must
    /// retain the owning package identity while its frozen context is still late-bound. The
    /// domain is shared by cheap handle clones and is not inferred from a colliding SourceId.
    domain: Option<Arc<StablePackageIdentity>>,
}

impl PartialEq for FrozenIdentityHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}

impl Eq for FrozenIdentityHandle {}

impl FrozenIdentityHandle {
    pub(crate) fn new() -> Self {
        Self {
            identity: Arc::new(std::sync::OnceLock::new()),
            domain: None,
        }
    }

    /// Create a handle whose spans belong to one stable project/package identity domain.
    pub(crate) fn for_domain(domain: StablePackageIdentity) -> Self {
        Self {
            identity: Arc::new(std::sync::OnceLock::new()),
            domain: Some(Arc::new(domain)),
        }
    }

    pub(crate) fn domain(&self) -> Option<&StablePackageIdentity> {
        self.domain.as_deref()
    }

    pub(crate) fn install(
        &self,
        identity: Arc<FrozenIdentityContext>,
    ) -> Result<(), CompilerError> {
        if let Some(existing) = self.identity.get() {
            if Arc::ptr_eq(existing, &identity) {
                return Ok(());
            }
            return Err(CompilerError::compiler_error(
                "frozen identity handle was assigned two different contexts",
            ));
        }
        self.identity.set(identity).map_err(|_| {
            CompilerError::compiler_error("frozen identity handle was assigned concurrently")
        })
    }

    /// Borrow the installed identity after the owning compilation boundary freezes.
    ///
    /// A missing identity is deliberately observable: callers carrying a donor handle must not
    /// reinterpret its source IDs through the requester's context.
    pub(crate) fn get(&self) -> Option<&FrozenIdentityContext> {
        self.identity.get().map(Arc::as_ref)
    }
}

/// Immutable source, string and logical-path identity for one compiler boundary.
///
/// This context owns no diagnostics, report-local type data, compiler driver or scheduler. It is
/// the lookup/rendering seam used by later source and diagnostic boundaries. The string table is
/// held behind a shared owner so independent project/package contexts can share the one merged
/// root string allocation without copying it; each context still owns its own frozen source
/// database.
#[derive(Debug)]
pub(crate) struct FrozenIdentityContext {
    sources: FrozenSourceDatabase,
    strings: Arc<FrozenStringTable>,
}
impl FrozenIdentityContext {
    /// Consume the merged root string table and finalized source database.
    ///
    /// The source database's [`SourceDatabase::freeze`] operation moves retained snapshots,
    /// installed extended-span tables and its path trie. Construction-only load failures and
    /// canonical-path lookup state are dropped before the frozen owner is published. No source
    /// text or span storage is cloned, and no independent path table is created. The frozen
    /// string allocation is placed behind a shared owner so later package domains can reuse it
    /// through [`Self::from_shared_strings`] without copying.
    pub(crate) fn from_parts(strings: StringTable, sources: SourceDatabase) -> Self {
        Self {
            sources: sources.freeze(),
            strings: Arc::new(strings.freeze()),
        }
    }

    /// Reuse an already-frozen shared string allocation for another finalized source database.
    ///
    /// Only the source owner moves: [`SourceDatabase::freeze`] moves this domain's retained
    /// snapshots, installed extended-span tables and path trie into a fresh
    /// [`FrozenSourceDatabase`]. Construction-only load failures and canonical-path lookup state
    /// are dropped before publication. Sharing the owner shares the one merged root string
    /// allocation created by [`Self::from_parts`]; no source text or string allocation is copied.
    /// Callers must pass the exact shared table that issued the [`StringId`] components stored
    /// in `sources`' path trie.
    pub(crate) fn from_shared_strings(
        strings: Arc<FrozenStringTable>,
        sources: SourceDatabase,
    ) -> Self {
        Self {
            sources: sources.freeze(),
            strings,
        }
    }

    /// Borrow the immutable string table used by compact string IDs.
    #[inline]
    pub(crate) fn strings(&self) -> &FrozenStringTable {
        &self.strings
    }

    /// Share this context's frozen string allocation with another project/package domain.
    ///
    /// Cloning the shared owner reuses the one merged root allocation; no string storage is
    /// copied. Pass the result to [`Self::from_shared_strings`].
    #[inline]
    pub(crate) fn shared_strings(&self) -> Arc<FrozenStringTable> {
        Arc::clone(&self.strings)
    }

    /// Borrow the source owner's immutable parent-linked path table.
    #[inline]
    pub(crate) fn paths(&self) -> &PathTable {
        self.sources.paths()
    }

    /// Fallibly resolve a string ID in this identity context for source/diagnostic model tests.
    #[cfg(test)]
    pub(crate) fn try_resolve_string(&self, id: StringId) -> Option<&str> {
        self.strings.try_resolve(id)
    }

    /// Render a path using this context's frozen source path and string tables.
    ///
    /// `scratch` is caller-owned and reusable; rendering allocates only the returned spelling.
    #[inline]
    pub(crate) fn render_path(&self, path: PathId, scratch: &mut Vec<StringId>) -> String {
        self.paths()
            .render_portable_frozen(path, &self.strings, scratch)
    }

    /// Resolve one physical source slot by compact identity.
    #[inline]
    pub(crate) fn get(&self, id: SourceId) -> Option<&super::SourceSlot> {
        self.sources.get(id)
    }

    /// Return the compact logical-path identity assigned to one physical source.
    #[inline]
    pub(crate) fn source_logical_path(&self, id: SourceId) -> Option<PathId> {
        self.sources.source_logical_path(id)
    }

    /// Construct a line index over one retained source snapshot.
    #[inline]
    pub(crate) fn line_index(&self, id: SourceId) -> Option<LineIndex<'_>> {
        self.sources.line_index(id)
    }

    /// Resolve the loaded source record for exact frozen span resolution.
    #[inline]
    pub(crate) fn source_record(&self, id: SourceId) -> &SourceRecord {
        self.sources.source_record(id)
    }
}
