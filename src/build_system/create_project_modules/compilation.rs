//! Single-file and directory frontend compilation.
//!
//! WHAT: compiles project modules through the frontend pipeline for single-file and directory entries.
//! WHY: separating the two flows keeps each path readable as orchestration over named steps.

use crate::build_system::build::{CompiledModuleArtifact, ModuleSemanticDraft};
use crate::build_system::output::ValidatedDirectoryOutputSettings;
use crate::timed_manual_finish;
use crate::timed_manual_finish_attributed;

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::module_diagnostics::ModuleDiagnostics;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::{
    ProviderImportKind, SourceProviderImport, SourceProviderImportSet,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::ImportShellId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::source_packages::root_file::file_name_is_normal_module_root_file;
use crate::projects::settings::{Config, LANGUAGE_SOURCE_EXTENSION};

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use super::compiled_boundary::{
    BlockedModule, BlockedProvider, CompiledGraphBoundary, CompiledSourcePackage,
    CompletedSourcePackageRegistry, DiagnosedModule, PackageBoundaryId, ProjectFrontendCompilation,
};
use super::frontend_orchestration::{
    FrontendModuleBuildContext, ModuleCompilationOutcome, ModulePreparationContext,
    SourceProviderMaterialisationSet, record_module_input_counters,
};

#[cfg(feature = "timers")]
use super::frontend_orchestration::module_timing_label;
use super::generated_worklist::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::{ModuleArtifactStore, ProviderSlot};
use super::module_identity::ModuleId;
use super::module_inventory;
use super::module_namespace::DirectoryImportResolution;
use super::prepared_module::PreparedModule;
use super::project_module_graph::ProjectModuleGraph;
use super::project_roots;
use super::project_structure_diagnostics::non_utf8_filesystem_name_error;
use super::source_discovery;
use super::source_discovery::{ResolvedDependencyEdge, ResolvedSourcePackageImport};
use super::source_package_discovery::build_source_package_boundary_indexes;
use super::source_tree_index::SourceTreeIndex;

// -------------------------
//  Single-File Compilation
// -------------------------

/// Compile a single `.moth` file as its own module.
pub(crate) fn compile_single_file_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    // 1. Verify standard Moth file extension.
    //
    // A non-UTF-8 extension is an unrepresentable filesystem input. Reject it before
    // any lossy conversion can collapse it into the empty extension.
    let extension_text = match extension.to_str() {
        Some(text) => text,
        None => {
            let error = CompilerError::file_error(
                &config.entry_dir,
                "Entry file extension is not valid UTF-8".to_owned(),
                string_table,
            );
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };

    if extension_text != LANGUAGE_SOURCE_EXTENSION {
        if SourceFileKind::from_extension(extension_text).is_some() {
            let interned_path =
                match InternedPath::try_from_filesystem_path(&config.entry_dir, string_table) {
                    Ok(path) => path,
                    Err(non_utf8) => {
                        return Err(non_utf8_filesystem_name_error(
                            &non_utf8.path,
                            "single-file entry path",
                            string_table,
                        ));
                    }
                };
            let extension = string_table.intern(extension_text);
            let location = SourceLocation {
                scope: interned_path.clone(),
                ..Default::default()
            };
            let diagnostic =
                CompilerDiagnostic::invalid_source_file_entry(interned_path, extension, location);

            return Err(CompilerMessages::from_diagnostic(
                diagnostic,
                string_table.clone(),
            ));
        }

        let err = CompilerError::file_error(
            &config.entry_dir,
            format!(
                "Unsupported file extension for compilation. Moth files use .{LANGUAGE_SOURCE_EXTENSION}"
            ),
            string_table,
        );

        return Err(CompilerMessages::from_error_ref(err, string_table));
    }

    #[cfg(feature = "timers")]
    let total_start = crate::timing::start_pipeline_timing();

    // 2. Resolve canonical entry path.
    #[cfg(feature = "timers")]
    let entry_canonicalize_start = crate::timing::start_pipeline_timing();
    let entry_path = match fs::canonicalize(&config.entry_dir) {
        Ok(path) => path,
        Err(error) => {
            let file_error = CompilerError::file_error(
                &config.entry_dir,
                format!("Failed to resolve entry file path: {error}"),
                string_table,
            );

            timed_manual_finish!(
                "stage0.single_file.entry_canonicalize",
                entry_canonicalize_start,
            );
            timed_manual_finish!("stage0.single_file.total", total_start);
            return Err(CompilerMessages::from_error_ref(file_error, string_table));
        }
    };
    timed_manual_finish!(
        "stage0.single_file.entry_canonicalize",
        entry_canonicalize_start,
    );

    let source_root = entry_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    // 3. Initialize path resolver for imports.
    #[cfg(feature = "timers")]
    let path_resolver_start = crate::timing::start_pipeline_timing();
    // Build one independent source-package boundary index per registered package. The traversal
    // owns direct root discovery and sibling collision checks, so the resolver view is derived
    // from indexed facts and no separate package-root or package-tree scan remains.
    let prepared_source_package_roots = match build_source_package_boundary_indexes(
        &builder_surface.source_packages,
        &builder_surface.source_file_kinds,
        &builder_surface.external_import_providers,
        string_table,
    ) {
        Ok(indexes) => indexes.prepared_source_package_roots(),
        Err(messages) => {
            timed_manual_finish!("stage0.single_file.path_resolver", path_resolver_start);
            timed_manual_finish!("stage0.single_file.total", total_start);
            return Err(messages);
        }
    };

    let entry_file_name = match entry_path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name,
        None => {
            let messages =
                non_utf8_filesystem_name_error(&entry_path, "single-file entry name", string_table);
            timed_manual_finish!("stage0.single_file.path_resolver", path_resolver_start);
            timed_manual_finish!("stage0.single_file.total", total_start);
            return Err(messages);
        }
    };

    let module_roots = if file_name_is_normal_module_root_file(entry_file_name) {
        match SourceTreeIndex::bounded_module_roots_for_single_file(
            &entry_path,
            config,
            &builder_surface.source_packages,
            &builder_surface.source_file_kinds,
            &builder_surface.external_import_providers,
            string_table,
        ) {
            Ok(module_roots) => module_roots,
            Err(messages) => {
                timed_manual_finish!("stage0.single_file.path_resolver", path_resolver_start);
                timed_manual_finish!("stage0.single_file.total", total_start);
                return Err(messages);
            }
        }
    } else {
        crate::compiler_frontend::paths::module_roots::ModuleRootTable::empty()
    };
    let project_path_resolver = match ProjectPathResolver::new_with_module_roots(
        source_root.clone(),
        source_root.clone(),
        prepared_source_package_roots,
        &builder_surface.source_file_kinds,
        module_roots,
    ) {
        Ok(resolver) => resolver,
        Err(error) => {
            timed_manual_finish!("stage0.single_file.path_resolver", path_resolver_start);
            timed_manual_finish!("stage0.single_file.total", total_start);
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };
    timed_manual_finish!("stage0.single_file.path_resolver", path_resolver_start);

    // 4. Discover all transitively reachable files.
    let mut external_imports = source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut builder_surface.binding_packages,
        providers: &builder_surface.external_import_providers,
        cache: &mut builder_surface.external_import_cache,
        resolution_table: &mut builder_surface.external_import_resolution_table,
    };

    // Register the synthetic main-project boundary before its inventory so the human summary can
    // attribute reachable discovery and module compilation as accumulated boundary work.
    #[cfg(feature = "timers")]
    let timing_boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        config.project_name.clone(),
    );
    #[cfg(feature = "timers")]
    let boundary_inventory_start = crate::timing::start_pipeline_timing();
    #[cfg(feature = "timers")]
    let reachable_files_start = crate::timing::start_pipeline_timing();
    let input_files = match source_discovery::collect_reachable_input_files(
        &entry_path,
        &project_path_resolver,
        style_directives,
        &mut external_imports,
        string_table,
    ) {
        Ok(collected) => collected.input_files,
        Err(messages) => {
            timed_manual_finish!("stage0.single_file.reachable_files", reachable_files_start);
            timed_manual_finish_attributed!(
                "build.boundary.inventory",
                boundary_inventory_start,
                None,
                crate::timing::TimingModuleContext::for_boundary(timing_boundary),
            );
            timed_manual_finish!("stage0.single_file.total", total_start);
            return Err(messages);
        }
    };
    timed_manual_finish!("stage0.single_file.reachable_files", reachable_files_start);
    timed_manual_finish_attributed!(
        "build.boundary.inventory",
        boundary_inventory_start,
        None,
        crate::timing::TimingModuleContext::for_boundary(timing_boundary),
    );

    // Share the effective external package registry immutably for the rest of the frontend
    // pipeline so each stage does not need its own deep clone.
    let external_packages = Arc::new(builder_surface.binding_packages.clone());

    // 5. Run the module compilation pipeline with a local string-table delta.
    add_frontend_counter(FrontendCounter::ModuleCompilationSerialCount, 1);

    #[cfg(feature = "timers")]
    let string_table_fork_start = crate::timing::start_pipeline_timing();
    let string_table_fork = string_table.fork_for_module();
    let (local_table, base_len) = string_table_fork.into_parts();
    timed_manual_finish!(
        "stage0.single_file.string_table_fork",
        string_table_fork_start,
    );

    #[cfg(feature = "timers")]
    let compile_module_start = crate::timing::start_pipeline_timing();

    // Record module-input counters and the per-module timing label before preparation so the
    // frontend module total can be attributed even when preparation fails.
    let source_byte_count = record_module_input_counters(&input_files);
    #[cfg(feature = "timers")]
    let module_label_text = module_timing_label(&entry_path, input_files.len(), source_byte_count);
    #[cfg(feature = "timers")]
    let module_label: Option<&str> = Some(&module_label_text);

    // Record the total frontend time for this module (success or error).
    #[cfg(feature = "timers")]
    let module_total_start = crate::timing::start_pipeline_timing();

    // Register the single synthetic module with its portable logical identity and source facts.
    // The empty path is this mode's fixed entry-root logical spelling, matching the origin
    // constructed below.
    #[cfg(feature = "timers")]
    let timing_module_key = crate::timing::register_timing_module(
        timing_boundary,
        0,
        "",
        input_files.len() as u64,
        source_byte_count as u64,
    );
    #[cfg(feature = "timers")]
    let timing_module_context = crate::timing::TimingModuleContext::for_module(timing_module_key);

    // Single-file compilation is a separate synthetic-module mode: it builds one deterministic
    // normal-module origin from the configured project identity, the empty logical module path
    // and `ModuleRootRole::Normal`. The empty path is the entry-root spelling and is always valid,
    // so construction failure is a proven internal invariant surfaced through the existing
    // `CompilerError`/`CompilerMessages` lane rather than a panic. The origin travels through
    // preparation into semantic compilation so the single-file module receives the same canonical
    // identity contract as a directory-discovered module.
    let stable_origin = match StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local(&config.project_name),
        Path::new(""),
        ModuleRootRole::Normal,
    ) {
        Ok(origin) => origin,
        Err(error) => {
            timed_manual_finish_attributed!(
                "frontend.module.total",
                module_total_start,
                module_label,
                timing_module_context,
            );
            timed_manual_finish_attributed!(
                "build.boundary.compile",
                compile_module_start,
                None,
                crate::timing::TimingModuleContext::for_boundary(timing_boundary),
            );
            timed_manual_finish!("stage0.single_file.compile_module", compile_module_start);
            timed_manual_finish!("stage0.single_file.total", total_start);
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };

    // Preparation is provider-independent: it owns no external package registry, import
    // resolution table or builder runtime packages. Construct it before the semantic context so
    // Phase 5 can schedule provider binding between `prepare_module` and `compile_module_semantic`.
    let preparation_context = ModulePreparationContext {
        style_directives,
        project_path_resolver: Some(project_path_resolver.clone()),
    };

    let graph_stable_origin = stable_origin.clone();
    #[cfg(feature = "timers")]
    let prepare_result = preparation_context.prepare_module(
        stable_origin,
        &input_files,
        &entry_path,
        local_table,
        source_byte_count,
        crate::timing::TimingModuleAttribution::new(module_label, timing_module_context),
    );
    #[cfg(not(feature = "timers"))]
    let prepare_result = preparation_context.prepare_module(
        stable_origin,
        &input_files,
        &entry_path,
        local_table,
        source_byte_count,
    );
    let prepared = match prepare_result {
        Ok(prepared) => prepared,
        Err(messages) => {
            timed_manual_finish_attributed!(
                "frontend.module.total",
                module_total_start,
                module_label,
                timing_module_context,
            );
            timed_manual_finish_attributed!(
                "build.boundary.compile",
                compile_module_start,
                None,
                crate::timing::TimingModuleContext::for_boundary(timing_boundary),
            );
            timed_manual_finish!("stage0.single_file.compile_module", compile_module_start);
            timed_manual_finish!("stage0.single_file.total", total_start);
            return Err(messages);
        }
    };

    // Semantic compilation is provider-dependent: it binds retained `PreparedHeaderSyntax`
    // against provider interfaces, then resolves dependencies, builds AST, lowers HIR and runs
    // borrow validation.
    let source_provider_imports = SourceProviderImportSet::default();
    let source_provider_materialisations = SourceProviderMaterialisationSet::default();
    let mut generated_store = BoundaryGeneratedFunctionStore::default();
    let compile_context = FrontendModuleBuildContext {
        config,
        build_profile,
        project_path_resolver: Some(project_path_resolver),
        style_directives,
        external_packages: Arc::clone(&external_packages),
        external_import_resolution_table: &builder_surface.external_import_resolution_table,
        source_provider_imports: &source_provider_imports,
        source_provider_materialisations: &source_provider_materialisations,
        builder_runtime_packages: &builder_surface.builder_runtime_packages,
    };

    #[cfg(feature = "timers")]
    let semantic_total_start = crate::timing::start_pipeline_timing();
    #[cfg(feature = "timers")]
    let semantic_result = compile_context.compile_module_semantic(
        prepared,
        &entry_path,
        crate::timing::TimingModuleAttribution::new(module_label, timing_module_context),
        generated_store.session(),
    );
    #[cfg(not(feature = "timers"))]
    let semantic_result =
        compile_context.compile_module_semantic(prepared, &entry_path, generated_store.session());
    #[cfg(feature = "timers")]
    timed_manual_finish_attributed!(
        "frontend.module.semantic_total",
        semantic_total_start,
        module_label,
        timing_module_context,
    );
    let result = match semantic_result {
        Ok(ModuleCompilationOutcome::Success(compiled)) => *compiled,
        Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)) => {
            timed_manual_finish_attributed!(
                "frontend.module.total",
                module_total_start,
                module_label,
                timing_module_context,
            );
            timed_manual_finish_attributed!(
                "build.boundary.compile",
                compile_module_start,
                None,
                crate::timing::TimingModuleContext::for_boundary(timing_boundary),
            );
            timed_manual_finish!("stage0.single_file.compile_module", compile_module_start);
            timed_manual_finish!("stage0.single_file.total", total_start);

            let mut messages = diagnostics.into_messages();
            let remap = string_table.merge_delta_from(&messages.string_table, base_len);
            if !remap.is_identity() {
                messages.remap_string_ids(&remap);
            }
            let diagnosed = ModuleDiagnostics::from_messages(messages)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            let graph = ProjectModuleGraph::from_normal_roots(vec![(
                graph_stable_origin,
                source_root,
                entry_path,
            )]);
            let module_id = graph
                .entry_modules()
                .first()
                .copied()
                .expect("a single-module graph has one normal entry");
            let mut modules = ModuleArtifactStore::new(1);
            modules
                .mark_diagnosed(module_id)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

            let boundary = CompiledGraphBoundary {
                structure: graph,
                modules,
                generated: generated_store,
                diagnosed: vec![DiagnosedModule {
                    module_id,
                    diagnostics: diagnosed,
                }],
                blocked: Vec::new(),
            };
            return ProjectFrontendCompilation::new(
                boundary
                    .finish()
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
                CompletedSourcePackageRegistry::new(),
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table));
        }
        Err(error) => {
            timed_manual_finish_attributed!(
                "frontend.module.total",
                module_total_start,
                module_label,
                timing_module_context,
            );
            timed_manual_finish_attributed!(
                "build.boundary.compile",
                compile_module_start,
                None,
                crate::timing::TimingModuleContext::for_boundary(timing_boundary),
            );
            timed_manual_finish!("stage0.single_file.compile_module", compile_module_start);
            timed_manual_finish!("stage0.single_file.total", total_start);
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };
    timed_manual_finish_attributed!(
        "frontend.module.total",
        module_total_start,
        module_label,
        timing_module_context,
    );
    timed_manual_finish_attributed!(
        "build.boundary.compile",
        compile_module_start,
        None,
        crate::timing::TimingModuleContext::for_boundary(timing_boundary),
    );
    timed_manual_finish!("stage0.single_file.compile_module", compile_module_start);

    // 6. Merge local results back into the global build context.
    #[cfg(feature = "timers")]
    let merge_delta_start = crate::timing::start_pipeline_timing();
    let remap = string_table.merge_delta_from(&result.string_table, base_len);
    let ModuleSemanticDraft {
        mut module,
        mut generated_worklist_delta,
        string_table: _,
        public_interface,
    } = result;
    if !remap.is_identity() {
        module.remap_string_ids(&remap);
        generated_worklist_delta.remap_string_ids(&remap);
    }
    generated_store
        .publish(generated_worklist_delta)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    timed_manual_finish!("stage0.single_file.merge_delta", merge_delta_start);

    timed_manual_finish!("stage0.single_file.total", total_start);

    let graph =
        ProjectModuleGraph::from_normal_roots(vec![(graph_stable_origin, source_root, entry_path)]);
    let module_id = graph
        .entry_modules()
        .first()
        .copied()
        .expect("a single-module graph has one normal entry");
    let mut modules = ModuleArtifactStore::new(1);
    modules
        .publish_success(
            module_id,
            CompiledModuleArtifact {
                module,
                interface: public_interface,
            },
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let boundary = CompiledGraphBoundary {
        structure: graph,
        modules,
        generated: generated_store,
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    ProjectFrontendCompilation::new(
        boundary
            .finish()
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
        CompletedSourcePackageRegistry::new(),
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}

// -------------------------
//  Directory Compilation
// -------------------------

struct DirectoryModuleTaskResult {
    module_id: ModuleId,
    string_table_base_len: usize,
    outcome: DirectoryModuleTaskOutcome,
}

enum DirectoryModuleTaskOutcome {
    Success(Box<ModuleSemanticDraft>),
    Diagnosed(ModuleDiagnostics),
    Infrastructure(CompilerError),
}

struct DirectoryModuleCompileContext<'a> {
    config: &'a Config,
    build_profile: FrontendBuildProfile,
    project_path_resolver: &'a ProjectPathResolver,
    style_directives: &'a StyleDirectiveRegistry,
    external_packages: &'a Arc<ExternalPackageRegistry>,
    builder_surface: &'a BuilderSurface,
    provider_store: &'a ModuleArtifactStore,
    provider_bindings: &'a [ResolvedDependencyEdge],
    provider_binding_index: &'a FxHashMap<(ModuleId, ImportShellId), usize>,
    source_package_imports: &'a [ResolvedSourcePackageImport],
    source_package_import_index: &'a FxHashMap<(ModuleId, ImportShellId), usize>,
    completed_packages: &'a CompletedSourcePackageRegistry,
    #[cfg(feature = "timers")]
    timing_boundary: crate::timing::TimingBoundaryId,
}

struct SourcePackageModuleInventory {
    import_prefix: String,
    package_identity: StablePackageIdentity,
    root_module_id: ModuleId,
    path_resolver: ProjectPathResolver,
    graph: ProjectModuleGraph,
    module_waves: Vec<Vec<module_inventory::ModuleCompilationJob>>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    source_package_imports: Vec<ResolvedSourcePackageImport>,
    #[cfg(feature = "timers")]
    timing_boundary: crate::timing::TimingBoundaryId,
}

/// Index every resolved provider edge once by consumer module and retained import shell.
///
/// WHAT: gives module binding a direct shell lookup instead of scanning all edges and comparing
///       path components for each retained import.
/// WHY: the shell identity is stamped during header preparation and copied onto the graph edge,
///       so a duplicate key here means the same retained shell resolved twice, which is a proven
///       build invariant violation rather than a user failure.
pub(crate) fn build_provider_binding_index(
    provider_bindings: &[ResolvedDependencyEdge],
) -> Result<FxHashMap<(ModuleId, ImportShellId), usize>, CompilerError> {
    let mut index = FxHashMap::default();
    for (binding_index, binding) in provider_bindings.iter().enumerate() {
        let import_shell_id = binding.provider.import_shell_id;
        let key = (binding.consumer_module_id, import_shell_id);
        if index.insert(key, binding_index).is_some() {
            return Err(CompilerError::compiler_error(format!(
                "ModuleId {} resolved import shell {:?} to more than one provider edge",
                binding.consumer_module_id.index(),
                import_shell_id
            )));
        }
    }

    Ok(index)
}

/// Index every resolved source-package import once by consumer module and retained shell.
pub(crate) fn build_source_package_import_index(
    provider_binding_index: &FxHashMap<(ModuleId, ImportShellId), usize>,
    source_package_imports: &[ResolvedSourcePackageImport],
) -> Result<FxHashMap<(ModuleId, ImportShellId), usize>, CompilerError> {
    let mut index = FxHashMap::default();
    for (import_index, package_import) in source_package_imports.iter().enumerate() {
        let import_shell_id = package_import.provider.import_shell_id;
        let key = (package_import.consumer_module_id, import_shell_id);
        if provider_binding_index.contains_key(&key) {
            return Err(CompilerError::compiler_error(format!(
                "ModuleId {} resolved import shell {:?} to both a provider module and a source package",
                package_import.consumer_module_id.index(),
                import_shell_id
            )));
        }
        if index.insert(key, import_index).is_some() {
            return Err(CompilerError::compiler_error(format!(
                "ModuleId {} resolved import shell {:?} to more than one source-package import",
                package_import.consumer_module_id.index(),
                import_shell_id
            )));
        }
    }

    Ok(index)
}

/// Index every consumer module's direct package dependencies once per boundary.
///
/// WHAT: resolves each resolved source-package import to its dense [`PackageBoundaryId`] and
///       groups the IDs by consumer module, deduplicated and sorted in package order.
/// WHY: readiness checks must walk only the current module's package dependencies. Building
///      the grouped index once per boundary keeps that walk proportional to direct imports.
pub(crate) fn build_module_package_dependency_index(
    source_package_imports: &[ResolvedSourcePackageImport],
    completed_packages: &CompletedSourcePackageRegistry,
) -> Result<FxHashMap<ModuleId, Vec<PackageBoundaryId>>, CompilerError> {
    let mut dependencies: FxHashMap<ModuleId, Vec<PackageBoundaryId>> = FxHashMap::default();

    for package_import in source_package_imports {
        let package_id = completed_packages
            .by_prefix(package_import.import_prefix.as_str())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "ModuleId {} depends on unindexed source package @{}",
                    package_import.consumer_module_id.index(),
                    package_import.import_prefix
                ))
            })?;
        dependencies
            .entry(package_import.consumer_module_id)
            .or_default()
            .push(package_id);
    }

    for package_ids in dependencies.values_mut() {
        package_ids.sort_unstable();
        package_ids.dedup();
    }

    Ok(dependencies)
}

