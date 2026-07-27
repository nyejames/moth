//! Per-module frontend compilation pipeline for Moth projects.
//!
//! Drives a single discovered module through the full frontend pipeline:
//! provider-independent source preparation → provider binding → dependency sort → AST → HIR →
//! borrow checking.

use crate::build_system::build::{
    Module, ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts, ModuleRootActivity,
    ModuleSemanticDraft, ResolvedConstFragment,
};

use crate::builder_surface::external_import_providers::provider::BuilderRuntimePackageMetadata;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::{Ast, AstBuildResult};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::ModuleDiagnostics;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, FileFrontendPrepareError, FileFrontendPrepareOutput, HeaderKind,
    HeaderParseOptions, PreparedHeaderSyntax, bind_module_headers, prepare_header_syntax,
};
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::collect_module_function_link_facts;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::module_dependencies::SortedHeaders;
use crate::compiler_frontend::module_metadata::HirLoweringResult;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::{
    PublicInterfaceDraftBuilder, PublicInterfaceDraftBuilderInput, SourceProviderImportSet,
};
use crate::compiler_frontend::public_interface::{
    build_direct_export_seed, build_public_source_nominal_origin_index,
    build_public_source_trait_origin_index,
};
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StableModuleOriginIdentity};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::string_interning::{StringTable, StringTableForkSource};
use crate::compiler_frontend::validated_generic_template_metadata::extract_validated_generic_template_artefacts;
use crate::compiler_frontend::{
    CompilerFrontend, FrontendBuildProfile, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};

use super::prepared_module::PreparedModule;
use super::prepared_source::PreparedSourceInput;

#[cfg(feature = "detailed_timers")]
use crate::benchmark_timer_log;
use crate::borrow_log;
use crate::projects::settings::Config;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "detailed_timers")]
use std::time::Instant;

/// Parallel file-preparation scheduling policy.
///
/// WHAT: keeps the production strategy thresholds near the code that applies them.
/// WHY: these values are benchmark policy, not language semantics. `RAYON_NUM_THREADS` remains
/// the external concurrency override; this pass deliberately does not add a custom Rayon pool,
/// unsafe scheduling, or hidden per-build thread control.
///
/// File count at or below which Rayon scheduling is consistently more expensive than useful.
///
/// WHY: benchmark checks showed tiny modules regressing under Rayon, while fanout-style modules
/// and the documentation build still benefit from parallel file preparation. Medium modules stay
/// serial unless their total source size crosses `FILE_PREPARATION_MEDIUM_PARALLEL_MIN_BYTES`.
const FILE_PREPARATION_ALWAYS_SERIAL_FILE_COUNT: usize = 2;

/// File count at which chunked Rayon scheduling is consistently worth the overhead.
///
/// WHY: eight-file fanout is the first stable win from the Phase 1 benchmark set, but running one
/// task per small file over-schedules many tiny-file modules. Chunking starts here.
const FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT: usize = 8;

/// Source-size threshold that lets medium-sized modules use parallel file preparation.
///
/// This is benchmark policy, not a language semantic: 3-7 file modules avoid Rayon overhead by
/// default, but a large enough source payload can amortize scheduling and string-table fork costs.
const FILE_PREPARATION_MEDIUM_PARALLEL_MIN_BYTES: usize = 64 * 1024;

/// Target parallel chunks per Rayon worker for many-file module preparation.
///
/// WHY: a small multiple of the worker count gives Rayon enough tasks to balance uneven source
/// sizes without returning to one scheduling task per tiny file.
const FILE_PREPARATION_TARGET_TASKS_PER_THREAD: usize = 2;

/// Lower bound for chunk size when planning chunked file preparation.
///
/// WHY: chunking only helps if each scheduled task does enough serial file preparation to amortize
/// fork and scheduling overhead.
const FILE_PREPARATION_MIN_CHUNK_SIZE: usize = 4;

struct FilePreparationChunk {
    chunk_index: usize,
    file_range: Range<usize>,
    local_string_table: StringTable,
    results: Vec<PreparedFileResult>,
}

struct PreparedFileResult {
    file_index: usize,
    result: Result<FileFrontendPrepareOutput, FileFrontendPrepareError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilePreparationChunkPlan {
    chunk_index: usize,
    file_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilePreparationStrategy {
    Serial,
    ParallelPerFile,
    ParallelChunked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilePreparationStrategyReason {
    SmallSerial,
    ByteThresholdSerial,
    MediumByteThresholdParallel,
    LargeChunkedParallel,
}

impl FilePreparationStrategy {
    #[cfg(test)]
    fn for_module(source_file_count: usize, source_byte_count: usize) -> Self {
        Self::selection_for_module(source_file_count, source_byte_count).0
    }

    fn selection_for_module(
        source_file_count: usize,
        source_byte_count: usize,
    ) -> (Self, FilePreparationStrategyReason) {
        if source_file_count <= FILE_PREPARATION_ALWAYS_SERIAL_FILE_COUNT {
            (Self::Serial, FilePreparationStrategyReason::SmallSerial)
        } else if source_file_count >= FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT {
            (
                Self::ParallelChunked,
                FilePreparationStrategyReason::LargeChunkedParallel,
            )
        } else if source_byte_count >= FILE_PREPARATION_MEDIUM_PARALLEL_MIN_BYTES {
            (
                Self::ParallelPerFile,
                FilePreparationStrategyReason::MediumByteThresholdParallel,
            )
        } else {
            (
                Self::Serial,
                FilePreparationStrategyReason::ByteThresholdSerial,
            )
        }
    }
}

/// The origin input for module preparation, carrying the expected active origin and either the
/// graph-owned source-origin lookup (directory mode) or nothing (single-file synthetic mode).
///
/// WHAT: bundles the two origin facts preparation needs so `prepare_module` stays under the
///       Clippy argument limit without splitting related inputs across loose parameters.
/// WHY: the expected active origin is validated against the per-file source-origin table during
///      preparation and then discarded, so `PreparedModule` carries only the retained active
///      root `FileId` rather than a loose origin argument. The optional graph lookup controls
///      how the per-file `SourceModuleOriginTable` is built.
pub(super) enum ModuleOriginInput<'a> {
    /// Single-file compilation: every source file maps to the one synthetic origin.
    Synthetic(StableModuleOriginIdentity),
    /// Directory compilation: the graph-owned lookup resolves each source file to its owning
    /// origin, and `stable_origin` is the expected active origin validated against the table.
    Graph {
        stable_origin: StableModuleOriginIdentity,
        origin_by_canonical_path: &'a FxHashMap<PathBuf, StableModuleOriginIdentity>,
    },
}

// -------------------------
//  Preparation Context (provider-independent)
// -------------------------

/// Provider-independent context for preparing one module's source files and aggregating
/// `PreparedHeaderSyntax` without requiring provider interfaces.
///
/// WHAT: owns only the inputs file preparation actually requires — style directives and the
///       project path resolver — and works against a caller-owned `StringTable` and
///       `SourceFileTable`. It deliberately excludes `ExternalPackageRegistry`, the import
///       resolution table and builder runtime packages.
/// WHY: the compiler design overview requires `PreparedHeaderSyntax` to be produced before the
///      provider graph is compiled. Keeping provider-interface values out of this context makes
///      the preparation phase genuinely provider-independent, so it cannot reach provider state
///      and the orchestrator can schedule provider binding between `prepare_module` and
///      semantic compilation without touching this context.
pub(super) struct ModulePreparationContext<'a> {
    pub(super) style_directives: &'a StyleDirectiveRegistry,
    pub(super) project_path_resolver: Option<ProjectPathResolver>,
}

// -------------------------
//  Semantic Compilation Context (provider-dependent)
// -------------------------

/// Lifetime-bound context for compiling one retained module through the provider-dependent
/// semantic pipeline.
///
/// WHAT: bundles the provider interfaces and long-lived inputs shared across header binding,
/// dependency sorting, AST, HIR, and borrow checking for a single module.
/// WHY: bundling these together keeps call sites in the coordinator short and makes the
/// `StringTable` handoff between orchestration and `CompilerFrontend` explicit in one place.
///      Preparation is owned by `ModulePreparationContext`; this context begins with
///      `bind_module_headers` over the retained `PreparedHeaderSyntax`.
pub(super) struct FrontendModuleBuildContext<'a> {
    pub(super) config: &'a Config,
    pub(super) build_profile: FrontendBuildProfile,
    pub(super) project_path_resolver: Option<ProjectPathResolver>,
    pub(super) style_directives: &'a StyleDirectiveRegistry,
    pub(super) external_packages: Arc<ExternalPackageRegistry>,
    pub(super) external_import_resolution_table: &'a ExternalImportResolutionTable,
    pub(super) source_provider_imports: &'a SourceProviderImportSet<'a>,
    pub(super) builder_runtime_packages: &'a [BuilderRuntimePackageMetadata],
}

