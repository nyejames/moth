//! Boundary-wide source-origin side table for prepared module sources.
//!
//! WHAT: owns the immutable, remap-free mapping from each `SourceId` in one project or package
//!       compilation boundary to its owning `StableModuleOriginIdentity`. The graph-owned
//!       canonical-path lookup builds the table once, and prepared modules share an `Arc` handle
//!       instead of retaining independent row copies.
//! WHY: canonical public type projection needs to resolve a nominal declaration's defining source
//!      file to its graph-owned stable module origin. Sharing one table preserves that per-source
//!      fact across every module in the same boundary while keeping source identity single-owned.
//!      The table carries no `StringId` values, so it requires no remap during string-table
//!      fork/merge.
//!
//! Boundary: this table is a semantic side table, not a topology owner. It is populated from the
//! existing graph/source-index ownership and adds no filesystem scan, longest-prefix ownership
//! guess or parallel topology table. A source record outside the current graph's owned set has an
//! explicit `None` entry; it is not directly defined public export material and does not
//! participate in active-root origin projection. Single-file compilation intentionally uses a
//! separate synthetic table for its temporary identity domain.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::source::{SourceDatabase, SourceId};

use rustc_hash::FxHashMap;

use std::path::PathBuf;

/// Immutable boundary-wide side table mapping every source identity in one compilation boundary to
/// its owning stable module origin.
///
/// Keyed by `SourceId` (the deterministic boundary source identity from `SourceDatabase`).
/// Values are `StableModuleOriginIdentity` (the graph-owned cross-build origin). Entries for
/// source files not owned by a graph module are `None`.
///
/// The table is immutable after construction and remap-free by construction:
/// `StableModuleOriginIdentity` carries only owned `String` values and a `ModuleRootRole`,
/// never `StringId` or `InternedPath`, so string-table fork/merge does not touch it. One
/// `Arc<SourceModuleOriginTable>` is built per project or source-package boundary and shared by
/// its prepared modules; single-file compilation uses a separate synthetic table.
pub(crate) struct SourceModuleOriginTable {
    origins: Vec<Option<StableModuleOriginIdentity>>,
}

impl SourceModuleOriginTable {
    /// Build one shared table for a project or source-package compilation boundary from the
    /// graph-owned source-origin lookup.
    ///
    /// Each boundary source identity is mapped to its owning origin by looking up its canonical OS
    /// path in `origin_by_canonical_path`. Files not present in the lookup (for example,
    /// source-package files outside a project module graph) map to `None` and are not an error:
    /// they are not directly defined public exports and do not participate in active-root origin
    /// projection.
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

    /// Resolve the owning stable module origin for one boundary source identity.
    ///
    /// Returns `Ok(Some(origin))` for a source file owned by a graph module and `Ok(None)` for an
    /// in-range source file without a graph-owned module origin. An out-of-range `SourceId` is an
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
