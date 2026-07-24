//! Core build orchestration and output writing for Moth projects.
//!
//! This module provides the canonical project build flow (`build_project`) and a dedicated output
//! writer (`write_project_outputs`). Build tools can compile once and choose where artifacts are
//! written without reimplementing frontend/backend orchestration.

use crate::build_system::create_project_modules::compile_project_frontend;
pub use crate::build_system::output_cleanup::CleanupPolicy;
use crate::build_system::output_cleanup::{
    finalize_output_cleanup, prepare_output_cleanup, validate_relative_output_path,
};
use crate::build_system::path_validation::check_if_valid_path;
use crate::build_system::project_config::{ProjectConfigParseServices, load_project_config};
use crate::build_system::utils::{file_error_messages, should_skip_unchanged_write};

use crate::compiler_frontend::Flag;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::module_metadata::{HirLoweringMetadata, ModuleDocFragment};
use crate::compiler_frontend::public_interface_draft::PublicInterfaceDraft;
use crate::compiler_frontend::style_directives::{StyleDirectiveRegistry, StyleDirectiveSpec};
use crate::compiler_frontend::symbols::compiler_symbols::CompilerSymbolSet;
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use crate::compiler_frontend::validated_generic_template_metadata::ValidatedGenericTemplateStore;

use crate::builder_surface::BuilderSurface;
use crate::builder_surface::external_import_providers::provider::{
    RequiredRuntimeImport, RuntimeAssetIdentity,
};
use crate::compiler_frontend::external_packages::{ExternalPackageId, ExternalPackageRegistry};
use crate::compiler_frontend::paths::rendered_path_usage::RenderedPathUsage;
use crate::projects::settings::{Config, ProjectConfigError};

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FILE_MIN_UNIQUE_SYMBOLS_CAPACITY: usize = 32;

// -------------------------
//  Build Payload Types
// -------------------------

/// A resolved const top-level fragment: a static string and its runtime insertion index.
///
/// WHAT: carries a fully resolved (not interned) const fragment string plus the count of
/// runtime fragments that precede it in source order.
/// WHY: builders merge const strings with the runtime fragment list returned by entry start()
/// using the insertion index to reconstruct source-order interleaving.
pub struct ResolvedConstFragment {
    /// Number of runtime fragments preceding this const fragment in source order.
    pub runtime_insertion_index: usize,
    /// The rendered text content of this const fragment.
    pub rendered_text: String,
}

/// Build-system-owned metadata for one external import used by a compiled module.
///
/// WHAT: carries the backend-facing identity for a provider-resolved external import after
///       deduplication across a module's source files.
/// WHY: backends emit runtime assets and generated glue based on this metadata without needing
///      the full per-source-file resolution table.
#[derive(Debug, Clone)]
// Kept ahead of the backend handoff: external provider/runtime metadata is recorded by the
// frontend today, while the current HTML path only reads the subset it can lower.
pub(crate) struct ModuleExternalImport {
    pub(crate) package_id: ExternalPackageId,
    pub(crate) runtime_asset: Option<RuntimeAssetIdentity>,
    pub(crate) required_runtime_imports: Vec<RequiredRuntimeImport>,
}

/// Header-derived root activity metadata passed to backend builders.
///
/// WHAT: records the builder-relevant activity of the active module root.
/// WHY: header parsing already classifies root bodies and page fragments, so builders can apply
///      artifact policy without scanning tokens or HIR for the same facts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ModuleRootActivity {
    pub(crate) has_non_trivial_root_body: bool,
    pub(crate) const_fragment_count: usize,
    pub(crate) runtime_fragment_count: usize,
}

impl ModuleRootActivity {
    /// Return whether the HTML builder has any root activity from which to assemble a page.
    pub(crate) fn has_html_artifact_activity(&self) -> bool {
        self.has_non_trivial_root_body
            || self.const_fragment_count > 0
            || self.runtime_fragment_count > 0
    }
}

/// Module-local executable semantic state: validated HIR, paired type environment and borrow
/// facts.
///
/// WHAT: the sole owner of the typed HIR, its paired `TypeEnvironment` and the
///       `BorrowCheckReport` produced by borrow validation.
/// WHY: keeping these together in one executable lane makes the HIR/type/borrow pairing obvious
///      at every backend call site and lets string-ID remapping touch HIR and type identity
///      exactly once. Borrow facts carry only HIR IDs and need no string remap.
pub(crate) struct ModuleExecutable {
    pub(crate) hir: HirModule,
    pub(crate) type_environment: TypeEnvironment,
    pub(crate) borrow_analysis: BorrowCheckReport,
}

