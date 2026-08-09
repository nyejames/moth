//! Per-module frontend compilation pipeline for Moth projects.
//!
//! Drives a single discovered module through the full frontend pipeline:
//! provider-independent source preparation → provider binding → dependency sort → AST → HIR →
//! borrow checking.

use crate::build_system::build::{
    GeneratedFunctionSidecar, Module, ModuleCompilerMetadata, ModuleExecutable,
    ModuleExternalImport, ModuleLinkFacts, ModuleRootActivity, ModuleSemanticDraft,
    ResolvedConstFragment,
};
use crate::timed_stage_attributed;

use crate::builder_surface::external_import_providers::provider::BuilderRuntimePackageMetadata;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstantiationRequest, GenericFunctionTemplate, ModuleMaterialisationInput,
    bootstrap_call_summary_from_signature, concrete_argument_mapping,
    recursive_generic_function_instantiation, substitute_function_signature,
};
use crate::compiler_frontend::ast::{Ast, AstBuildResult, AstImportedFunctionContract};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalEvidenceIdentity, CanonicalTypeIdentity, CanonicalTypeProjectionContext,
    ExportedGenericParameterIdentity, GenericParameterOriginResolver, NominalOriginResolver,
    project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::ModuleDiagnostics;
use crate::compiler_frontend::external_packages::{ExternalPackageId, ExternalPackageRegistry};
use crate::compiler_frontend::headers::import_environment::SourceFunctionTarget;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, FileFrontendPrepareError, FileFrontendPrepareOutput, HeaderKind,
    HeaderParseOptions, PreparedHeaderSyntax, bind_module_headers, prepare_header_syntax,
};
use crate::compiler_frontend::hir::functions::{
    HirFunctionOriginLookup, PrivateFunctionOriginSeed,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::{
    collect_module_function_link_facts, collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::instrumentation::{
    FrontendCounter, add_frontend_counter, increment_frontend_counter,
};
use crate::compiler_frontend::module_dependencies::SortedHeaders;
use crate::compiler_frontend::module_metadata::HirLoweringResult;
use crate::compiler_frontend::paths::const_paths::RetainedProviderReference;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_call_summary::{
    PublicCallSummaryTransition, validate_public_call_summary_transition,
};
use crate::compiler_frontend::public_interface::{
    PublicInterfaceDraftBuilder, PublicInterfaceDraftBuilderInput, PublicSemanticInterface,
    SourceProviderImportSet,
};
use crate::compiler_frontend::public_interface::{
    build_direct_export_seed, build_public_source_nominal_origin_index,
    build_public_source_trait_origin_index,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModuleRootRole, OriginTypeId,
    StableModuleOriginIdentity,
};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::string_interning::{
    StringId, StringTable, StringTableForkSource,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::validated_generic_template_metadata::validate_materialisation_context_templates;
use crate::compiler_frontend::{
    CompilerFrontend, FrontendBuildProfile, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};

use super::compiled_boundary::CompletedSourcePackageRegistry;
use super::generated_summary_convergence::{
    exact_generated_sidecar_summary, run_generated_summary_convergence,
};
use super::generated_worklist::{
    GeneratedFunctionWorklist, GeneratedRequestEntry, GeneratedRequestFacts, GeneratedRequestId,
};
use super::module_artifact_store::ModuleArtifactStore;
use super::prepared_module::PreparedModule;
use super::prepared_source::PreparedSourceInput;

use crate::borrow_log;
use crate::projects::settings::Config;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
/// Stage 0 prepares each selected source once, reads its retained import shells from the same
/// header output and only then decides which same-module source to prepare next. This keeps
/// semantic reachability and header ownership aligned without a second lexical import scanner.
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
    pub(super) source_provider_materialisations: &'a SourceProviderMaterialisationSet<'a>,
    pub(super) builder_runtime_packages: &'a [BuilderRuntimePackageMetadata],
}

#[derive(Default)]
pub(super) struct SourceProviderMaterialisationSet<'a> {
    project_contexts: Option<&'a ModuleArtifactStore>,
    completed_packages: Option<&'a CompletedSourcePackageRegistry>,
}

enum DeclaringMaterialisation<'a> {
    Published {
        context: &'a crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext,
        template_index: usize,
    },
    Preparing(
        &'a crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationPreparation,
    ),
}

impl<'a> SourceProviderMaterialisationSet<'a> {
    pub(super) fn new(
        project_contexts: &'a ModuleArtifactStore,
        completed_packages: &'a CompletedSourcePackageRegistry,
    ) -> Self {
        Self {
            project_contexts: Some(project_contexts),
            completed_packages: Some(completed_packages),
        }
    }

