//! Lookup-only source, path and string identity after the mutable build boundary.
//!
//! WHAT: owns the finalized source database beside the merged immutable string table used by its
//!       logical-path components.
//! WHY:  source snapshots, line starts, extended spans, canonical paths and path nodes must cross
//!       one consuming boundary without a second source database or copied source text.
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
use super::{FrozenSourceDatabase, SourceDatabase, SourceId, SourceRecord, SourceSlot};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringId, StringTable,
};
use std::path::Path;
use std::sync::Arc;

/// Late-bound owner for one generic body or scope chain's final frozen identity.
///
/// The handle is created before semantic work can finish its source span builders, then installed
/// exactly once at the final render boundary. Retained tokens always carry this handle alongside
/// their `SourceId`; they never expose a donor ID without an owner that can resolve it after the
/// mutable source builder is dropped.
#[derive(Clone, Debug)]
pub(crate) struct FrozenIdentityHandle(Arc<std::sync::OnceLock<Arc<FrozenIdentityContext>>>);

impl FrozenIdentityHandle {
    pub(crate) fn new() -> Self {
        Self(Arc::new(std::sync::OnceLock::new()))
    }

    pub(crate) fn install(
        &self,
        identity: Arc<FrozenIdentityContext>,
    ) -> Result<(), CompilerError> {
        if let Some(existing) = self.0.get() {
            if Arc::ptr_eq(existing, &identity) {
                return Ok(());
            }
            return Err(CompilerError::compiler_error(
                "frozen identity handle was assigned two different contexts",
            ));
        }
        self.0.set(identity).map_err(|_| {
            CompilerError::compiler_error("frozen identity handle was assigned concurrently")
        })
    }

    #[cfg(test)]
    pub(crate) fn get(&self) -> Option<&FrozenIdentityContext> {
        self.0.get().map(Arc::as_ref)
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
    /// installed extended-span tables, failures, canonical paths and its path trie. No source
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
    /// snapshots, installed extended-span tables, failures, canonical paths and path trie into a
    /// fresh [`FrozenSourceDatabase`]. Sharing the owner shares the one merged root string
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

    /// Borrow the lookup-only source owner.
    #[inline]
    #[allow(dead_code)] // Retained for deferred frozen-source lookup consumers.
    pub(crate) fn sources(&self) -> &FrozenSourceDatabase {
        &self.sources
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

    /// Resolve a string ID in this identity context.
    #[inline]
    #[allow(dead_code)] // Retained for deferred frozen-identity string lookup consumers.
    pub(crate) fn resolve_string(&self, id: StringId) -> &str {
        self.strings.resolve(id)
    }

    /// Fallibly resolve a string ID in this identity context.
    #[inline]
    #[allow(dead_code)] // Retained for deferred frozen-identity string lookup consumers.
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

    /// Iterate over physical source slots in deterministic source-identity order.
    #[inline]
    #[allow(dead_code)] // Retained for deferred frozen-source iteration consumers.
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, SourceSlot> {
        self.sources.iter()
    }

    /// Resolve one physical source slot by compact identity.
    #[inline]
    pub(crate) fn get(&self, id: SourceId) -> Option<&SourceSlot> {
        self.sources.get(id)
    }

    /// Resolve one physical source slot by canonical filesystem path.
    #[inline]
    #[allow(dead_code)] // Retained for deferred frozen-source canonical lookup consumers.
    pub(crate) fn get_by_canonical_path(&self, canonical_path: &Path) -> Option<&SourceSlot> {
        self.sources.get_by_canonical_path(canonical_path)
    }

    /// Resolve the unique physical source for an exact frozen logical-path identity.
    #[inline]
    #[allow(dead_code)] // Retained for deferred frozen-source logical-path lookup consumers.
    pub(crate) fn unique_record_for_logical_path(
        &self,
        logical_path: PathId,
    ) -> Option<&SourceSlot> {
        self.sources.unique_record_for_logical_path(logical_path)
    }

    /// Return the compact logical-path identity assigned to one physical source.
    #[inline]
    pub(crate) fn source_logical_path(&self, id: SourceId) -> Option<PathId> {
        self.sources.source_logical_path(id)
    }

    /// Reconstruct the legacy path view for one frozen source identity.
    ///
    /// This is the frozen equivalent of [`SourceDatabase::legacy_logical_path`]. The
    /// component vector is rebuilt from the frozen path table on each call and is never
    /// retained.
    #[inline]
    #[allow(dead_code)] // Retained for deferred frozen-source path compatibility consumers.
    pub(crate) fn legacy_logical_path(&self, id: SourceId) -> InternedPath {
        self.sources.legacy_logical_path(id)
    }

    /// Borrow the exact retained source snapshot for one physical source.
    #[inline]
    #[allow(dead_code)] // Retained for deferred frozen-source snapshot consumers.
    pub(crate) fn retained_text(&self, id: SourceId) -> Option<&str> {
        self.sources.retained_text(id)
    }

    /// Construct a line index over one retained source snapshot.
    #[inline]
    pub(crate) fn line_index(&self, id: SourceId) -> Option<LineIndex<'_>> {
        self.sources.line_index(id)
    }

    /// Return a source's structured load failure, when loading failed.
    #[inline]
    #[allow(dead_code)] // Retained for deferred frozen-source failure consumers.
    pub(crate) fn source_load_error(&self, id: SourceId) -> Option<&CompilerError> {
        self.sources.source_load_error(id)
    }

    /// Resolve the loaded source record for exact frozen span resolution.
    #[inline]
    pub(crate) fn source_record(&self, id: SourceId) -> &SourceRecord {
        self.sources.source_record(id)
    }
}

#[cfg(test)]
mod tests {
    use super::{FrozenIdentityContext, FrozenIdentityHandle};
    use crate::compiler_frontend::source::SourceDatabase;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use std::sync::Arc;

    #[test]
    fn frozen_identity_handle_is_single_assignment() {
        let identity = Arc::new(FrozenIdentityContext::from_parts(
            StringTable::new(),
            SourceDatabase::empty(),
        ));
        let other_identity = Arc::new(FrozenIdentityContext::from_parts(
            StringTable::new(),
            SourceDatabase::empty(),
        ));
        let handle = FrozenIdentityHandle::new();

        assert!(handle.get().is_none());
        handle
            .install(Arc::clone(&identity))
            .expect("first frozen identity assignment should succeed");
        assert!(handle.get().is_some());
        handle
            .install(Arc::clone(&identity))
            .expect("reinstalling the same frozen identity should be idempotent");
        let error = handle
            .install(other_identity)
            .expect_err("a handle must reject a different frozen identity");
        assert!(error.msg.contains("two different contexts"));
    }
}