impl ModuleExecutable {
    /// Remap interned string IDs after string-table merging.
    ///
    /// WHY: HIR and type identity remap exactly once here. `BorrowCheckReport` carries only HIR
    ///      IDs (no `StringId`s), so it is intentionally not remapped.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.hir.remap_string_ids(remap);
        self.type_environment.remap_string_ids(remap);
    }
}

/// Backend-neutral link facts for one compiled module.
///
/// WHAT: owns the deduplicated provider-resolved external imports and the effective external
///       package registry the frontend resolved this module against.
/// WHY: backends consume one flat list of runtime assets and required imports to emit glue and
///      copy assets, and validate against the same symbols the frontend resolved. The complete
///      effective registry is a current dependency carried here until Phase 7 replaces it with
///      immutable binding interfaces and per-function link facts; it is not itself a per-function
///      link fact.
pub(crate) struct ModuleLinkFacts {
    /// Effective external package registry after provider resolution for this module.
    ///
    /// WHY: provider-backed import discovery mutates the registry during Stage 0; the module
    ///      must carry the effective registry so backends validate and lower against the same
    ///      symbols the frontend resolved, rather than reconstructing a fresh registry that
    ///      loses provider-created packages. This is a temporary current dependency; Phase 7
    ///      narrows it to immutable binding interfaces and per-function link facts.
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    /// Provider-resolved external imports used by this module, deduplicated.
    ///
    /// WHY: backends need a flat list of runtime assets and required imports to emit glue
    ///      and copy assets, without carrying the full per-source-file resolution table.
    pub(crate) module_external_imports: Vec<ModuleExternalImport>,
}

/// Non-HIR compiler and builder-facing metadata for one compiled module.
///
/// WHAT: owns the resolved root-local entry path, module warnings, resolved const top-level
///       fragments, header-derived root activity, resolved documentation fragments, and
///       rendered-path usages.
/// WHY: these are compiler-metadata lanes, not executable HIR state or link facts. Consolidating
///      them into one owned lane on the `Module` payload keeps HIR limited to executable/semantic
///      IR and gives string-ID remapping, warning collection, and tracked-asset planning a single
///      owner. The architecture assigns resolved root-local entry metadata to this lane.
pub(crate) struct ModuleCompilerMetadata {
    /// Canonical entry file for the compiled module.
    pub(crate) entry_point: PathBuf,
    pub(crate) warnings: Vec<CompilerDiagnostic>,
    pub(crate) const_top_level_fragments: Vec<ResolvedConstFragment>,
    pub(crate) root_activity: ModuleRootActivity,
    pub(crate) doc_fragments: Vec<ModuleDocFragment>,
    pub(crate) rendered_path_usages: Vec<RenderedPathUsage>,
    /// Validated generic callable template body artefacts keyed by stable
    /// [`crate::compiler_frontend::semantic_identity::OriginFunctionId`].
    ///
    /// WHAT: one deterministic artefact per directly exported generic free-function or
    ///       receiver-method origin and none for non-generic or private callables. Each artefact
    ///       moves the one existing validated `GenericFunctionTemplate` body payload out of the
    ///       donor-local AST template map. The store is TIR-free and `Send`.
    /// WHY: locked decision 10 retains the declaring module's template body as a compiler
    ///      metadata checkpoint for the future build-owned generated sidecar worklist (R3). This
    ///      is a body-artefact checkpoint only, not the complete materialisation context: complete
    ///      materialisation also needs declaration, file-visibility, generic/type and related
    ///      frontend context that this slice intentionally does not retain. The legacy flat-module
    ///      handoff drops this store before string-table remap because the retained
    ///      `FunctionSignature` carries donor-local `StringId`s whose remap owner is not in scope
    ///      for the current slice.
    pub(crate) validated_generic_templates: ValidatedGenericTemplateStore,
}