    fn context_for(
        &self,
        identity: &GeneratedDeclarationIdentity,
        requester_context: &'a crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationPreparation,
    ) -> Result<Option<DeclaringMaterialisation<'a>>, CompilerError> {
        if let Some(project_contexts) = self.project_contexts
            && let Some(location) = project_contexts.materialisation_context_for(identity)?
        {
            let context = project_contexts.materialisation_context_at(location)?;
            return Ok(Some(DeclaringMaterialisation::Published {
                context,
                template_index: location.template_index,
            }));
        }

        if let Some(completed_packages) = self.completed_packages
            && let Some(location) = completed_packages.materialisation_location_for(identity)
        {
            let package = completed_packages.package(location.package_id)?;
            let context = package
                .boundary
                .modules
                .materialisation_context_at(location.location)?;
            return Ok(Some(DeclaringMaterialisation::Published {
                context,
                template_index: location.location.template_index,
            }));
        }

        Ok(requester_context
            .template_for_identity(identity)
            .is_some()
            .then_some(DeclaringMaterialisation::Preparing(requester_context)))
    }
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

        // 2. Prepare all files against one local string-table per worker chunk. Moth files
        //    parse retained Stage 0 tokens, Moth template tokenizes its body once and plain Markdown
        //    bypasses tokenization. Merge/remap once before aggregating header syntax.
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
            active_root_file_id,
            source_module_origins,
            prepared_header_syntax,
            string_table,
            source_files,
            contains_moth_template,
            warnings,
            source_file_count: module_file_count,
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
        module: Vec<PreparedSourceInput>,
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
                        if !remap_is_identity {
                            add_frontend_counter(FrontendCounter::FilePrepareOutputRemapCalls, 1);
                            #[cfg(feature = "benchmark_counters")]
                            add_frontend_counter(
                                FrontendCounter::FilePrepareNonIdentityPayloadRemaps,
                                1,
                            );
                            output.remap_string_ids(&remap);
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
                            error.remap_string_ids(&remap);
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
        let prepared = prepare_header_syntax(prepared_outputs, string_table).map_err(|bag| {
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
        module: Vec<PreparedSourceInput>,
        fork_source: &StringTableForkSource,
        prepare_context: &FrontendFilePrepareContext<'_>,
        const_template_offset: usize,
        runtime_fragment_offset: usize,
        strategy: FilePreparationStrategy,
    ) -> Vec<FilePreparationChunk> {
        let module_file_count = module.len();
        let plans = match strategy {
            FilePreparationStrategy::Serial => vec![FilePreparationChunkPlan {
                chunk_index: 0,
                file_range: 0..module_file_count,
            }],
            FilePreparationStrategy::ParallelPerFile => (0..module_file_count)
                .map(|index| FilePreparationChunkPlan {
                    chunk_index: index,
                    file_range: index..index + 1,
                })
                .collect(),
            FilePreparationStrategy::ParallelChunked => {
                plan_file_preparation_chunks(module_file_count, rayon::current_num_threads())
            }
        };
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

        match strategy {
            FilePreparationStrategy::Serial => planned_files
                .into_iter()
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
                .collect(),
            FilePreparationStrategy::ParallelPerFile | FilePreparationStrategy::ParallelChunked => {
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
        module: Vec<(usize, PreparedSourceInput)>,
        fork_source: &StringTableForkSource,
        prepare_context: &FrontendFilePrepareContext<'_>,
        const_template_offset: usize,
        runtime_fragment_offset: usize,
    ) -> FilePreparationChunk {
        let (mut local_string_table, _) = fork_source.fork_for_module().into_parts();
        let mut results = Vec::with_capacity(plan.file_range.len());

        for (file_index, file) in module {
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

impl ModuleSyntaxDiscovery<'_> {
    pub(super) fn string_table_mut(&mut self) -> &mut StringTable {
        &mut self.string_table
    }

    /// Prepare one selected source and return the retained provider references parsed from the
    /// same retained header output.
    pub(super) fn prepare_source(
        &mut self,
        source_order: usize,
        source: PreparedSourceInput,
    ) -> Result<Vec<RetainedProviderReference>, CompilerMessages> {
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
            Err(error) => {
                let mut messages = CompilerMessages::from_diagnostics(
                    vec![*error.diagnostic],
                    self.string_table.clone(),
                );
                messages.prepend_diagnostics_preserving_context(error.warnings);
                return Err(messages);
            }
        };

        self.source_byte_count += source_byte_len;
        self.warnings.extend(output.warnings.iter().cloned());
        let providers = output
            .file_imports
            .iter()
            .map(|import| {
                let mut provider = import.authored_provider.clone();
                provider.from_grouped = import.from_grouped;
                provider
            })
            .collect();
        self.prepared_outputs.push((source_order, output));

        Ok(providers)
    }

    /// Freeze the selected source outputs into the one retained module preparation payload.
    pub(super) fn finish(mut self) -> Result<PreparedModule, CompilerMessages> {
        self.prepared_outputs.sort_by_key(|(order, _)| *order);
        let prepared_outputs = self
            .prepared_outputs
            .into_iter()
            .map(|(_, output)| output)
            .collect::<Vec<_>>();
        let source_file_count = prepared_outputs.len();
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
            active_root_file_id,
            source_module_origins: self.source_module_origins,
            prepared_header_syntax,
            string_table: self.string_table,
            source_files: self.source_files,
            contains_moth_template: self.contains_moth_template,
            warnings: self.warnings,
            source_file_count,
            source_byte_count: self.source_byte_count,
        })
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
        #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
        mut generated_worklist: GeneratedFunctionWorklist<'_>,
    ) -> Result<ModuleCompilationOutcome, CompilerError> {
        let PreparedModule {
            active_root_file_id,
            source_module_origins,
            prepared_header_syntax,
            string_table,
            source_files,
            contains_moth_template: _contains_moth_template,
            mut warnings,
            source_file_count,
            source_byte_count,
        } = prepared;

        let active_module_origin = source_module_origins
            .origin_for(active_root_file_id)?
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "semantic module compilation: active root file id {} has no module origin",
                    active_root_file_id.0
                ))
            })?
            .clone();
        let active_root_role = active_module_origin.role();

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
            let module_headers = timed_stage_attributed!(
                crate::timing::TimingMetric::FrontendBindHeaders,
                timing_context,
                {
                    Self::bind_retained_headers(
                        &mut compiler,
                        prepared_header_syntax,
                        external_import_resolution_table,
                        self.source_provider_imports,
                        &warnings,
                    )
                }
            )?;

            let capacity_estimate = record_frontend_capacity_estimate(
                source_file_count,
                source_byte_count,
                &module_headers,
            );

            // 2. Resolve dependencies and sort headers for linear processing.
            let sorted = timed_stage_attributed!(
                crate::timing::TimingMetric::FrontendOrderDeclarations,
                timing_context,
                Self::sort_headers(&mut compiler, module_headers, &warnings)
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
                self.source_provider_imports,
                compiler.external_package_registry.as_ref(),
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
            let module_ast_build = self.build_ast(
                &mut compiler,
                sorted,
                entry_file_path,
                active_root_role,
                capacity_estimate,
                &mut warnings,
                #[cfg(feature = "timers")]
                timing_context,
            )?;

            // Destructure the build result once: the projection input and generic-template map
            // feed the draft and extraction owners, while only the executable `Ast` reaches HIR.
            let AstBuildResult {
                ast: mut module_ast,
                public_interface_projection_input,
                materialisation_context: mut materialisation_context_builder,
                deferred_generic_requests,
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
            let public_interface_build = timed_stage_attributed!(
                crate::timing::TimingMetric::FrontendPublicInterfaceProject,
                timing_context,
                PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
                    export_seed,
                    public_interface_projection_input,
                    public_source_nominal_type_origins: &public_source_nominal_type_origins,
                    public_source_trait_origins: &public_source_trait_origins,
                    type_environment: &module_ast.type_environment,
                    external_registry: compiler.external_package_registry.as_ref(),
                    string_table: &compiler.string_table,
                    generic_function_templates: materialisation_context_builder
                        .context()
                        .generic_function_templates(),
                    module_constants: &module_ast.module_constants,
                })
                .build(),
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
            let public_interface_draft = public_interface_build.draft;

            let public_origins_by_path = public_interface_build
                .callable_seeds
                .iter()
                .map(|seed| (seed.path.clone(), seed.origin.clone()))
                .collect::<FxHashMap<_, _>>();
            let private_function_origin_seeds = materialisation_context_builder
                .install_concrete_executable_contracts(
                    &active_module_origin,
                    &public_origins_by_path,
                    &public_source_nominal_type_origins,
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?
                .into_iter()
                .map(|(path, origin)| PrivateFunctionOriginSeed { path, origin })
                .collect::<Vec<_>>();

            let generated_requests = install_generated_request_contracts(
                &deferred_generic_requests,
                materialisation_context_builder.context(),
                materialisation_context_builder
                    .context()
                    .generic_function_templates(),
                compiler.external_package_registry.as_ref(),
                &mut module_ast,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
            let generated_request_ids =
                generated_worklist.register_requests(generated_requests.iter().map(|request| {
                    GeneratedRequestFacts {
                        identity: request.identity.clone(),
                        display_name: request
                            .function_name
                            .map(|name| compiler.string_table.resolve(name).to_owned())
                            .unwrap_or_else(|| "<generated>".to_owned()),
                        diagnostic_location: request.call_location.clone(),
                    }
                }));
            // 4b. Extract validated generic-template body artefacts before HIR consumes AST
            //     state. The transient public callable seed table is the exact path-to-origin
            //     authority for every directly exported generic free function or receiver method;
            //     the donor-local template map is the authority for the validated body payload.
            //     The extraction/join owner moves matching templates out of the donor-local map
            //     and keys them by the exact `OriginFunctionId` already retained by the draft.
            //     Private and non-generic templates remain intentional exclusions.
            //     This runs after generic body validation and before HIR so the templates never
            //     re-enter donor AST state.
            validate_materialisation_context_templates(
                &public_interface_draft,
                &public_interface_build.callable_seeds,
                materialisation_context_builder.generic_function_templates_mut(),
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

            let function_origin_lookup = HirFunctionOriginLookup::from_public_and_private_seeds(
                public_interface_build.function_origin_seeds,
                private_function_origin_seeds,
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
            let hir_lowering = timed_stage_attributed!(
                crate::timing::TimingMetric::FrontendHir,
                timing_context,
                Self::lower_hir(&mut compiler, module_ast, &warnings, function_origin_lookup)
            )?;
            let HirLoweringResult {
                mut hir_module,
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

            // Link facts are the validated-HIR owner for direct call targets. The convergence
            // observation model consumes these facts after HIR validation rather than scanning
            // source or introducing a second HIR call graph.
            let function_link_facts = collect_module_function_link_facts(&hir_module)
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

            // 8. Run static analysis (Borrow Checker).
            increment_frontend_counter(FrontendCounter::ConvergenceInitialBaseBorrowPasses);
            let bootstrap_borrow_analysis = timed_stage_attributed!(
                crate::timing::TimingMetric::FrontendBorrowInitial,
                timing_context,
                Self::check_borrows(&compiler, &hir_module, &warnings)
            )?;
            install_exact_concrete_call_summaries(
                &mut materialisation_context_builder,
                &hir_module,
                &bootstrap_borrow_analysis,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
            timed_stage_attributed!(
                crate::timing::TimingMetric::FrontendGeneratedMaterialise,
                timing_context,
                self.materialise_generated_request_roots(
                    &generated_request_ids,
                    &mut generated_worklist,
                    materialisation_context_builder.context(),
                    &mut compiler,
                    entry_file_path,
                    #[cfg(feature = "timers")]
                    timing_context,
                )
            )?;
            let borrow_analysis = run_generated_summary_convergence(
                &compiler,
                &mut hir_module,
                &function_link_facts,
                &mut generated_worklist,
                bootstrap_borrow_analysis,
                &warnings,
                #[cfg(feature = "timers")]
                timing_context,
            )?;
            let _ = install_exact_concrete_call_summaries(
                &mut materialisation_context_builder,
                &hir_module,
                &borrow_analysis,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
            let generated_worklist_delta = generated_worklist
                .finish()
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
            record_borrow_counters(&borrow_analysis);

            // Concrete call-summary finalization runs exactly once after HIR and borrow
            // validation produce the stable local-function relationship and complete call
            // summaries. The post-AST `CallableSeed` table owns the receiver and callable
            // identity that this finalization joins; only concrete-local callables receive a
            // summary record here. Generic templates remain declaration contracts whose
            // generated summaries belong to the future sidecar worklist, distinct from these
            // direct concrete summaries. Private functions and implicit start retain local
            // summaries but never enter declaration records.
            let public_interface = timed_stage_attributed!(
                crate::timing::TimingMetric::FrontendPublicInterfaceFinalise,
                timing_context,
                {
                    let local_public_interface = public_interface_draft
                        .finalize_after_borrow_validation(&borrow_analysis.analysis, &hir_module)
                        .map_err(|error| {
                            CompilerMessages::from_error_ref(error, &compiler.string_table)
                        })?;
                    PublicSemanticInterface::close_from_local(
                        local_public_interface,
                        self.source_provider_imports,
                        compiler.external_package_registry.as_ref(),
                    )
                    .map_err(|error| {
                        CompilerMessages::from_error_ref(error, &compiler.string_table)
                    })
                }
            )?;
            let materialisation_context = materialisation_context_builder
                .freeze(&public_interface)
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
                        materialisation_context,
                    ),
                },
                public_interface,
                generated_worklist_delta,
            ))
        })();

        // Normalize the deeper stages' mixed `CompilerMessages` once at this semantic boundary.
        // A successful compilation becomes `Success`. A failing stage becomes either
        // `Diagnosed` (user-facing diagnostics the renderer surfaces) or `Err(CompilerError)`
        // (an infrastructure failure recovered losslessly from its structured payload). This is
        // the single lossless ownership transfer; graph and render consumers never re-classify.
        match compile_result {
            Ok((module, public_interface, generated_worklist_delta)) => {
                let string_table = compiler.string_table;
                Ok(ModuleCompilationOutcome::Success(Box::new(
                    ModuleSemanticDraft {
                        module,
                        generated_worklist_delta,
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

    // The timing context is a cfg-gated parameter that disappears from
    // no-timer builds; bundling it would add a context struct for one field.
    #[allow(clippy::too_many_arguments)]
    fn build_ast(
        &self,
        compiler: &mut CompilerFrontend,
        sorted: SortedHeaders,
        entry_file_path: &Path,
        root_role: ModuleRootRole,
        capacity_estimate: FrontendArenaCapacityEstimate,
        warnings: &mut Vec<CompilerDiagnostic>,
        #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
    ) -> Result<AstBuildResult, CompilerMessages> {
        match compiler.headers_to_ast(
            sorted,
            entry_file_path,
            root_role,
            self.build_profile,
            capacity_estimate,
            #[cfg(feature = "timers")]
            timing_context,
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

    fn materialise_generated_request_roots(
        &self,
        request_ids: &[GeneratedRequestId],
        worklist: &mut GeneratedFunctionWorklist<'_>,
        requester_context: &crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationPreparation,
        compiler: &mut CompilerFrontend,
        entry_file_path: &Path,
        #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
    ) -> Result<(), CompilerMessages> {
        for request_id in request_ids {
            let identity = worklist
                .identity(*request_id)
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?
                .clone();
            let (display_name, diagnostic_location) = worklist
                .request_facts(*request_id)
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
            self.materialise_generated_request(
                *request_id,
                &MaterialisingRequest {
                    identity,
                    display_name,
                    diagnostic_location,
                    #[cfg(feature = "timers")]
                    timing_context,
                },
                worklist,
                requester_context,
                compiler,
                entry_file_path,
            )?;
        }
        Ok(())
    }

    fn materialise_generated_request(
        &self,
        request_id: GeneratedRequestId,
        request: &MaterialisingRequest,
        worklist: &mut GeneratedFunctionWorklist<'_>,
        requester_context: &crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationPreparation,
        compiler: &mut CompilerFrontend,
        entry_file_path: &Path,
    ) -> Result<(), CompilerMessages> {
        match worklist
            .enter(request_id)
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?
        {
            GeneratedRequestEntry::Complete => return Ok(()),
            GeneratedRequestEntry::Recursive => {
                return Err(CompilerMessages::from_diagnostic(
                    recursive_generic_function_instantiation(
                        Some(compiler.string_table.intern(&request.display_name)),
                        request.diagnostic_location.clone(),
                    ),
                    compiler.string_table.clone(),
                ));
            }
            GeneratedRequestEntry::Materialise => {}
        }
        let declaring_context = self
            .source_provider_materialisations
            .context_for(request.identity.declaration(), requester_context)
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "Generated request for '{}' has no completed declaring-module materialisation context",
                        request.identity.declaration().defining_name()
                    )),
                    &compiler.string_table,
                )
            })?;
        let materialised = match declaring_context {
            DeclaringMaterialisation::Published {
                context,
                template_index,
            } => context.materialise_ast_at(
                template_index,
                ModuleMaterialisationInput {
                    identity: &request.identity,
                    requester_context,
                    requester_call_location: &request.diagnostic_location,
                    external_package_registry: self.external_packages.as_ref(),
                    style_directives: self.style_directives,
                    build_profile: self.build_profile,
                    project_path_resolver: self
                        .project_path_resolver
                        .clone()
                        .or_else(|| requester_context.project_path_resolver.clone()),
                    template_const_loop_iteration_limit: self
                        .config
                        .template_const_loop_iteration_limit,
                    #[cfg(feature = "timers")]
                    timing_context: request.timing_context,
                },
            ),
            DeclaringMaterialisation::Preparing(context) => context.materialise_ast(
                &request.identity,
                requester_context,
                &request.diagnostic_location,
                self.project_path_resolver
                    .clone()
                    .or_else(|| requester_context.project_path_resolver.clone()),
                #[cfg(feature = "timers")]
                request.timing_context,
            ),
        }?;
        let crate::compiler_frontend::ast::generic_functions::MaterialisedGenericAst {
            build_result,
            string_table: generated_string_table,
            instance_path,
        } = materialised;
        let AstBuildResult {
            ast: mut generated_ast,
            materialisation_context: generated_context_builder,
            deferred_generic_requests: nested_requests,
            ..
        } = build_result;
        let generated_context = generated_context_builder.finish_preparation();
        let nested_requests = install_generated_request_contracts(
            &nested_requests,
            &generated_context,
            generated_context.generic_function_templates(),
            self.external_packages.as_ref(),
            &mut generated_ast,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &generated_string_table))?;
        let nested_request_ids =
            worklist.register_requests(nested_requests.iter().map(|request| {
                GeneratedRequestFacts {
                    identity: request.identity.clone(),
                    display_name: request
                        .function_name
                        .map(|name| generated_string_table.resolve(name).to_owned())
                        .unwrap_or_else(|| "<generated>".to_owned()),
                    diagnostic_location: request.call_location.clone(),
                }
            }));

        let first_nested_sidecar = worklist.sidecar_count();
        let mut generated_compiler = CompilerFrontend::new(
            self.config,
            generated_string_table,
            self.style_directives.clone(),
            Arc::clone(&self.external_packages),
            self.project_path_resolver.clone(),
        );
        for nested_request_id in &nested_request_ids {
            let nested_identity = worklist
                .identity(*nested_request_id)
                .map_err(|error| {
                    CompilerMessages::from_error_ref(error, &generated_compiler.string_table)
                })?
                .clone();
            let (nested_name, nested_location) = worklist
                .request_facts(*nested_request_id)
                .map_err(|error| {
                    CompilerMessages::from_error_ref(error, &generated_compiler.string_table)
                })?;
            self.materialise_generated_request(
                *nested_request_id,
                &MaterialisingRequest {
                    identity: nested_identity,
                    display_name: nested_name,
                    diagnostic_location: nested_location,
                    #[cfg(feature = "timers")]
                    timing_context: request.timing_context,
                },
                worklist,
                &generated_context,
                &mut generated_compiler,
                entry_file_path,
            )?;
        }

        let generated_warnings = generated_ast.warnings.clone();
        let generated_lowering = Self::lower_hir(
            &mut generated_compiler,
            generated_ast,
            &generated_warnings,
            HirFunctionOriginLookup::default(),
        )?;
        let HirLoweringResult {
            mut hir_module,
            type_environment,
            metadata: lowering_metadata,
        } = generated_lowering;
        let function_id = hir_module
            .functions
            .iter()
            .find_map(|function| {
                (hir_module.side_table.function_name_path(function.id) == Some(&instance_path))
                    .then_some(function.id)
            })
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Generated HIR omitted its requested root function",
                    ),
                    &generated_compiler.string_table,
                )
            })?;
        hir_module
            .function_ids_by_generated
            .insert(request.identity.clone(), function_id);
        increment_frontend_counter(FrontendCounter::ConvergenceGeneratedSidecarBorrowPasses);
        let borrow_analysis =
            Self::check_borrows(&generated_compiler, &hir_module, &generated_warnings)?;
        let functions = collect_module_function_link_facts(&hir_module).map_err(|error| {
            CompilerMessages::from_error_ref(error, &generated_compiler.string_table)
        })?;
        let reachability =
            collect_reachability_from_function_link_facts(&functions, &[function_id]).map_err(
                |error| CompilerMessages::from_error_ref(error, &generated_compiler.string_table),
            )?;
        let mut reachable_package_ids = rustc_hash::FxHashSet::default();
        for external_function_id in &reachability.reachable_external_functions {
            let package_id = self
                .external_packages
                .resolve_function_package_id(*external_function_id)
                .ok_or_else(|| {
                    CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(format!(
                            "Generated external function {external_function_id:?} has no owning package"
                        )),
                        &generated_compiler.string_table,
                    )
                })?;
            reachable_package_ids.insert(package_id);
        }
        let external_import_candidates = collect_external_import_candidates_for_packages(
            &reachable_package_ids,
            self.external_import_resolution_table,
            self.builder_runtime_packages,
        );
        let mut generated_module = Module {
            executable: ModuleExecutable {
                hir: hir_module,
                type_environment,
                borrow_analysis,
            },
            link_facts: ModuleLinkFacts {
                external_package_registry: Arc::clone(&self.external_packages),
                external_import_candidates,
                functions,
            },
            metadata: ModuleCompilerMetadata {
                entry_point: entry_file_path.to_path_buf(),
                warnings: generated_warnings,
                const_top_level_fragments: Vec::new(),
                root_activity: ModuleRootActivity::default(),
                doc_fragments: lowering_metadata.doc_fragments,
                rendered_path_usages: lowering_metadata.rendered_path_usages,
                materialisation_context: None,
            },
        };
        let summary = exact_generated_sidecar_summary(&request.identity, &generated_module)
            .map_err(|error| {
                CompilerMessages::from_error_ref(error, &generated_compiler.string_table)
            })?;
        let generated_remap = compiler
            .string_table
            .merge_from(&generated_compiler.string_table);
        if !generated_remap.is_identity() {
            worklist.remap_sidecars_from(first_nested_sidecar, &generated_remap);
            generated_module.remap_string_ids(&generated_remap);
        }
        worklist
            .complete(
                request_id,
                summary,
                GeneratedFunctionSidecar::new(request.identity.clone(), generated_module),
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))
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

