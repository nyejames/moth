//! Provider-independent prepared source for one module compilation.
//!
//! WHAT: the retained result of preparing one module's source files — its role-keyed root
//!       `SourceId`s, ordered candidate source IDs, the shared boundary source-origin table,
//!       aggregated `PreparedHeaderSyntax`, the module string table and the input-size facts arena
//!       capacity estimation needs.
//! WHY:  the compiler design overview requires `PreparedHeaderSyntax` to exist before the provider
//!       graph is compiled, and Stage 0 decides when to prepare each candidate. This value is the
//!       compiler-owned payload that crosses back into semantic compilation, so the phase boundary
//!       is unrepresentable as an invalid state: semantic compilation consumes retained syntax and
//!       a string table, never source text or token streams. The enclosing compilation boundary
//!       owns the immutable source database and source-origin table shared by its prepared modules.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::declaration_syntax::build_config_contract::SourceBuildConfigContract;
use crate::compiler_frontend::headers::parse_file_headers::{FileRole, PreparedHeaderSyntax};
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceTable;
use crate::compiler_frontend::semantic_identity::{ModuleRootFiles, ModuleRootRole};
use crate::compiler_frontend::source::{SourceDatabase, SourceId};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::path::Path;
use std::sync::Arc;

/// Retained result of preparing one module's source files and aggregating header syntax.
///
/// Construct this only from the module-preparation path. The `string_table` is the local module
/// fork built during file preparation; every `StringId` in `prepared_header_syntax` is valid in
/// it. Semantic compilation consumes this payload and continues mutating the same string table
/// through binding, AST, HIR and borrow validation. Source records remain in the enclosing
/// compilation boundary's immutable database; this payload retains only module-local `SourceId`
/// candidates and a shared immutable source-origin-table handle.
pub(crate) struct PreparedModuleInput {
    /// The structural module's fixed role-keyed root sources, resolved once through
    /// `SourceDatabase` during preparation and validated against the shared boundary
    /// source-origin table.
    ///
    /// Semantic compilation resolves every surface and the preparation anchor from this
    /// inventory, so no root role travels as a loose argument.
    pub(crate) root_file_ids: ModuleRootFiles<SourceId>,
    /// The one source explicitly selected for a standalone/synthetic/check-only unit, with the
    /// file role its preparation used. `None` for Stage 0 directory modules.
    pub(crate) selected_file: Option<(SourceId, FileRole)>,
    /// The module's ordered candidate source IDs.
    ///
    /// Directory modules receive every owned compiler-semantic ID from Stage 0 ownership,
    /// including sources not reached by the header discovery walk. Single-file compilation
    /// receives every ID in its temporary source database. The module-local source table is
    /// built from the corresponding candidate set, so external-import resolution stays
    /// scoped to the module.
    pub(crate) candidate_source_ids: Vec<SourceId>,

    /// Shared immutable source-origin table for the enclosing project or package boundary.
    ///
    /// The table maps every boundary `SourceId` to its graph-owned
    /// `StableModuleOriginIdentity`. It is remap-free by construction and is retained through an
    /// `Arc` so each prepared module shares the one boundary allocation rather than cloning rows.
    pub(crate) source_module_origins: Arc<SourceModuleOriginTable>,
    /// Provider-independent retained header syntax, including normalized source `#Config`
    /// contract shells. The shells remain outside provider binding and are available to the later
    /// project-wide resolution barrier through this payload.
    pub(crate) prepared_header_syntax: PreparedHeaderSyntax,
    /// Stage 0 resolved file-value targets, keyed with the preparing file and path-syntax handle.
    ///
    /// Binding passes this through uninterpreted. AST consumes it and is given no filesystem
    /// resolver of its own.
    pub(crate) resolved_file_references: ResolvedFileReferenceTable,
    /// Local module string table forked for this module during file preparation.
    pub(crate) string_table: StringTable,
    /// Local module path fork carrying every `PathId` interned during file preparation.
    /// Merged into the boundary builder once per module alongside the string table, so
    /// downstream consumers see single remapped identities. Source-prefix `PathId`s stay
    /// identity through the merge.
    pub(crate) path_fork: PathInternerFork,
    /// Number of source files in the module, for arena capacity estimation.
    pub(crate) source_file_count: usize,
    /// Total source byte count, for arena capacity estimation.
    pub(crate) source_byte_count: usize,
}