impl ModuleCompilerMetadata {
    pub(crate) fn from_hir_lowering(
        entry_point: PathBuf,
        warnings: Vec<CompilerDiagnostic>,
        lowering_metadata: HirLoweringMetadata,
        const_top_level_fragments: Vec<ResolvedConstFragment>,
        root_activity: ModuleRootActivity,
        validated_generic_templates: ValidatedGenericTemplateStore,
    ) -> Self {
        Self {
            entry_point,
            warnings,
            doc_fragments: lowering_metadata.doc_fragments,
            rendered_path_usages: lowering_metadata.rendered_path_usages,
            const_top_level_fragments,
            root_activity,
            validated_generic_templates,
        }
    }

    /// Drop the unconsumed validated generic-template body-artefact store.
    ///
    /// WHY: the retained `GenericFunctionTemplate` values carry `FunctionSignature` donor-local
    ///      `StringId`s whose remap owner is not in scope for this slice. The store must never
    ///      remain reachable by a backend after a legacy flat-module handoff, so each handoff
    ///      path calls this before `remap_string_ids`. R3 will consume the store for the
    ///      generated sidecar worklist before this discard.
    pub(crate) fn discard_validated_generic_templates(&mut self) {
        let _ = std::mem::take(&mut self.validated_generic_templates);
    }

    /// Remap interned string IDs after string-table merging.
    ///
    /// WHY: warnings, documentation locations, and rendered-path interned fields must all remap
    ///      exactly once. Const fragment rendered text is already a resolved `String`, root
    ///      activity carries no interned fields, and the entry path is a `PathBuf`.
    ///
    /// The `validated_generic_templates` store is intentionally not remapped here. Its retained
    /// `GenericFunctionTemplate` values carry `FunctionSignature` donor-local `StringId`s whose
    /// remap owner is not in scope for the current slice. The legacy flat-module handoff drops
    /// the store before calling this method so no stale local `StringId` reaches backends. R3
    /// will add a dedicated remap path when the generated sidecar worklist consumes the store.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for warning in &mut self.warnings {
            warning.remap_string_ids(remap);
        }

        for fragment in &mut self.doc_fragments {
            fragment.location.remap_string_ids(remap);
        }

        for usage in &mut self.rendered_path_usages {
            usage.source_path.remap_string_ids(remap);
            usage.public_path.remap_string_ids(remap);
            usage.source_file_scope.remap_string_ids(remap);
            usage.render_location.remap_string_ids(remap);
        }
    }
}

/// Frontend output for one module root ready for backend lowering.
///
/// WHAT: a lane container with exactly an executable lane (typed HIR, paired type environment
///       and borrow facts), a link-facts lane (external imports and the effective registry), and
///       a compiler-metadata lane (entry path, warnings, fragments, root activity, docs and
///       rendered paths).
/// WHY: backends consume one stable module payload shape regardless of project type, with
///      explicit ownership keeping HIR/type/borrow pairing obvious at call sites.
pub struct Module {
    pub(crate) executable: ModuleExecutable,
    pub(crate) link_facts: ModuleLinkFacts,
    pub(crate) metadata: ModuleCompilerMetadata,
}

impl Module {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        increment_frontend_counter(FrontendCounter::ModuleRemapStringIdsCalls);

        self.executable.remap_string_ids(remap);
        self.metadata.remap_string_ids(remap);

        // Link facts carry no interned StringIds: external import metadata uses resolved
        // runtime asset identities and package IDs.
    }
}

/// Per-module frontend compilation result carrying the evolved local string table.
pub(crate) struct CompiledModuleResult {
    pub module: Module,
    pub string_table: StringTable,
    /// The one public-interface draft for declarations defined directly in the active module root,
    /// retained alongside the successful compile result so later graph/interface slices can
    /// consume it. It internalizes the direct export-origin, canonical type-surface and corrected
    /// trait-requirement projections behind one builder, with joined local callable summaries and
    /// explicit pending states for exported generic templates awaiting R3 sidecars.
    /// It carries only owned stable values: no `TypeId`, `NominalTypeId`,
    /// `GenericParameterId`, `TraitId`, `InternedPath` or `StringId` crosses this boundary. It
    /// is not part of the accepted three-lane `Module` and is not the final
    /// `PublicSemanticInterface`. The legacy flat `Vec<Module>` handoff explicitly drops it
    /// until the graph consumer lands.
    pub public_interface_draft: PublicInterfaceDraft,
}

// -------------------------
//  Backend Abstractions
// -------------------------

