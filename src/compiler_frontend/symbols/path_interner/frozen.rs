//! Dense path-table storage and lookup-only operations.
//!
//! WHAT: resolves parents and components, then renders portable spellings from dense path nodes
//!       while accepting caller-owned scratch storage for component walks.
//! WHY:  the mutable builder and frozen readers share one parent-linked table, while filesystem
//!       `PathBuf` and source snapshot identity remain separate owners.

use super::builder::PathNode;
use super::id::PathId;
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringId, StringTable,
};

/// Immutable path trie storage shared by the mutable builder and frozen readers.
///
/// The table owns only parent links, component IDs and depths. The builder owns the reverse
/// lookup while paths are being interned, then moves this table out when it freezes.
#[derive(Debug, PartialEq, Eq)]
pub struct PathTable {
    nodes: Vec<PathNode>,
    depths: Vec<u32>,
}

impl PathTable {
    pub(super) fn new() -> Self {
        // The root's absent parent is the table terminator. Its component is a valid-shaped
        // placeholder that is never read.
        let root = PathNode {
            parent: None,
            component: StringId::from_index(0),
        };
        Self {
            nodes: vec![root],
            depths: vec![0],
        }
    }

    /// Append one child node and return its complete-path identity.
    ///
    /// `None` reports authored exhaustion of the compact path-node domain.
    pub(super) fn try_append_child(
        &mut self,
        parent: PathId,
        component: StringId,
    ) -> Option<PathId> {
        let child = PathId::try_from_index(self.nodes.len())?;
        let child_depth = self.depth(parent).checked_add(1)?;

        self.nodes.push(PathNode {
            parent: Some(parent),
            component,
        });
        self.depths.push(child_depth);
        Some(child)
    }

    /// Return the parent path, or `None` for the root.
    pub fn parent(&self, path: PathId) -> Option<PathId> {
        self.nodes[path.index()].parent
    }

    /// Return the final component, or `None` for the root path.
    pub fn component(&self, path: PathId) -> Option<StringId> {
        if path == PathId::ROOT {
            return None;
        }
        Some(self.nodes[path.index()].component)
    }

    /// Return the number of components in `path`.
    pub fn depth(&self, path: PathId) -> u32 {
        self.depths[path.index()]
    }

    /// Fill `scratch` with `path`'s components in forward order and return that slice.
    ///
    /// Existing scratch entries are discarded so one caller-owned allocation can be reused across
    /// paths without stale components. Components are collected by walking parents, then reversed
    /// in place into forward order.
    pub fn resolve_components<'a>(
        &self,
        path: PathId,
        scratch: &'a mut Vec<StringId>,
    ) -> &'a [StringId] {
        scratch.clear();
        let mut current = path;
        let mut remaining = self.depth(path);
        while remaining > 0 {
            scratch.push(
                self.component(current)
                    .expect("a non-root path must carry a component"),
            );
            current = self
                .parent(current)
                .expect("a non-root path must carry a parent");
            remaining -= 1;
        }
        debug_assert_eq!(current, PathId::ROOT);
        scratch.reverse();
        scratch
    }

    /// Render a path with portable forward-slash separators.
    pub fn render_portable(
        &self,
        path: PathId,
        string_table: &StringTable,
        scratch: &mut Vec<StringId>,
    ) -> String {
        self.render_portable_with(path, string_table, scratch)
    }

    /// Render a path using immutable strings after the identity freeze boundary.
    ///
    /// This leaves [`Self::render_portable`] unchanged for mutable build-stage callers while
    /// allowing a frozen identity context to render the same stable `PathId` without copying or
    /// rebuilding its string table.
    pub fn render_portable_frozen(
        &self,
        path: PathId,
        string_table: &FrozenStringTable,
        scratch: &mut Vec<StringId>,
    ) -> String {
        self.render_portable_with(path, string_table, scratch)
    }

    fn render_portable_with<T: StringTableResolver + ?Sized>(
        &self,
        path: PathId,
        string_table: &T,
        scratch: &mut Vec<StringId>,
    ) -> String {
        let components = self.resolve_components(path, scratch);
        let mut rendered = String::new();
        for (index, component) in components.iter().enumerate() {
            if index > 0 {
                rendered.push('/');
            }
            rendered.push_str(string_table.resolve(*component));
        }
        rendered
    }
}

trait StringTableResolver {
    fn resolve(&self, id: StringId) -> &str;
}

impl StringTableResolver for StringTable {
    fn resolve(&self, id: StringId) -> &str {
        StringTable::resolve(self, id)
    }
}

impl StringTableResolver for FrozenStringTable {
    fn resolve(&self, id: StringId) -> &str {
        FrozenStringTable::resolve(self, id)
    }
}
