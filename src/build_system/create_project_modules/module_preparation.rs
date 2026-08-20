//! Stage 0 module source preparation for Moth projects.
//!
//! WHAT: schedules provider-independent preparation of one discovered module's source files —
//!       serial, per-file or chunked — merges the resulting local string tables in deterministic
//!       input order, and aggregates the retained `PreparedHeaderSyntax` a module compile needs.
//! WHY: deciding which source belongs to a module, when to prepare it and how to spread that work
//!      across threads is build-system scheduling policy. Tokenization and header-preparation
//!      semantics stay compiler-owned behind one preparation call.
//!
//! This module stops at prepared syntax. Interface binding, declaration ordering, AST, HIR, borrow
//! validation and generated completion belong to `compiler_frontend::module_compilation`.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, FileFrontendPrepareOutput, HeaderParseOptions,
    PreparedHeaderSyntax, prepare_header_syntax,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::module_compilation::PreparedModuleInput;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StableModuleOriginIdentity};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::string_interning::{StringTable, StringTableForkSource};
use crate::compiler_frontend::{
    CompilerFrontend, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};
use crate::timed_stage_attributed;

use super::prepared_module::PreparedModule;
use super::prepared_source::PreparedSourceInput;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

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
    string_domain: PreparedFileStringDomain,
    result: Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure>,
}

/// String-ID domain carried with a prepared file before chunk aggregation.
///
/// WHAT: distinguishes an output produced against the current module-global table during
///       synthetic discovery from output produced against this chunk's local table.
/// WHY: source-kind information is deliberately erased once file preparation starts. The merge
///      boundary still needs an explicit fact so an already-global retained output never receives
///      a second exhaustive token/header/path/clause remap when another file makes the chunk
///      remap non-identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreparedFileStringDomain {
    ChunkLocal,
    AlreadyGlobal,
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

/// Incremental provider-independent syntax preparation for one indexed directory module.
///
/// Stage 0 prepares each selected source once, reads its retained dependency shells from the same
/// header output and only then decides which same-module source to prepare next. This keeps
/// semantic reachability and header ownership aligned without a second lexical dependency scanner.
pub(super) struct ModuleSyntaxDiscovery<'a> {
    context: &'a ModulePreparationContext<'a>,
    entry_file_path: PathBuf,
    active_root_role: ModuleRootRole,
    expected_active_origin: StableModuleOriginIdentity,
    source_module_origins: SourceModuleOriginTable,
    source_files: SourceFileTable,
    string_table: StringTable,
    prepared_outputs: Vec<(usize, FileFrontendPrepareOutput)>,
    warnings: Vec<CompilerDiagnostic>,
    source_byte_count: usize,
    contains_moth_template: bool,
    #[cfg(feature = "timers")]
    timing_context: Option<crate::timing::TimingContext>,
}