/// Unified build interface for all project types
pub trait BackendBuilder {
    /// Build the project with the given configuration
    fn build_backend(
        &self,
        modules: Vec<Module>, // Each collection of files the frontend has compiled into modules
        config: &Config,      // Persistent settings across the whole project
        flags: &[Flag],       // Settings only relevant to this build
        string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages>;

    /// Validate the project configuration
    fn validate_project_config(
        &self,
        config: &Config,
        string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError>;

    /// Project-specific frontend style directives provided by this backend.
    ///
    /// Frontend-owned directives are always present in registry construction and cannot be
    /// overridden by project builders. This hook supplies only project-owned additions for
    /// tokenization/template parsing.
    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec>;

    /// Builder-provided surface.
    ///
    /// WHAT: returns the complete builder surface this builder exposes, including
    /// external platform packages (e.g. `@core/math`) and source-backed package roots
    /// (e.g. `@html`).
    /// WHY: backends own the runtime and package surface, so they must declare
    /// everything the compiler frontend is allowed to see and resolve.
    fn frontend_surface(&self) -> BuilderSurface;
}

/// Build-system entrypoint that owns the selected backend implementation.
///
/// WHAT: stores the backend strategy object used by `build_project`.
/// WHY: callers can swap backends while keeping one orchestration surface.
pub struct ProjectBuilder {
    pub backend: Box<dyn BackendBuilder + Send>,
}

impl ProjectBuilder {
    pub fn new(backend: Box<dyn BackendBuilder + Send>) -> Self {
        Self { backend }
    }
}

pub(crate) struct BuildBootstrap {
    pub(crate) config: Config,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) string_table: StringTable,
    pub(crate) frontend_surface: BuilderSurface,
}

// -------------------------
//  Output Payload
// -------------------------

pub struct OutputFile {
    relative_output_path: PathBuf,
    file_kind: FileKind,
}

pub enum FileKind {
    // This signals for the build system to not create this file.
    // Good for error checking / LSPs etc.
    NotBuilt,

    Wasm(Vec<u8>),
    Bytes(Vec<u8>),
    Js(String), // Either just glue code for web or pure JS backend
    Html(String),
    Directory, // So the build system can create empty folders if needed
}

impl OutputFile {
    /// Create an output artifact with an explicit relative path under the chosen output root.
    pub fn new(relative_output_path: PathBuf, file_kind: FileKind) -> Self {
        Self {
            relative_output_path,
            file_kind,
        }
    }

    /// Relative output path including any desired extension.
    pub fn relative_output_path(&self) -> &Path {
        &self.relative_output_path
    }