fn install_exact_concrete_call_summaries(
    context: &mut crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationPreparationBuilder,
    hir: &HirModule,
    borrow_analysis: &BorrowCheckReport,
) -> Result<bool, CompilerError> {
    let mut changed = false;
    for contract in context.imported_functions_mut().values_mut() {
        let function_id = match &contract.target {
            SourceFunctionTarget::Imported { origin, .. } => {
                let Some(function_id) = hir.function_ids_by_origin.get(origin).copied() else {
                    continue;
                };
                function_id
            }
            SourceFunctionTarget::ModulePrivate { identity, .. } => hir
                .function_ids_by_private_origin
                .get(identity)
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Module materialisation context could not resolve private executable {identity:?}"
                    ))
                })?,
            SourceFunctionTarget::Local(_) | SourceFunctionTarget::Generated { .. } => continue,
        };
        let exact_summary = borrow_analysis
            .analysis
            .public_call_summaries
            .get(&function_id)
            .cloned()
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Module materialisation context is missing the exact call summary for {function_id:?}"
                ))
            })?;
        add_frontend_counter(FrontendCounter::ConvergenceSummaryComparisons, 1);
        let transition =
            validate_public_call_summary_transition(&contract.summary, &exact_summary)?;
        if transition == PublicCallSummaryTransition::Widened {
            add_frontend_counter(FrontendCounter::ConvergenceSummaryChanges, 1);
            contract.summary = exact_summary;
            changed = true;
        }
    }
    Ok(changed)
}

