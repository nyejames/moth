//! Provider-independent prepared source for one module compilation.
//!
//! WHAT: the retained result of preparing one module's source files — the active root's `SourceId`,
//!       ordered candidate source IDs, the shared boundary source-origin table, aggregated
//!       `PreparedHeaderSyntax`, the module string table, preparation warnings and the input-size
//!       facts arena capacity estimation needs.
//! WHY:  the compiler design overview requires `PreparedHeaderSyntax` to exist before the provider
//!       graph is compiled, and Stage 0 decides when to prepare each candidate. This value is the
//!       compiler-owned payload that crosses back into semantic compilation, so the phase boundary
//!       is unrepresentable as an invalid state: semantic compilation consumes retained syntax and
//!       a string table, never source text or token streams. The enclosing compilation boundary
//!       owns the immutable source database and source-origin table shared by its prepared modules.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::declaration_syntax::build_config_contract::SourceBuildConfigContract;
use crate::compiler_frontend::headers::parse_file_headers::PreparedHeaderSyntax;
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceTable;
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
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
    /// The retained `SourceId` of the active module root, resolved once through `SourceDatabase`
    /// during preparation and validated against the shared boundary source-origin table.
    ///
    /// Semantic compilation resolves both the active module origin and the entry file path from
    /// this identity, so neither travels as a loose argument.
    pub(crate) active_root_file_id: SourceId,
    /// The module's ordered candidate source IDs.
    ///
    /// Directory modules receive every owned compiler-semantic ID from Stage 0 ownership,
    /// including sources not reached by the header discovery walk. Single-file compilation
    /// receives every ID in its temporary source database. The pre-slice module-local source table
    /// was built from the corresponding candidate set, so external-import resolution retains its
    /// historical module-local scope.
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
    /// Warnings accumulated during file preparation.
    pub(crate) warnings: Vec<CompilerDiagnostic>,
    /// Number of source files in the module, for arena capacity estimation.
    pub(crate) source_file_count: usize,
    /// Total source byte count, for arena capacity estimation.
    pub(crate) source_byte_count: usize,
}

impl PreparedModuleInput {
    /// The canonical path of the active module root.
    ///
    /// WHY: preparation already resolved the entry file identity through the boundary-owned
    /// `SourceDatabase`, so the path is a retained identity fact rather than a second argument the
    /// caller must keep in sync.
    pub(crate) fn entry_file_path<'a>(
        &self,
        source_files: &'a crate::compiler_frontend::source::SourceDatabase,
    ) -> Result<&'a Path, CompilerError> {
        source_files
            .get(self.active_root_file_id)
            .and_then(|identity| identity.canonical_os_path.as_deref())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "prepared module: active root file id {} is not in the source file table",
                    self.active_root_file_id.index()
                ))
            })
    }
    /// Borrow normalized source `#Config` shells retained during header preparation.
    ///
    /// The accessor exposes retained source contracts to the project-wide build-config barrier
    /// without exposing provider-bound or mutable resolution state through `PreparedModuleInput`.
    pub(crate) fn source_build_config_contracts(&self) -> &[SourceBuildConfigContract] {
        &self.prepared_header_syntax.source_build_config_contracts
    }
}
