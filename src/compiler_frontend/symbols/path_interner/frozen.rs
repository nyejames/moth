//! Lookup-only path-table operations.
//!
//! WHAT: resolves parents and components, then renders portable spellings from dense path nodes
//!       while accepting caller-owned scratch storage for component walks.
//! WHY:  frozen build identities must be inspectable without mutable interning or per-operation
//!       component-vector ownership.

use super::builder::PathNode;
use super::id::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

/// Immutable path trie produced by [`super::builder::PathInternerBuilder::freeze`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathTable {
    nodes: Vec<PathNode>,
    depths: Vec<u32>,
}

impl PathTable {
    pub(super) fn from_parts(nodes: Vec<PathNode>, depths: Vec<u32>) -> Self {
        debug_assert_eq!(nodes.len(), depths.len());
        debug_assert!(!nodes.is_empty());
        debug_assert!(nodes[PathId::ROOT.index()].parent.is_none());
        Self { nodes, depths }
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