impl<'a> DirectoryModuleCompileContext<'a> {
    /// Build the per-module provider input set by direct retained-shell lookup.
    ///
    /// WHAT: resolves every retained import shell through the boundary indexes built once per
    ///       graph, so binding never scans all edges, all source-package imports or all
    ///       completed packages for each shell.
    fn build_source_provider_imports(
        &self,
        consumer_module_id: ModuleId,
        prepared: &PreparedModule,
    ) -> Result<SourceProviderImportSet<'a>, CompilerError> {
        let mut imports = Vec::new();

        for file_imports in prepared
            .prepared_header_syntax
            .module_symbols
            .file_imports_by_source
            .values()
        {
            for import in file_imports {
                if let Some(binding_index) = self
                    .provider_binding_index
                    .get(&(consumer_module_id, import.provider.import_shell_id))
                {
                    let binding = &self.provider_bindings[*binding_index];
                    let interface = self
                        .provider_store
                        .interface(binding.provider_module_id)?
                        .ok_or_else(|| {
                            CompilerError::compiler_error(format!(
                                "ModuleId {} started semantic binding before provider ModuleId {} published a complete interface",
                                consumer_module_id.index(),
                                binding.provider_module_id.index()
                            ))
                        })?;

                    imports.push(SourceProviderImport {
                        kind: ProviderImportKind::Authored {
                            shell_id: import.provider.import_shell_id,
                        },
                        interface,
                    });
                    continue;
                }

                let Some(package_index) = self
                    .source_package_import_index
                    .get(&(consumer_module_id, import.provider.import_shell_id))
                else {
                    continue;
                };
                let package_import = &self.source_package_imports[*package_index];
                let package_id = self
                    .completed_packages
                    .by_prefix(package_import.import_prefix.as_str())
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "ModuleId {} started semantic binding before source package @{} completed",
                            consumer_module_id.index(),
                            package_import.import_prefix
                        ))
                    })?;
                let completed_package = self.completed_packages.package(package_id)?;

                imports.push(SourceProviderImport {
                    kind: ProviderImportKind::Authored {
                        shell_id: import.provider.import_shell_id,
                    },
                    interface: completed_package.root_interface()?,
                });
            }
        }

        // Builder source-backed packages are implicitly available only to modules that actually
        // contain a `.mtf` semantic source. The package capability is supplied by the active
        // builder surface; generic orchestration must not infer it from a package-name list.
        if prepared.contains_moth_template {
            let implicit_provider_imports: Vec<SourceProviderImport<'a>> = self
                .completed_packages
                .iter()
                .filter(|package| {
                    self.builder_surface
                        .implicit_template_scope_source_packages
                        .contains(package.import_prefix())
                })
                .map(|package| {
                    let interface = package.root_interface()?;
                    Ok(SourceProviderImport {
                        kind: ProviderImportKind::ImplicitTemplate {
                            package_prefix: package.import_prefix(),
                        },
                        interface,
                    })
                })
                .collect::<Result<_, CompilerError>>()?;

            imports.extend(implicit_provider_imports);
        }

        SourceProviderImportSet::new(imports)
    }

    fn compile(
        &self,
        job: module_inventory::ModuleCompilationJob,
        generated_worklist: super::generated_worklist::GeneratedFunctionWorklist<'_>,
    ) -> DirectoryModuleTaskResult {
        let module_inventory::ModuleCompilationJob {
            module_id,
            entry_point,
            string_table_base_len: base_len,
            prepared,
            ..
        } = job;

        // Record module-input counters and the per-module timing label before preparation so
        // the frontend module total can be attributed even when preparation fails.
        #[cfg(feature = "timers")]
        let module_label_text = module_timing_label(
            &entry_point,
            prepared.source_file_count,
            prepared.source_byte_count,
        );
        #[cfg(feature = "timers")]
        let module_label: Option<&str> = Some(&module_label_text);

        // Record the total frontend time for this module (success or error).
        #[cfg(feature = "timers")]
        let module_total_start = crate::timing::start_pipeline_timing();

        // The dense graph `ModuleId` is the module key inside this boundary, so attribution
        // stays deterministic and independent of worker completion order.
        #[cfg(feature = "timers")]
        let module_context =
            crate::timing::TimingModuleContext::for_module(crate::timing::TimingModuleKey {
                boundary: self.timing_boundary,
                module_index: job.module_id.index() as u32,
            });

        // Semantic compilation is provider-dependent: it binds retained `PreparedHeaderSyntax`
        // against provider interfaces, then resolves dependencies, builds AST, lowers HIR and
        // runs borrow validation.
        let source_provider_imports = match self.build_source_provider_imports(module_id, &prepared)
        {
            Ok(imports) => imports,
            Err(error) => {
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome: DirectoryModuleTaskOutcome::Infrastructure(error),
                };
            }
        };
        let compile_context = FrontendModuleBuildContext {
            config: self.config,
            build_profile: self.build_profile,
            project_path_resolver: Some(self.project_path_resolver.clone()),
            style_directives: self.style_directives,
            external_packages: Arc::clone(self.external_packages),
            external_import_resolution_table: &self
                .builder_surface
                .external_import_resolution_table,
            source_provider_imports: &source_provider_imports,
            source_provider_materialisations: &SourceProviderMaterialisationSet::new(
                self.provider_store,
                self.completed_packages,
            ),
            builder_runtime_packages: &self.builder_surface.builder_runtime_packages,
        };

        // The typed semantic boundary already classified user diagnostics from infrastructure
        // failures, so the task outcome carries the retained `ModuleDiagnostics` unchanged.
        #[cfg(feature = "timers")]
        let semantic_total_start = crate::timing::start_pipeline_timing();
        #[cfg(feature = "timers")]
        let semantic_result = compile_context.compile_module_semantic(
            prepared,
            &entry_point,
            crate::timing::TimingModuleAttribution::new(module_label, module_context),
            generated_worklist,
        );
        #[cfg(not(feature = "timers"))]
        let semantic_result =
            compile_context.compile_module_semantic(prepared, &entry_point, generated_worklist);
        #[cfg(feature = "timers")]
        timed_manual_finish_attributed!(
            "frontend.module.semantic_total",
            semantic_total_start,
            module_label,
            module_context,
        );
        let outcome = match semantic_result {
            Ok(ModuleCompilationOutcome::Success(compiled)) => {
                DirectoryModuleTaskOutcome::Success(compiled)
            }
            Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)) => {
                DirectoryModuleTaskOutcome::Diagnosed(diagnostics)
            }
            Err(error) => DirectoryModuleTaskOutcome::Infrastructure(error),
        };
        timed_manual_finish_attributed!(
            "frontend.module.total",
            module_total_start,
            module_label,
            module_context,
        );

        DirectoryModuleTaskResult {
            module_id,
            string_table_base_len: base_len,
            outcome,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_module_waves(
    config: &Config,
    build_profile: FrontendBuildProfile,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_packages: &Arc<ExternalPackageRegistry>,
    builder_surface: &BuilderSurface,
    graph: ProjectModuleGraph,
    module_waves: Vec<Vec<module_inventory::ModuleCompilationJob>>,
    provider_bindings: &[ResolvedDependencyEdge],
    source_package_imports: &[ResolvedSourcePackageImport],
    completed_packages: &CompletedSourcePackageRegistry,
    string_table: &mut StringTable,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<CompiledGraphBoundary, CompilerMessages> {
    let mut provider_store = ModuleArtifactStore::new(graph.nodes().len());
    let mut generated_store = BoundaryGeneratedFunctionStore::default();

    // Register every module in deterministic wave order before semantic work starts. The dense
    // graph `ModuleId` is the module key, so registration order never affects attribution.
    #[cfg(feature = "timers")]
    for wave in &module_waves {
        for job in wave {
            let module_origin = job
                .prepared
                .source_module_origins
                .origin_for(job.prepared.active_root_file_id)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
                .ok_or_else(|| {
                    CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(format!(
                            "module timing registration: active root file id {} has no module origin",
                            job.prepared.active_root_file_id.0
                        )),
                        string_table,
                    )
                })?;
            crate::timing::register_timing_module(
                timing_boundary,
                job.module_id.index() as u32,
                module_origin.logical_module_path(),
                job.prepared.source_file_count as u64,
                job.prepared.source_byte_count as u64,
            );
        }
    }

    // One direct lookup index per boundary so module binding never scans every provider edge,
    // source-package import or completed package for each retained import shell.
    let provider_binding_index = build_provider_binding_index(provider_bindings)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let source_package_import_index =
        build_source_package_import_index(&provider_binding_index, source_package_imports)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // Index each consumer module's direct package dependencies once per boundary so readiness
    // walks only the packages that module actually imports and never filters the full import
    // vector for every job.
    let module_package_dependencies =
        build_module_package_dependency_index(source_package_imports, completed_packages)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let mut diagnosed = Vec::new();
    let mut blocked = Vec::new();

    for wave in module_waves {
        add_frontend_counter(FrontendCounter::ModuleCompilationSerialCount, wave.len());
        let mut ready = Vec::new();
        for job in wave {
            let mut blocked_provider = None;
            for provider_id in graph
                .dependency_providers(job.module_id)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
            {
                match provider_store
                    .slot(*provider_id)
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
                {
                    ProviderSlot::Successful(_) => {}
                    ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                        blocked_provider = Some(BlockedProvider::Module(*provider_id));
                        break;
                    }
                    ProviderSlot::Unavailable => {
                        let error = CompilerError::compiler_error(format!(
                            "ModuleId {} became ready before provider ModuleId {} completed",
                            job.module_id.index(),
                            provider_id.index()
                        ));
                        return Err(CompilerMessages::from_error_ref(error, string_table));
                    }
                }
            }

            if blocked_provider.is_none()
                && let Some(package_ids) = module_package_dependencies.get(&job.module_id)
            {
                for package_id in package_ids {
                    let package = completed_packages
                        .package(*package_id)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

                    match package
                        .root_slot()
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
                    {
                        ProviderSlot::Successful(_) => {}
                        ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                            blocked_provider = Some(BlockedProvider::SourcePackage(
                                package.package_identity.clone(),
                            ));
                            break;
                        }
                        ProviderSlot::Unavailable => {
                            let error = CompilerError::compiler_error(format!(
                                "ModuleId {} became ready before source package @{} completed its facade",
                                job.module_id.index(),
                                package.import_prefix()
                            ));
                            return Err(CompilerMessages::from_error_ref(error, string_table));
                        }
                    }
                }
            }

            if let Some(required_provider) = blocked_provider {
                provider_store
                    .mark_blocked(job.module_id)
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                blocked.push(BlockedModule {
                    module_id: job.module_id,
                    required_provider,
                });
            } else {
                ready.push(job);
            }
        }

        ready.sort_by_key(|job| job.module_id.index());
        for job in ready {
            // The boundary worklist publishes each successful module transaction before the
            // next ModuleId so duplicate requests in one ready wave materialise exactly once.
            // File preparation remains parallel inside each module. Module-wave parallelism can
            // return once worklist sessions can commit deterministic deltas concurrently.
            let outcome = {
                let compile_context = DirectoryModuleCompileContext {
                    config,
                    build_profile,
                    project_path_resolver,
                    style_directives,
                    external_packages,
                    builder_surface,
                    provider_store: &provider_store,
                    provider_bindings,
                    provider_binding_index: &provider_binding_index,
                    source_package_imports,
                    source_package_import_index: &source_package_import_index,
                    completed_packages,
                    #[cfg(feature = "timers")]
                    timing_boundary,
                };
                compile_context.compile(job, generated_store.session())
            };
            match outcome.outcome {
                DirectoryModuleTaskOutcome::Success(compiled) => {
                    let compiled = *compiled;
                    let remap = string_table
                        .merge_delta_from(&compiled.string_table, outcome.string_table_base_len);
                    let ModuleSemanticDraft {
                        mut module,
                        mut generated_worklist_delta,
                        string_table: _,
                        public_interface,
                    } = compiled;
                    if !remap.is_identity() {
                        module.remap_string_ids(&remap);
                        generated_worklist_delta.remap_string_ids(&remap);
                    }
                    let artifact = CompiledModuleArtifact {
                        module,
                        interface: public_interface,
                    };
                    provider_store
                        .publish_success(outcome.module_id, artifact)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                    generated_store
                        .publish(generated_worklist_delta)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                }
                DirectoryModuleTaskOutcome::Diagnosed(diagnostics) => {
                    provider_store
                        .mark_diagnosed(outcome.module_id)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                    let mut messages = diagnostics.into_messages();
                    let remap = string_table
                        .merge_delta_from(&messages.string_table, outcome.string_table_base_len);
                    if !remap.is_identity() {
                        messages.remap_string_ids(&remap);
                    }
                    let diagnostics = ModuleDiagnostics::from_messages(messages)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                    diagnosed.push(DiagnosedModule {
                        module_id: outcome.module_id,
                        diagnostics,
                    });
                }
                DirectoryModuleTaskOutcome::Infrastructure(error) => {
                    return Err(CompilerMessages::from_error_ref(error, string_table));
                }
            }
        }
    }

    let diagnosed_provider_exists = !diagnosed.is_empty()
        || completed_packages
            .iter()
            .any(|package| !package.boundary.diagnosed.is_empty());
    if !blocked.is_empty() && !diagnosed_provider_exists {
        return Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(format!(
                "Graph retained {} blocked modules without a diagnosed provider",
                blocked.len()
            )),
            string_table,
        ));
    }

    let boundary = CompiledGraphBoundary {
        structure: graph,
        modules: provider_store,
        generated: generated_store,
        diagnosed,
        blocked,
    };
    boundary
        .finish()
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}