/// Typed result of one retained module's semantic compilation.
///
/// WHAT: separates a successfully compiled module from a diagnosed source failure at the
///       retained-module semantic boundary. `Success` carries the current unmerged module plus
///       its local string-table delta; `Diagnosed` carries the user-facing diagnostics that the
///       renderer surfaces.
/// WHY: the prior boundary returned a mixed `CompilerMessages` for every failure, so a diagnosed
///      module and an internal `CompilerError` were indistinguishable result classes. This outcome
///      makes them distinct: a structured user diagnostic becomes `Ok(Diagnosed(...))` while an
///      infrastructure failure originating from a `CompilerError` becomes `Err(CompilerError)` via
///      the central lossless normalization in `ModuleDiagnostics::from_messages`.
///
/// The success payload keeps the internal `ModuleSemanticDraft` carrying the unmerged module
/// plus local string-table state. It is not the final `CompiledModuleArtifact`, which remains
/// deferred.
pub(crate) enum ModuleCompilationOutcome {
    // `ModuleSemanticDraft` carries the full unmerged module (HIR, type environment and borrow
    // facts) and is far larger than `ModuleDiagnostics`, so the success payload is boxed to keep
    // the boundary outcome small. The box is transient: the caller unboxes once before merging.
    Success(Box<ModuleSemanticDraft>),
    Diagnosed(ModuleDiagnostics),
}

