//! Mutable parent-linked path interning.
//!
//! WHAT: builds one canonical node for each unique `(parent, StringId)` path extension in the
//!       table shared with readers, then moves that table across the freeze boundary.
//! WHY:  parent links share every common prefix without storing a component vector in each path
//!       identity or requiring a globally locked interner.

use super::fork::{PathInternerFork, PathInternerForkSource};
use super::frozen::PathTable;
use super::id::PathId;
use super::remap::PathIdRemap;
use crate::compiler_frontend::symbols::interned_path::NonUtf8PathComponent;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
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

    /// Return the number of path nodes interned so far, including the root.
    #[allow(dead_code)] // Slice 2B wires path-table sizing into module compilation.
    pub fn len(&self) -> usize {
        self.table.len()
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

    /// Walk the interned components of `suffix` onto `prefix` without allocating the identity.
    ///
    /// Caller-owned `scratch` carries the forward component walk. `None` reports authored
    /// exhaustion of the compact path-node domain.
    #[allow(dead_code)] // Slice 2B wires allocation-free path joins into module compilation.
    pub fn try_join(
        &mut self,
        prefix: PathId,
        suffix: PathId,
        scratch: &mut Vec<StringId>,
    ) -> Option<PathId> {
        self.table.resolve_components(suffix, scratch);

        let suffix_len = scratch.len();
        let mut joined = prefix;

        for index in 0..suffix_len {
            let component = scratch[index];
            joined = self.try_intern_child(joined, component)?;
        }

        Some(joined)
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

    /// Snapshot the live table into a reusable fork source without consuming the builder.
    ///
    /// Building the shared base copies the current nodes once. Each fork after that clones only
    /// an `Arc` and starts with an empty local delta.
    #[allow(dead_code)] // Slice 2B creates one shared path fork source per module wave.
    pub fn fork_source(&self) -> PathInternerForkSource {
        let (nodes, depths) = self.table.snapshot();

        PathInternerForkSource::new(nodes, depths, self.lookup.clone())
    }

    /// Merge one worker delta into this builder in node-index order.
    ///
    /// WHAT: re-interns each worker-local node through the destination lookup so independently
    ///       interned complete paths collapse to one destination `PathId`.
    /// WHY:  path nodes store `StringId` components from the worker string table, so each local
    ///       component is rewritten through `string_remap` first. Inherited prefix IDs stay
    ///       identity; only the local suffix remaps. Merge order is the caller's canonical order.
    ///
    /// Returns [`PathInternError::TableFull`] when a new destination node cannot be addressed.
    #[allow(dead_code)] // Slice 2B merges module-local path deltas in canonical order.
    pub fn merge_delta_from(
        &mut self,
        delta: &PathInternerFork,
        string_remap: &StringIdRemap,
    ) -> Result<PathIdRemap, PathInternError> {
        let base_len = delta.base_len();

        debug_assert!(base_len <= self.table.len());
        debug_assert!(base_len <= delta.len());

        #[cfg(debug_assertions)]
        for index in 0..base_len {
            let expected = PathId::try_from_index(index)
                .expect("a fork base must address its own prefix");

            debug_assert_eq!(self.table.try_parent(expected), delta.try_parent(expected));
            debug_assert_eq!(
                self.table.try_component(expected),
                delta.try_component(expected)
            );
            debug_assert_eq!(self.table.try_depth(expected), delta.try_depth(expected));
            debug_assert_eq!(Some(delta.base_depth(index)), self.table.try_depth(expected));
        }

        let local_len = delta.local_len();
        let mut mapped_suffix = Vec::with_capacity(local_len);
        let mut is_identity = true;

        for offset in 0..local_len {
            let old_index = base_len + offset;
            let node = delta.local_node(offset);
            let parent = node
                .parent
                .expect("a worker-local node must carry a parent");

            let parent_index = parent.index();
            let remapped_parent = if parent_index < base_len {
                parent
            } else {
                let parent_offset = parent_index - base_len;

                debug_assert!(
                    parent_offset < offset,
                    "a delta parent must precede its child in node-index order"
                );

                mapped_suffix[parent_offset]
            };

            let remapped_component = string_remap.get(node.component);
            let merged = self
                .try_intern_child(remapped_parent, remapped_component)
                .ok_or(PathInternError::TableFull)?;

            if merged.index() != old_index {
                is_identity = false;
            }

            mapped_suffix.push(merged);
        }

        Ok(PathIdRemap::new(base_len, mapped_suffix, is_identity))
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
