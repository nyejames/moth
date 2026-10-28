//! Dense path-table storage and lookup-only operations.
//!
//! WHAT: resolves parents and components, then renders portable and native spellings from dense
//!       path nodes while accepting caller-owned scratch storage for component walks.
//! WHY:  the mutable builder and frozen readers share one parent-linked table, while filesystem
//!       `PathBuf` and source snapshot identity remain separate owners.

use super::builder::PathNode;
use super::id::PathId;
use crate::compiler_frontend::instrumentation::record_path_table_copy;
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringId, StringIdRemap, StringTable, StringTableResolver,
};
use std::path::PathBuf;
use std::sync::Arc;

/// One immutable contiguous run of path nodes.
#[derive(Debug)]
struct PathTableSegment {
    nodes: Vec<PathNode>,
    depths: Vec<u32>,
}

/// Immutable path storage shared by builders, forks and captured readers.
///
/// Segments let a builder publish a new prefix by moving only its local suffix into shared
/// storage. Existing path rows remain in their original allocations.
#[derive(Debug)]
pub(super) struct PathTableBase {
    segments: Box<[Arc<PathTableSegment>]>,
    segment_ends: Box<[usize]>,
    len: usize,
    max_component_index: usize,
}

impl PathTableBase {
    pub(super) fn from_parts(nodes: Vec<PathNode>, depths: Vec<u32>) -> Self {
        let segment = Arc::new(PathTableSegment { nodes, depths });
        let len = segment.nodes.len();
        let max_component_index = segment
            .nodes
            .iter()
            .map(|node| node.component.index() as usize)
            .max()
            .unwrap_or(0);
        Self {
            segments: vec![segment].into_boxed_slice(),
            segment_ends: vec![len].into_boxed_slice(),
            len,
            max_component_index,
        }
    }

    fn with_segment(&self, segment: Arc<PathTableSegment>) -> Self {
        let mut segments = Vec::with_capacity(self.segments.len() + 1);
        segments.extend(self.segments.iter().cloned());
        segments.push(segment);

        let segment_len = segments.last().expect("new segment exists").nodes.len();
        let len = self.len + segment_len;
        let mut segment_ends = Vec::with_capacity(segments.len());
        segment_ends.extend(self.segment_ends.iter().copied());
        segment_ends.push(len);
        let max_component_index = segments
            .last()
            .expect("new segment exists")
            .nodes
            .iter()
            .map(|node| node.component.index() as usize)
            .max()
            .map_or(self.max_component_index, |max| {
                self.max_component_index.max(max)
            });

        Self {
            segments: segments.into_boxed_slice(),
            segment_ends: segment_ends.into_boxed_slice(),
            len,
            max_component_index,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn node(&self, index: usize) -> Option<PathNode> {
        if index >= self.len {
            return None;
        }
        if self.segments.len() == 1 {
            return self.segments[0].nodes.get(index).copied();
        }
        let last_segment = self.segments.last()?;
        let last_segment_start = self.len - last_segment.nodes.len();
        if index >= last_segment_start {
            return last_segment.nodes.get(index - last_segment_start).copied();
        }

        let segment_index = self.segment_ends.partition_point(|end| *end <= index);
        let start = if segment_index == 0 {
            0
        } else {
            self.segment_ends[segment_index - 1]
        };
        self.segments
            .get(segment_index)?
            .nodes
            .get(index - start)
            .copied()
    }

    pub(super) fn depth(&self, index: usize) -> Option<u32> {
        if index >= self.len {
            return None;
        }
        if self.segments.len() == 1 {
            return self.segments[0].depths.get(index).copied();
        }
        let last_segment = self.segments.last()?;
        let last_segment_start = self.len - last_segment.depths.len();
        if index >= last_segment_start {
            return last_segment.depths.get(index - last_segment_start).copied();
        }

        let segment_index = self.segment_ends.partition_point(|end| *end <= index);
        let start = if segment_index == 0 {
            0
        } else {
            self.segment_ends[segment_index - 1]
        };
        self.segments
            .get(segment_index)?
            .depths
            .get(index - start)
            .copied()
    }

    pub(super) fn snapshot_with_suffix(&self, nodes: &[PathNode], depths: &[u32]) -> Arc<Self> {
        let copied_bytes = nodes
            .len()
            .saturating_mul(std::mem::size_of::<PathNode>())
            .saturating_add(depths.len().saturating_mul(std::mem::size_of::<u32>()));
        record_path_table_copy(nodes.len(), copied_bytes);
        Arc::new(self.with_segment(Arc::new(PathTableSegment {
            nodes: nodes.to_vec(),
            depths: depths.to_vec(),
        })))
    }

    pub(super) fn copy_into(&self, nodes: &mut Vec<PathNode>, depths: &mut Vec<u32>) {
        for segment in self.segments.iter() {
            nodes.extend_from_slice(&segment.nodes);
            depths.extend_from_slice(&segment.depths);
        }
    }

    #[cfg(feature = "data_layout_memory_probe")]
    fn storage_bytes(&self) -> usize {
        self.segments
            .iter()
            .map(|segment| {
                segment
                    .nodes
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PathNode>())
                    .saturating_add(
                        segment
                            .depths
                            .capacity()
                            .saturating_mul(std::mem::size_of::<u32>()),
                    )
            })
            .sum()
    }
}

/// Immutable path trie storage shared by the mutable builder and frozen readers.
///
/// A table retains its inherited prefix through `base` and owns only its producer-local suffix.
#[derive(Debug)]
pub struct PathTable {
    base: Arc<PathTableBase>,
    nodes: Vec<PathNode>,
    depths: Vec<u32>,
}

impl PartialEq for PathTable {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && (0..self.len()).all(|index| {
                self.node(index) == other.node(index)
                    && self.depth_at(index) == other.depth_at(index)
            })
    }
}

