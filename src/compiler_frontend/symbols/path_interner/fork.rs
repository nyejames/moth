//! Module-local path forks over one immutable base.
//!
//! WHAT: snapshots the live builder into a reusable shared base, then lets each worker intern
//!       children of base paths and of its own local nodes without cloning the full table.
//! WHY:  parallel workers must share inherited source paths by numeric identity while keeping
//!       their own append-only suffix, mirroring the string-table fork pattern.

use super::builder::{PathInternError, PathNode};
use super::id::PathId;
use crate::compiler_frontend::symbols::interned_path::NonUtf8PathComponent;
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringId, StringTable, StringTableResolver,
};
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Immutable prefix shared by every path fork in one parallel wave.
#[derive(Debug)]
#[allow(dead_code)] // Slice 2B wires module workers to this shared path base.
struct PathTableBase {
    nodes: Box<[PathNode]>,
    depths: Box<[u32]>,
    lookup: FxHashMap<(PathId, StringId), PathId>,
}

/// Reusable source for cheap module-local path forks that share one inherited prefix.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Slice 2B wires module workers to this fork source.
pub struct PathInternerForkSource {
    base: Arc<PathTableBase>,
}

#[allow(dead_code)] // Slice 2B wires module workers to this fork source.
impl PathInternerForkSource {
    pub(super) fn new(
        nodes: Box<[PathNode]>,
        depths: Box<[u32]>,
        lookup: FxHashMap<(PathId, StringId), PathId>,
    ) -> Self {
        Self {
            base: Arc::new(PathTableBase {
                nodes,
                depths,
                lookup,
            }),
        }
    }

    /// Return the number of inherited path nodes shared by every fork.
    pub fn base_len(&self) -> usize {
        self.base.nodes.len()
    }

    /// Create one worker delta that starts with an empty local suffix.
    pub fn fork_for_module(&self) -> PathInternerFork {
        PathInternerFork {
            base: Arc::clone(&self.base),
            base_len: self.base.nodes.len(),
            nodes: Vec::new(),
            depths: Vec::new(),
            lookup: FxHashMap::default(),
        }
    }
}

/// A module-local path delta plus the inherited prefix length used at merge time.
///
/// Worker `PathId`s below the base length equal the builder's IDs. Local IDs continue after the
/// prefix in append order. A fork never freezes into its own identity domain; deltas merge into
/// the root builder before the consuming freeze.
#[derive(Debug)]
#[allow(dead_code)] // Slice 2B wires module workers to this path delta.
pub struct PathInternerFork {
    base: Arc<PathTableBase>,
    base_len: usize,
    nodes: Vec<PathNode>,
    depths: Vec<u32>,
    lookup: FxHashMap<(PathId, StringId), PathId>,
}

#[allow(dead_code)] // Slice 2B wires module workers to this path delta.
impl PathInternerFork {
    /// Return the inherited prefix length this fork was created from.
    pub fn base_len(&self) -> usize {
        self.base_len
    }

    /// Return the total number of addressable nodes, including the inherited prefix.
    pub fn len(&self) -> usize {
        self.base_len + self.nodes.len()
    }

    /// Return whether `path` was issued by this fork or its inherited base.
    pub fn contains(&self, path: PathId) -> bool {
        path.index() < self.len()
    }

    /// Return the number of worker-local nodes interned after the inherited prefix.
    pub(super) fn local_len(&self) -> usize {
        self.nodes.len()
    }

    /// Borrow one worker-local node by its suffix offset.
    pub(super) fn local_node(&self, offset: usize) -> PathNode {
        self.nodes[offset]
    }

    /// Borrow one inherited base depth for merge-time prefix checks.
    pub(super) fn base_depth(&self, index: usize) -> u32 {
        self.base.depths[index]
    }

    /// Return the parent path, or `None` for the root.
    pub fn parent(&self, path: PathId) -> Option<PathId> {
        let index = path.index();

        if index < self.base_len {
            return self.base.nodes[index].parent;
        }

        self.nodes[index - self.base_len].parent
    }

    /// Return the parent path, or `None` for the root and for foreign IDs.
    pub fn try_parent(&self, path: PathId) -> Option<PathId> {
        let index = path.index();

        if index < self.base_len {
            return self.base.nodes.get(index)?.parent;
        }

        self.nodes.get(index - self.base_len)?.parent
    }

    /// Return the final component, or `None` for the root path.
    pub fn component(&self, path: PathId) -> Option<StringId> {
        if path == PathId::ROOT {
            return None;
        }

        let index = path.index();

        if index < self.base_len {
            return Some(self.base.nodes[index].component);
        }

        Some(self.nodes[index - self.base_len].component)
    }

    /// Return the final component, or `None` for the root and for foreign IDs.
    pub fn try_component(&self, path: PathId) -> Option<StringId> {
        if path == PathId::ROOT {
            return None;
        }

        let index = path.index();

        if index < self.base_len {
            return Some(self.base.nodes.get(index)?.component);
        }

        Some(self.nodes.get(index - self.base_len)?.component)
    }

    /// Return the number of components in `path`.
    pub fn depth(&self, path: PathId) -> u32 {
        let index = path.index();

        if index < self.base_len {
            return self.base.depths[index];
        }

        self.depths[index - self.base_len]
    }

    /// Return the depth, or `None` when `path` was not issued by this fork.
    pub fn try_depth(&self, path: PathId) -> Option<u32> {
        let index = path.index();

        if index < self.base_len {
            return self.base.depths.get(index).copied();
        }

        self.depths.get(index - self.base_len).copied()
    }

    /// Return whether `path` descends from `prefix` without allocating.
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

    /// Intern one component below `parent`, reusing an existing child when present.
    ///
    /// `None` reports authored exhaustion of the compact path-node domain.
    pub fn try_intern_child(&mut self, parent: PathId, component: StringId) -> Option<PathId> {
        if let Some(&existing) = self.lookup.get(&(parent, component)) {
            return Some(existing);
        }

        if let Some(&existing) = self.base.lookup.get(&(parent, component)) {
            return Some(existing);
        }

        let parent_depth = self.depth(parent);
        let child_depth = parent_depth.checked_add(1)?;
        let child = PathId::try_from_index(self.base_len + self.nodes.len())?;

        self.nodes.push(PathNode {
            parent: Some(parent),
            component,
        });
        self.depths.push(child_depth);
        self.lookup.insert((parent, component), child);

        Some(child)
    }

    /// Walk the interned components of `suffix` onto `prefix` without allocating the identity.
    ///
    /// Caller-owned `scratch` carries the forward component walk. `None` reports authored
    /// exhaustion of the compact path-node domain.
    pub fn try_join(
        &mut self,
        prefix: PathId,
        suffix: PathId,
        scratch: &mut Vec<StringId>,
    ) -> Option<PathId> {
        self.resolve_components(suffix, scratch);

        // The component walk borrows only the caller scratch, so interning below can reuse the
        // collected suffix without holding the fork borrow across the loop.
        let suffix_len = scratch.len();
        let mut joined = prefix;

        for index in 0..suffix_len {
            let component = scratch[index];
            joined = self.try_intern_child(joined, component)?;
        }

        Some(joined)
    }

    /// Intern a filesystem path using the exact component semantics shared with `InternedPath`.
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
}