impl ModulePreparationContext<'_> {
    /// Begin header-owned reachability discovery for one indexed directory module.
    pub(super) fn begin_syntax_discovery<'a>(
        &'a self,
        stable_origin: StableModuleOriginIdentity,
        origin_by_canonical_path: &FxHashMap<PathBuf, StableModuleOriginIdentity>,
        candidate_source_paths: impl ExactSizeIterator<Item = &'a Path>,
        entry_file_path: &Path,
        mut string_table: StringTable,
        #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
    ) -> Result<ModuleSyntaxDiscovery<'a>, CompilerMessages> {
        let source_files = SourceFileTable::build(
            candidate_source_paths,
            entry_file_path,
            self.project_path_resolver.as_ref(),
            &mut string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))?;
        let source_module_origins =
            SourceModuleOriginTable::from_graph_ownership(&source_files, origin_by_canonical_path);

        Ok(ModuleSyntaxDiscovery {
            context: self,
            entry_file_path: entry_file_path.to_path_buf(),
            active_root_role: stable_origin.role(),
            expected_active_origin: stable_origin,
            source_module_origins,
            source_files,
            string_table,
            prepared_outputs: Vec::new(),
            warnings: Vec::new(),
            source_byte_count: 0,
            contains_moth_template: false,
            #[cfg(feature = "timers")]
            timing_context,
        })
    }

    /// Prepare one discovered module's source files and aggregate provider-independent header
    /// syntax, retaining it with the module string-table context and the active root's file
    /// identity for semantic compilation.
    ///
    /// WHAT: prepares every source file against local string-table forks, merges chunk-local
    ///       string tables in deterministic input order, and runs `prepare_header_syntax` to
    ///       produce the retained `PreparedHeaderSyntax`. Directory Moth inputs consume retained
    ///       token streams; synthetic Moth inputs consume complete outputs retained during
    ///       discovery. Stops before provider-dependent binding.
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
        stable_origin: StableModuleOriginIdentity,
        module: Vec<PreparedSourceInput>,
        entry_file_path: &Path,
        mut string_table: StringTable,
        source_byte_count: usize,
        #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
    ) -> Result<PreparedModule, CompilerMessages> {
        let mut warnings = Vec::new();
        let module_file_count = module.len();
        let contains_moth_template = module.iter().any(PreparedSourceInput::is_moth_template);

        // Entry identity and root semantics are separate. The stable module origin owns whether
        // the active file is a normal runtime-capable root or an API-only support/facade root.
        let active_root_role = stable_origin.role();

        // 1. Build the module source identity table against the caller-owned string table. Source
        //    identities are deterministic and provider-free, so this needs no provider interface.
        let source_files = Self::attach_source_files(
            &mut string_table,
            &self.project_path_resolver,
            &module,
            entry_file_path,
        )?;

        // 2. Rebind retained synthetic outputs against this module-owned source table, then
        //    prepare all remaining files against one local string-table per worker chunk. Directory
        //    Moth files parse retained Stage 0 tokens, synthetic Moth files consume their complete
        //    retained output, Moth templates tokenize their body once and plain Markdown bypasses
        //    tokenization. Merge/remap once before aggregating header syntax.
        let (prepared_header_syntax, file_warnings) = timed_stage_attributed!(
            crate::timing::TimingMetric::FrontendPrepare,
            timing_context,
            {
                self.prepare_module_files(
                    &mut string_table,
                    &source_files,
                    module,
                    entry_file_path,
                    active_root_role,
                    source_byte_count,
                )
            }
        )?;
        warnings.extend(file_warnings);

        // 3. Build the immutable per-file source-origin side table from the origin input. For
        //    directory modules the graph-owned lookup resolves each source file to its owning
        //    origin; for single-file compilation every file maps to the synthetic origin. The
        //    table is remap-free and provider-independent: it carries no StringIds and needs no
        //    provider interface.
        let source_module_origins =
            SourceModuleOriginTable::from_synthetic_origin(&source_files, &stable_origin);

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
            &stable_origin,
            entry_file_path,
            &string_table,
        )?;

        // Retain the deterministic preparation context so semantic compilation can continue against
        // the same string table and source identities. The payload owns no `CompilerFrontend` or
        // provider state: only syntax, the string table, source identities and warnings.
        Ok(PreparedModule {
            semantic: PreparedModuleInput {
                active_root_file_id,
                source_module_origins,
                prepared_header_syntax,
                string_table,
                source_files,
                warnings,
                source_file_count: module_file_count,
                source_byte_count,
            },
            contains_moth_template,
        })
    }

    /// Build the module `SourceFileTable` from input source paths against a caller-owned string
    /// table and the project path resolver.
    ///
    /// WHAT: assigns deterministic source identities for the prepared module without touching any
    ///       provider interface.
    /// WHY: preparation needs source identities to drive file preparation and header syntax, but
    ///      not the external package registry or dependency resolution table.
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
        mut module: Vec<PreparedSourceInput>,
        entry_file_path: &Path,
        active_root_role: ModuleRootRole,
        source_byte_count: usize,
    ) -> Result<(PreparedHeaderSyntax, Vec<CompilerDiagnostic>), CompilerMessages> {
        Self::rebind_synthetic_prepared_inputs(&mut module, source_files, string_table)?;

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

        let module_file_count = module.len();
        add_frontend_counter(
            FrontendCounter::FilePreparationInputFileCount,
            module_file_count,
        );
        add_frontend_counter(
            FrontendCounter::FilePreparationInputByteCount,
            source_byte_count,
        );

        let (strategy, strategy_reason) =
            FilePreparationStrategy::selection_for_module(module_file_count, source_byte_count);
        record_file_preparation_strategy(strategy, strategy_reason);

        let preparation_chunks = Self::prepare_module_file_chunks(
            module,
            &fork_source,
            &prepare_context,
            const_template_offset,
            runtime_fragment_offset,
            strategy,
        );

        Self::merge_file_preparation_chunks(
            string_table,
            preparation_chunks,
            module_file_count,
            base_len,
        )
    }

    /// Rebind complete synthetic discovery outputs against the module's retained source table.
    ///
    /// WHAT: moves every provisional synthetic `FileId` and source scope onto the exact
    ///       `SourceFileTable` that `PreparedModule` retains for later semantic binding.
    /// WHY: Stage 0 must discover and prepare files before the complete closure is known, but
    ///      module preparation is the sole owner of final source identity. Keeping rebinding here
    ///      prevents a discarded discovery-time table from becoming a second identity authority.
    fn rebind_synthetic_prepared_inputs(
        module: &mut [PreparedSourceInput],
        source_files: &SourceFileTable,
        string_table: &StringTable,
    ) -> Result<(), CompilerMessages> {
        for input in module {
            let PreparedSourceInput::MothPrepared {
                source_path,
                output,
                ..
            } = input
            else {
                continue;
            };

            let identity = source_files
                .get_by_canonical_path(source_path)
                .ok_or_else(|| {
                    CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(format!(
                            "module source identity table is missing retained synthetic file {:?}",
                            source_path
                        )),
                        string_table,
                    )
                })?;
            output
                .rebind_source_identity(
                    identity.file_id,
                    identity.logical_path.clone(),
                    identity.canonical_os_path.clone(),
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            output
                .freeze_path_syntax(string_table)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        }

        Ok(())
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
        preparation_chunks.sort_by_key(|chunk| chunk.chunk_index);

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
            let remap = string_table.merge_delta_from(&chunk.local_string_table, base_len);
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
                        match prepared_file.string_domain {
                            PreparedFileStringDomain::ChunkLocal => {
                                if !remap_is_identity {
                                    add_frontend_counter(
                                        FrontendCounter::FilePrepareOutputRemapCalls,
                                        1,
                                    );
                                    #[cfg(feature = "benchmark_counters")]
                                    add_frontend_counter(
                                        FrontendCounter::FilePrepareNonIdentityPayloadRemaps,
                                        1,
                                    );
                                    output.remap_string_ids(&remap).map_err(|error| {
                                        CompilerMessages::from_error_ref(error, string_table)
                                    })?;
                                }
                                output.freeze_path_syntax(string_table).map_err(|error| {
                                    CompilerMessages::from_error_ref(error, string_table)
                                })?;
                            }
                            PreparedFileStringDomain::AlreadyGlobal => {
                                if !remap_is_identity {
                                    add_frontend_counter(
                                        FrontendCounter::AlreadyGlobalPreparedOutputRemapSkipCount,
                                        1,
                                    );
                                }
                                output.require_frozen_path_syntax().map_err(|error| {
                                    CompilerMessages::from_error_ref(error, string_table)
                                })?;
                            }
                        }
                        warnings.append(&mut output.warnings);
                        prepared_outputs.push(output);
                    }
                    Err(FileFrontendPrepareFailure::Diagnosed(mut error)) => {
                        if prepared_file.string_domain == PreparedFileStringDomain::ChunkLocal
                            && !remap_is_identity
                        {
                            add_frontend_counter(FrontendCounter::FilePrepareErrorRemapCalls, 1);
                            #[cfg(feature = "benchmark_counters")]
                            add_frontend_counter(
                                FrontendCounter::FilePrepareNonIdentityPayloadRemaps,
                                1,
                            );
                            error.remap_string_ids(&remap);
                        }
                        warnings.extend(error.warnings);
                        diagnostics.push(*error.diagnostic);
                    }
                    Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                        return Err(CompilerMessages::from_error_ref(error, string_table));
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

        record_successful_prepared_outputs(&prepared_outputs);
        let prepared = prepare_header_syntax(prepared_outputs, string_table).map_err(|bag| {
            let mut messages =
                CompilerMessages::from_diagnostics(bag.into_diagnostics(), string_table.clone());
            messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
            messages
        })?;

        Ok((prepared, warnings))
    }

    fn prepare_module_file_chunks(
        module: Vec<PreparedSourceInput>,
        fork_source: &StringTableForkSource,
        prepare_context: &FrontendFilePrepareContext<'_>,
        const_template_offset: usize,
        runtime_fragment_offset: usize,
        strategy: FilePreparationStrategy,
    ) -> Vec<FilePreparationChunk> {
        let module_file_count = module.len();
        match strategy {
            FilePreparationStrategy::Serial => vec![Self::prepare_module_file_chunk(
                FilePreparationChunkPlan {
                    chunk_index: 0,
                    file_range: 0..module_file_count,
                },
                module.into_iter().enumerate(),
                fork_source,
                prepare_context,
                const_template_offset,
                runtime_fragment_offset,
            )],
            FilePreparationStrategy::ParallelPerFile => module
                .into_par_iter()
                .enumerate()
                .map(|(file_index, file)| {
                    Self::prepare_module_file_chunk(
                        FilePreparationChunkPlan {
                            chunk_index: file_index,
                            file_range: file_index..file_index + 1,
                        },
                        std::iter::once((file_index, file)),
                        fork_source,
                        prepare_context,
                        const_template_offset,
                        runtime_fragment_offset,
                    )
                })
                .collect(),
            FilePreparationStrategy::ParallelChunked => {
                let plans =
                    plan_file_preparation_chunks(module_file_count, rayon::current_num_threads());
                let mut module_files = module.into_iter().enumerate();
                let planned_files = plans
                    .into_iter()
                    .map(|plan| {
                        let files = module_files
                            .by_ref()
                            .take(plan.file_range.len())
                            .collect::<Vec<_>>();
                        (plan, files)
                    })
                    .collect::<Vec<_>>();

                planned_files
                    .into_par_iter()
                    .map(|(plan, files)| {
                        Self::prepare_module_file_chunk(
                            plan,
                            files,
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
        module: impl IntoIterator<Item = (usize, PreparedSourceInput)>,
        fork_source: &StringTableForkSource,
        prepare_context: &FrontendFilePrepareContext<'_>,
        const_template_offset: usize,
        runtime_fragment_offset: usize,
    ) -> FilePreparationChunk {
        let (mut local_string_table, _) = fork_source.fork_for_module().into_parts();
        let mut results = Vec::with_capacity(plan.file_range.len());

        for (file_index, file) in module {
            let (string_domain, result) = match file {
                PreparedSourceInput::MothPrepared { output, .. } => {
                    (PreparedFileStringDomain::AlreadyGlobal, Ok(*output))
                }
                file => {
                    let source = match file {
                        PreparedSourceInput::Moth {
                            source_path,
                            tokens,
                            ..
                        } => FrontendFilePrepareSource::Moth {
                            source_path,
                            tokens,
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
                        PreparedSourceInput::MothPrepared { .. } => {
                            unreachable!(
                                "prepared Moth output was handled before source conversion"
                            )
                        }
                    };
                    let input = FrontendFilePrepareInput {
                        source,
                        const_template_offset,
                        runtime_fragment_offset,
                    };
                    (
                        PreparedFileStringDomain::ChunkLocal,
                        CompilerFrontend::prepare_file_frontend_local(
                            prepare_context,
                            input,
                            &mut local_string_table,
                        ),
                    )
                }
            };
            results.push(PreparedFileResult {
                file_index,
                string_domain,
                result,
            });
        }

        FilePreparationChunk {
            chunk_index: plan.chunk_index,
            file_range: plan.file_range,
            local_string_table,
            results,
        }
    }
}

impl ModuleSyntaxDiscovery<'_> {
    pub(super) fn string_table_mut(&mut self) -> &mut StringTable {
        &mut self.string_table
    }

    /// Prepare one selected source and return the retained provider dependencies parsed from the
    /// same retained header output.
    pub(super) fn prepare_source(
        &mut self,
        source: PreparedSourceInput,
    ) -> Result<FileFrontendPrepareOutput, CompilerMessages> {
        if matches!(&source, PreparedSourceInput::MothPrepared { .. }) {
            return Err(CompilerMessages::from_error_ref(
                CompilerError::compiler_error(
                    "indexed module syntax discovery received an already-prepared synthetic source",
                ),
                &self.string_table,
            ));
        }
        let source_byte_len = source.source_byte_len();
        self.contains_moth_template |= source.is_moth_template();
        let entry_file_id = self
            .source_files
            .get_by_canonical_path(&self.entry_file_path)
            .map(|identity| identity.file_id);
        let options = HeaderParseOptions {
            entry_file_id,
            project_path_resolver: self.context.project_path_resolver.clone(),
            active_root_role: self.active_root_role,
        };
        let prepare_context = FrontendFilePrepareContext {
            source_files: &self.source_files,
            style_directives: self.context.style_directives,
            entry_file_path: &self.entry_file_path,
            options: &options,
        };
        let frontend_source = match source {
            PreparedSourceInput::Moth {
                source_path,
                tokens,
                ..
            } => FrontendFilePrepareSource::Moth {
                source_path,
                tokens,
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
            PreparedSourceInput::MothPrepared { .. } => {
                unreachable!("already-prepared synthetic source was rejected above")
            }
        };
        let input = FrontendFilePrepareInput {
            source: frontend_source,
            const_template_offset: 0,
            runtime_fragment_offset: 0,
        };

        let output = match timed_stage_attributed!(
            crate::timing::TimingMetric::FrontendPrepare,
            self.timing_context,
            CompilerFrontend::prepare_file_frontend_local(
                &prepare_context,
                input,
                &mut self.string_table,
            ),
        ) {
            Ok(output) => output,
            Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
                let mut messages = CompilerMessages::from_diagnostics(
                    vec![*error.diagnostic],
                    self.string_table.clone(),
                );
                messages.prepend_diagnostics_preserving_context(error.warnings);
                return Err(messages);
            }
            Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                return Err(CompilerMessages::from_error_ref(error, &self.string_table));
            }
        };

        self.source_byte_count += source_byte_len;
        self.warnings.extend(output.warnings.iter().cloned());
        Ok(output)
    }

    /// Retain one completed source output after Stage 0 has consumed its dependency facts.
    ///
    /// WHAT: commits the already prepared file output to the module's deterministic source-order
    ///      collection.
    /// WHY: Stage 0 must consume the retained clause and flat-selection facts before the output is
    ///      frozen, while source preparation itself remains exactly once.
    pub(super) fn retain_prepared_output(
        &mut self,
        source_order: usize,
        output: FileFrontendPrepareOutput,
    ) {
        self.prepared_outputs.push((source_order, output));
    }

    /// Freeze the selected source outputs into the one retained module preparation payload.
    pub(super) fn finish(mut self) -> Result<PreparedModule, CompilerMessages> {
        self.prepared_outputs.sort_by_key(|(order, _)| *order);
        let mut prepared_outputs = self
            .prepared_outputs
            .into_iter()
            .map(|(_, output)| output)
            .collect::<Vec<_>>();
        for output in &mut prepared_outputs {
            output
                .freeze_path_syntax(&self.string_table)
                .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;
        }
        let source_file_count = prepared_outputs.len();
        record_successful_prepared_outputs(&prepared_outputs);
        let prepared_header_syntax = timed_stage_attributed!(
            crate::timing::TimingMetric::FrontendPrepare,
            self.timing_context,
            prepare_header_syntax(prepared_outputs, &mut self.string_table),
        )
        .map_err(|bag| {
            let mut messages = CompilerMessages::from_diagnostics(
                bag.into_diagnostics(),
                self.string_table.clone(),
            );
            messages.prepend_diagnostics_preserving_context(self.warnings.iter().cloned());
            messages
        })?;
        let active_root_file_id = ModulePreparationContext::resolve_and_validate_active_root(
            &self.source_files,
            &self.source_module_origins,
            &self.expected_active_origin,
            &self.entry_file_path,
            &self.string_table,
        )?;

        Ok(PreparedModule {
            semantic: PreparedModuleInput {
                active_root_file_id,
                source_module_origins: self.source_module_origins,
                prepared_header_syntax,
                string_table: self.string_table,
                source_files: self.source_files,
                warnings: self.warnings,
                source_file_count,
                source_byte_count: self.source_byte_count,
            },
            contains_moth_template: self.contains_moth_template,
        })
    }
}

/// Record successful prepared-file volume at the common pre-aggregation boundary.
///
/// Both indexed directory discovery and chunk/synthetic preparation retain the complete file
/// outputs here. Counting before `prepare_header_syntax` consumes them avoids a second token walk
/// and keeps attempts (`FilePreparationPassCount`) distinct from successfully retained outputs.
fn record_successful_prepared_outputs(outputs: &[FileFrontendPrepareOutput]) {
    let token_count = outputs.iter().map(|output| output.token_count).sum();

    add_frontend_counter(FrontendCounter::PreparedFileCount, outputs.len());
    add_frontend_counter(FrontendCounter::TokenCount, token_count);
}

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
        .map(PreparedSourceInput::source_byte_len)
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

#[cfg(test)]
#[path = "../tests/module_preparation_tests.rs"]
mod tests;
