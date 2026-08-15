//! Efficient path representation using interned string components.
//!
//! WHAT: `InternedPath` stores path components as `Vec<StringId>`, enabling memory-efficient
//!       storage, fast path operations, and efficient comparison when paths share common prefixes.
//! WHY: scope tracking in the compiler frontend produces many paths with shared prefixes; interning
//!      avoids redundant string storage and makes path comparisons cheap.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use std::path::{Path, PathBuf};

/// An efficient path representation using interned string components.
///
/// InternedPath stores path components as a Vec<StringId>, allowing for:
/// - Memory-efficient storage when paths share common components
/// - Fast path operations (append, parent, join_str) using vector operations
/// - Efficient comparison and hashing using StringId equality
/// - Conversion to/from standard PathBuf when needed for file system operations
///
/// This is particularly useful for scope tracking in the compiler_frontend where many
/// paths share common prefixes (like module names or directory structures).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InternedPath {
    /// Path components stored as interned string IDs
    /// Empty vector represents the root path
    components: Vec<StringId>,
}

/// Error returned when a filesystem path contains a non-UTF-8 component.
///
/// WHAT: retains the original `PathBuf` so the owning stage can report the
///      offending path through its correct diagnostic lane.
/// WHY: stage-independent conversion must not guess the error channel. The
///      caller decides whether this is a `CompilerError` or a `CompilerDiagnostic`.
#[derive(Debug, Clone)]
pub(crate) struct NonUtf8PathComponent {
    /// The filesystem path whose component could not be represented as UTF-8.
    pub(crate) path: PathBuf,
}

impl InternedPath {
    /// Create a new empty path (equivalent to root)
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    /// Convert a filesystem path to an `InternedPath`, failing on the first
    /// non-UTF-8 component.
    ///
    /// WHAT: interns each path component's UTF-8 string. If any component's
    ///      `OsStr` is not valid UTF-8, returns the original path unchanged
    ///      inside `NonUtf8PathComponent`.
    /// WHY: lossy conversion can collapse distinct filesystem names into one
    ///      compiler identity. Filesystem identity must be exact or rejected.
    ///      The owning stage maps the failure to its correct error channel.
    pub(crate) fn try_from_filesystem_path(
        path: &Path,
        string_table: &mut StringTable,
    ) -> Result<Self, NonUtf8PathComponent> {
        let mut components = Vec::new();
        for component in path.components() {
            let component_str = component.as_os_str().to_str().ok_or(NonUtf8PathComponent {
                path: path.to_path_buf(),
            })?;
            components.push(string_table.intern(component_str));
        }
        Ok(Self { components })
    }

    /// Create an InternedPath from a vector of StringIds
    pub fn from_components(components: Vec<StringId>) -> Self {
        Self { components }
    }

    pub fn from_single_str(entry: &str, string_table: &mut StringTable) -> Self {
        let interned = string_table.intern(entry);
        Self {
            components: vec![interned],
        }
    }

    /// Convert this InternedPath back to a PathBuf
    pub fn to_path_buf(&self, string_table: &StringTable) -> PathBuf {
        if self.components.is_empty() {
            return PathBuf::new();
        }

        let mut path = PathBuf::new();
        for &component_id in &self.components {
            let component_str = string_table.resolve(component_id);
            path.push(component_str);
        }
        path
    }

    /// Push a string component to the end of this path (interns the string)
    pub fn push_str(&mut self, component: &str, string_table: &mut StringTable) {
        let component_id = string_table.intern(component);
        self.components.push(component_id);
    }

    /// Get the parent path (all components except the last)
    /// Returns None if this is the root path
    pub fn parent(&self) -> Option<InternedPath> {
        if self.components.is_empty() {
            None
        } else {
            Some(InternedPath {
                components: self.components[..self.components.len() - 1].to_vec(),
            })
        }
    }