impl ModulePreparationContext<'_> {
    /// Prepare one discovered module's source files and aggregate provider-independent header
    /// syntax, retaining it with the module string-table context and the active root's file
    /// identity for semantic compilation.
    ///
    /// WHAT: prepares every source file against local string-table forks, merges chunk-local
    ///       string tables in deterministic input order, and runs `prepare_header_syntax` to
    ///       produce the retained `PreparedHeaderSyntax`. Stops before provider-dependent binding.
    ///       After building the per-file source-origin table, resolves the entry file's `FileId`
    ///       through `SourceFileTable` once and validates that the table maps it to the expected
    ///       active origin from `ModuleOriginInput`, then retains the `FileId` and discards the
    ///       loose origin.
    /// WHY: the compiler design overview requires `PreparedHeaderSyntax` to be produced before
    ///      the provider graph is compiled. This context owns no provider-interface values, so
    ///      preparation cannot reach provider state. Retaining the syntax, string-table context,
    ///      source identities and the active root `FileId` lets semantic compilation begin with
    ///      `bind_module_headers` without retokenizing or reparsing source and without
    ///      reconstructing module identity from paths, and leaves a clean boundary where the
    ///      orchestrator can schedule provider binding between this call and
    ///      `FrontendModuleBuildContext::compile_module_semantic`.
    pub(super) fn prepare_module(
        &self,
        origin_input: ModuleOriginInput<'_>,
        module: &[PreparedSourceInput],
        entry_file_path: &Path,
        mut string_table: StringTable,
        source_byte_count: usize,
        module_label: Option<&str>,
    ) -> Result<PreparedModule, CompilerMessages> {
        let mut warnings = Vec::new();

        // Entry identity and root semantics are separate. The stable module origin owns whether
        // the active file is a normal runtime-capable root or an API-only support/facade root.
        let active_root_role = match &origin_input {
            ModuleOriginInput::Synthetic(origin) => origin.role(),
            ModuleOriginInput::Graph { stable_origin, .. } => stable_origin.role(),
        };

        // 1. Build the module source identity table against the caller-owned string table. Source
        //    identities are deterministic and provider-free, so this needs no provider interface.
        let source_files = Self::attach_source_files(
            &mut string_table,
            &self.project_path_resolver,
            module,
            entry_file_path,
        )?;

        // 2. Prepare all files against one local string-table per worker chunk. Moth files
        //    parse retained Stage 0 tokens, Moth template tokenizes its body once and plain Markdown
        //    bypasses tokenization. Merge/remap once before aggregating header syntax.
        let (prepared_header_syntax, file_warnings) = timed_frontend_stage(
            "frontend.file_prepare",
            "Files Prepared in: ",
            module_label,
            || {
                self.prepare_module_files(
                    &mut string_table,
                    &source_files,
                    module,
                    entry_file_path,
                    active_root_role,
                    source_byte_count,
                )
            },
        )?;
        warnings.extend(file_warnings);

        // 3. Build the immutable per-file source-origin side table from the origin input. For
        //    directory modules the graph-owned lookup resolves each source file to its owning
        //    origin; for single-file compilation every file maps to the synthetic origin. The
        //    table is remap-free and provider-independent: it carries no StringIds and needs no
        //    provider interface.
        let (expected_active_origin, source_module_origins) = match origin_input {
            ModuleOriginInput::Synthetic(origin) => {
                let table = SourceModuleOriginTable::from_synthetic_origin(&source_files, &origin);
                (origin, table)
            }
            ModuleOriginInput::Graph {
                stable_origin,
                origin_by_canonical_path,
            } => {
                let table = SourceModuleOriginTable::from_graph_ownership(
                    &source_files,
                    origin_by_canonical_path,
                );
                (stable_origin, table)
            }
        };

        // 4. Resolve the entry file's FileId through the source file table once and validate that
        //    the per-file origin table maps it to the expected active origin. The active root must
        //    have an owning origin, and that origin must match the origin declared by the
        //    discovery/graph path. A missing entry identity, an unowned active source or an origin
        //    mismatch is an internal CompilerError surfaced through the build-boundary messages.
        //    The loose origin is then discarded: `PreparedModule` carries only the retained FileId
        //    so semantic compilation resolves the active origin from the table, not a loose
        //    argument.
        let active_root_file_id = Self::resolve_and_validate_active_root(
            &source_files,
            &source_module_origins,
            &expected_active_origin,
            entry_file_path,
            &string_table,
        )?;

        // Retain the deterministic preparation context so semantic compilation can continue against
        // the same string table and source identities. The payload owns no `CompilerFrontend` or
        // provider state: only syntax, the string table, source identities and warnings.
        Ok(PreparedModule {
            active_root_file_id,
            source_module_origins,
            prepared_header_syntax,
            string_table,
            source_files,
            warnings,
            source_file_count: module.len(),
            source_byte_count,
        })
    }

    /// Build the module `SourceFileTable` from input source paths against a caller-owned string
    /// table and the project path resolver.
    ///
    /// WHAT: assigns deterministic source identities for the prepared module without touching any
    ///       provider interface.
    /// WHY: preparation needs source identities to drive file preparation and header syntax, but
    ///      not the external package registry or import resolution table.
    fn attach_source_files(
        string_table: &mut StringTable,
        project_path_resolver: &Option<ProjectPathResolver>,
        module: &[PreparedSourceInput],
        entry_file_path: &Path,
    ) -> Result<SourceFileTable, CompilerMessages> {
        SourceFileTable::build(
            module.iter().map(|input_file| input_file.source_path()),
            entry_file_path,
            project_path_resolver.as_ref(),
            string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
    }

    /// Resolve the entry file's `FileId` from the `SourceFileTable` and validate that the
    /// per-file source-origin table maps it to the expected active origin.
    ///
    /// WHAT: the active root must be present in the source file table and must have an owning
    ///       origin in the source-origin table. That origin must equal the expected active origin
    ///       declared by the discovery or single-file path. A missing entry identity, an unowned
    ///       active source or an origin mismatch is an internal `CompilerError`.
    /// WHY: validating the active root origin during preparation lets `PreparedModule` discard the
    ///      loose origin and carry only the retained `FileId`, so the semantic projection resolves
    ///      the active origin from the table rather than trusting a loose argument.
    fn resolve_and_validate_active_root(
        source_files: &SourceFileTable,
        source_module_origins: &SourceModuleOriginTable,
        expected_active_origin: &StableModuleOriginIdentity,
        entry_file_path: &Path,
        string_table: &StringTable,
    ) -> Result<FileId, CompilerMessages> {
        let active_root_file_id = source_files
            .get_by_canonical_path(entry_file_path)
            .map(|identity| identity.file_id)
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "module preparation: the entry file path {:?} is not in the source file table",
                        entry_file_path
                    )),
                    string_table,
                )
            })?;

        let table_origin = source_module_origins
            .origin_for(active_root_file_id)
            .map_err(|error| {
                CompilerMessages::from_error_ref(error, string_table)
            })?
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "module preparation: the active root (file id {}) has no owning module origin in the source module origin table",
                        active_root_file_id.0
                    )),
                    string_table,
                )
            })?;

        if table_origin != expected_active_origin {
            return Err(CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "module preparation: the active root's table-resolved origin ({:?}) does not match the expected active origin ({:?})",
                    table_origin, expected_active_origin
                )),
                string_table,
            ));
        }

        Ok(active_root_file_id)
    }

    /// Prepare all source files in the module against local string-table forks, then merge and
    /// remap them in deterministic input order.
    ///
    /// WHAT: small modules use a serial fast path, while large modules use Rayon. Both paths
    ///       produce the same per-file result records and share the same merge/remap aggregation.
    /// WHY: keeping scheduling separate from aggregation avoids Rayon overhead on tiny modules
    ///      without changing deterministic merge order or frontend ownership boundaries.
    fn prepare_module_files(
        &self,
        string_table: &mut StringTable,
        source_files: &SourceFileTable,
        module: &[PreparedSourceInput],
        entry_file_path: &Path,
        active_root_role: ModuleRootRole,
        source_byte_count: usize,
    ) -> Result<(PreparedHeaderSyntax, Vec<CompilerDiagnostic>), CompilerMessages> {
        let entry_file_id = source_files
            .get_by_canonical_path(entry_file_path)
            .map(|identity| identity.file_id);

        let options = HeaderParseOptions {
            entry_file_id,
            project_path_resolver: self.project_path_resolver.clone(),
            active_root_role,
        };

        // Create one shared fork source for all file-preparation workers. Each scheduled chunk
        // gets a local table forked from this immutable base, so preparation never needs mutable
        // access to the module string table during tokenization or header parsing.
        let fork_source = string_table.fork_source();
        let base_len = fork_source.base_len();

        // Offsets are only relevant for the active module root, and there is exactly one root per
        // module. Imported and ordinary files produce zero const templates and runtime fragments, so
        // every file can safely start from offset zero without name collisions.
        let const_template_offset = 0usize;
        let runtime_fragment_offset = 0usize;

        let prepare_context = FrontendFilePrepareContext {
            source_files,
            style_directives: self.style_directives,
            entry_file_path,
            options: &options,
        };

        add_frontend_counter(FrontendCounter::FilePreparationInputFileCount, module.len());
        add_frontend_counter(
            FrontendCounter::FilePreparationInputByteCount,
            source_byte_count,
        );

        let (strategy, strategy_reason) = timed_frontend_substep(
            "file_prepare_strategy_selection_ms",
            "File preparation strategy selected in: ",
            || FilePreparationStrategy::selection_for_module(module.len(), source_byte_count),
        );
        record_file_preparation_strategy(strategy, strategy_reason);

        let preparation_chunks = timed_frontend_substep(
            "file_prepare_result_production_ms",
            "File preparation results produced in: ",
            || {
                Self::prepare_module_file_chunks(
                    module,
                    &fork_source,
                    &prepare_context,
                    const_template_offset,
                    runtime_fragment_offset,
                    strategy,
                )
            },
        );

        Self::merge_file_preparation_chunks(
            string_table,
            preparation_chunks,
            module.len(),
            base_len,
        )
    }

    /// Merge chunk-local string tables and aggregate prepared file outputs.
    ///
    /// WHAT: all scheduling strategies converge here after producing ordered chunk records.
    /// WHY: chunk-local workers may finish in any order, but the frontend's source identity,
    /// warning, diagnostic, and header order must follow the original module input order.
    fn merge_file_preparation_chunks(
        string_table: &mut StringTable,
        mut preparation_chunks: Vec<FilePreparationChunk>,
        module_file_count: usize,
        base_len: usize,
    ) -> Result<(PreparedHeaderSyntax, Vec<CompilerDiagnostic>), CompilerMessages> {
        // Completion order is a scheduler detail. Merge order is the module input order encoded
        // by deterministic chunk indexes.
        timed_frontend_substep(
            "file_prepare_result_sort_ms",
            "File preparation results sorted in: ",
            || preparation_chunks.sort_by_key(|chunk| chunk.chunk_index),
        );

        // Release-safe validation replaces the previous ordering debug_asserts so release
        // builds reject malformed scheduler payloads with a CompilerError instead of silently
        // dropping, reordering or truncating prepared files.
        validate_preparation_chunk_order(&preparation_chunks, module_file_count)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

        let mut prepared_outputs = Vec::new();
        let mut warnings = Vec::new();
        let mut diagnostics = Vec::new();
        let mut const_fragment_source_count = 0usize;
        let mut runtime_fragment_source_count = 0usize;

        let prepared_file_capacity = preparation_chunks
            .iter()
            .map(|chunk| chunk.results.len())
            .sum();
        prepared_outputs.reserve(prepared_file_capacity);

        for chunk in preparation_chunks {
            let remap = timed_frontend_substep(
                "file_prepare_string_table_delta_merge_ms",
                "File preparation string-table delta merged in: ",
                || string_table.merge_delta_from(&chunk.local_string_table, base_len),
            );
            let remap_is_identity = remap.is_identity();
            add_frontend_counter(FrontendCounter::FilePreparationResultMergeCount, 1);
            if remap_is_identity {
                add_frontend_counter(FrontendCounter::FilePreparationIdentityRemapCount, 1);
            } else {
                add_frontend_counter(FrontendCounter::FilePreparationNonIdentityRemapCount, 1);
            }

            for prepared_file in chunk.results {
                match prepared_file.result {
                    Ok(mut output) => {
                        if output.const_template_count > 0 {
                            const_fragment_source_count += 1;
                        }
                        if output.runtime_fragment_count > 0 {
                            runtime_fragment_source_count += 1;
                        }
                        if !remap_is_identity {
                            add_frontend_counter(FrontendCounter::FilePrepareOutputRemapCalls, 1);
                            #[cfg(feature = "benchmark_counters")]
                            add_frontend_counter(
                                FrontendCounter::FilePrepareNonIdentityPayloadRemaps,
                                1,
                            );
                            timed_frontend_substep(
                                "file_prepare_payload_remap_ms",
                                "File preparation payload remapped in: ",
                                || output.remap_string_ids(&remap),
                            );
                        }
                        warnings.append(&mut output.warnings);
                        prepared_outputs.push(output);
                    }
                    Err(mut error) => {
                        if !remap_is_identity {
                            add_frontend_counter(FrontendCounter::FilePrepareErrorRemapCalls, 1);
                            #[cfg(feature = "benchmark_counters")]
                            add_frontend_counter(
                                FrontendCounter::FilePrepareNonIdentityPayloadRemaps,
                                1,
                            );
                            timed_frontend_substep(
                                "file_prepare_payload_remap_ms",
                                "File preparation payload remapped in: ",
                                || error.remap_string_ids(&remap),
                            );
                        }
                        warnings.extend(error.warnings);
                        diagnostics.push(*error.diagnostic);
                    }
                }
            }
        }

        debug_assert!(
            const_fragment_source_count <= 1,
            "only the active module root may contribute top-level const templates"
        );
        debug_assert!(
            runtime_fragment_source_count <= 1,
            "only the active module root may contribute runtime fragments"
        );

        if !diagnostics.is_empty() {
            let mut messages =
                CompilerMessages::from_diagnostics(diagnostics, string_table.clone());
            messages.prepend_diagnostics_preserving_context(warnings);
            return Err(messages);
        }

        let prepared_file_count = prepared_outputs.len();
        let token_count = prepared_outputs
            .iter()
            .map(|output| output.token_count)
            .sum();
        let prepared = timed_frontend_substep(
            "file_prepare_header_syntax_preparation_ms",
            "File preparation header syntax prepared in: ",
            || prepare_header_syntax(prepared_outputs, string_table),
        )
        .map_err(|bag| {
            let mut messages =
                CompilerMessages::from_diagnostics(bag.into_diagnostics(), string_table.clone());
            messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
            messages
        })?;

        add_frontend_counter(FrontendCounter::PreparedFileCount, prepared_file_count);
        add_frontend_counter(FrontendCounter::TokenCount, token_count);

        Ok((prepared, warnings))
    }

    fn prepare_module_file_chunks(
        module: &[PreparedSourceInput],
        fork_source: &StringTableForkSource,
        prepare_context: &FrontendFilePrepareContext<'_>,
        const_template_offset: usize,
        runtime_fragment_offset: usize,
        strategy: FilePreparationStrategy,
    ) -> Vec<FilePreparationChunk> {
        match strategy {
            FilePreparationStrategy::Serial => {
                let plan = FilePreparationChunkPlan {
                    chunk_index: 0,
                    file_range: 0..module.len(),
                };
                vec![Self::prepare_module_file_chunk(
                    plan,
                    module,
                    fork_source,
                    prepare_context,
                    const_template_offset,
                    runtime_fragment_offset,
                )]
            }

            FilePreparationStrategy::ParallelPerFile => (0..module.len())
                .into_par_iter()
                .map(|index| {
                    let plan = FilePreparationChunkPlan {
                        chunk_index: index,
                        file_range: index..index + 1,
                    };
                    Self::prepare_module_file_chunk(
                        plan,
                        module,
                        fork_source,
                        prepare_context,
                        const_template_offset,
                        runtime_fragment_offset,
                    )
                })
                .collect(),

            FilePreparationStrategy::ParallelChunked => {
                let plans =
                    plan_file_preparation_chunks(module.len(), rayon::current_num_threads());
                plans
                    .into_par_iter()
                    .map(|plan| {
                        Self::prepare_module_file_chunk(
                            plan,
                            module,
                            fork_source,
                            prepare_context,
                            const_template_offset,
                            runtime_fragment_offset,
                        )
                    })
                    .collect()
            }
        }
    }

    fn prepare_module_file_chunk(
        plan: FilePreparationChunkPlan,
        module: &[PreparedSourceInput],
        fork_source: &StringTableForkSource,
        prepare_context: &FrontendFilePrepareContext<'_>,
        const_template_offset: usize,
        runtime_fragment_offset: usize,
    ) -> FilePreparationChunk {
        let (mut local_string_table, _) = fork_source.fork_for_module().into_parts();
        let mut results = Vec::with_capacity(plan.file_range.len());

        for file_index in plan.file_range.clone() {
            let file = &module[file_index];
            let source = match file {
                PreparedSourceInput::Moth {
                    source_path,
                    tokens,
                    ..
                } => FrontendFilePrepareSource::Moth {
                    source_path,
                    tokens: tokens.as_ref(),
                },
                PreparedSourceInput::MothTemplate {
                    source_code,
                    source_path,
                } => FrontendFilePrepareSource::MothTemplate {
                    source_code,
                    source_path,
                },
                PreparedSourceInput::PlainMarkdown {
                    source_code,
                    source_path,
                } => FrontendFilePrepareSource::PlainMarkdown {
                    source_code,
                    source_path,
                },
            };
            let input = FrontendFilePrepareInput {
                source,
                const_template_offset,
                runtime_fragment_offset,
            };
            let result = CompilerFrontend::prepare_file_frontend_local(
                prepare_context,
                input,
                &mut local_string_table,
            );
            results.push(PreparedFileResult { file_index, result });
        }

        FilePreparationChunk {
            chunk_index: plan.chunk_index,
            file_range: plan.file_range,
            local_string_table,
            results,
        }
    }
}