fn collect_external_import_candidates_for_packages(
    package_ids: &rustc_hash::FxHashSet<ExternalPackageId>,
    resolution_table: &ExternalImportResolutionTable,
    builder_runtime_packages: &[BuilderRuntimePackageMetadata],
) -> Vec<ModuleExternalImport> {
    let mut candidates = Vec::with_capacity(package_ids.len());

    for package_id in package_ids {
        if let Some(resolved) = resolution_table.get_by_package_id(*package_id) {
            candidates.push(ModuleExternalImport {
                package_id: *package_id,
                runtime_asset: resolved.runtime_asset.clone(),
                required_runtime_imports: resolved.required_runtime_imports.clone(),
            });
            continue;
        }

        if let Some(builder_runtime) = builder_runtime_packages
            .iter()
            .find(|runtime| runtime.package_id == *package_id)
        {
            candidates.push(ModuleExternalImport {
                package_id: *package_id,
                runtime_asset: builder_runtime.runtime_asset.clone(),
                required_runtime_imports: builder_runtime.required_runtime_imports.clone(),
            });
        }
    }

    candidates.sort_by_key(|candidate| candidate.package_id.0);
    candidates
}

struct GeneratedRequestNominalOrigins<'a> {
    type_environment: &'a crate::compiler_frontend::datatypes::environment::TypeEnvironment,
}