fn order_source_package_inventories(
    inventories: Vec<SourcePackageModuleInventory>,
    string_table: &StringTable,
) -> Result<Vec<SourcePackageModuleInventory>, CompilerMessages> {
    let import_prefixes = inventories
        .iter()
        .map(|inventory| inventory.import_prefix.clone())
        .collect::<Vec<_>>();
    let dependency_prefixes = inventories
        .iter()
        .map(|inventory| {
            inventory
                .source_package_imports
                .iter()
                .map(|dependency| dependency.import_prefix.clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let order = order_packages_by_dependency(&import_prefixes, &dependency_prefixes)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let mut remaining = inventories.into_iter().map(Some).collect::<Vec<_>>();
    let ordered = order
        .into_iter()
        .map(|index| {
            remaining[index]
                .take()
                .expect("each package index is selected exactly once")
        })
        .collect();

    Ok(ordered)
}

/// Order packages by their direct provider dependencies using one deterministic dense schedule.
///
/// WHAT: builds the package dependency graph once over dense indices, then runs a Kahn schedule
///       whose ready set leaves a min-heap in input order. The returned indices are the package
///       positions in dependency-first order.
/// WHY: package readiness and publication need one deterministic order without rebuilding
///      dependency sets per pass; the dense schedule also detects unknown providers and cycles.
pub(crate) fn order_packages_by_dependency(
    import_prefixes: &[String],
    dependency_prefixes: &[Vec<String>],
) -> Result<Vec<usize>, CompilerError> {
    let package_count = import_prefixes.len();
    if dependency_prefixes.len() != package_count {
        return Err(CompilerError::compiler_error(format!(
            "package dependency schedule received {} packages but {} dependency rows",
            package_count,
            dependency_prefixes.len()
        )));
    }

    let mut index_by_prefix: FxHashMap<&str, usize> = FxHashMap::default();
    for (index, prefix) in import_prefixes.iter().enumerate() {
        if index_by_prefix.insert(prefix.as_str(), index).is_some() {
            return Err(CompilerError::compiler_error(format!(
                "source package @{} appears more than once in the package inventory",
                prefix
            )));
        }
    }

    // Build the deterministic dense dependency graph once: package -> direct consumers and the
    // indegree of each package over its provider edges.
    let mut consumer_lists: Vec<Vec<usize>> = vec![Vec::new(); package_count];
    let mut indegree: Vec<usize> = vec![0; package_count];
    for (index, dependencies) in dependency_prefixes.iter().enumerate() {
        let mut seen_providers: FxHashSet<usize> = FxHashSet::default();
        for dependency in dependencies {
            let provider_index = index_by_prefix
                .get(dependency.as_str())
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Source package @{} depends on unindexed source package @{}",
                        import_prefixes[index], dependency
                    ))
                })?;
            if seen_providers.insert(provider_index) {
                consumer_lists[provider_index].push(index);
                indegree[index] += 1;
            }
        }
    }

    // Deterministic Kahn schedule: ready packages leave the heap in lexicographic prefix order,
    // and consumer lists are visited in the same deterministic discovery order.
    let mut ready: std::collections::BinaryHeap<std::cmp::Reverse<(&str, usize)>> =
        std::collections::BinaryHeap::new();
    for (index, package_indegree) in indegree.iter().enumerate() {
        if *package_indegree == 0 {
            ready.push(std::cmp::Reverse((import_prefixes[index].as_str(), index)));
        }
    }

    let mut ordered = Vec::with_capacity(package_count);
    while let Some(std::cmp::Reverse((_, index))) = ready.pop() {
        ordered.push(index);
        for consumer_index in &consumer_lists[index] {
            indegree[*consumer_index] -= 1;
            if indegree[*consumer_index] == 0 {
                ready.push(std::cmp::Reverse((
                    import_prefixes[*consumer_index].as_str(),
                    *consumer_index,
                )));
            }
        }
    }

    if ordered.len() != package_count {
        let blocked = (0..package_count)
            .filter(|index| !ordered.contains(index))
            .map(|index| format!("@{}", import_prefixes[index]))
            .collect::<Vec<_>>();
        return Err(CompilerError::compiler_error(format!(
            "Source package dependency cycle detected; no package is ready among {}",
            blocked.join(", ")
        )));
    }

    Ok(ordered)
}