    /// Test-only helper for verifying component concatenation semantics directly.
    #[cfg(test)]
    pub fn join(&self, other: &InternedPath) -> InternedPath {
        let mut new_components = Vec::with_capacity(self.components.len() + other.components.len());
        new_components.extend_from_slice(&self.components);
        new_components.extend_from_slice(&other.components);
        InternedPath {
            components: new_components,
        }
    }

    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.try_remap_string_ids(&mut |id| {
            Ok::<StringId, std::convert::Infallible>(remap.get(id))
        })
        .expect("string-ID remapping is infallible");
    }

    /// Remap every interned component through one exhaustive, in-place, fallible walker.
    ///
    /// WHAT: the single canonical walker for `InternedPath` payloads, shared by normal string
    ///       merges and frozen-token pool remapping.
    /// WHY: path components are interned strings; one in-place traversal owner prevents callers
    ///      from walking some component classes while a future payload class silently bypasses
    ///      it, and keeps the components vector allocation intact.
    pub fn try_remap_string_ids<E>(
        &mut self,
        map: &mut impl FnMut(StringId) -> Result<StringId, E>,
    ) -> Result<(), E> {
        for component in &mut self.components {
            *component = map(*component)?;
        }
        Ok(())
    }

    pub fn append(&self, new: StringId) -> Self {
        let mut new_components = Vec::with_capacity(self.components.len() + 1);
        new_components.extend_from_slice(&self.components);
        new_components.push(new);
        Self {
            components: new_components,
        }
    }

    /// Join this path with a string component (interns the string)
    pub fn join_str(&self, component: &str, string_table: &mut StringTable) -> InternedPath {
        self.append(string_table.intern(component))
    }

    /// Get the number of components in this path
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Returns whether this path is the empty root path.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Get the last component of this path (the "file name")
    pub fn name(&self) -> Option<StringId> {
        self.components.last().copied()
    }

    /// Get the last component as a string
    pub fn name_str<'a>(&self, string_table: &'a StringTable) -> Option<&'a str> {
        self.name().map(|id| string_table.resolve(id))
    }

    /// Get the components as a slice
    pub fn as_components(&self) -> &[StringId] {
        &self.components
    }

    /// Check if this path starts with the given prefix path
    pub fn starts_with(&self, prefix: &InternedPath) -> bool {
        if prefix.components.len() > self.components.len() {
            return false;
        }

        self.components
            .iter()
            .zip(prefix.components.iter())
            .all(|(a, b)| a == b)
    }

    /// Replace one source-owned path prefix while preserving the semantic suffix.
    ///
    /// WHAT: rebases declaration and ordering paths from a provisional source-file identity to
    ///       the final logical source identity assigned after synthetic discovery.
    /// WHY: header token substreams use paths such as `source_file/declaration`; replacing the
    ///      whole path would erase the declaration component and make dependency graph keys
    ///      diverge from the retained declaration headers.
    pub fn rebind_prefix(&self, old_prefix: &InternedPath, new_prefix: &InternedPath) -> Self {
        if !self.starts_with(old_prefix) {
            return self.clone();
        }

        let suffix = &self.components[old_prefix.components.len()..];
        let mut components = Vec::with_capacity(new_prefix.len() + suffix.len());
        components.extend_from_slice(&new_prefix.components);
        components.extend_from_slice(suffix);
        Self { components }
    }

    /// Rebind a path that is required to be owned by one provisional source identity.
    ///
    /// WHAT: checks that the old source prefix is present before preserving the semantic suffix.
    /// WHY: provider spellings may intentionally be prefix-free and use [`Self::rebind_prefix`],
    ///      but retained header paths, source-local ordering hints and declaration-member paths
    ///      must never silently survive final source-identity rebinding unchanged.
    pub fn try_rebind_required_prefix(
        &self,
        old_prefix: &InternedPath,
        new_prefix: &InternedPath,
    ) -> Result<Self, CompilerError> {
        if !self.starts_with(old_prefix) {
            return Err(CompilerError::compiler_error(
                "source-owned retained path is missing its provisional source prefix",
            ));
        }
        Ok(self.rebind_prefix(old_prefix, new_prefix))
    }

    /// Check if this path ends with the given suffix path
    pub fn ends_with(&self, suffix: &InternedPath) -> bool {
        if suffix.components.len() > self.components.len() {
            return false;
        }

        let start_idx = self.components.len() - suffix.components.len();
        self.components[start_idx..]
            .iter()
            .zip(suffix.components.iter())
            .all(|(a, b)| a == b)
    }

    /// Render with the platform-native path separator.
    /// Use this only for diagnostics and filesystem-adjacent display.
    pub fn to_native_string(&self, string_table: &StringTable) -> String {
        self.to_path_buf(string_table).to_string_lossy().to_string()
    }

    /// Render with forward slashes so string output is deterministic across OSes.
    /// This is the preferred renderer for compiler logic, snapshots, and tests.
    pub fn to_portable_string(&self, string_table: &StringTable) -> String {
        self.to_native_string(string_table).replace('\\', "/")
    }

    pub fn to_string(&self, string_table: &StringTable) -> String {
        self.to_portable_string(string_table)
    }
}

impl Default for InternedPath {
    fn default() -> Self {
        Self::new()
    }
}