impl FrontendModuleBuildContext<'_> {
    /// Compile one retained module through the provider-dependent semantic pipeline.
    ///
    /// WHAT: begins with `bind_module_headers` over the retained `PreparedHeaderSyntax`, then
    ///       resolves dependencies, builds AST, lowers HIR, and runs borrow validation. It
    ///       receives no `PreparedSourceInput`, source text or tokens and cannot rerun file
    ///       preparation. The active module origin is resolved from the per-file source-origin
    ///       table using the retained active root `FileId`, not reconstructed from
    ///       `entry_file_path` or source paths.
    /// WHY: binding depends on provider interfaces, so it belongs after preparation in the
    ///      semantic phase. The retained string table, source identities and source-origin table
    ///      carry every fact binding and later stages need without revisiting source. The active
    ///      root `FileId` is the semantic module-compilation identity contract consumed by the
    ///      stable defined-public-export identity component built at the sort boundary and
    ///      retained alongside the transient successful compile result.
    pub(super) fn compile_module_semantic(
        &self,
        prepared: PreparedModule,
        entry_file_path: &Path,
        module_label: Option<&str>,
    ) -> Result<ModuleCompilationOutcome, CompilerError> {
        let PreparedModule {
            active_root_file_id,
            source_module_origins,
            prepared_header_syntax,
            string_table,
            source_files,
            mut warnings,
            source_file_count,
            source_byte_count,
        } = prepared;

        let active_root_role = source_module_origins
            .origin_for(active_root_file_id)?
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "semantic module compilation: active root file id {} has no module origin",
                    active_root_file_id.0
                ))
            })?
            .role();

        // The active module origin is resolved from the per-file source-origin table using the
        // retained active root FileId, not from a loose origin argument. Preparation already
        // validated the active root's table origin against the expected active origin, so the
        // semantic projection re-derives the same origin from the table and validates every
        // directly-defined public header against it.

        let external_import_resolution_table = self.external_import_resolution_table;

        let mut compiler = CompilerFrontend::new(
            self.config,
            string_table,
            self.style_directives.to_owned(),
            Arc::clone(&self.external_packages),
            self.project_path_resolver.clone(),
        );
        compiler.set_source_files(source_files);

        let compile_result = (|| {
            // 1. Bind retained header syntax against provider interfaces.
            let module_headers = timed_frontend_stage(
                "frontend.header_bind",
                "Headers bound in: ",
                module_label,
                || {
                    Self::bind_retained_headers(
                        &mut compiler,
                        prepared_header_syntax,
                        external_import_resolution_table,
                        self.source_provider_imports,
                        &warnings,
                    )
                },
            )?;

            let capacity_estimate = record_frontend_capacity_estimate(
                source_file_count,
                source_byte_count,
                &module_headers,
            );

            // 2. Resolve dependencies and sort headers for linear processing.
            let sorted = timed_frontend_stage(
                "frontend.dependency_sort",
                "Dependency graph created in: ",
                module_label,
                || Self::sort_headers(&mut compiler, module_headers, &warnings),
            )?;

            let root_activity = ModuleRootActivity {
                has_non_trivial_root_body: sorted.has_non_trivial_root_body,
                const_fragment_count: sorted.const_fragment_count,
                runtime_fragment_count: sorted.entry_runtime_fragment_count,
            };

            // Project the pre-AST `DirectExportSeed` from the bound, sorted declaration shells
            // and header-built public export metadata. This is the immediate consumer of the
            // per-file source-origin side table: the bindings and the public nominal-type origin
            // index depend only on header shells, so they are projected here before `sorted`
            // moves into AST construction. `DirectExportSeed` is the pre-AST export-identity
            // authority: it carries only header-shell facts and resolves no callable identities.
            // The post-AST `CallableSeed` table, built inside the public-interface draft builder,
            // joins this seed with the resolved receiver-method catalog to own receiver and
            // callable identity, so best-effort header receiver names never mask valid generic
            // receiver methods or preempt AST receiver diagnostics. The draft is retained only
            // on overall semantic success, so a diagnosed module exposes no component.
            let export_seed = build_direct_export_seed(
                &source_module_origins,
                active_root_file_id,
                &sorted.headers,
                &sorted.module_symbols,
                &compiler.string_table,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

            // Build the transient expanded public source-nominal origin index before `sorted`
            // moves into AST construction. Each origin is derived from the header's retained
            // FileId through the per-file SourceModuleOriginTable, so imported project-graph
            // nominals resolve to their defining provider origin. The type-surface projection
            // consumes this index to resolve imported nominal references in this module's public
            // signatures and fields.
            // The index mirrors the AST `source_path_is_public_from_root_file` nameability owner:
            // a nominal is included when a retained module-root or source-package public export
            // entry targets its canonical source path, so a privately-authored nominal exposed
            // through a public alias resolves to its graph-derived module origin. The retained
            // `module_symbols` is borrowed before `sorted` moves into AST construction.
            let public_source_nominal_type_origins = build_public_source_nominal_origin_index(
                &source_module_origins,
                &sorted.headers,
                &sorted.module_symbols,
                &compiler.string_table,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

            // Build the transient expanded public source-trait origin index before `sorted`
            // moves into AST construction. Analogous to the nominal origin index, this maps
            // each trait header whose canonical declaration path is targeted by a retained
            // public export entry to its stable OriginTraitId. Directly-defined, imported
            // project-graph and public-alias-target traits are included; private and unowned
            // source-package traits are excluded. The type-surface projection consumes this
            // index to resolve source-trait generic bounds to stable trait origin identities.
            let public_source_trait_origins = build_public_source_trait_origin_index(
                &source_module_origins,
                &sorted.headers,
                &sorted.module_symbols,
                &compiler.string_table,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

            // 3. Build the Abstract Syntax Tree (AST).
            // The build result carries the executable `Ast` plus the two closed side results
            // consumed before HIR: the public-interface projection input and the validated
            // donor-local generic-template map. HIR receives the executable `Ast` only. The
            // projection input carries the resolved receiver-method catalog and public type-root
            // table that step 4 joins with the pre-AST `DirectExportSeed` to build the post-AST
            // `CallableSeed` table, the one receiver and callable identity owner.
            let module_ast_build =
                timed_frontend_stage("frontend.ast", "AST created in: ", module_label, || {
                    self.build_ast(
                        &mut compiler,
                        sorted,
                        entry_file_path,
                        active_root_role,
                        capacity_estimate,
                        &mut warnings,
                    )
                })?;

            // Destructure the build result once: the projection input and generic-template map
            // feed the draft and extraction owners, while only the executable `Ast` reaches HIR.
            let AstBuildResult {
                ast: module_ast,
                public_interface_projection_input,
                generic_function_templates,
            } = module_ast_build;

            // 4. Build the one aggregate public-interface draft before HIR consumes the AST. The
            //    draft is the sole pre-HIR public-semantic handoff: the export-origin
            //    finalization, the canonical type-surface projection and the corrected
            //    trait-requirement projection are internal builder steps. They run from
            //    already-resolved facts (the receiver catalog, the resolved public type-root
            //    table, the resolved public trait-root vector, the `DirectExportSeed` and the
            //    module TypeEnvironment) without a second source scan or HIR scan, and are
            //    retained only on overall semantic success. The builder also produces the
            //    transient post-AST `CallableSeed` table, the one receiver and callable
            //    identity owner consumed by direct projection, declaration-record projection,
            //    HIR origin seeding and generic-template extraction. No donor-local TypeId,
            //    NominalTypeId, GenericParameterId, TraitId, CoreTraitKind or InternedPath
            //    crosses the module result boundary. It is not the final
            //    PublicSemanticInterface: reusable evidence is now an internal builder step
            //    and draft collection, generic template body extraction is already completed in
            //    step 4b, and concrete call-summary finalization is already completed after
            //    borrow validation, while provenance, re-export interfaces, cross-module call
            //    lowering and future generated-generic summaries remain for later phases.
            //    Folded constant values are now owned by each constant declaration record.
            let public_interface_build =
                PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
                    export_seed,
                    public_interface_projection_input,
                    public_source_nominal_type_origins: &public_source_nominal_type_origins,
                    public_source_trait_origins: &public_source_trait_origins,
                    type_environment: &module_ast.type_environment,
                    external_registry: compiler.external_package_registry.as_ref(),
                    string_table: &compiler.string_table,
                    generic_function_templates: &generic_function_templates,
                    module_constants: &module_ast.module_constants,
                })
                .build()
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
            let public_interface_draft = public_interface_build.draft;

            // 4b. Extract validated generic-template body artefacts before HIR consumes AST
            //     state. The transient public callable seed table is the exact path-to-origin
            //     authority for every directly exported generic free function or receiver method;
            //     the donor-local template map is the authority for the validated body payload.
            //     The extraction/join owner moves matching templates out of the donor-local map
            //     and keys them by the exact `OriginFunctionId` already retained by the draft.
            //     Private and non-generic templates remain intentional exclusions.
            //     This runs after generic body validation and before HIR so the templates never
            //     re-enter donor AST state.
            let validated_generic_templates = extract_validated_generic_template_artefacts(
                &public_interface_draft,
                &public_interface_build.callable_seeds,
                generic_function_templates,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

            let function_origin_lookup = HirFunctionOriginLookup::from_seeds(
                public_interface_build.function_origin_seeds,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

            // 5. Resolve const fragment StringIds to strings before AST is consumed by HIR.
            let const_top_level_fragments = module_ast
                .const_top_level_fragments
                .iter()
                .map(|fragment| ResolvedConstFragment {
                    runtime_insertion_index: fragment.runtime_insertion_index,
                    rendered_text: compiler.string_table.resolve(fragment.value).to_owned(),
                })
                .collect::<Vec<_>>();

            // 6. Lower AST to Higher-level Intermediate Representation (HIR).
            let hir_lowering =
                timed_frontend_stage("frontend.hir", "HIR generated in: ", module_label, || {
                    Self::lower_hir(&mut compiler, module_ast, &warnings, function_origin_lookup)
                })?;
            let HirLoweringResult {
                hir_module,
                type_environment,
                metadata: lowering_metadata,
            } = hir_lowering;

            // 7. Validate extracted non-HIR compiler metadata before a successful module is
            // returned. Invalid compiler metadata is an internal CompilerError.
            if let Err(error) = lowering_metadata.validate() {
                return Err(CompilerMessages::from_error_ref(
                    error,
                    &compiler.string_table,
                ));
            }

            // 8. Run static analysis (Borrow Checker).
            let borrow_analysis = timed_frontend_stage(
                "frontend.borrow",
                "Borrow checking completed in: ",
                module_label,
                || Self::check_borrows(&compiler, &hir_module, &warnings),
            )?;
            record_borrow_counters(&borrow_analysis);

            // Concrete call-summary finalization runs exactly once after HIR and borrow
            // validation produce the stable local-function relationship and complete call
            // summaries. The post-AST `CallableSeed` table owns the receiver and callable
            // identity that this finalization joins; only concrete-local callables receive a
            // summary record here. Generic templates remain declaration contracts whose
            // generated summaries belong to the future sidecar worklist, distinct from these
            // direct concrete summaries. Private functions and implicit start retain local
            // summaries but never enter declaration records.
            let public_interface = public_interface_draft
                .finalize_after_borrow_validation(&borrow_analysis.analysis, &hir_module)
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

            // Record every base function before entry selection. Build-owned assembly later
            // traverses these direct facts from the selected root and derives runtime unions.
            let function_link_facts = collect_module_function_link_facts(&hir_module)
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

            // -------------------------
            //  Finalize Module Build
            // -------------------------

            borrow_log!("=== BORROW CHECKER OUTPUT ===");
            borrow_log!(format!(
                "Borrow checking completed successfully (states={} functions={} blocks={} conflicts_checked={} stmt_facts={} term_facts={} value_facts={})",
                borrow_analysis.analysis.total_state_snapshots(),
                borrow_analysis.stats.functions_analyzed,
                borrow_analysis.stats.blocks_analyzed,
                borrow_analysis.stats.conflicts_checked,
                borrow_analysis.analysis.statement_facts.len(),
                borrow_analysis.analysis.terminator_facts.len(),
                borrow_analysis.analysis.value_facts.len()
            ));
            borrow_log!("=== END BORROW CHECKER OUTPUT ===");

            // Collect provider-resolved imports used by this module after the frontend has
            // consumed them. HIR still carries only stable external IDs; this side payload is for
            // backend asset/glue planning. Source logical paths are derived from the retained
            // source identity table, not from raw source inputs.
            let source_logical_paths = collect_source_logical_paths_from_table(
                &compiler.source_files,
                &compiler.string_table,
                self.project_path_resolver.is_some(),
            );

            let mut external_import_candidates: Vec<
                crate::build_system::build::ModuleExternalImport,
            > = external_import_resolution_table
                .collect_unique_resolved_imports_for_source_files(&source_logical_paths)
                .into_iter()
                .map(
                    |resolved| crate::build_system::build::ModuleExternalImport {
                        package_id: resolved.package_id,
                        runtime_asset: resolved.runtime_asset,
                        required_runtime_imports: resolved.required_runtime_imports,
                    },
                )
                .collect();

            // Builder runtime packages share the same candidate store as provider imports. Entry
            // assembly selects only packages called by its reachable function union.
            for builder_runtime in self.builder_runtime_packages {
                external_import_candidates.push(crate::build_system::build::ModuleExternalImport {
                    package_id: builder_runtime.package_id,
                    runtime_asset: builder_runtime.runtime_asset.clone(),
                    required_runtime_imports: builder_runtime.required_runtime_imports.clone(),
                });
            }

            external_import_candidates.sort_by_key(|import| import.package_id.0);
            external_import_candidates.dedup_by_key(|import| import.package_id);

            Ok((
                Module {
                    executable: ModuleExecutable {
                        hir: hir_module,
                        type_environment,
                        borrow_analysis,
                    },
                    link_facts: ModuleLinkFacts {
                        external_package_registry: Arc::clone(&compiler.external_package_registry),
                        external_import_candidates,
                        functions: function_link_facts,
                    },
                    metadata: ModuleCompilerMetadata::from_hir_lowering(
                        entry_file_path.to_path_buf(),
                        warnings,
                        lowering_metadata,
                        const_top_level_fragments,
                        root_activity,
                        validated_generic_templates,
                    ),
                },
                public_interface,
            ))
        })();

        // Normalize the deeper stages' mixed `CompilerMessages` once at this semantic boundary.
        // A successful compilation becomes `Success`. A failing stage becomes either
        // `Diagnosed` (user-facing diagnostics the renderer surfaces) or `Err(CompilerError)`
        // (an infrastructure failure recovered losslessly from its structured payload). This is
        // the single lossless ownership transfer; graph and render consumers never re-classify.
        match compile_result {
            Ok((module, public_interface)) => {
                let string_table = compiler.string_table;
                Ok(ModuleCompilationOutcome::Success(Box::new(
                    ModuleSemanticDraft {
                        module,
                        string_table,
                        public_interface,
                    },
                )))
            }
            Err(messages) => {
                // The failing stage already cloned the live `compiler.string_table` into the
                // messages, so the diagnosed payload carries every render identity produced so
                // far. `compiler` itself is no longer needed.
                match ModuleDiagnostics::from_messages(messages) {
                    Ok(diagnostics) => Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)),
                    Err(error) => Err(error),
                }
            }
        }
    }

    /// Bind retained `PreparedHeaderSyntax` against provider interfaces.
    ///
    /// WHAT: resolves public exports, builds the import environment, canonicalizes dependency
    ///       edges, and completes constant initializer dependencies. Consumes only the retained
    ///       syntax carried in from preparation — it never retokenizes or reparses source.
    /// WHY: these facts depend on provider interfaces and the project path resolver, so they
    ///      belong in the semantic phase after preparation has produced `PreparedHeaderSyntax`.
    fn bind_retained_headers(
        compiler: &mut CompilerFrontend,
        prepared_header_syntax: PreparedHeaderSyntax,
        external_import_resolution_table: &ExternalImportResolutionTable,
        source_provider_imports: &SourceProviderImportSet<'_>,
        warnings: &[CompilerDiagnostic],
    ) -> Result<BoundModuleHeaders, CompilerMessages> {
        let headers = bind_module_headers(
            prepared_header_syntax,
            compiler.external_package_registry.as_ref(),
            external_import_resolution_table,
            source_provider_imports,
            compiler.project_path_resolver.as_ref(),
            &mut compiler.string_table,
        )
        .map_err(|bag| {
            let mut messages = CompilerMessages::from_diagnostics(
                bag.into_diagnostics(),
                compiler.string_table.clone(),
            );
            messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
            messages
        })?;

        record_header_counters(&headers);
        Ok(headers)
    }

    fn sort_headers(
        compiler: &mut CompilerFrontend,
        module_headers: BoundModuleHeaders,
        warnings: &[CompilerDiagnostic],
    ) -> Result<SortedHeaders, CompilerMessages> {
        compiler.sort_headers(module_headers).map_err(|bag| {
            let mut messages = CompilerMessages::from_diagnostics(
                bag.into_diagnostics(),
                compiler.string_table.clone(),
            );
            messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
            messages
        })
    }

    fn build_ast(
        &self,
        compiler: &mut CompilerFrontend,
        sorted: SortedHeaders,
        entry_file_path: &Path,
        root_role: ModuleRootRole,
        capacity_estimate: FrontendArenaCapacityEstimate,
        warnings: &mut Vec<CompilerDiagnostic>,
    ) -> Result<AstBuildResult, CompilerMessages> {
        match compiler.headers_to_ast(
            sorted,
            entry_file_path,
            root_role,
            self.build_profile,
            capacity_estimate,
        ) {
            Ok(build_result) => {
                warnings.extend(build_result.ast.warnings.clone());
                Ok(build_result)
            }

            Err(messages) => Err(merge_stage_messages(
                messages,
                warnings,
                &compiler.string_table,
            )),
        }
    }

    fn lower_hir(
        compiler: &mut CompilerFrontend,
        module_ast: Ast,
        warnings: &[CompilerDiagnostic],
        function_origin_lookup: HirFunctionOriginLookup,
    ) -> Result<HirLoweringResult, CompilerMessages> {
        compiler
            .generate_hir(module_ast, function_origin_lookup)
            .map_err(|messages| merge_stage_messages(messages, warnings, &compiler.string_table))
    }

    fn check_borrows(
        compiler: &CompilerFrontend,
        hir_module: &HirModule,
        warnings: &[CompilerDiagnostic],
    ) -> Result<BorrowCheckReport, CompilerMessages> {
        compiler
            .check_borrows(hir_module)
            .map_err(|messages| merge_stage_messages(messages, warnings, &compiler.string_table))
    }
}