/// Discover all entry modules in a directory project and compile each one.
pub(crate) fn compile_directory_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    validated_output_settings: Option<&ValidatedDirectoryOutputSettings>,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    string_table: &mut StringTable,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    #[cfg(feature = "timers")]
    let total_start = crate::timing::start_pipeline_timing();

    // 1. Setup path resolution based on config settings.
    #[cfg(feature = "timers")]
    let path_resolver_start = crate::timing::start_pipeline_timing();
    let mut project_setup = match project_roots::build_project_path_resolver_with_index(
        config,
        validated_output_settings,
        &builder_surface.source_packages,
        &builder_surface.source_file_kinds,
        &builder_surface.external_import_providers,
        &builder_surface.binding_packages,
        string_table,
    ) {
        Ok(resolver) => resolver,
        Err(error) => {
            timed_manual_finish!("stage0.directory.path_resolver", path_resolver_start);
            timed_manual_finish!("stage0.directory.total", total_start);
            return Err(error);
        }
    };
    timed_manual_finish!("stage0.directory.path_resolver", path_resolver_start);
    let project_path_resolver = project_setup.resolver;

    // 2. Build every source-package inventory and the project inventory before semantic
    // compilation. Provider-backed discovery may extend the binding registry, so all boundaries
    // finish that serial mutation phase before the registry becomes the immutable frontend view.
    let mut external_imports = source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut builder_surface.binding_packages,
        providers: &builder_surface.external_import_providers,
        cache: &mut builder_surface.external_import_cache,
        resolution_table: &mut builder_surface.external_import_resolution_table,
    };

    #[cfg(feature = "timers")]
    let module_inventory_start = crate::timing::start_pipeline_timing();
    let mut source_package_inventories = Vec::new();
    for (import_prefix, package_index) in project_setup
        .module_namespace_set
        .source_package_boundaries()
    {
        // Register the package boundary before its inventory so inventory and compile
        // observations share one dense id for the human boundary total.
        #[cfg(feature = "timers")]
        let timing_boundary = crate::timing::register_timing_boundary(
            crate::timing::TimingBoundaryKind::SourcePackage,
            format!("@{import_prefix}"),
        );
        let mut package_graph = ProjectModuleGraph::from_source_tree_index(package_index);
        let package_path_resolver = project_path_resolver.for_source_package_boundary(
            package_index.entry_root().to_path_buf(),
            package_index
                .module_identities()
                .derive_compilation_root_table(),
        );
        let package_resolution = DirectoryImportResolution::package(
            &project_setup.module_namespace_set,
            import_prefix,
            package_index,
        );
        #[cfg(feature = "timers")]
        let package_inventory_start = crate::timing::start_pipeline_timing();
        let package_waves = match module_inventory::discover_all_modules_in_package(
            config,
            &package_path_resolver,
            &mut package_graph,
            style_directives,
            &mut external_imports,
            package_resolution,
            string_table,
        ) {
            Ok(module_waves) => module_waves,
            Err(messages) => {
                timed_manual_finish!("stage0.directory.module_inventory", module_inventory_start);
                timed_manual_finish_attributed!(
                    "build.boundary.inventory",
                    package_inventory_start,
                    None,
                    crate::timing::TimingModuleContext::for_boundary(timing_boundary),
                );
                timed_manual_finish!("stage0.directory.total", total_start);
                return Err(messages);
            }
        };
        timed_manual_finish_attributed!(
            "build.boundary.inventory",
            package_inventory_start,
            None,
            crate::timing::TimingModuleContext::for_boundary(timing_boundary),
        );
        let root_module_id = package_index
            .module_identities()
            .module_id_for_directory(package_index.entry_root())
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "Source package @{import_prefix} has no module rooted at its indexed entry root"
                    )),
                    string_table,
                )
            })?;
        let (module_waves, provider_bindings, source_package_imports) = package_waves.into_parts();

        source_package_inventories.push(SourcePackageModuleInventory {
            import_prefix: import_prefix.to_owned(),
            package_identity: package_index.stable_package_identity().clone(),
            root_module_id,
            path_resolver: package_path_resolver,
            graph: package_graph,
            module_waves,
            provider_bindings,
            source_package_imports,
            #[cfg(feature = "timers")]
            timing_boundary,
        });
    }

    // Register the main-project boundary before its inventory so its accumulated total is
    // attributed separately from every source package.
    #[cfg(feature = "timers")]
    let project_timing_boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        config.project_name.clone(),
    );

    let directory_import_resolution = DirectoryImportResolution::project(
        &project_setup.module_namespace_set,
        &project_setup.source_tree_index,
    );
    #[cfg(feature = "timers")]
    let project_inventory_start = crate::timing::start_pipeline_timing();
    let module_waves = match module_inventory::discover_all_modules_in_project(
        config,
        &project_path_resolver,
        &mut project_setup.project_module_graph,
        style_directives,
        &mut external_imports,
        directory_import_resolution,
        string_table,
    ) {
        Ok(module_waves) => module_waves,
        Err(messages) => {
            timed_manual_finish!("stage0.directory.module_inventory", module_inventory_start);
            timed_manual_finish_attributed!(
                "build.boundary.inventory",
                project_inventory_start,
                None,
                crate::timing::TimingModuleContext::for_boundary(project_timing_boundary),
            );
            timed_manual_finish!("stage0.directory.total", total_start);
            return Err(messages);
        }
    };
    timed_manual_finish!("stage0.directory.module_inventory", module_inventory_start);
    timed_manual_finish_attributed!(
        "build.boundary.inventory",
        project_inventory_start,
        None,
        crate::timing::TimingModuleContext::for_boundary(project_timing_boundary),
    );

    let source_package_inventories =
        order_source_package_inventories(source_package_inventories, string_table)?;

    // Share the effective external package registry immutably across all boundary compilations;
    // directory modules may compile in parallel and can safely read the same Arc.
    let external_packages = Arc::new(builder_surface.binding_packages.clone());

    // 3. Compile source packages in package-dependency order, then compile the project against
    // their immutable facade interfaces. Each boundary owns independent dense IDs, graphs and
    // provider stores; only the stable public interface crosses into a consuming boundary.
    #[cfg(feature = "timers")]
    let module_compile_batch_start = crate::timing::start_pipeline_timing();
    let mut completed_source_packages = CompletedSourcePackageRegistry::new();
    for inventory in source_package_inventories {
        let SourcePackageModuleInventory {
            package_identity,
            root_module_id,
            path_resolver,
            graph,
            module_waves,
            provider_bindings,
            source_package_imports,
            import_prefix: _,
            #[cfg(feature = "timers")]
            timing_boundary,
        } = inventory;
        #[cfg(feature = "timers")]
        let package_compile_start = crate::timing::start_pipeline_timing();
        let compiled = compile_module_waves(
            config,
            build_profile,
            &path_resolver,
            style_directives,
            &external_packages,
            builder_surface,
            graph,
            module_waves,
            &provider_bindings,
            &source_package_imports,
            &completed_source_packages,
            string_table,
            #[cfg(feature = "timers")]
            timing_boundary,
        );
        timed_manual_finish_attributed!(
            "build.boundary.compile",
            package_compile_start,
            None,
            crate::timing::TimingModuleContext::for_boundary(timing_boundary),
        );
        let boundary = compiled?;
        let dependency_prefixes = source_package_imports
            .iter()
            .map(|dependency| dependency.import_prefix.clone())
            .collect::<Vec<_>>();
        completed_source_packages
            .publish(
                CompiledSourcePackage {
                    package_identity,
                    root_module_id,
                    boundary,
                },
                &dependency_prefixes,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    }

    completed_source_packages
        .validate_dependency_edges()
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let (project_module_waves, project_provider_bindings, project_source_package_imports) =
        module_waves.into_parts();
    #[cfg(feature = "timers")]
    let project_compile_start = crate::timing::start_pipeline_timing();
    let compiled_project = compile_module_waves(
        config,
        build_profile,
        &project_path_resolver,
        style_directives,
        &external_packages,
        builder_surface,
        project_setup.project_module_graph,
        project_module_waves,
        &project_provider_bindings,
        &project_source_package_imports,
        &completed_source_packages,
        string_table,
        #[cfg(feature = "timers")]
        project_timing_boundary,
    );
    timed_manual_finish_attributed!(
        "build.boundary.compile",
        project_compile_start,
        None,
        crate::timing::TimingModuleContext::for_boundary(project_timing_boundary),
    );
    let project_boundary = compiled_project?;
    timed_manual_finish!(
        "stage0.directory.module_compile_batch",
        module_compile_batch_start,
    );

    timed_manual_finish!("stage0.directory.total", total_start);

    ProjectFrontendCompilation::new(project_boundary, completed_source_packages)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}