impl NominalOriginResolver for GeneratedRequestNominalOrigins<'_> {
    fn resolve_nominal_origin(
        &self,
        nominal_id: crate::compiler_frontend::datatypes::ids::NominalTypeId,
    ) -> Result<OriginTypeId, CompilerError> {
        let type_id = self
            .type_environment
            .type_id_for_nominal_id(nominal_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request references a nominal type without a local type identity",
                )
            })?;
        let canonical_identity = self
            .type_environment
            .canonical_identity_for_type_id(type_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request nominal type has no canonical identity",
                )
            })?;
        let CanonicalTypeIdentity::SourceNominal(origin) = canonical_identity else {
            return Err(CompilerError::compiler_error(
                "Generated request nominal type has a non-source canonical identity",
            ));
        };

        Ok(origin.clone())
    }
}

struct NoGeneratedRequestGenericParameters;

impl GenericParameterOriginResolver for NoGeneratedRequestGenericParameters {
    fn resolve_generic_parameter_origin(
        &self,
        _parameter_id: crate::compiler_frontend::datatypes::ids::GenericParameterId,
    ) -> Result<ExportedGenericParameterIdentity, CompilerError> {
        Err(CompilerError::compiler_error(
            "Concrete generated request retained an unresolved generic parameter",
        ))
    }
}