// -------------------------
//  Shared Helpers
// -------------------------

fn plan_file_preparation_chunks(
    source_file_count: usize,
    worker_thread_count: usize,
) -> Vec<FilePreparationChunkPlan> {
    if source_file_count == 0 {
        return Vec::new();
    }

    let worker_thread_count = worker_thread_count.max(1);
    let target_chunk_count =
        worker_thread_count.saturating_mul(FILE_PREPARATION_TARGET_TASKS_PER_THREAD);
    let max_chunk_count_by_size = (source_file_count / FILE_PREPARATION_MIN_CHUNK_SIZE).max(1);
    let chunk_count = target_chunk_count
        .min(max_chunk_count_by_size)
        .min(source_file_count)
        .max(1);

    let base_chunk_size = source_file_count / chunk_count;
    let larger_chunk_count = source_file_count % chunk_count;

    let mut plans = Vec::with_capacity(chunk_count);
    let mut start_file_index = 0usize;
    for chunk_index in 0..chunk_count {
        let chunk_size = base_chunk_size + usize::from(chunk_index < larger_chunk_count);
        let end_file_index = start_file_index + chunk_size;
        plans.push(FilePreparationChunkPlan {
            chunk_index,
            file_range: start_file_index..end_file_index,
        });
        start_file_index = end_file_index;
    }

    plans
}

