//! Retained module preparation payload.
//!
//! WHAT: the build-system-owned handoff between Stage 0 source-file preparation and semantic
//!       module compilation. Carries the active root's retained `FileId`, the per-file
//!       source-origin table, the provider-independent `PreparedHeaderSyntax`, the
//!       deterministic module string-table context, source identities, preparation warnings,
//!       and the input-size facts semantic compilation needs for arena capacity estimation.
//! WHY: the compiler design overview requires `PreparedHeaderSyntax` to be produced before the
//!      provider graph is compiled and retained so semantic compilation begins with
//!      provider-dependent `bind_module_headers` without retokenizing or reparsing source.
//!      This type makes that phase boundary unrepresentable as an invalid state: semantic
//!      compilation consumes retained syntax and a string table, never `PreparedSourceInput`,
//!      source text or tokens.

use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::parse_file_headers::PreparedHeaderSyntax;
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Retained result of preparing one module's source files and aggregating header syntax.
///
/// Construct this only from the module-preparation path. The `string_table` is the local module
/// fork built during file preparation; every `StringId` in `prepared_header_syntax` and
/// `source_files` is valid in it. Semantic compilation consumes this payload and continues
/// mutating the same string table through binding, AST, HIR and borrow validation.
///
/// This payload carries no source text or token streams, so semantic compilation cannot rerun
/// file preparation or retokenize source. It carries the active root's retained `FileId` so the
/// semantic module-compilation boundary resolves the active module origin from the per-file
/// source-origin table instead of reconstructing one from an entry path or trusting a loose
/// origin argument. The shape is ready for dependency-ordered provider scheduling: preparation
/// and binding are independently schedulable around the retained syntax, string-table context
/// and source-origin table.
pub(crate) struct PreparedModule {
    /// The retained `FileId` of the active module root (the entry file), resolved once through
    /// `SourceFileTable` during preparation and validated against the per-file source-origin
    /// table.
    ///
    /// Semantic compilation resolves the active module origin from `source_module_origins` using
    /// this `FileId`, so the owning origin is a per-file fact derived from the graph rather than a
    /// loose argument travelling through the handoff. The loose declared origin is validated
    /// during preparation and then discarded, so `PreparedModule` carries only the file identity
    /// the projection needs.
    pub(crate) active_root_file_id: FileId,
    /// Immutable per-file source-origin side table mapping each prepared source file to its
    /// owning `StableModuleOriginIdentity`.
    ///
    /// Populated from the graph owned-source-set authority for directory modules or from the
    /// single synthetic normal-module origin for single-file compilation. The table is remap-free
    /// by construction and is consumed by direct export-origin projection to resolve and validate
    /// the active root's origin from the retained `active_root_file_id` and each
    /// directly-defined public header's defining source file.
    pub(crate) source_module_origins: SourceModuleOriginTable,
    /// Provider-independent retained header syntax, produced before provider interfaces exist.
    pub(crate) prepared_header_syntax: PreparedHeaderSyntax,
    /// Local module string table forked for this module during file preparation.
    pub(crate) string_table: StringTable,
    /// Source identities built from the prepared source paths.
    pub(crate) source_files: SourceFileTable,
    /// Whether the selected, actually prepared semantic source set contains a `.mtf` file.
    ///
    /// This is deliberately separate from `source_files`: that table also retains candidate
    /// source identities needed for ownership and diagnostics, while implicit builder providers
    /// may be enabled only by sources that Stage 0 actually prepared and reached.
    pub(crate) contains_moth_template: bool,
    /// Warnings accumulated during file preparation.
    pub(crate) warnings: Vec<CompilerDiagnostic>,
    /// Number of source files in the module, for arena capacity estimation.
    pub(crate) source_file_count: usize,
    /// Total source byte count, for arena capacity estimation.
    pub(crate) source_byte_count: usize,
}
