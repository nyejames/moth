//! Mutable parent-linked path interning.
//!
//! WHAT: builds one canonical node for each unique `(parent, StringId)` path extension in the
//!       table shared with readers, then moves that table across the freeze boundary.
//! WHY:  parent links share every common prefix without storing a component vector in each path
//!       identity or requiring a globally locked interner.

use super::frozen::PathTable;
use super::id::PathId;
use crate::compiler_frontend::symbols::interned_path::NonUtf8PathComponent;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use rustc_hash::FxHashMap;
use std::collections::hash_map::Entry;
use std::path::Path;

/// One path-trie node. The root's component is deliberately uninterpreted.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PathNode {
    pub(super) parent: Option<PathId>,
    pub(super) component: StringId,
}

/// Mutable builder for a dense parent-linked logical path table.
///
/// The table is shared with reader operations while the builder is live. Its `lookup` map is used
/// only while interning and is dropped when the table freezes.
#[derive(Debug)]
pub struct PathInternerBuilder {
    table: PathTable,
    lookup: FxHashMap<(PathId, StringId), PathId>,
}

impl PathInternerBuilder {
    /// Create an empty interner whose first node is the root path.
    pub fn new() -> Self {
        Self {
            table: PathTable::new(),
            lookup: FxHashMap::default(),
        }
    }

    /// Borrow the path table while this builder remains mutable.
    pub fn paths(&self) -> &PathTable {
        &self.table
    }

    /// Intern one component below `parent`, reusing an existing child when present.
    ///
    /// `None` reports authored exhaustion of the compact path-node domain.
    pub fn try_intern_child(&mut self, parent: PathId, component: StringId) -> Option<PathId> {
        match self.lookup.entry((parent, component)) {
            Entry::Occupied(entry) => Some(*entry.get()),
            Entry::Vacant(entry) => {
                let child = self.table.try_append_child(parent, component)?;
                Some(*entry.insert(child))
            }
        }
    }

    /// Intern a filesystem path using the exact component semantics shared with `InternedPath`.
    ///
    /// Filesystem components are validated as strict UTF-8 before their string IDs enter the
    /// table. No separator normalization or spelling rewrite is performed here. Exhaustion of
    /// the compact path-node domain is reported so the owning database can surface the typed
    /// source-capacity failure.
    pub fn try_intern_filesystem_path(
        &mut self,
        path: &Path,
        string_table: &mut StringTable,
    ) -> Result<PathId, PathInternError> {
        let mut logical_path = PathId::ROOT;
        for component in path.components() {
            let component_str = component.as_os_str().to_str().ok_or_else(|| {
                PathInternError::NonUtf8(NonUtf8PathComponent {
                    path: path.to_path_buf(),
                })
            })?;
            let component_id = string_table.intern(component_str);
            logical_path = self
                .try_intern_child(logical_path, component_id)
                .ok_or(PathInternError::TableFull)?;
        }
        Ok(logical_path)
    }

    /// Intern a portable forward-slash path without changing its exact separator spelling.
    ///
    /// WHY: source logical spellings are already canonical at production boundaries, and silently
    /// dropping empty segments would make distinct string identities equal. Empty components are
    /// therefore interned exactly like non-empty components; only the empty spelling denotes root.
    /// Backslashes are ordinary component text because this API accepts portable logical spelling
    /// rather than a filesystem path.
    #[allow(dead_code)] // Phase 2 migrates semantic path producers to this table.
    pub fn try_intern_portable_path(
        &mut self,
        spelling: &str,
        string_table: &mut StringTable,
    ) -> Result<PathId, PathInternError> {
        if spelling.is_empty() {
            return Ok(PathId::ROOT);
        }

        let mut path = PathId::ROOT;
        for component in spelling.split('/') {
            let component_id = string_table.intern(component);
            path = self
                .try_intern_child(path, component_id)
                .ok_or(PathInternError::TableFull)?;
        }
        Ok(path)
    }

    /// Freeze the append-only table into lookup-only path storage.
    pub fn freeze(self) -> PathTable {
        self.table
    }
}

/// Failure to intern one path in the build-lifetime table.
///
/// Authored interning must never panic: a project that exhausts the compact path-node domain
/// reaches the deterministic source-capacity lane through [`PathInternError::TableFull`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathInternError {
    /// A filesystem component is not strict UTF-8.
    NonUtf8(NonUtf8PathComponent),
    /// The build-lifetime table cannot address another node.
    TableFull,
}