/// Validate that sorted file-preparation chunks cover the module input exactly, in order, with
/// no gaps, overlaps, mismatched record counts or wrong internal file indexes.
///
/// WHAT: release-safe replacement for the ordering `debug_assert`s that previously guarded the
///      merge loop. Malformed scheduler payloads produce a `CompilerError` instead of silently
///      dropping, reordering or truncating prepared files.
/// WHY:  release builds must reject corrupted chunk payloads with the same invariant checks as
///      debug builds, and the merge path must not silently heal a broken scheduler result.
fn validate_preparation_chunk_order(
    preparation_chunks: &[FilePreparationChunk],
    module_file_count: usize,
) -> Result<(), CompilerError> {
    let mut expected_file_index = 0usize;

    for chunk in preparation_chunks {
        if chunk.file_range.start != expected_file_index {
            return Err(CompilerError::compiler_error(format!(
                "file preparation chunk {} starts at file index {} but expected \
                 {expected_file_index}; chunks must be ordered, non-overlapping and gap-free",
                chunk.chunk_index, chunk.file_range.start,
            )));
        }

        if chunk.file_range.end < chunk.file_range.start {
            return Err(CompilerError::compiler_error(format!(
                "file preparation chunk {} has reversed range {:?}",
                chunk.chunk_index, chunk.file_range,
            )));
        }

        if chunk.file_range.end > module_file_count {
            return Err(CompilerError::compiler_error(format!(
                "file preparation chunk {} ends at file index {} but the module has only \
                 {module_file_count} files",
                chunk.chunk_index, chunk.file_range.end,
            )));
        }

        if chunk.results.len() != chunk.file_range.len() {
            return Err(CompilerError::compiler_error(format!(
                "file preparation chunk {} declares range {:?} ({} files) but carries {} results",
                chunk.chunk_index,
                chunk.file_range,
                chunk.file_range.len(),
                chunk.results.len(),
            )));
        }

        for (expected_index, prepared_file) in (chunk.file_range.start..).zip(&chunk.results) {
            if prepared_file.file_index != expected_index {
                return Err(CompilerError::compiler_error(format!(
                    "file preparation chunk {} record carries file index {} but expected \
                     {expected_index}",
                    chunk.chunk_index, prepared_file.file_index,
                )));
            }
        }

        expected_file_index = chunk.file_range.end;
    }

    if expected_file_index != module_file_count {
        return Err(CompilerError::compiler_error(format!(
            "file preparation chunks cover {expected_file_index} files but the module has \
             {module_file_count} files",
        )));
    }

    Ok(())
}

