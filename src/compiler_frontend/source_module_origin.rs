//! Boundary-wide source-origin side table for prepared module sources.
//!
//! WHAT: owns the immutable, remap-free mapping from each physical `SourceId` in one project or
//!       package compilation boundary to its owning `StableModuleOriginIdentity`. The graph-owned
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
//! guess or parallel topology table. A physical source record outside the current graph's owned
//! set has an explicit `None` entry; it is not directly defined public export material and does not
//! participate in active-root origin projection. The compilation-root record is not physical and
//! is rejected as an internal error rather than assigned a module origin.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::source::{SourceDatabase, SourceId};

use rustc_hash::FxHashMap;

use std::path::PathBuf;

/// Immutable boundary-wide side table mapping each physical source identity to its owning stable
/// module origin.
///
/// The table is dense over physical sources in `SourceDatabase::iter()` order. The synthetic
/// compilation-root identity is intentionally absent and is rejected by [`Self::origin_for`].
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
    /// Each physical source identity is mapped to its owning origin by looking up its canonical OS
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
                identity
                    .canonical_os_path
                    .as_ref()
                    .and_then(|canonical_path| origin_by_canonical_path.get(canonical_path))
                    .cloned()
            })
            .collect();

        Self { origins }
    }

    /// Build the table for single-file compilation from the one synthetic normal-module origin.
    ///
    /// Every physical source file maps to the same synthetic origin. The compilation root is not
    /// a physical source and is intentionally not included.
    pub(crate) fn from_synthetic_origin(
        source_files: &SourceDatabase,
        origin: &StableModuleOriginIdentity,
    ) -> Self {
        let origins = source_files.iter().map(|_| Some(origin.clone())).collect();

        Self { origins }
    }

    /// Resolve the owning stable module origin for one physical source identity.
    ///
    /// The compilation root has no module origin. Asking for it is an internal invariant
    /// violation, as is resolving an identity outside the physical source table.
    pub(crate) fn origin_for(
        &self,
        source_id: SourceId,
    ) -> Result<Option<&StableModuleOriginIdentity>, CompilerError> {
        let physical_index = source_id.physical_index().ok_or_else(|| {
            CompilerError::compiler_error(
                "source module origin table: compilation-root SourceId has no module origin",
            )
        })?;

        match self.origins.get(physical_index) {
            Some(origin) => Ok(origin.as_ref()),
            None => Err(CompilerError::compiler_error(format!(
                "source module origin table: out-of-range {source_id:?} at physical index {physical_index} (table has {} entries)",
                self.origins.len()
            ))),
        }
    }

    /// The number of physical source entries in the table.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.origins.len()
    }
}

#[cfg(test)]
#[path = "tests/source_module_origin_tests.rs"]
mod tests;