    pub(crate) fn file_kind(&self) -> &FileKind {
        &self.file_kind
    }
}

pub struct Project {
    pub output_files: Vec<OutputFile>,
    pub entry_page_rel: Option<PathBuf>,
    /// Builder-owned cleanup contract for manifest tracking and stale artifact removal.
    pub cleanup_policy: CleanupPolicy,
    pub warnings: Vec<CompilerDiagnostic>,
}

/// Result of a successful core build orchestration run.
pub struct BuildResult {
    pub project: Project,
    pub config: Config,
    pub warnings: Vec<CompilerDiagnostic>,
    pub string_table: StringTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    AlwaysWrite,
    SkipUnchanged,
}

/// Options for writing a compiled project to disk.
pub struct WriteOptions {
    pub output_root: PathBuf,
    /// When set, enables stale artifact cleanup via manifest tracking and output root safety
    /// validation. Should be the project's entry directory so safety checks can verify the output
    /// root is in a sensible location relative to the project.
    pub project_entry_dir: Option<PathBuf>,
    pub write_mode: WriteMode,
}

// -------------------------
//  Build Orchestration
// -------------------------

/// Resolve the output root for a directory project based on the build profile.
///
/// The config owns the default folder names. If a config explicitly clears a folder path, outputs
/// fall back to the project root.
pub fn resolve_project_output_root(config: &Config, flags: &[Flag]) -> PathBuf {
    let release_build = flags.contains(&Flag::Release);
    let configured_folder = if release_build {
        &config.release_folder
    } else {
        &config.dev_folder
    };

    if configured_folder.is_absolute() {
        return configured_folder.clone();
    }

    if configured_folder.as_os_str().is_empty() {
        return config.entry_dir.clone();
    }

    config.entry_dir.join(configured_folder)
}

/// Build a Moth project by running path validation, frontend compilation, and backend build.
///
/// This function intentionally does not write output files so callers can decide where artifacts
/// should be emitted.
pub fn build_project(
    project_builder: &ProjectBuilder,
    entry_path: &str,
    flags: &[Flag],
) -> Result<BuildResult, CompilerMessages> {
    let total_start = crate::timing::start_pipeline_timing();
    let mut path_string_table = StringTable::new();
    let path_validation_start = crate::timing::start_pipeline_timing();
    let valid_path = match check_if_valid_path(entry_path, &mut path_string_table) {
        Ok(path) => {
            log_stage_timing("build_project.path_validation", path_validation_start);
            path
        }
        Err(error) => {
            log_stage_timing("build_project.path_validation", path_validation_start);
            log_stage_timing("build_project.total", total_start);
            return Err(CompilerMessages::from_error(error, path_string_table));
        }
    };

    // --------------------------------------------
    //   PERFORM THE CORE COMPILER FRONTEND BUILD
    // --------------------------------------------
    // This discovers all the modules, parses the config,
    // and compiles each module to HIR for backend lowering.
    let bootstrap_start = crate::timing::start_pipeline_timing();
    let BuildBootstrap {
        mut config,
        style_directives,
        mut string_table,
        mut frontend_surface,
    } = match bootstrap_project_build(project_builder, valid_path) {
        Ok(bootstrap) => {
            log_stage_timing("build_project.bootstrap", bootstrap_start);
            bootstrap
        }
        Err(messages) => {
            log_stage_timing("build_project.bootstrap", bootstrap_start);
            log_stage_timing("build_project.total", total_start);
            return Err(messages);
        }
    };

    let compile_frontend_start = crate::timing::start_pipeline_timing();
    let modules = match compile_project_frontend(
        &mut config,
        flags,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    ) {
        Ok(modules) => {
            log_stage_timing(
                "build_project.compile_project_frontend",
                compile_frontend_start,
            );
            modules
        }
        Err(messages) => {
            log_stage_timing(
                "build_project.compile_project_frontend",
                compile_frontend_start,
            );
            log_stage_timing("build_project.total", total_start);
            return Err(messages);
        }
    };
    let mut warnings = collect_frontend_warnings(&modules);

    // --------------------------------------------
    // BUILD PROJECT USING THE APPROPRIATE BUILDER
    // --------------------------------------------

    let backend_start = crate::timing::start_pipeline_timing();
    let project =
        match project_builder
            .backend
            .build_backend(modules, &config, flags, &mut string_table)
        {
            Ok(project) => {
                log_stage_timing("build_project.backend", backend_start);
                project
            }
            Err(mut compiler_messages) => {
                log_stage_timing("build_project.backend", backend_start);
                log_stage_timing("build_project.total", total_start);
                compiler_messages.string_table = string_table;
                return Err(compiler_messages);
            }
        };

    warnings.extend(project.warnings.iter().cloned());

    log_stage_timing("build_project.total", total_start);

    Ok(BuildResult {
        project,
        config,
        warnings,
        string_table,
    })
}

/// Build the shared Stage 0/bootstrap state used by both CLI builds and the dev server.
///
/// WHAT: merges frontend/project directives, loads `config.moth`, and runs backend-specific
/// config validation into one reusable setup step.
/// WHY: directory builds and the dev server must share one bootstrap path so config/output
/// behavior does not drift between "build" and "serve" flows.
pub(crate) fn bootstrap_project_build(
    project_builder: &ProjectBuilder,
    entry_path: PathBuf,
) -> Result<BuildBootstrap, CompilerMessages> {
    let bootstrap_total_start = crate::timing::start_pipeline_timing();

    let config_init_start = crate::timing::start_pipeline_timing();
    let mut config = Config::new(entry_path);
    log_stage_timing("bootstrap.config_init", config_init_start);

    // Seed the build table with the compiler-owned symbols that per-file frontend tables will
    // also need as a stable prefix once file preparation becomes independent.
    let symbol_preseed_start = crate::timing::start_pipeline_timing();
    let preseeded = CompilerSymbolSet::preseeded_table(FILE_MIN_UNIQUE_SYMBOLS_CAPACITY);
    let mut string_table = preseeded.string_table;
    // The bootstrap path only needs the preseeded table today. File-local preparation will keep
    // these typed IDs alongside its local outputs once fixed-symbol IDs are consumed directly.
    let _compiler_symbol_ids = preseeded.compiler_symbol_ids;
    log_stage_timing("bootstrap.symbol_preseed", symbol_preseed_start);

    // Compute the builder's frontend surface once so config loading and frontend compilation
    // see the same set of allowed config keys, external packages, and source-backed packages.
    let frontend_surface_start = crate::timing::start_pipeline_timing();
    let frontend_surface = project_builder.backend.frontend_surface();
    log_stage_timing("bootstrap.frontend_surface", frontend_surface_start);

    let style_directives_start = crate::timing::start_pipeline_timing();
    let frontend_style_directives = project_builder.backend.frontend_style_directives();
    let style_directives = match StyleDirectiveRegistry::merged(&frontend_style_directives) {
        Ok(style_directives) => style_directives,
        Err(error) => {
            log_stage_timing("bootstrap.style_directives", style_directives_start);
            log_stage_timing("bootstrap.total", bootstrap_total_start);
            return Err(CompilerMessages::from_error(error, string_table.clone()));
        }
    };
    log_stage_timing("bootstrap.style_directives", style_directives_start);

    // WHAT: Load and validate project config before compilation begins (Stage 0).
    // WHY: Backends and serving code both depend on the same validated config surface.
    let config_services = ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
    };
    let load_project_config_start = crate::timing::start_pipeline_timing();
    if let Err(messages) = load_project_config(&mut config, &config_services, &mut string_table) {
        log_stage_timing("bootstrap.load_project_config", load_project_config_start);
        log_stage_timing("bootstrap.total", bootstrap_total_start);
        return Err(messages);
    }
    log_stage_timing("bootstrap.load_project_config", load_project_config_start);