pub(super) fn record_module_input_counters(module: &[PreparedSourceInput]) -> usize {
    add_frontend_counter(FrontendCounter::ModuleCount, 1);
    add_frontend_counter(FrontendCounter::SourceFileCount, module.len());

    let source_byte_count = module
        .iter()
        .map(|input_file| input_file.source_code().len())
        .sum();
    add_frontend_counter(FrontendCounter::SourceByteCount, source_byte_count);
    source_byte_count
}

fn record_file_preparation_strategy(
    strategy: FilePreparationStrategy,
    reason: FilePreparationStrategyReason,
) {
    match strategy {
        FilePreparationStrategy::Serial => {
            add_frontend_counter(FrontendCounter::FilePreparationSerialModuleCount, 1);
        }

        FilePreparationStrategy::ParallelPerFile | FilePreparationStrategy::ParallelChunked => {
            add_frontend_counter(FrontendCounter::FilePreparationParallelModuleCount, 1);
        }
    }

    match reason {
        FilePreparationStrategyReason::SmallSerial => {
            add_frontend_counter(FrontendCounter::FilePreparationStrategySmallSerialCount, 1);
        }

        FilePreparationStrategyReason::ByteThresholdSerial => {
            add_frontend_counter(
                FrontendCounter::FilePreparationStrategyByteThresholdSerialCount,
                1,
            );
        }

        FilePreparationStrategyReason::MediumByteThresholdParallel => {
            add_frontend_counter(
                FrontendCounter::FilePreparationStrategyParallelPerFileCount,
                1,
            );
            add_frontend_counter(FrontendCounter::FilePreparationStrategyParallelCount, 1);
        }

        FilePreparationStrategyReason::LargeChunkedParallel => {
            add_frontend_counter(FrontendCounter::FilePreparationStrategyChunkedCount, 1);
            add_frontend_counter(FrontendCounter::FilePreparationStrategyParallelCount, 1);
        }
    }
}

