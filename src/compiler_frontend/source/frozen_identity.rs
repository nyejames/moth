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
//! [`FrozenStringTable`]. Every `PathId` therefore remains paired with the exact `StringId` table
//! that issued its components.

use super::line_index::LineIndex;
use super::{FrozenSourceDatabase, SourceDatabase, SourceId, SourceRecord, SourceSlot};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringId, StringTable,
};
use std::path::Path;

/// Immutable source, string and logical-path identity for one compiler boundary.
///
/// This context owns no diagnostics, report-local type data, compiler driver or scheduler. It is
/// the lookup/rendering seam used by later source and diagnostic boundaries.
#[derive(Debug)]
pub(crate) struct FrozenIdentityContext {
    sources: FrozenSourceDatabase,
    strings: FrozenStringTable,
}

impl FrozenIdentityContext {
    /// Consume the merged root string table and finalized source database.
    ///
    /// The source database's [`SourceDatabase::freeze`] operation moves retained snapshots,
    /// installed extended-span tables, failures, canonical paths and its path trie. No source
    /// text or span storage is cloned, and no independent path table is created.
    pub(crate) fn from_parts(strings: StringTable, sources: SourceDatabase) -> Self {
        Self {
            sources: sources.freeze(),
            strings: strings.freeze(),
        }
    }

    /// Borrow the lookup-only source owner.
    #[inline]
    pub(crate) fn sources(&self) -> &FrozenSourceDatabase {
        &self.sources
    }

    /// Borrow the immutable string table used by compact string IDs.
    #[inline]
    pub(crate) fn strings(&self) -> &FrozenStringTable {
        &self.strings
    }

    /// Borrow the source owner's immutable parent-linked path table.
    #[inline]
    pub(crate) fn paths(&self) -> &PathTable {
        self.sources.paths()
    }

    /// Resolve a string ID in this identity context.
    #[inline]
    pub(crate) fn resolve_string(&self, id: StringId) -> &str {
        self.strings.resolve(id)
    }

    /// Fallibly resolve a string ID in this identity context.
    #[inline]
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
    pub(crate) fn get_by_canonical_path(&self, canonical_path: &Path) -> Option<&SourceSlot> {
        self.sources.get_by_canonical_path(canonical_path)
    }

    /// Resolve the unique physical source for an exact frozen logical-path identity.
    #[inline]
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

    /// Borrow the exact retained source snapshot for one physical source.
    #[inline]
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
    pub(crate) fn source_load_error(&self, id: SourceId) -> Option<&CompilerError> {
        self.sources.source_load_error(id)
    }

    /// Resolve the loaded source record for exact frozen span resolution.
    #[inline]
    pub(crate) fn source_record(&self, id: SourceId) -> &SourceRecord {
        self.sources.source_record(id)
    }
}
