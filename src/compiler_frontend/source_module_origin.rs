//! Module-local source-origin side table mapping prepared source files to owning module origins.
//!
//! WHAT: owns the immutable, remap-free side table that resolves each prepared source file
//! identity (`SourceId`) to its owning `StableModuleOriginIdentity`. The table is
//!       populated from the build-system-owned central `SourceTreeIndex` through the
//!       `ProjectModuleGraph` for directory modules, or from the single synthetic normal-module
//!       origin for single-file compilation. It carries no `StringId` values, so it requires no
//!       remap during string-table fork/merge.
//! WHY: canonical public type projection needs to resolve a nominal declaration's defining
//!      source file to its graph-owned stable module origin. Without this table the projection
//!      path trusts one loose module-origin argument for every declaration, which cannot
//!      distinguish an active root from an imported root or a donor file. The table makes the
//!      owning origin a per-file fact derived from the graph, so the projection validates the
//!      active root's origin instead of assuming it.
//!
//! Boundary: this table is a semantic side table, not a topology owner. It is populated from
//! the existing graph/source-index ownership and adds no filesystem scan, longest-prefix
//! ownership guess or parallel topology table. Source-package files outside the project module
//! graph are not owned by any project module: their entry is an explicit `None` until separate
//! package graphs exist. They are not directly defined public exports and do not participate in
//! active-root origin projection.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::source::{SourceDatabase, SourceId};

use rustc_hash::FxHashMap;

use std::path::PathBuf;

/// Immutable module-local side table mapping each prepared source file to its owning stable
/// module origin.
///
/// Keyed by `SourceId` (the deterministic local source identity from `SourceDatabase`).
/// Values are `StableModuleOriginIdentity` (the graph-owned cross-build origin). Entries for
/// source files not owned by any project module (source-package files) are `None`.
///
/// The table is immutable after construction and remap-free by construction:
/// `StableModuleOriginIdentity` carries only owned `String` values and a `ModuleRootRole`,
/// never `StringId` or `InternedPath`, so string-table fork/merge does not touch it.
pub(crate) struct SourceModuleOriginTable {
    origins: Vec<Option<StableModuleOriginIdentity>>,
}

impl SourceModuleOriginTable {
    /// Build the table for directory-module compilation from the graph-owned source-origin
    /// lookup.
    ///
    /// Each source file identity in `source_files` is mapped to its owning origin by looking up
    /// its canonical OS path in `origin_by_canonical_path`. Files not present in the lookup
    /// (source-package files outside the project module graph) map to `None` and are not an
    /// error: they are not directly defined public exports and do not participate in
    /// active-root origin projection.
    pub(crate) fn from_graph_ownership(
        source_files: &SourceDatabase,
        origin_by_canonical_path: &FxHashMap<PathBuf, StableModuleOriginIdentity>,
    ) -> Self {
        let origins = source_files
            .iter()
            .map(|identity| {
                origin_by_canonical_path
                    .get(&identity.canonical_os_path)
                    .cloned()
            })
            .collect();

        Self { origins }
    }

    /// Build the table for single-file compilation from the one synthetic normal-module origin.
    ///
    /// Every prepared source file maps to the same synthetic origin, matching the single-file
    /// compilation path's one-module semantics.
    pub(crate) fn from_synthetic_origin(
        source_files: &SourceDatabase,
        origin: &StableModuleOriginIdentity,
    ) -> Self {
        let origins = source_files.iter().map(|_| Some(origin.clone())).collect();

        Self { origins }
    }

    /// Resolve the owning stable module origin for one source file.
    ///
    /// Returns `Ok(Some(origin))` for a project-module-owned source file and `Ok(None)` for an
    /// in-range source-package file not owned by the current project module graph (an explicit
    /// migration state until separate package graphs exist). An out-of-range `SourceId` is an
    /// internal invariant violation surfaced through `Err(CompilerError)` rather than silently
    /// returning `None`, so callers cannot conflate an unowned file with a corrupt identity.
    pub(crate) fn origin_for(
        &self,
        source_id: SourceId,
    ) -> Result<Option<&StableModuleOriginIdentity>, CompilerError> {
        match self.origins.get(source_id.index()) {
            Some(origin) => Ok(origin.as_ref()),
            None => Err(CompilerError::compiler_error(format!(
                "source module origin table: out-of-range {source_id:?} at index {} (table has {} entries)",
                source_id.index(),
                self.origins.len()
            ))),
        }
    }

    /// The number of source entries in the table (one per `SourceId`).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.origins.len()
    }
}

#[cfg(test)]
#[path = "tests/source_module_origin_tests.rs"]
mod tests;