fn record_header_counters(headers: &BoundModuleHeaders) {
    add_frontend_counter(FrontendCounter::HeaderCount, headers.headers.len());

    let import_count = headers
        .module_symbols
        .file_imports_by_source
        .values()
        .map(Vec::len)
        .sum();
    add_frontend_counter(FrontendCounter::ImportCount, import_count);

    let top_level_declaration_count = headers
        .headers
        .iter()
        .filter(|header| {
            matches!(
                header.kind,
                HeaderKind::Function { .. }
                    | HeaderKind::Constant { .. }
                    | HeaderKind::Struct { .. }
                    | HeaderKind::Choice { .. }
                    | HeaderKind::TypeAlias { .. }
            )
        })
        .count();
    add_frontend_counter(
        FrontendCounter::TopLevelDeclarationCount,
        top_level_declaration_count,
    );
}

fn record_frontend_capacity_estimate(
    source_file_count: usize,
    source_byte_count: usize,
    headers: &BoundModuleHeaders,
) -> FrontendArenaCapacityEstimate {
    let const_fragment_count = headers.const_fragment_count;
    let capacity = FrontendArenaCapacityEstimate::new(
        source_file_count,
        source_byte_count,
        headers.token_stats,
        headers.header_stats,
        const_fragment_count,
        headers.entry_runtime_fragment_count,
    );

    // Phase 1 wires the scope-frame estimate because scope-frame arenas are the first typed arena
    // target. Phase 4 records actual frame allocation and arena capacity growth from the scope
    // arena owner; this site records only the policy estimate.
    add_frontend_counter(FrontendCounter::EstimatedScopeFrames, capacity.scope_frames);
    add_frontend_counter(
        FrontendCounter::CappedCapacityEstimates,
        capacity.capped_field_count,
    );

    capacity
}

fn record_borrow_counters(report: &BorrowCheckReport) {
    add_frontend_counter(
        FrontendCounter::BorrowFunctionCount,
        report.stats.functions_analyzed,
    );
    add_frontend_counter(
        FrontendCounter::BorrowBlockCount,
        report.stats.blocks_analyzed,
    );
    add_frontend_counter(
        FrontendCounter::BorrowConflictCheckCount,
        report.stats.conflicts_checked,
    );

    let state_snapshot_count = report.analysis.block_entry_states.len()
        + report.analysis.block_exit_states.len()
        + report.analysis.statement_entry_states.len();
    add_frontend_counter(
        FrontendCounter::BorrowStateSnapshotCount,
        state_snapshot_count,
    );
    add_frontend_counter(
        FrontendCounter::BorrowStatementVisitCount,
        report.stats.statements_analyzed,
    );
    add_frontend_counter(
        FrontendCounter::BorrowTerminatorVisitCount,
        report.stats.terminators_analyzed,
    );
    add_frontend_counter(
        FrontendCounter::BorrowWorklistIterationCount,
        report.stats.worklist_iterations,
    );
    add_frontend_counter(
        FrontendCounter::BorrowStateJoinCount,
        report.stats.state_joins,
    );

    add_frontend_counter(
        FrontendCounter::BorrowStatementFactCount,
        report.analysis.statement_facts.len(),
    );
    add_frontend_counter(
        FrontendCounter::BorrowTerminatorFactCount,
        report.analysis.terminator_facts.len(),
    );
    add_frontend_counter(
        FrontendCounter::BorrowValueFactCount,
        report.analysis.value_facts.len(),
    );
}

fn merge_stage_messages(
    messages: CompilerMessages,
    warnings: &[CompilerDiagnostic],
    string_table: &StringTable,
) -> CompilerMessages {
    let mut messages = messages;
    messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
    messages.string_table = string_table.clone();
    messages
}

/// Render the module's source logical paths from the retained source identity table.
///
/// WHAT: iterates the `SourceFileTable` built during preparation and renders each identity's
///       portable logical path. Returns an empty vector when no project path resolver was used
///       during preparation, matching the prior raw-source path behaviour.
/// WHY: semantic compilation derives source logical paths from retained identities instead of
///      carrying raw source paths, so the preparation/semantic boundary stays free of
///      `PreparedSourceInput`. UTF-8 validity was already enforced when the table was built.
fn collect_source_logical_paths_from_table(
    source_files: &SourceFileTable,
    string_table: &StringTable,
    has_project_path_resolver: bool,
) -> Vec<String> {
    if !has_project_path_resolver {
        return Vec::new();
    }

    source_files
        .iter()
        .map(|identity| identity.logical_path.to_portable_string(string_table))
        .collect()
}

/// Format a bounded, human-readable attribution label for one module's timing.
///
/// WHAT: combines the entry path, source file count, and source byte count into a
///      short label suitable for the concise timing summary's "slowest module" line.
/// WHY:  the label stays out of stable `MOTH_BENCH timing` lines; it is display-only
///       evidence so the summary can attribute the max sample without per-module listing.
pub(super) fn module_timing_label(
    entry_file_path: &Path,
    source_file_count: usize,
    source_byte_count: usize,
) -> String {
    let file_word = if source_file_count == 1 {
        "file"
    } else {
        "files"
    };
    format!(
        "{} ({} {}, {:.1}KB)",
        entry_file_path.display(),
        source_file_count,
        file_word,
        source_byte_count as f64 / 1024.0,
    )
}

/// Record one frontend pipeline stage through the central `timers` substrate.
///
/// WHAT: wraps a `Result`-returning stage so its duration is always recorded,
///      regardless of success or error, with an optional module attribution label.
/// WHY:  Phase 4 migrates public frontend stage timings from the `detailed_timers`
///       macro to the central `timers` collector so concise `timers`-only builds
///       see project-level aggregates.  Human prose stays gated by `detailed_timers`
///       for verbose developer output; the xtask legacy parser reads that prose to
///       attribute legacy metric names until Phase 6 updates xtask ratio definitions.
fn timed_frontend_stage<T>(
    metric: &str,
    prose_label: &str,
    module_label: Option<&str>,
    stage: impl FnOnce() -> Result<T, CompilerMessages>,
) -> Result<T, CompilerMessages> {
    let start = crate::timing::start_pipeline_timing();
    let result = stage();
    crate::timing::record_started_pipeline_timing_with_label(metric, start, module_label);

    // Human prose stays gated by detailed_timers for verbose developer output.
    #[cfg(feature = "detailed_timers")]
    {
        if crate::compiler_frontend::compiler_messages::compiler_dev_logging::detailed_timer_output_enabled() {
            saying::say!(prose_label, Green #start.elapsed());
        }
    }

    // Keep parameters visibly used when detailed_timers is off.
    let _ = prose_label;

    result
}

#[cfg(feature = "detailed_timers")]
fn timed_frontend_substep<T>(metric_name: &str, label: &str, substep: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = substep();
    benchmark_timer_log!(start, metric_name, label);
    result
}

#[cfg(not(feature = "detailed_timers"))]
fn timed_frontend_substep<T>(_metric_name: &str, _label: &str, substep: impl FnOnce() -> T) -> T {
    substep()
}

#[cfg(test)]
#[path = "../tests/frontend_orchestration_tests.rs"]
mod tests;