#[derive(Clone)]
struct CanonicalGeneratedRequest {
    identity: GeneratedFunctionIdentity,
    function_name: Option<StringId>,
    call_location: crate::compiler_frontend::tokenizer::tokens::SourceLocation,
}

/// The identity and diagnostic facts one generated request needs while materialising.
struct MaterialisingRequest {
    identity: GeneratedFunctionIdentity,
    display_name: String,
    diagnostic_location: SourceLocation,
    #[cfg(feature = "timers")]
    timing_context: Option<crate::timing::TimingContext>,
}

fn install_generated_request_contracts(
    requests: &[GenericFunctionInstantiationRequest],
    materialisation_context: &crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationPreparation,
    templates: &FxHashMap<
        crate::compiler_frontend::symbols::interned_path::InternedPath,
        GenericFunctionTemplate,
    >,
    external_registry: &ExternalPackageRegistry,
    module_ast: &mut Ast,
) -> Result<Vec<CanonicalGeneratedRequest>, CompilerError> {
    let mut identities = Vec::with_capacity(requests.len());
    for request in requests {
        let template = templates.get(&request.key.function_path).ok_or_else(|| {
            CompilerError::compiler_error(
                "Deferred generic request has no requester-local generic contract",
            )
        })?;
        let declaration_identity = template.declaration_identity.clone().ok_or_else(|| {
            CompilerError::compiler_error(
                "Deferred generic request template has no stable declaration identity",
            )
        })?;
        if let Some(request_identity) = request.declaration_identity.as_ref()
            && request_identity != &declaration_identity
        {
            return Err(CompilerError::compiler_error(
                "Deferred generic request declaration identity disagrees with its template",
            ));
        }

        let canonical_type_arguments = {
            let nominal_origins = GeneratedRequestNominalOrigins {
                type_environment: &materialisation_context.type_environment,
            };
            let generic_parameter_origins = NoGeneratedRequestGenericParameters;
            let projection_context = CanonicalTypeProjectionContext::new(
                &nominal_origins,
                &generic_parameter_origins,
                external_registry,
            );
            let mut canonical_type_arguments = Vec::with_capacity(request.key.type_arguments.len());
            for type_id in request.key.type_arguments.iter().copied() {
                canonical_type_arguments.push(project_type_id_to_canonical_identity(
                    type_id,
                    &materialisation_context.type_environment,
                    &projection_context,
                )?);
            }
            canonical_type_arguments
        };
        let identity = GeneratedFunctionIdentity::new(
            declaration_identity,
            canonical_type_arguments.into_boxed_slice(),
            canonicalize_generated_request_evidence(
                request,
                materialisation_context,
                external_registry,
            )?,
        );

        let mapping = concrete_argument_mapping(
            template.generic_parameter_list_id,
            request.key.type_arguments.as_ref(),
            &module_ast.type_environment,
        )
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Deferred generic request does not match its projected parameter list",
            )
        })?;
        let signature = substitute_function_signature(
            &template.signature,
            &mapping,
            &mut module_ast.type_environment,
        );
        let fallible_carrier_type_id = signature.error_return_type_id().map(|error_type_id| {
            let success_type_id = match signature.success_return_type_ids().as_slice() {
                [] => crate::compiler_frontend::datatypes::builtin_type_ids::NONE,
                [single] => *single,
                many => module_ast.type_environment.intern_tuple(many.to_vec()),
            };
            module_ast
                .type_environment
                .intern_fallible_carrier(success_type_id, error_type_id)
        });
        let summary = bootstrap_call_summary_from_signature(&signature);
        identities.push(CanonicalGeneratedRequest {
            identity: identity.clone(),
            function_name: request.key.function_path.name(),
            call_location: request.call_location.clone(),
        });
        let contract = AstImportedFunctionContract {
            target: SourceFunctionTarget::Generated {
                identity,
                local_path: request.instance_path.clone(),
            },
            summary,
            fallible_carrier_type_id,
        };

        if module_ast
            .imported_functions_by_local_path
            .insert(request.instance_path.clone(), contract)
            .is_some()
        {
            return Err(CompilerError::compiler_error(
                "Generated request path collides with another imported or generated callable",
            ));
        }
    }

    Ok(identities)
}

