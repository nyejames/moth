//! Dense path-table storage and lookup-only operations.
//!
//! WHAT: resolves parents and components, then renders portable and native spellings from dense
//!       path nodes while accepting caller-owned scratch storage for component walks.
//! WHY:  the mutable builder and frozen readers share one parent-linked table, while filesystem
//!       `PathBuf` and source snapshot identity remain separate owners.

use super::builder::PathNode;
use super::id::PathId;
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringId, StringTable, StringTableResolver,
};
use std::path::PathBuf;

/// Immutable path trie storage shared by the mutable builder and frozen readers.
///
/// The table owns only parent links, component IDs and depths. The builder owns the reverse
/// lookup while paths are being interned, then moves this table out when it freezes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathTable {
    nodes: Vec<PathNode>,
    depths: Vec<u32>,
}

#[allow(dead_code)] // Slice 2B wires complete path operations into module compilation.
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

    /// Return the number of path nodes in this table, including the root.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Return whether `path` was issued by this table.
    pub fn contains(&self, path: PathId) -> bool {
        path.index() < self.nodes.len()
    }

    /// Return the parent path, or `None` for the root.
    pub fn parent(&self, path: PathId) -> Option<PathId> {
        self.nodes[path.index()].parent
    }

    /// Return the parent path, or `None` when `path` is the root or was not issued here.
    pub fn try_parent(&self, path: PathId) -> Option<PathId> {
        self.nodes.get(path.index())?.parent
    }

    /// Return the final component, or `None` for the root path.
    pub fn component(&self, path: PathId) -> Option<StringId> {
        if path == PathId::ROOT {
            return None;
        }
        Some(self.nodes[path.index()].component)
    }

    /// Return the final component, or `None` for the root and for foreign IDs.
    pub fn try_component(&self, path: PathId) -> Option<StringId> {
        if path == PathId::ROOT {
            return None;
        }
        Some(self.nodes.get(path.index())?.component)
    }

    /// Return the number of components in `path`.
    pub fn depth(&self, path: PathId) -> u32 {
        self.depths[path.index()]
    }

    /// Return the depth, or `None` when `path` was not issued by this table.
    pub fn try_depth(&self, path: PathId) -> Option<u32> {
        self.depths.get(path.index()).copied()
    }

    /// Return whether `path` descends from `prefix` without allocating.
    ///
    /// The check lifts `path` to the prefix depth through parent links, then compares identities.
    /// Equal paths share the prefix, and the root prefixes every path.
    pub fn starts_with(&self, path: PathId, prefix: PathId) -> bool {
        let path_depth = self.depth(path);
        let prefix_depth = self.depth(prefix);

        if prefix_depth > path_depth {
            return false;
        }

        let mut current = path;

        for _ in 0..(path_depth - prefix_depth) {
            current = self
                .parent(current)
                .expect("a non-root path must carry a parent");
        }

        current == prefix
    }

    /// Return whether `path` ends with `suffix` without allocating.
    ///
    /// Both tips walk towards the root together while their components match. Only a suffix at
    /// the tip counts; an interior-only match is not a suffix.
    pub fn ends_with(&self, path: PathId, suffix: PathId) -> bool {
        let path_depth = self.depth(path);
        let suffix_depth = self.depth(suffix);

        if suffix_depth > path_depth {
            return false;
        }

        let mut path_cursor = path;
        let mut suffix_cursor = suffix;

        for _ in 0..suffix_depth {
            if self.component(path_cursor) != self.component(suffix_cursor) {
                return false;
            }

            path_cursor = self
                .parent(path_cursor)
                .expect("a non-root path must carry a parent");
            suffix_cursor = self
                .parent(suffix_cursor)
                .expect("a non-root path must carry a parent");
        }

        true
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

    /// Render a path as a native `PathBuf` by pushing each resolved component.
    ///
    /// The root renders as an empty `PathBuf`. No rendered text is stored on the node.
    pub fn render_native(
        &self,
        path: PathId,
        string_table: &StringTable,
        scratch: &mut Vec<StringId>,
    ) -> PathBuf {
        self.render_native_with(path, string_table, scratch)
    }

    /// Render a native path using immutable strings after the freeze boundary.
    pub fn render_native_frozen(
        &self,
        path: PathId,
        string_table: &FrozenStringTable,
        scratch: &mut Vec<StringId>,
    ) -> PathBuf {
        self.render_native_with(path, string_table, scratch)
    }

    fn render_native_with<T: StringTableResolver + ?Sized>(
        &self,
        path: PathId,
        string_table: &T,
        scratch: &mut Vec<StringId>,
    ) -> PathBuf {
        let components = self.resolve_components(path, scratch);

        if components.is_empty() {
            return PathBuf::new();
        }

        let mut native = PathBuf::new();

        for component in components {
            native.push(string_table.resolve(*component));
        }

        native
    }

    /// Clone the dense node arrays into an immutable fork base.
    pub(super) fn snapshot(&self) -> (Box<[PathNode]>, Box<[u32]>) {
        (
            self.nodes.clone().into_boxed_slice(),
            self.depths.clone().into_boxed_slice(),
        )
    }
}
