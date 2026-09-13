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
use super::NonUtf8PathComponent;
use crate::compiler_frontend::instrumentation::{
    add_frontend_counter, increment_frontend_counter, record_path_max_depth, FrontendCounter,
};
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
#[derive(Clone, Debug)]
pub struct PathInternerBuilder {
    table: PathTable,
    lookup: FxHashMap<(PathId, StringId), PathId>,
}

impl PathInternerBuilder {
    /// Rebuild a mutable builder from a complete fork snapshot.
    ///
    /// This preserves every inherited and worker-local `PathId` while giving the
    /// destination source database the same lookup domain as the discovery fork.
    pub(super) fn from_parts(
        table: PathTable,
        lookup: FxHashMap<(PathId, StringId), PathId>,
    ) -> Self {
        Self { table, lookup }
    }

    /// Create an empty interner whose first node is the root path.
    pub fn new() -> Self {
        increment_frontend_counter(FrontendCounter::PathNodeCount);
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
                increment_frontend_counter(FrontendCounter::PathNodeCount);
                record_path_max_depth(self.table.depth(child));
                Some(*entry.insert(child))
            }
        }
    }

    /// Intern a filesystem path using the exact component semantics of the path table.
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
    pub fn merge_delta_from(
        &mut self,
        delta: &PathInternerFork,
        string_remap: &StringIdRemap,
    ) -> Result<PathIdRemap, PathInternError> {
        let base_len = delta.base_len();

        if base_len > self.table.len() || base_len > delta.len() {
            return Err(PathInternError::BaseMismatch {
                base_len,
                destination_len: self.table.len(),
                delta_len: delta.len(),
            });
        }

        // The claimed base rows must be structurally equivalent to the destination rows they
        // replace; a mismatch would remap valid IDs onto silently different paths.
        for index in 0..base_len {
            let Some(expected) = PathId::try_from_index(index) else {
                return Err(PathInternError::BaseMismatch {
                    base_len,
                    destination_len: self.table.len(),
                    delta_len: delta.len(),
                });
            };

            if self.table.try_parent(expected) != delta.try_parent(expected)
                || self.table.try_component(expected) != delta.try_component(expected)
                || self.table.try_depth(expected) != delta.try_depth(expected)
            {
                return Err(PathInternError::BaseMismatch {
                    base_len,
                    destination_len: self.table.len(),
                    delta_len: delta.len(),
                });
            }
        }

        // A rejected malformed delta must leave no counter trace, so every local parent
        // link is validated before any counter increments or the table is extended.
        let local_len = delta.local_len();
        for offset in 0..local_len {
            let node = delta.local_node(offset);
            let Some(parent) = node.parent else {
                return Err(PathInternError::BaseMismatch {
                    base_len,
                    destination_len: self.table.len(),
                    delta_len: delta.len(),
                });
            };
            let parent_index = parent.index();
            if parent_index >= base_len && parent_index - base_len >= offset {
                // A delta parent must precede its child in node-index order; a forward
                // or self reference would index unmapped suffix rows.
                return Err(PathInternError::BaseMismatch {
                    base_len,
                    destination_len: self.table.len(),
                    delta_len: delta.len(),
                });
            }
        }
        increment_frontend_counter(FrontendCounter::PathDeltaMergeCalls);

        let mut mapped_suffix = Vec::with_capacity(local_len);
        let mut is_identity = true;
        let mut non_identity_entries = 0usize;

        for offset in 0..local_len {
            let old_index = base_len + offset;
            let node = delta.local_node(offset);
            let Some(parent) = node.parent else {
                return Err(PathInternError::BaseMismatch {
                    base_len,
                    destination_len: self.table.len(),
                    delta_len: delta.len(),
                });
            };

            let parent_index = parent.index();
            let remapped_parent = if parent_index < base_len {
                parent
            } else {
                let parent_offset = parent_index - base_len;
                if parent_offset >= offset {
                    // A delta parent must precede its child in node-index order; a forward or
                    // self reference would index unmapped suffix rows.
                    return Err(PathInternError::BaseMismatch {
                        base_len,
                        destination_len: self.table.len(),
                        delta_len: delta.len(),
                    });
                }

                mapped_suffix
                    .get(parent_offset)
                    .copied()
                    .ok_or(PathInternError::BaseMismatch {
                        base_len,
                        destination_len: self.table.len(),
                        delta_len: delta.len(),
                    })?
            };

            let remapped_component = string_remap.get(node.component);
            let merged = self
                .try_intern_child(remapped_parent, remapped_component)
                .ok_or(PathInternError::TableFull)?;

            if merged.index() != old_index {
                is_identity = false;
                non_identity_entries += 1;
            }

            mapped_suffix.push(merged);
        }
        add_frontend_counter(FrontendCounter::PathDeltaEntriesScanned, local_len);
        if is_identity {
            increment_frontend_counter(FrontendCounter::PathDeltaIdentityRemaps);
        } else {
            increment_frontend_counter(FrontendCounter::PathDeltaNonIdentityRemaps);
            add_frontend_counter(
                FrontendCounter::PathDeltaNonIdentityEntries,
                non_identity_entries,
            );
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
/// Merge-base or local-delta invariant violations use [`PathInternError::BaseMismatch`] and
/// stay in the infrastructure failure lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathInternError {
    /// The merge base is not a numeric and structural prefix of the destination table.
    ///
    /// The release merge-base contract requires the claimed base rows of a delta to be
    /// structurally equivalent (same parent, component, and depth) to the destination rows
    /// they replace, and to fit inside both tables. Violations would otherwise remap valid
    /// IDs onto silently different paths, so merging rejects them in all build profiles.
    BaseMismatch {
        /// The claimed shared base length of the merge.
        base_len: usize,
        /// The node count of the destination table.
        destination_len: usize,
        /// The node count of the delta table.
        delta_len: usize,
    },
    NonUtf8(NonUtf8PathComponent),
    /// The build-lifetime table cannot address another node.
    TableFull,
}
