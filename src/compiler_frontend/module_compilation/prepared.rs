//! Provider-independent prepared source for one module compilation.
//!
//! WHAT: the retained result of preparing one module's source files — the active root's `FileId`,
//!       the per-file source-origin table, aggregated `PreparedHeaderSyntax`, the module string
//!       table, source identities, preparation warnings and the input-size facts arena capacity
//!       estimation needs.
//! WHY:  the compiler design overview requires `PreparedHeaderSyntax` to exist before the provider
//!       graph is compiled, and Stage 0 decides when to prepare each candidate. This value is the
//!       compiler-owned payload that crosses back into semantic compilation, so the phase boundary
//!       is unrepresentable as an invalid state: semantic compilation consumes retained syntax and
//!       a string table, never source text or token streams.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::parse_file_headers::PreparedHeaderSyntax;
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::path::Path;

/// Retained result of preparing one module's source files and aggregating header syntax.
///
/// Construct this only from the module-preparation path. The `string_table` is the local module
/// fork built during file preparation; every `StringId` in `prepared_header_syntax` and
/// `source_files` is valid in it. Semantic compilation consumes this payload and continues
/// mutating the same string table through binding, AST, HIR and borrow validation.
pub(crate) struct PreparedModuleInput {
    /// The retained `FileId` of the active module root, resolved once through `SourceFileTable`
    /// during preparation and validated against the per-file source-origin table.
    ///
    /// Semantic compilation resolves both the active module origin and the entry file path from
    /// this identity, so neither travels as a loose argument.
    pub(crate) active_root_file_id: FileId,
    /// Immutable per-file source-origin side table mapping each prepared source file to its
    /// owning `StableModuleOriginIdentity`.
    ///
    /// Populated from the graph owned-source-set authority for directory modules or from the
    /// single synthetic normal-module origin for single-file compilation. The table is remap-free
    /// by construction and is consumed by direct export-origin projection to resolve and validate
    /// the active root's origin and each directly-defined public header's defining source file.
    pub(crate) source_module_origins: SourceModuleOriginTable,
    /// Provider-independent retained header syntax, produced before provider interfaces exist.
    pub(crate) prepared_header_syntax: PreparedHeaderSyntax,
    /// Local module string table forked for this module during file preparation.
    pub(crate) string_table: StringTable,
    /// Source identities built from the prepared source paths.
    pub(crate) source_files: SourceFileTable,
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
    /// WHY: preparation already resolved the entry file through `SourceFileTable`, so the path is
    ///      a retained identity fact rather than a second argument the caller must keep in sync.
    pub(crate) fn entry_file_path(&self) -> Result<&Path, CompilerError> {
        self.source_files
            .get(self.active_root_file_id)
            .map(|identity| identity.canonical_os_path.as_path())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "prepared module: active root file id {} is not in the source file table",
                    self.active_root_file_id.0
                ))
            })
    }
}
