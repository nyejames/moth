//! Mutable parent-linked path interning.
//!
//! WHAT: builds one canonical node for each unique `(parent, StringId)` path extension and then
//!       hands the append-only data to the lookup-only [`PathTable`].
//! WHY:  parent links share every common prefix without storing a component vector in each path
//!       identity or requiring a globally locked interner.

use super::frozen::PathTable;
use super::id::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use rustc_hash::FxHashMap;
use std::collections::hash_map::Entry;

/// One path-trie node. The root's component is deliberately uninterpreted.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PathNode {
    pub(super) parent: Option<PathId>,
    pub(super) component: StringId,
}

/// Mutable builder for a dense parent-linked logical path table.
///
/// The builder owns one node and one depth entry per unique path prefix. Its `lookup` map is used
/// only while interning and is dropped when the table freezes.
pub struct PathInternerBuilder {
    nodes: Vec<PathNode>,
    depths: Vec<u32>,
    lookup: FxHashMap<(PathId, StringId), PathId>,
}

impl PathInternerBuilder {
    /// Create an empty interner whose first node is the root path.
    pub fn new() -> Self {
        // The root's absent parent is the table terminator. Its component is a valid-shaped
        // placeholder that is never read.
        let root = PathNode {
            parent: None,
            component: StringId::from_index(0),
        };
        Self {
            nodes: vec![root],
            depths: vec![0],
            lookup: FxHashMap::default(),
        }
    }

    /// Intern one component below `parent`, reusing an existing child when present.
    pub fn intern_child(&mut self, parent: PathId, component: StringId) -> PathId {
        let Self {
            nodes,
            depths,
            lookup,
        } = self;
        match lookup.entry((parent, component)) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let parent_index = parent.index();
                let parent_depth = depths[parent_index];
                let child = PathId::from_index(nodes.len());
                let child_depth = parent_depth
                    .checked_add(1)
                    .expect("path depth must fit in u32");

                nodes.push(PathNode {
                    parent: Some(parent),
                    component,
                });
                depths.push(child_depth);
                entry.insert(child);
                child
            }
        }
    }

    /// Intern a portable forward-slash path without changing its exact separator spelling.
    ///
    /// WHY: source logical spellings are already canonical at production boundaries, and silently
    /// dropping empty segments would make distinct string identities equal. Empty components are
    /// therefore interned exactly like non-empty components; only the empty spelling denotes root.
    /// Backslashes are ordinary component text because this API accepts portable logical spelling
    /// rather than a filesystem path.
    pub fn intern_portable_path(
        &mut self,
        spelling: &str,
        string_table: &mut StringTable,
    ) -> PathId {
        if spelling.is_empty() {
            return PathId::ROOT;
        }

        let mut path = PathId::ROOT;
        for component in spelling.split('/') {
            let component_id = string_table.intern(component);
            path = self.intern_child(path, component_id);
        }
        path
    }

    /// Freeze the append-only nodes into a lookup-only path table.
    pub fn freeze(self) -> PathTable {
        let Self { nodes, depths, .. } = self;
        PathTable::from_parts(nodes, depths)
    }
}