impl Eq for PathTable {}

impl Clone for PathTable {
    fn clone(&self) -> Self {
        if !self.nodes.is_empty() {
            record_path_table_copy(self.nodes.len(), self.copied_delta_bytes());
        }
        Self {
            base: Arc::clone(&self.base),
            nodes: self.nodes.clone(),
            depths: self.depths.clone(),
        }
    }
}

impl PathTable {
    pub(super) fn from_parts(nodes: Vec<PathNode>, depths: Vec<u32>) -> Self {
        Self {
            base: Arc::new(PathTableBase::from_parts(nodes, depths)),
            nodes: Vec::new(),
            depths: Vec::new(),
        }
    }

    pub(super) fn from_shared_parts(
        base: Arc<PathTableBase>,
        nodes: Vec<PathNode>,
        depths: Vec<u32>,
    ) -> Self {
        if !nodes.is_empty() {
            let copied_bytes = nodes
                .len()
                .saturating_mul(std::mem::size_of::<PathNode>())
                .saturating_add(depths.len().saturating_mul(std::mem::size_of::<u32>()));
            record_path_table_copy(nodes.len(), copied_bytes);
        }
        Self {
            base,
            nodes,
            depths,
        }
    }

    /// Rewrite component IDs after the owning diagnostic string table merges.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if !remap.is_identity() && self.base.max_component_index >= remap.identity_prefix_len() {
            let node_count = self.base.len();
            let copied_bytes = node_count
                .saturating_mul(std::mem::size_of::<PathNode>())
                .saturating_add(node_count.saturating_mul(std::mem::size_of::<u32>()));
            record_path_table_copy(node_count, copied_bytes);
            let mut nodes = Vec::with_capacity(node_count);
            let mut depths = Vec::with_capacity(node_count);
            self.base.copy_into(&mut nodes, &mut depths);
            for node in &mut nodes {
                node.component = remap.get(node.component);
            }
            self.base = Arc::new(PathTableBase::from_parts(nodes, depths));
        }
        for node in &mut self.nodes {
            node.component = remap.get(node.component);
        }
    }

    pub(super) fn new() -> Self {
        // The root's absent parent is the table terminator. Its component is a valid-shaped
        // placeholder that is never read.
        let root = PathNode {
            parent: None,
            component: StringId::from_index(0),
        };
        Self::from_parts(vec![root], vec![0])
    }

    /// Append one child node and return its complete-path identity.
    ///
    /// `None` reports authored exhaustion of the compact path-node domain.
    pub(super) fn try_append_child(
        &mut self,
        parent: PathId,
        component: StringId,
    ) -> Option<PathId> {
        #[cfg(test)]
        if super::test_exhaustion::forced_exhaustion() {
            // Test-only: reject the new node so tests reach the exhaustion boundary without
            // allocating the full compact domain. Callers that reuse an already-interned child
            // never reach this method and keep succeeding.
            return None;
        }
        let child = PathId::try_from_index(self.len())?;
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
        self.base.len() + self.nodes.len()
    }

    /// Return the backing storage represented by this table, excluding Arc headers.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn storage_bytes(&self) -> usize {
        self.base
            .storage_bytes()
            .saturating_add(
                self.nodes
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PathNode>()),
            )
            .saturating_add(
                self.depths
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
    }

    fn copied_delta_bytes(&self) -> usize {
        self.nodes
            .len()
            .saturating_mul(std::mem::size_of::<PathNode>())
            .saturating_add(self.depths.len().saturating_mul(std::mem::size_of::<u32>()))
    }

    fn node(&self, index: usize) -> Option<PathNode> {
        if index < self.base.len() {
            self.base.node(index)
        } else {
            self.nodes.get(index - self.base.len()).copied()
        }
    }

    fn depth_at(&self, index: usize) -> Option<u32> {
        if index < self.base.len() {
            self.base.depth(index)
        } else {
            self.depths.get(index - self.base.len()).copied()
        }
    }

    pub(super) fn shared_base(&self) -> Arc<PathTableBase> {
        Arc::clone(&self.base)
    }

    /// Share the current prefix and clone only this table's local suffix into a new prefix.
    pub(super) fn snapshot_base(&self) -> Arc<PathTableBase> {
        if self.nodes.is_empty() {
            return Arc::clone(&self.base);
        }
        let copied_bytes = self.copied_delta_bytes();
        record_path_table_copy(self.nodes.len(), copied_bytes);
        let segment = Arc::new(PathTableSegment {
            nodes: self.nodes.clone(),
            depths: self.depths.clone(),
        });
        Arc::new(self.base.with_segment(segment))
    }

    /// Move this table's mutable suffix into an immutable shared prefix.
    pub(super) fn seal_shared_base(&mut self) {
        if self.nodes.is_empty() {
            return;
        }
        let segment = Arc::new(PathTableSegment {
            nodes: std::mem::take(&mut self.nodes),
            depths: std::mem::take(&mut self.depths),
        });
        self.base = Arc::new(self.base.with_segment(segment));
    }

    /// Return whether the numeric index of `path` is addressable by this table.
    ///
    /// This proves range only, not provenance: a table from an independent identity domain can
    /// address the same numeric index with different components. Path identity is meaningful
    /// only together with the string table that issued the component IDs.
    pub fn contains(&self, path: PathId) -> bool {
        path.index() < self.len()
    }

    /// Return the parent path, or `None` for the root.
    pub fn parent(&self, path: PathId) -> Option<PathId> {
        self.node(path.index())
            .expect("path ID is addressable")
            .parent
    }

    /// Return the parent path, or `None` when `path` is the root or was not issued here.
    pub fn try_parent(&self, path: PathId) -> Option<PathId> {
        self.node(path.index())?.parent
    }

    /// Return the final component, or `None` for the root path.
    pub fn component(&self, path: PathId) -> Option<StringId> {
        if path == PathId::ROOT {
            return None;
        }
        Some(
            self.node(path.index())
                .expect("path ID is addressable")
                .component,
        )
    }

    /// Return the final component, or `None` for the root and for foreign IDs.
    pub fn try_component(&self, path: PathId) -> Option<StringId> {
        if path == PathId::ROOT {
            return None;
        }
        Some(self.node(path.index())?.component)
    }

    /// Return the number of components in `path`.
    pub fn depth(&self, path: PathId) -> u32 {
        self.depth_at(path.index()).expect("path ID is addressable")
    }

    /// Return the depth, or `None` when `path` was not issued by this table.
    pub fn try_depth(&self, path: PathId) -> Option<u32> {
        self.depth_at(path.index())
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
}
#[cfg(feature = "data_layout_memory_probe")]
impl Drop for PathTable {
    fn drop(&mut self) {
        crate::compiler_frontend::instrumentation::release_donor_identity_path(self);
    }
}