    // WHAT: Validate backend-specific config requirements before compilation.
    // WHY: Backends should reject unsupported settings before frontend compilation does work.
    let backend_config_validate_start = crate::timing::start_pipeline_timing();
    if let Err(error) = project_builder
        .backend
        .validate_project_config(&config, &mut string_table)
    {
        log_stage_timing(
            "bootstrap.backend_config_validate",
            backend_config_validate_start,
        );
        log_stage_timing("bootstrap.total", bootstrap_total_start);
        return Err(error.into_messages(string_table.clone()));
    }
    log_stage_timing(
        "bootstrap.backend_config_validate",
        backend_config_validate_start,
    );

    log_stage_timing("bootstrap.total", bootstrap_total_start);

    Ok(BuildBootstrap {
        config,
        style_directives,
        string_table,
        frontend_surface,
    })
}

/// Record a build-system stage timing through the central `timers` substrate.
///
/// WHAT: delegates to `timing::record_started_pipeline_timing`, which stores the
///      observation in the active collection scope and emits the stable
///      `MOTH_BENCH timing` line when the output mode permits.
/// WHY:  `build_project` and `write_project_outputs` use dotted `build_project.*`
///      and `output.*` metric names through the concise `timers` substrate.
///      The start token is zero-sized when `timers` is off, so regular builds
///      do not read clocks for these instrumentation-only measurements.
fn log_stage_timing(metric: &str, start: crate::timing::PipelineTimingStart) {
    crate::timing::record_started_pipeline_timing(metric, start);
}

// -------------------------
//  Output Emission
// -------------------------

/// Write built project artifacts to the provided output root.
///
/// Artifact paths are explicit and must already include any desired extension.
/// When `options.project_entry_dir` is set, stale artifacts from previous builds are cleaned up
/// using a manifest file to track which files the build system owns.
pub fn write_project_outputs(
    project: &Project,
    options: &WriteOptions,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    let write_total_start = crate::timing::start_pipeline_timing();

    // Keep the aggregate output timing visible even when filesystem validation or writes fail.
    let result = write_project_outputs_inner(project, options, string_table);
    log_stage_timing("output.write_total", write_total_start);

    result
}