impl PreparedModuleInput {
    /// The source ID selected to anchor preparation: the normal, then support, then entry root,
    /// or the standalone selection for transient units. It has no entry or surface authority.
    pub(crate) fn preparation_anchor(&self) -> Result<SourceId, InfrastructureFailure> {
        self.root_file_ids
            .preparation_anchor()
            .copied()
            .or(self.selected_file.map(|(source_id, _)| source_id))
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    "prepared module: the payload has neither a root inventory nor a selected source",
                )
            })
    }

    /// The present API facade roots in the fixed normal, then support slots.
    ///
    /// A directory module reads its role-keyed root inventory. A standalone synthetic or
    /// check-only unit reads the explicit surface its preparation selected. Entry roots,
    /// ordinary helpers and a synthetic project context own no importable API surface, so their
    /// slot stays empty. The caller publishes one interface per present slot and none when both
    /// are empty, so an entry-only module compiles without manufacturing a facade.
    pub(crate) fn facade_surface_roots(&self) -> [Option<(SourceId, ModuleRootRole)>; 2] {
        if !self.root_file_ids.is_empty() {
            return [
                self.root_file_ids
                    .normal
                    .map(|source_id| (source_id, ModuleRootRole::Normal)),
                self.root_file_ids
                    .support
                    .map(|source_id| (source_id, ModuleRootRole::Support)),
            ];
        }

        let mut roots = [None, None];
        if let Some((source_id, file_role)) = self.selected_file {
            match file_role.active_surface() {
                Some(ModuleRootRole::Normal) => {
                    roots[0] = Some((source_id, ModuleRootRole::Normal));
                }
                Some(ModuleRootRole::Support) => {
                    roots[1] = Some((source_id, ModuleRootRole::Support));
                }
                Some(
                    ModuleRootRole::Entry
                    | ModuleRootRole::Shared
                    | ModuleRootRole::SyntheticProjectContext,
                )
                | None => {}
            }
        }
        roots
    }

    /// The canonical path of the preparation anchor.
    ///
    /// WHY: preparation already resolved the anchor identity through the boundary-owned
    /// `SourceDatabase`, so the path is a retained identity fact rather than a second argument the
    /// caller must keep in sync.
    pub(crate) fn preparation_anchor_path<'a>(
        &self,
        source_files: &'a SourceDatabase,
    ) -> Result<&'a Path, InfrastructureFailure> {
        let anchor = self.preparation_anchor()?;
        source_files
            .get(anchor)
            .and_then(|identity| identity.canonical_os_path.as_deref())
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "prepared module: preparation anchor file id {} is not in the source file table",
                        anchor.index()
                    ),
                )
            })
    }

    /// The canonical path of the executable entry root, falling back to the preparation anchor
    /// for modules without an entry root.
    pub(crate) fn entry_point_path<'a>(
        &self,
        source_files: &'a SourceDatabase,
    ) -> Result<&'a Path, InfrastructureFailure> {
        let entry_root = self.root_file_ids.entry;
        let Some(entry_root) = entry_root else {
            return self.preparation_anchor_path(source_files);
        };
        source_files
            .get(entry_root)
            .and_then(|identity| identity.canonical_os_path.as_deref())
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "prepared module: entry root file id {} is not in the source file table",
                        entry_root.index()
                    ),
                )
            })
    }

    /// Whether preparation synthesized a start-bearing root for this module.
    ///
    /// A directory module is start-bearing when it owns an entry root, or when a normal root
    /// still acts as today's legacy combined root. A standalone selection declares the same fact
    /// through its explicit file role.
    pub(crate) fn synthesizes_entry_start(&self) -> bool {
        self.root_file_ids.entry.is_some()
            || self.root_file_ids.normal_root_is_legacy_combined()
            || self
                .selected_file
                .is_some_and(|(_, role)| role.synthesizes_entry_start())
    }

    /// Borrow normalized source `#Config` shells retained during header preparation.
    ///
    /// The accessor exposes retained source contracts to the project-wide build-config barrier
    /// without exposing provider-bound or mutable resolution state through `PreparedModuleInput`.
    pub(crate) fn source_build_config_contracts(&self) -> &[SourceBuildConfigContract] {
        &self.prepared_header_syntax.source_build_config_contracts
    }
}