fn canonicalize_generated_request_evidence(
    request: &GenericFunctionInstantiationRequest,
    materialisation_context: &crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationPreparation,
    external_registry: &ExternalPackageRegistry,
) -> Result<Box<[CanonicalEvidenceIdentity]>, CompilerError> {
    if request.evidence.is_empty() {
        return Ok(Box::new([]));
    }

    let nominal_origins = GeneratedRequestNominalOrigins {
        type_environment: &materialisation_context.type_environment,
    };
    let generic_parameter_origins = NoGeneratedRequestGenericParameters;
    let projection_context = CanonicalTypeProjectionContext::new(
        &nominal_origins,
        &generic_parameter_origins,
        external_registry,
    );
    let mut canonical_evidence = Vec::with_capacity(request.evidence.len());
    for evidence_id in request.evidence.iter().copied() {
        let evidence = materialisation_context
            .trait_evidence_environment()
            .get(evidence_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request retained a missing requester-local evidence selection",
                )
            })?;
        let target_type_identity = project_type_id_to_canonical_identity(
            evidence.target_type_id,
            &materialisation_context.type_environment,
            &projection_context,
        )?;
        let trait_identity = materialisation_context
            .trait_environment()
            .canonical_identity_for_id(evidence.trait_id)
            .cloned()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request evidence has no stable canonical trait identity",
                )
            })?;
        canonical_evidence.push(CanonicalEvidenceIdentity::new(
            target_type_identity,
            trait_identity,
        ));
    }

    Ok(canonical_evidence.into_boxed_slice())
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

pub(super) fn merge_stage_messages(
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

#[cfg(test)]
#[path = "../tests/frontend_orchestration_tests.rs"]
mod tests;