fn write_project_outputs_inner(
    project: &Project,
    options: &WriteOptions,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    // ---------------------------------------
    //  Prepare cleanup and create output root
    // ---------------------------------------

    let cleanup_state = {
        let prepare_start = crate::timing::start_pipeline_timing();
        let result = prepare_output_cleanup(
            &options.output_root,
            options.project_entry_dir.as_deref(),
            &project.cleanup_policy,
            string_table,
        );
        log_stage_timing("output.prepare_cleanup", prepare_start);
        result?
    };

    {
        let create_root_start = crate::timing::start_pipeline_timing();
        let result = fs::create_dir_all(&options.output_root).map_err(|error| {
            file_error_messages(
                &options.output_root,
                format!(
                    "Failed to create output root '{}': {error}",
                    options.output_root.display()
                ),
                string_table,
            )
        });
        log_stage_timing("output.create_root", create_root_start);
        result?;
    }

    let mut current_managed_artifact_paths: HashSet<PathBuf> = HashSet::new();

    // ---------------------------------------
    //  Emit individual output files
    // ---------------------------------------

    {
        let emit_files_start = crate::timing::start_pipeline_timing();
        let result = emit_project_output_files(
            project,
            options,
            string_table,
            &mut current_managed_artifact_paths,
        );
        log_stage_timing("output.emit_files_total", emit_files_start);
        result?;
    }

    // ---------------------------------------
    //  Finalize cleanup and write manifest
    // ---------------------------------------
    // WHAT: Clean up stale artifacts and write updated manifest when cleanup is enabled
    // WHY: Artifacts from removed pages must not persist in the output folder between builds
    {
        let finalize_start = crate::timing::start_pipeline_timing();
        let result = finalize_output_cleanup(
            &cleanup_state,
            &options.output_root,
            &current_managed_artifact_paths,
            &project.cleanup_policy,
            options.write_mode,
            string_table,
        );
        log_stage_timing("output.finalize_cleanup", finalize_start);
        result?;
    }

    Ok(())
}

fn emit_project_output_files(
    project: &Project,
    options: &WriteOptions,
    string_table: &StringTable,
    current_managed_artifact_paths: &mut HashSet<PathBuf>,
) -> Result<(), CompilerMessages> {
    for output_file in &project.output_files {
        if matches!(output_file.file_kind(), FileKind::NotBuilt) {
            continue;
        }

        let relative_output_path = output_file.relative_output_path();
        validate_relative_output_path(relative_output_path, string_table)?;

        // Track managed paths for the cleanup manifest.
        if !matches!(output_file.file_kind(), FileKind::Directory)
            && (project.cleanup_policy.manages_path(relative_output_path)
                || matches!(output_file.file_kind(), FileKind::Bytes(_)))
        {
            current_managed_artifact_paths.insert(relative_output_path.to_path_buf());
        }

        let destination = options.output_root.join(relative_output_path);

        let emit_file_start = crate::timing::start_pipeline_timing();
        let emit_file_result = match output_file.file_kind() {
            FileKind::NotBuilt => Ok(()),

            FileKind::Directory => fs::create_dir_all(&destination).map_err(|error| {
                file_error_messages(
                    &destination,
                    format!(
                        "Failed to create output directory '{}': {error}",
                        destination.display()
                    ),
                    string_table,
                )
            }),

            FileKind::Js(content) | FileKind::Html(content) => {
                write_string_output(&destination, content, options.write_mode, string_table)
            }

            FileKind::Wasm(bytes) | FileKind::Bytes(bytes) => {
                write_bytes_output(&destination, bytes, options.write_mode, string_table)
            }
        };
        log_stage_timing("output.emit_file", emit_file_start);
        emit_file_result?;
    }

    Ok(())
}

pub fn collect_frontend_warnings(modules: &[Module]) -> Vec<CompilerDiagnostic> {
    let mut warnings = Vec::new();
    for module in modules {
        warnings.extend(module.metadata.warnings.iter().cloned());
    }
    warnings
}

// -------------------------
//  Low-level File Helpers
// -------------------------

fn create_parent_dir_if_needed(
    path: &Path,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    fs::create_dir_all(parent).map_err(|error| {
        file_error_messages(
            parent,
            format!(
                "Failed to create parent directory '{}': {error}",
                parent.display()
            ),
            string_table,
        )
    })
}

fn write_string_output(
    destination: &Path,
    content: &str,
    write_mode: WriteMode,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    write_bytes_output(destination, content.as_bytes(), write_mode, string_table)
}

fn write_bytes_output(
    destination: &Path,
    content: &[u8],
    write_mode: WriteMode,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    create_parent_dir_if_needed(destination, string_table)?;

    if should_skip_unchanged_write(destination, content, write_mode) {
        return Ok(());
    }

    fs::write(destination, content).map_err(|error| {
        file_error_messages(
            destination,
            format!(
                "Failed to write output file '{}': {error}",
                destination.display()
            ),
            string_table,
        )
    })
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
