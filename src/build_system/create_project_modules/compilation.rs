//! Single-file and directory frontend compilation.
//!
//! WHAT: compiles project modules through the frontend pipeline for single-file and directory entries.
//! WHY: separating the two flows keeps each path readable as orchestration over named steps.
use crate::{timing_scope, timing_scope_attributed};

use crate::build_system::output::ValidatedDirectoryOutputSettings;
#[cfg(feature = "boracle")]
use crate::compiler_frontend::module_compilation::BoracleModuleInput;
#[cfg(feature = "boracle")]
use crate::compiler_frontend::module_compilation::compile_module_for_boracle;
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, GeneratedFunctionDelta, KnownGeneratedFunctions,
    ModuleCompilationContext, ModuleCompilationOutcome, ModuleSemanticResult,
    ProviderMaterialisationRegistry, compile_module,
};

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::build_config::{
    BuildConfigContractFact, BuildConfigInputSet, BuildConfigResolutionError,
    BuildConfigResolutionIndex, BuilderConfigGlobalSet, ResolvedBuildConfigMap,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidDependencyClauseReason, ModuleDiagnostics,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::project_globals::{
    ProjectGlobalsInterface, is_project_globals_dependency,
};
use crate::compiler_frontend::public_interface::{
    ProviderDependencyKind, SourceProviderDependency, SourceProviderDependencySet,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source_packages::root_file::file_name_is_normal_module_root_file;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::projects::settings::{Config, LANGUAGE_SOURCE_EXTENSION};

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use super::FrontendCompilationMode;
use super::compiled_boundary::{
    BlockedModule, BlockedProvider, CompiledGraphBoundary, CompiledSourcePackage,
    CompletedSourcePackageRegistry, DiagnosedModule, PackageBoundaryId, ProjectFrontendCompilation,
};
use super::config_boundary;
use super::module_preparation::{ModulePreparationContext, record_module_input_counters};

use super::file_reference_resolution::{SingleFileReferenceOutcome, SingleFileResolvedReference};
use super::generated_store::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::{ModuleArtifactStore, ProviderSlot};
use super::module_identity::ModuleId;
use super::module_inventory;
use super::module_namespace::DirectoryDependencyResolution;
use super::prepared_module::PreparedModule;
use super::project_module_graph::ProjectModuleGraph;
use super::project_roots;
use super::project_structure_diagnostics::non_utf8_filesystem_name_error;
use super::resource_inputs::ResourceInputRegistry;
use super::source_discovery;
use super::source_discovery::{ResolvedDependencyEdge, ResolvedSourcePackageDependency};
use super::source_package_discovery::build_source_package_boundary_indexes;
use super::source_tree_index::SourceTreeIndex;

#[cfg(test)]
#[path = "../tests/compilation_tests.rs"]
mod tests;

/// Inputs for one atomic module, generated and resource-association publication.
pub(super) struct ModuleBoundaryPublication<'a> {
    pub modules: &'a mut ModuleArtifactStore,
    pub generated: &'a mut BoundaryGeneratedFunctionStore,
    pub materialisations: &'a mut ProviderMaterialisationRegistry,
    pub resource_inputs: &'a mut ResourceInputRegistry,
    pub module_id: ModuleId,
    pub expected_origin: &'a StableModuleOriginIdentity,
    pub artifact: CompiledModuleArtifact,
    pub generated_delta: GeneratedFunctionDelta,
    pub resource_source_associations: Vec<ResourceSourceAssociation>,
}

/// Publish one successful module, its generated sidecars and its resource-source associations as
/// one boundary transaction.
///
/// WHAT: runs every fallible check before reserving or committing any registry, then executes only
///       infallible reservations and commits.
/// WHY: separating collision detection from the successful publication path keeps a rejected
///      materialisation from partially publishing module, generated or resource state.
pub(super) fn publish_module_and_generated(
    publication: ModuleBoundaryPublication<'_>,
) -> Result<(), CompilerError> {
    let ModuleBoundaryPublication {
        modules,
        generated,
        materialisations,
        resource_inputs,
        module_id,
        expected_origin,
        artifact,
        generated_delta,
        resource_source_associations,
    } = publication;
    // Every fallible check runs before anything mutates. The reservations and commits that follow
    // cannot fail, so a rejected publication leaves module, generated and resource registries
    // unchanged.
    let module_publication = modules.preflight_success(module_id, &artifact, expected_origin)?;
    let generated_publication = generated.preflight(&generated_delta)?;
    let resource_publication =
        resource_inputs.preflight_resource_source_associations(&resource_source_associations)?;
    publish_materialisation_templates(materialisations, &artifact)?;

    modules.reserve_success_commit(&module_publication);
    generated.reserve_commit(&generated_publication);
    resource_inputs.reserve_resource_source_associations(&resource_publication);
    modules.commit_success(module_publication, artifact);
    generated.commit(generated_publication, generated_delta);
    resource_inputs.commit_resource_source_associations(resource_publication);
    Ok(())
}

/// Add one newly published module's generic templates to the boundary materialisation registry.
///
/// WHY: later modules in this boundary materialise concrete generics from their declaring module's
///      validated templates. The registry is the compiler's immutable lookup for that; the store's
///      own declaration index stays behind for publication provenance and duplicate detection.
fn publish_materialisation_templates(
    materialisations: &mut ProviderMaterialisationRegistry,
    artifact: &CompiledModuleArtifact,
) -> Result<(), CompilerError> {
    let Some(context) = artifact.module.metadata.materialisation_context.as_ref() else {
        return Ok(());
    };

    materialisations.publish_context(context)
}

/// Seed a boundary registry with every generic template completed source packages already expose.
///
/// WHY: a project module may instantiate a generic declared in a package it depends on, and those
///      packages finished before this boundary started.
fn seed_completed_package_materialisations(
    completed_packages: &CompletedSourcePackageRegistry,
) -> Result<ProviderMaterialisationRegistry, CompilerError> {
    let mut registry = ProviderMaterialisationRegistry::default();
    let mut rows = Vec::new();
    for (identity, location) in completed_packages.materialisation_locations() {
        let package = completed_packages.package(location.package_id)?;
        let context = package
            .boundary
            .modules
            .materialisation_context_at(location.location)?;
        rows.push((
            identity.clone(),
            Arc::clone(context),
            location.location.template_index,
        ));
    }
    for (identity, context, template_index) in &rows {
        registry.preflight_publish(identity, context, *template_index)?;
    }
    for (identity, context, template_index) in rows {
        registry.publish(identity, context, template_index)?;
    }
    Ok(registry)
}

// -------------------------
//  Single-File Compilation
// -------------------------

#[allow(dead_code)]
/// Compile a single `.moth` file as its own module.
pub(crate) fn compile_single_file_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    compile_single_file_frontend_with_inputs(
        config,
        build_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
        &BuildConfigInputSet::new(),
        FrontendCompilationMode::Canonical,
    )
}

/// Compile one source file with an explicit command-owned build-config input set.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_single_file_frontend_with_inputs(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
    build_config_inputs: &BuildConfigInputSet,
    mode: FrontendCompilationMode,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    match compile_single_file_frontend_with_target(
        config,
        build_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
        build_config_inputs,
        mode,
        SingleFileFrontendTarget::Normal,
    )? {
        SingleFileFrontendResult::Project(compilation) => Ok(*compilation),
        #[cfg(feature = "boracle")]
        SingleFileFrontendResult::Boracle(_) => Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "normal single-file compilation unexpectedly returned a Boracle payload",
            ),
            string_table,
        )),
    }
}

#[cfg(feature = "boracle")]
pub(crate) fn compile_single_file_boracle_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
) -> Result<BoracleModuleInput, CompilerMessages> {
    match compile_single_file_frontend_with_target(
        config,
        build_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
        &BuildConfigInputSet::new(),
        FrontendCompilationMode::Canonical,
        SingleFileFrontendTarget::Boracle,
    )? {
        SingleFileFrontendResult::Boracle(input) => Ok(*input),
        SingleFileFrontendResult::Project(_) => Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "Boracle source compilation unexpectedly returned a complete project payload",
            ),
            string_table,
        )),
    }
}

enum SingleFileFrontendResult {
    Project(Box<ProjectFrontendCompilation>),
    #[cfg(feature = "boracle")]
    Boracle(Box<BoracleModuleInput>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingleFileFrontendTarget {
    Normal,
    #[cfg(feature = "boracle")]
    Boracle,
}

#[allow(clippy::too_many_arguments)]
fn compile_single_file_frontend_with_target(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
    build_config_inputs: &BuildConfigInputSet,
    _mode: FrontendCompilationMode,
    target: SingleFileFrontendTarget,
) -> Result<SingleFileFrontendResult, CompilerMessages> {
    let mut resource_inputs = ResourceInputRegistry::new();
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

    timing_scope!(
        timing_guard_stage0_single_file_total,
        crate::timing::TimingMetric::Stage0SingleFileTotal
    );

    // 2. Resolve canonical entry path.
    let entry_path = match fs::canonicalize(&config.entry_dir) {
        Ok(path) => path,
        Err(error) => {
            let file_error = CompilerError::file_error(
                &config.entry_dir,
                format!("Failed to resolve entry file path: {error}"),
                string_table,
            );

            return Err(CompilerMessages::from_error_ref(file_error, string_table));
        }
    };
    let source_root = entry_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    // 3. Initialize dependency-path resolution.
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
            return Err(messages);
        }
    };

    let entry_file_name = match entry_path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name,
        None => {
            let messages =
                non_utf8_filesystem_name_error(&entry_path, "single-file entry name", string_table);
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
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };
    // 4. Discover all transitively reachable files.
    let mut external_imports = source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut builder_surface.binding_packages,
        providers: &builder_surface.external_import_providers,
        cache: &mut builder_surface.external_import_cache,
        resolution_table: &mut builder_surface.external_dependency_resolution_table,
    };

    // Register the synthetic main-project boundary before its inventory so the human summary can
    // attribute reachable discovery and module compilation as accumulated boundary work.
    #[cfg(feature = "timers")]
    let timing_boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || config.project_name.clone(),
    );
    timing_scope_attributed!(
        timing_guard_build_boundary_inventory,
        crate::timing::TimingMetric::BoundaryInventory,
        Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
    );
    let collected = match source_discovery::collect_reachable_input_files(
        &entry_path,
        &project_path_resolver,
        style_directives,
        &mut external_imports,
        &builder_surface.source_file_kinds,
        &mut resource_inputs,
        string_table,
    ) {
        Ok(collected) => collected,
        Err(messages) => {
            return Err(messages);
        }
    };
    let input_files = collected.input_files;
    let resolved_file_references = collected.resolved_file_references;
    #[cfg(feature = "timers")]
    timing_guard_build_boundary_inventory.finish();
    // Share the effective external package registry immutably for the rest of the frontend
    // pipeline so each stage does not need its own deep clone.
    let external_packages = Arc::new(builder_surface.binding_packages.clone());

    // 5. Run the module compilation pipeline with a local string-table delta.
    add_frontend_counter(FrontendCounter::ModuleCompilationSerialCount, 1);

    let string_table_fork = string_table.fork_for_module();
    let (local_table, base_len) = string_table_fork.into_parts();

    timing_scope_attributed!(
        timing_guard_boundary_compile,
        crate::timing::TimingMetric::BoundaryCompile,
        Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
    );

    // Record module-input counters before preparation so the frontend module
    // total can be attributed even when preparation fails.
    let source_byte_count = record_module_input_counters(&input_files);

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
    let timing_module_context = Some(crate::timing::TimingContext::for_module(timing_module_key));

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
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };

    // Preparation is provider-independent: it owns no external package registry, dependency
    // resolution table or builder runtime packages. Constructing it before the compilation context
    // keeps that separation visible at the one place both are built.
    let preparation_context = ModulePreparationContext {
        style_directives,
        project_path_resolver: Some(project_path_resolver.clone()),
    };

    let graph_stable_origin = stable_origin.clone();
    #[cfg(feature = "timers")]
    let prepare_result = preparation_context.prepare_module(
        stable_origin,
        input_files,
        &entry_path,
        local_table,
        source_byte_count,
        timing_module_context,
    );
    #[cfg(not(feature = "timers"))]
    let prepare_result = preparation_context.prepare_module(
        stable_origin,
        input_files,
        &entry_path,
        local_table,
        source_byte_count,
    );
    let mut prepared = match prepare_result {
        Ok(prepared) => prepared,
        Err(messages) => {
            return Err(messages);
        }
    };
    attach_single_file_resolved_references(&mut prepared, resolved_file_references, string_table)?;

    let source_facts =
        config_boundary::source_contract_facts_from_prepared(&prepared, string_table, base_len);
    let effective_project_fields = config_boundary::effective_project_fields(config, string_table)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let fixed_project_facts =
        config_boundary::fixed_project_contract_facts(&effective_project_fields);
    let direct_project_facts =
        config_boundary::direct_project_contract_facts(&effective_project_fields);
    let fallback_location = SourceLocation::from_path(&config.entry_dir, string_table);
    let build_config_values = config_boundary::resolve_boundary_build_config(
        &source_facts,
        &fixed_project_facts,
        &direct_project_facts,
        build_config_inputs,
        builder_surface.config_globals(),
        fallback_location,
        string_table,
    )?;
    // Semantic compilation is one compiler service call. A synthetic single-file module has no
    // completed providers, so it binds against empty provider and materialisation views.
    let source_provider_dependencies = SourceProviderDependencySet::default();
    let mut provider_materialisations = ProviderMaterialisationRegistry::default();
    let mut generated_store = BoundaryGeneratedFunctionStore::default();
    let compile_context = ModuleCompilationContext {
        options: config.frontend_options(),
        build_profile,
        root_role_override: None,
        project_path_resolver: Some(project_path_resolver),
        style_directives,
        external_packages: Arc::clone(&external_packages),
        build_config_values: Arc::new(build_config_values),
        external_dependency_resolution_table: &builder_surface.external_dependency_resolution_table,
        source_provider_dependencies: &source_provider_dependencies,
        provider_materialisations: &provider_materialisations,
        builder_runtime_packages: &builder_surface.builder_runtime_packages,
    };
    let semantic = prepared.semantic;

    timing_scope_attributed!(
        timing_guard_frontend_module_semantic_total,
        crate::timing::TimingMetric::FrontendModuleSemanticTotal,
        timing_module_context,
    );
    #[cfg(feature = "boracle")]
    if target == SingleFileFrontendTarget::Boracle {
        #[cfg(feature = "timers")]
        let boracle_result = compile_module_for_boracle(
            &compile_context,
            semantic,
            generated_store.known_generated(),
            timing_module_context,
        );
        #[cfg(not(feature = "timers"))]
        let boracle_result = compile_module_for_boracle(
            &compile_context,
            semantic,
            generated_store.known_generated(),
        );
        #[cfg(feature = "timers")]
        timing_guard_frontend_module_semantic_total.finish();
        #[cfg(feature = "timers")]
        timing_guard_boundary_compile.finish();
        return boracle_result.map(|input| SingleFileFrontendResult::Boracle(Box::new(input)));
    }
    #[cfg(not(feature = "boracle"))]
    let _ = target;
    #[cfg(feature = "timers")]
    let semantic_result = compile_module(
        &compile_context,
        semantic,
        generated_store.known_generated(),
        timing_module_context,
    );
    #[cfg(not(feature = "timers"))]
    let semantic_result = compile_module(
        &compile_context,
        semantic,
        generated_store.known_generated(),
    );
    #[cfg(feature = "timers")]
    timing_guard_frontend_module_semantic_total.finish();
    let result = match semantic_result {
        Ok(ModuleCompilationOutcome::Success(compiled)) => *compiled,
        Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)) => {
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
                resource_inputs,
            )
            .map(|compilation| SingleFileFrontendResult::Project(Box::new(compilation)))
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table));
        }
        Err(error) => {
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };
    #[cfg(feature = "timers")]
    timing_guard_boundary_compile.finish();

    // 6. Merge local results back into the global build context.
    let remap = string_table.merge_delta_from(&result.string_table, base_len);
    let ModuleSemanticResult {
        mut module,
        mut generated_delta,
        resource_source_associations,
        string_table: _,
        public_interface,
    } = result;
    if !remap.is_identity() {
        module.remap_string_ids(&remap);
        generated_delta.remap_string_ids(&remap);
    }
    let graph = ProjectModuleGraph::from_normal_roots(vec![(
        graph_stable_origin.clone(),
        source_root,
        entry_path,
    )]);
    let module_id = graph
        .entry_modules()
        .first()
        .copied()
        .expect("a single-module graph has one normal entry");
    let mut modules = ModuleArtifactStore::new(1);
    let artifact = CompiledModuleArtifact {
        module,
        interface: public_interface,
    };
    publish_module_and_generated(ModuleBoundaryPublication {
        modules: &mut modules,
        generated: &mut generated_store,
        materialisations: &mut provider_materialisations,
        resource_inputs: &mut resource_inputs,
        module_id,
        expected_origin: &graph_stable_origin,
        artifact,
        generated_delta,
        resource_source_associations,
    })
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
        resource_inputs,
    )
    .map(|compilation| SingleFileFrontendResult::Project(Box::new(compilation)))
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}

/// Rebind synthetic Stage 0 file-reference rows to the final module source identities.
///
/// Synthetic discovery must resolve paths before the complete source closure is known, so it
/// retains canonical target paths until `prepare_module` builds the authoritative `SourceFileTable`.
/// This helper performs that one identity join and publishes the same resolved table consumed by
/// directory modules; it does not probe the filesystem or reinterpret any path syntax.
fn attach_single_file_resolved_references(
    prepared: &mut PreparedModule,
    references: Vec<SingleFileResolvedReference>,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    let mut resolved_table = ResolvedFileReferenceTable::new();

    for reference in references {
        let source_file = prepared
            .semantic
            .source_files
            .get_by_canonical_path(&reference.source_path)
            .map(|identity| identity.file_id)
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "synthetic file-reference owner {:?} is absent from the module source table",
                        reference.source_path
                    )),
                    string_table,
                )
            })?;

        let outcome = match reference.outcome {
            SingleFileReferenceOutcome::NoPhysicalTarget => {
                ResolvedFileReferenceOutcome::NoPhysicalTarget
            }
            SingleFileReferenceOutcome::Diagnostic(diagnostic) => {
                ResolvedFileReferenceOutcome::Diagnostic(diagnostic)
            }
            SingleFileReferenceOutcome::Resource {
                source,
                owner_relative_path,
            } => {
                ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
                    source,
                    owner_relative_path,
                })
            }
            SingleFileReferenceOutcome::Source { canonical } => {
                let target_file = prepared
                    .semantic
                    .source_files
                    .get_by_canonical_path(&canonical)
                    .map(|identity| identity.file_id)
                    .ok_or_else(|| {
                        CompilerMessages::from_error_ref(
                            CompilerError::compiler_error(format!(
                                "synthetic file-reference target {:?} is absent from the module source table",
                                canonical
                            )),
                            string_table,
                        )
                    })?;
                if reference.class != PreparedFileReferenceClass::ContentSource {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "synthetic physical file-reference outcome has an incompatible class",
                        ),
                        string_table,
                    ));
                }
                ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ContentSource {
                    source: target_file,
                })
            }
            SingleFileReferenceOutcome::IdentifiedSourceKind => {
                if reference.class != PreparedFileReferenceClass::SourceKindNoFileValue {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "synthetic identified-source outcome has an incompatible class",
                        ),
                        string_table,
                    ));
                }
                ResolvedFileReferenceOutcome::Target(
                    ResolvedFileReferenceTarget::IdentifiedSourceKind,
                )
            }
        };

        resolved_table
            .push(ResolvedFileReference {
                source_file,
                path_syntax: reference.path_syntax,
                class: reference.class,
                outcome,
            })
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    }

    prepared.semantic.resolved_file_references = resolved_table;
    Ok(())
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
    Success(Box<ModuleSemanticResult>),
    Diagnosed(ModuleDiagnostics),
    /// A transient check-only unit whose required canonical provider already failed.
    ///
    /// Check-only units have no graph slot, so this outcome is intentionally not retained in the
    /// final boundary. The provider's own diagnosed/blocked record remains authoritative.
    Blocked,
    Infrastructure(CompilerError),
}

/// Immutable inputs shared by one project or source-package boundary compilation.
///
/// WHAT: keeps the boundary-wide compiler services together while each module task adds only its
///       retained provider indexes and publication stores.
/// WHY: project and source-package callers should pass one typed boundary context to the wave
///      coordinator instead of relying on a long positional argument list whose order can drift.
struct BoundaryCompilationContext<'a> {
    config: &'a Config,
    build_profile: FrontendBuildProfile,
    project_path_resolver: &'a ProjectPathResolver,
    style_directives: &'a StyleDirectiveRegistry,
    external_packages: &'a Arc<ExternalPackageRegistry>,
    builder_surface: &'a BuilderSurface,
    completed_packages: &'a CompletedSourcePackageRegistry,
    /// One immutable resolved configuration namespace for this project/package boundary.
    build_config_values: Arc<ResolvedBuildConfigMap>,
    /// Canonical source contracts retained by this boundary.
    ///
    /// Check-only jobs borrow this canonical view through an indexed resolver and resolve their
    /// own transient facts privately; sibling jobs and the retained canonical map never observe
    /// those transient declarations.
    canonical_source_facts: Vec<BuildConfigContractFact>,
    /// Explicit synthetic project-global provider, present only in the owning project boundary.
    project_globals: Option<&'a ProjectGlobalsInterface>,
    /// Inputs and facts retained for isolated check-only resolution; canonical jobs already consume
    /// the authoritative `build_config_values` map directly.
    build_config_inputs: BuildConfigInputSet,
    builder_globals: BuilderConfigGlobalSet,
    fixed_project_facts: Vec<BuildConfigContractFact>,
    direct_project_facts: Vec<BuildConfigContractFact>,
    implicit_template_package_ids: Vec<PackageBoundaryId>,
}

impl<'a> BoundaryCompilationContext<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        config: &'a Config,
        build_profile: FrontendBuildProfile,
        project_path_resolver: &'a ProjectPathResolver,
        style_directives: &'a StyleDirectiveRegistry,
        external_packages: &'a Arc<ExternalPackageRegistry>,
        builder_surface: &'a BuilderSurface,
        completed_packages: &'a CompletedSourcePackageRegistry,
        build_config_values: ResolvedBuildConfigMap,
        canonical_source_facts: Vec<BuildConfigContractFact>,
        build_config_inputs: BuildConfigInputSet,
        builder_globals: BuilderConfigGlobalSet,
        fixed_project_facts: Vec<BuildConfigContractFact>,
        direct_project_facts: Vec<BuildConfigContractFact>,
        project_globals: Option<&'a ProjectGlobalsInterface>,
    ) -> Self {
        let mut implicit_template_package_ids = builder_surface
            .implicit_template_scope_source_packages
            .iter()
            .filter_map(|prefix| completed_packages.by_prefix(prefix))
            .collect::<Vec<_>>();
        implicit_template_package_ids.sort_unstable();
        implicit_template_package_ids.dedup();

        Self {
            config,
            build_profile,
            project_path_resolver,
            style_directives,
            external_packages,
            builder_surface,
            completed_packages,
            build_config_values: Arc::new(build_config_values),
            canonical_source_facts,
            build_config_inputs,
            builder_globals,
            fixed_project_facts,
            direct_project_facts,
            project_globals,
            implicit_template_package_ids,
        }
    }
}

struct DirectoryModuleCompileContext<'boundary, 'services> {
    boundary: &'boundary BoundaryCompilationContext<'services>,
    provider_store: &'boundary ModuleArtifactStore,
    /// Declaring-module generic templates already published in this boundary.
    provider_materialisations: &'boundary ProviderMaterialisationRegistry,
    provider_bindings: &'boundary [ResolvedDependencyEdge],
    provider_binding_index: &'boundary FxHashMap<(ModuleId, DependencyShellId), usize>,
    source_package_dependencies: &'boundary [ResolvedSourcePackageDependency],
    source_package_dependency_index: &'boundary FxHashMap<(ModuleId, DependencyShellId), usize>,
}

struct SourcePackageModuleInventory {
    dependency_prefix: String,
    package_identity: StablePackageIdentity,
    root_module_id: ModuleId,
    path_resolver: ProjectPathResolver,
    graph: ProjectModuleGraph,
    schedule: module_inventory::ModuleCompilationSchedule,
    /// Canonical source facts merged before transient jobs fork their string-table base.
    canonical_source_facts: Vec<BuildConfigContractFact>,
    #[cfg(feature = "timers")]
    timing_boundary: crate::timing::TimingBoundaryId,
}

/// Source-package transient jobs retained until every canonical package facade is published.
///
/// Check-only package dependencies never participate in canonical package ordering. Keeping this
/// lane separate lets all canonical packages publish first, after which transient jobs can safely
/// consume any completed facade without changing the publication graph.
struct SourcePackageCheckOnlyInventory {
    dependency_prefix: String,
    path_resolver: ProjectPathResolver,
    check_only_jobs: Vec<module_inventory::CheckOnlyModuleCompilationJob>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    source_package_dependencies: Vec<ResolvedSourcePackageDependency>,
    /// Canonical source contracts used to resolve each deferred job independently.
    canonical_source_facts: Vec<BuildConfigContractFact>,
    build_config_values: ResolvedBuildConfigMap,
}

/// Index every resolved provider edge once by consumer module and retained dependency shell.
///
/// WHAT: gives module binding a direct shell-edge lookup instead of scanning all edges and comparing
///       path components for each retained dependency.
/// WHY: the shell identity is stamped during header preparation and copied onto the graph edge,
///       so a duplicate key here means the same retained clause resolved twice, which is a proven
///       build invariant violation rather than a user failure. One authored clause has one
///       provider surface, so the shell is the complete join identity.
pub(crate) fn build_provider_binding_index(
    provider_bindings: &[ResolvedDependencyEdge],
) -> Result<FxHashMap<(ModuleId, DependencyShellId), usize>, CompilerError> {
    let mut index = FxHashMap::default();
    for (binding_index, binding) in provider_bindings.iter().enumerate() {
        let shell_id = binding.dependency_shell_id;
        let key = (binding.consumer_module_id, shell_id);
        if index.insert(key, binding_index).is_some() {
            return Err(CompilerError::compiler_error(format!(
                "ModuleId {} resolved dependency shell {:?} to more than one provider edge",
                binding.consumer_module_id.index(),
                shell_id
            )));
        }
    }

    Ok(index)
}

/// Index every resolved source-package dependency once by consumer module and retained shell.
pub(crate) fn build_source_package_dependency_index(
    provider_binding_index: &FxHashMap<(ModuleId, DependencyShellId), usize>,
    source_package_dependencies: &[ResolvedSourcePackageDependency],
) -> Result<FxHashMap<(ModuleId, DependencyShellId), usize>, CompilerError> {
    let mut index = FxHashMap::default();
    for (dependency_index, package_dependency) in source_package_dependencies.iter().enumerate() {
        let shell_id = package_dependency.dependency_shell_id;
        let key = (package_dependency.consumer_module_id, shell_id);
        if provider_binding_index.contains_key(&key) {
            return Err(CompilerError::compiler_error(format!(
                "ModuleId {} resolved dependency shell {:?} to both a provider module and a source package",
                package_dependency.consumer_module_id.index(),
                shell_id
            )));
        }
        if index.insert(key, dependency_index).is_some() {
            return Err(CompilerError::compiler_error(format!(
                "ModuleId {} resolved dependency shell {:?} to more than one source-package dependency",
                package_dependency.consumer_module_id.index(),
                shell_id
            )));
        }
    }

    Ok(index)
}

/// Index every consumer module's direct package dependencies once per boundary.
///
/// WHAT: resolves each source-package dependency to its dense [`PackageBoundaryId`] and
///       groups the IDs by consumer module, deduplicated and sorted in package order.
/// WHY: readiness checks must walk only the current module's package dependencies. Building
///      the grouped index once per boundary keeps that walk proportional to direct dependencies.
pub(crate) fn build_module_package_dependency_index(
    source_package_dependencies: &[ResolvedSourcePackageDependency],
    completed_packages: &CompletedSourcePackageRegistry,
) -> Result<FxHashMap<ModuleId, Vec<PackageBoundaryId>>, CompilerError> {
    let mut dependencies: FxHashMap<ModuleId, Vec<PackageBoundaryId>> = FxHashMap::default();

    for package_dependency in source_package_dependencies {
        let package_id = completed_packages
            .by_prefix(package_dependency.dependency_prefix.as_str())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "ModuleId {} depends on unindexed source package @{}",
                    package_dependency.consumer_module_id.index(),
                    package_dependency.dependency_prefix
                ))
            })?;
        dependencies
            .entry(package_dependency.consumer_module_id)
            .or_default()
            .push(package_id);
    }

    for package_ids in dependencies.values_mut() {
        package_ids.sort_unstable();
        package_ids.dedup();
    }

    Ok(dependencies)
}

/// Find an authored `@project` dependency on the project package facade.
///
/// The facade is an API-only root, but its retained dependency clauses still include private and
/// otherwise unreachable source declarations. Rejecting the exact reserved root here, before
/// provider binding or AST reachability, enforces the package boundary for every declaration.
fn facade_project_globals_dependency(
    prepared: &PreparedModule,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    let active_origin = prepared
        .semantic
        .source_module_origins
        .origin_for(prepared.semantic.active_root_file_id)?
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "facade dependency validation found no owning origin for the active root",
            )
        })?;
    if active_origin.role() != ModuleRootRole::ProjectPackageFacade {
        return Ok(None);
    }

    for clauses in prepared
        .semantic
        .prepared_header_syntax
        .module_symbols
        .file_dependency_clauses_by_source
        .values()
    {
        for clause in clauses {
            if is_project_globals_dependency(
                &clause.dependency.path,
                &prepared.semantic.string_table,
            ) {
                return Ok(Some(CompilerDiagnostic::invalid_dependency_clause(
                    clause.binding.clause_kind(),
                    InvalidDependencyClauseReason::ProjectGlobalsFacadeDependencyNotAllowed,
                    clause.dependency.location.clone(),
                )));
            }
        }
    }
    Ok(None)
}

impl<'boundary, 'services> DirectoryModuleCompileContext<'boundary, 'services> {
    /// Build the per-module provider input set by direct retained-shell lookup.
    ///
    /// Canonical jobs use the boundary indexes built once per graph. Check-only jobs instead pass
    /// their own resolved module/package records; an isolated job never falls back to canonical
    /// shell indexes, because a shell from another source must not accidentally bind merely due
    /// to sharing the owner's `ModuleId`.
    fn build_source_provider_dependencies(
        &self,
        consumer_module_id: ModuleId,
        prepared: &PreparedModule,
        check_only_provider_bindings: Option<&[module_inventory::CheckOnlyProviderBinding]>,
        check_only_source_package_dependencies: Option<
            &[module_inventory::CheckOnlySourcePackageDependency],
        >,
    ) -> Result<SourceProviderDependencySet<'boundary>, CompilerError> {
        let check_only = check_only_provider_bindings.is_some()
            || check_only_source_package_dependencies.is_some();
        let mut transient_provider_index: FxHashMap<DependencyShellId, ModuleId> =
            FxHashMap::default();
        let mut transient_package_index: FxHashMap<DependencyShellId, &str> = FxHashMap::default();

        if let Some(bindings) = check_only_provider_bindings {
            for binding in bindings {
                if transient_provider_index
                    .insert(binding.dependency_shell_id, binding.provider_module_id)
                    .is_some()
                {
                    return Err(CompilerError::compiler_error(format!(
                        "check-only ModuleId {} resolved dependency shell {:?} to more than one provider module",
                        consumer_module_id.index(),
                        binding.dependency_shell_id
                    )));
                }
            }
        }
        if let Some(dependencies) = check_only_source_package_dependencies {
            for dependency in dependencies {
                if transient_provider_index.contains_key(&dependency.dependency_shell_id)
                    || transient_package_index
                        .insert(
                            dependency.dependency_shell_id,
                            dependency.dependency_prefix.as_str(),
                        )
                        .is_some()
                {
                    return Err(CompilerError::compiler_error(format!(
                        "check-only ModuleId {} resolved dependency shell {:?} to more than one provider",
                        consumer_module_id.index(),
                        dependency.dependency_shell_id
                    )));
                }
            }
        }

        let mut dependencies = Vec::new();
        for file_dependency_clauses in prepared
            .semantic
            .prepared_header_syntax
            .module_symbols
            .file_dependency_clauses_by_source
            .values()
        {
            for clause in file_dependency_clauses {
                let shell_id = clause.dependency.dependency_shell_id;
                if is_project_globals_dependency(
                    &clause.dependency.path,
                    &prepared.semantic.string_table,
                ) {
                    let Some(project_globals) = self.boundary.project_globals else {
                        return Err(CompilerError::compiler_error(format!(
                            "ModuleId {} attempted to bind reserved @project outside its owning project boundary",
                            consumer_module_id.index()
                        )));
                    };
                    dependencies.push(SourceProviderDependency {
                        kind: ProviderDependencyKind::Authored { shell: shell_id },
                        interface: project_globals.interface(),
                    });
                    continue;
                }
                if check_only {
                    if let Some(provider_module_id) =
                        transient_provider_index.get(&shell_id).copied()
                    {
                        let interface = self
                            .provider_store
                            .interface(provider_module_id)?
                            .ok_or_else(|| {
                                CompilerError::compiler_error(format!(
                                    "Check-only ModuleId {} started semantic binding before provider ModuleId {} published a complete interface",
                                    consumer_module_id.index(),
                                    provider_module_id.index()
                                ))
                            })?;
                        dependencies.push(SourceProviderDependency {
                            kind: ProviderDependencyKind::Authored { shell: shell_id },
                            interface,
                        });
                        continue;
                    }

                    if let Some(dependency_prefix) = transient_package_index.get(&shell_id).copied()
                    {
                        let package_id = self
                            .boundary
                            .completed_packages
                            .by_prefix(dependency_prefix)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(format!(
                                    "Check-only ModuleId {} started semantic binding before source package @{} completed",
                                    consumer_module_id.index(),
                                    dependency_prefix
                                ))
                            })?;
                        let completed_package =
                            self.boundary.completed_packages.package(package_id)?;
                        dependencies.push(SourceProviderDependency {
                            kind: ProviderDependencyKind::Authored { shell: shell_id },
                            interface: completed_package.root_interface()?,
                        });
                    }

                    // Same-owner source dependencies and provider clauses handled by the external
                    // registry intentionally have no transient interface record. Do not consult
                    // the canonical indexes for this shell.
                    continue;
                }

                if let Some(binding_index) = self
                    .provider_binding_index
                    .get(&(consumer_module_id, shell_id))
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

                    dependencies.push(SourceProviderDependency {
                        kind: ProviderDependencyKind::Authored { shell: shell_id },
                        interface,
                    });
                    continue;
                }

                let Some(package_index) = self
                    .source_package_dependency_index
                    .get(&(consumer_module_id, shell_id))
                else {
                    continue;
                };
                let package_dependency = &self.source_package_dependencies[*package_index];
                let package_id = self
                    .boundary
                    .completed_packages
                    .by_prefix(package_dependency.dependency_prefix.as_str())
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "ModuleId {} started semantic binding before source package @{} completed",
                            consumer_module_id.index(),
                            package_dependency.dependency_prefix
                        ))
                    })?;
                let completed_package = self.boundary.completed_packages.package(package_id)?;

                dependencies.push(SourceProviderDependency {
                    kind: ProviderDependencyKind::Authored { shell: shell_id },
                    interface: completed_package.root_interface()?,
                });
            }
        }

        // Builder source-backed packages are implicitly available only to modules that actually
        // contain a `.mtf` semantic source. The package capability is supplied by the active
        // builder surface; generic orchestration must not infer it from a package-name list.
        if prepared.contains_moth_template {
            let implicit_provider_dependencies: Vec<SourceProviderDependency<'boundary>> = self
                .boundary
                .implicit_template_package_ids
                .iter()
                .map(|package_id| {
                    let package = self.boundary.completed_packages.package(*package_id)?;
                    let interface = package.root_interface()?;
                    Ok(SourceProviderDependency {
                        kind: ProviderDependencyKind::ImplicitTemplate {
                            package_prefix: package.package_prefix(),
                        },
                        interface,
                    })
                })
                .collect::<Result<_, CompilerError>>()?;

            dependencies.extend(implicit_provider_dependencies);
        }

        SourceProviderDependencySet::new(dependencies)
    }

    /// Return the first failed canonical provider for a transient job, if any.
    ///
    /// Check-only units have no graph slot of their own. A diagnosed or blocked provider therefore
    /// suppresses the dependent unit instead of letting interface lookup turn the provider's
    /// authoritative diagnostic into a secondary infrastructure error. An unavailable slot is
    /// still an internal scheduling failure: canonical publication should have completed first.
    fn check_only_blocked_provider(
        &self,
        consumer_module_id: ModuleId,
        provider_bindings: &[module_inventory::CheckOnlyProviderBinding],
        source_package_dependencies: &[module_inventory::CheckOnlySourcePackageDependency],
    ) -> Result<Option<BlockedProvider>, CompilerError> {
        for binding in provider_bindings {
            match self.provider_store.slot(binding.provider_module_id)? {
                ProviderSlot::Successful(_) => {}
                ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                    return Ok(Some(BlockedProvider::Module(binding.provider_module_id)));
                }
                ProviderSlot::Unavailable => {
                    return Err(CompilerError::compiler_error(format!(
                        "Check-only ModuleId {} became ready before provider ModuleId {} completed",
                        consumer_module_id.index(),
                        binding.provider_module_id.index()
                    )));
                }
            }
        }

        for dependency in source_package_dependencies {
            let package_id = self
                .boundary
                .completed_packages
                .by_prefix(dependency.dependency_prefix.as_str())
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Check-only ModuleId {} depends on unindexed source package @{}",
                        consumer_module_id.index(),
                        dependency.dependency_prefix
                    ))
                })?;
            let package = self.boundary.completed_packages.package(package_id)?;
            match package.root_slot()? {
                ProviderSlot::Successful(_) => {}
                ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                    return Ok(Some(BlockedProvider::SourcePackage(
                        package.package_identity.clone(),
                    )));
                }
                ProviderSlot::Unavailable => {
                    return Err(CompilerError::compiler_error(format!(
                        "Check-only ModuleId {} became ready before source package @{} completed its facade",
                        consumer_module_id.index(),
                        package.package_prefix()
                    )));
                }
            }
        }

        Ok(None)
    }

    fn compile(
        &self,
        job: module_inventory::ModuleCompilationJob,
        known_generated: KnownGeneratedFunctions<'_>,
    ) -> DirectoryModuleTaskResult {
        let module_inventory::ModuleCompilationJob {
            module_id,
            string_table_base_len: base_len,
            prepared,
            #[cfg(feature = "timers")]
            timing_module_key,
            ..
        } = job;

        // The dense graph `ModuleId` is the module key inside this boundary, so attribution
        // stays deterministic and independent of worker completion order.
        #[cfg(feature = "timers")]
        let module_context = Some(crate::timing::TimingContext::for_module(timing_module_key));

        #[cfg(feature = "timers")]
        {
            self.compile_prepared(
                module_id,
                base_len,
                prepared,
                known_generated,
                None,
                None,
                None,
                None,
                None,
                module_context,
            )
        }
        #[cfg(not(feature = "timers"))]
        {
            self.compile_prepared(
                module_id,
                base_len,
                prepared,
                known_generated,
                None,
                None,
                None,
                None,
                None,
            )
        }
    }

    fn compile_check_only(
        &self,
        job: module_inventory::CheckOnlyModuleCompilationJob,
        known_generated: KnownGeneratedFunctions<'_>,
        build_config_index: &BuildConfigResolutionIndex<'_>,
    ) -> DirectoryModuleTaskResult {
        let module_inventory::CheckOnlyModuleCompilationJob {
            owner_module_id: module_id,
            string_table_base_len: base_len,
            provider_bindings,
            source_package_dependencies,
            external_packages,
            external_dependency_resolution_table,
            mut prepared,
            ..
        } = job;
        match self.check_only_blocked_provider(
            module_id,
            &provider_bindings,
            &source_package_dependencies,
        ) {
            Ok(Some(_provider)) => {
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome: DirectoryModuleTaskOutcome::Blocked,
                };
            }
            Ok(None) => {}
            Err(error) => {
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome: DirectoryModuleTaskOutcome::Infrastructure(error),
                };
            }
        }

        // Resolve this transient unit against borrowed canonical facts and only its own source
        // facts. The transient slice is private to this job: it validates compatibility and
        // selects values without copying or comparing a sibling check-only unit.
        let check_only_source_facts =
            config_boundary::source_contract_facts_for_current_module(&prepared);
        let check_only_inputs = build_config_index.filter_inputs_to_known_facts(
            &self.boundary.build_config_inputs,
            &check_only_source_facts,
        );
        let check_only_build_config_values = match build_config_index
            .resolve_with_transient_source_facts(
                &check_only_source_facts,
                &check_only_inputs,
                &self.boundary.builder_globals,
            ) {
            Ok(values) => values,
            Err(error) => {
                let fallback_location = error
                    .contract_location()
                    .cloned()
                    .unwrap_or_else(SourceLocation::default);
                let messages = config_boundary::build_config_resolution_messages(
                    error,
                    fallback_location,
                    &mut prepared.semantic.string_table,
                );
                let outcome = match ModuleDiagnostics::from_messages(messages) {
                    Ok(diagnostics) => DirectoryModuleTaskOutcome::Diagnosed(diagnostics),
                    Err(error) => DirectoryModuleTaskOutcome::Infrastructure(error),
                };
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome,
                };
            }
        };

        #[cfg(feature = "timers")]
        {
            self.compile_prepared(
                module_id,
                base_len,
                prepared,
                known_generated,
                Some(&check_only_build_config_values),
                Some(&provider_bindings),
                Some(&source_package_dependencies),
                Some(external_packages),
                Some(&external_dependency_resolution_table),
                None,
            )
        }
        #[cfg(not(feature = "timers"))]
        {
            self.compile_prepared(
                module_id,
                base_len,
                prepared,
                known_generated,
                Some(&check_only_build_config_values),
                Some(&provider_bindings),
                Some(&source_package_dependencies),
                Some(external_packages),
                Some(&external_dependency_resolution_table),
            )
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_prepared(
        &self,
        module_id: ModuleId,
        base_len: usize,
        prepared: PreparedModule,
        known_generated: KnownGeneratedFunctions<'_>,
        build_config_values_override: Option<&ResolvedBuildConfigMap>,
        check_only_provider_bindings: Option<&[module_inventory::CheckOnlyProviderBinding]>,
        check_only_source_package_dependencies: Option<
            &[module_inventory::CheckOnlySourcePackageDependency],
        >,
        external_packages: Option<Arc<ExternalPackageRegistry>>,
        external_dependency_resolution_table: Option<&ExternalImportResolutionTable>,
        #[cfg(feature = "timers")] module_context: Option<crate::timing::TimingContext>,
    ) -> DirectoryModuleTaskResult {
        match facade_project_globals_dependency(&prepared) {
            Ok(Some(diagnostic)) => {
                let messages = CompilerMessages::from_diagnostic(
                    diagnostic,
                    prepared.semantic.string_table.clone(),
                );
                let outcome = match ModuleDiagnostics::from_messages(messages) {
                    Ok(diagnostics) => DirectoryModuleTaskOutcome::Diagnosed(diagnostics),
                    Err(error) => DirectoryModuleTaskOutcome::Infrastructure(error),
                };
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome,
                };
            }
            Ok(None) => {}
            Err(error) => {
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome: DirectoryModuleTaskOutcome::Infrastructure(error),
                };
            }
        }

        // Semantic compilation is provider-dependent, so every required provider interface must
        // already be published before this call. Canonical jobs use the graph indexes; check-only
        // jobs use the isolated metadata prepared with their own source headers.
        let source_provider_dependencies = match self.build_source_provider_dependencies(
            module_id,
            &prepared,
            check_only_provider_bindings,
            check_only_source_package_dependencies,
        ) {
            Ok(dependencies) => dependencies,
            Err(error) => {
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome: DirectoryModuleTaskOutcome::Infrastructure(error),
                };
            }
        };
        // Canonical modules consume the one boundary-owned map directly. Only transient
        // check-only jobs provide an isolated override resolved from their own source facts.
        let effective_build_config_values = build_config_values_override
            .map(|values| Arc::new(values.clone()))
            .unwrap_or_else(|| Arc::clone(&self.boundary.build_config_values));

        let effective_external_packages =
            external_packages.unwrap_or_else(|| Arc::clone(self.boundary.external_packages));
        let effective_external_dependency_resolution_table = external_dependency_resolution_table
            .unwrap_or(
                &self
                    .boundary
                    .builder_surface
                    .external_dependency_resolution_table,
            );
        let compile_context = ModuleCompilationContext {
            options: self.boundary.config.frontend_options(),
            build_profile: self.boundary.build_profile,
            root_role_override: (check_only_provider_bindings.is_some()
                || check_only_source_package_dependencies.is_some())
            .then_some(ModuleRootRole::Support),
            project_path_resolver: Some(self.boundary.project_path_resolver.clone()),
            style_directives: self.boundary.style_directives,
            external_packages: effective_external_packages,
            build_config_values: effective_build_config_values,
            external_dependency_resolution_table: effective_external_dependency_resolution_table,
            source_provider_dependencies: &source_provider_dependencies,
            provider_materialisations: self.provider_materialisations,
            builder_runtime_packages: &self.boundary.builder_surface.builder_runtime_packages,
        };

        // The typed semantic boundary already classified user diagnostics from infrastructure
        // failures, so the task outcome carries the retained `ModuleDiagnostics` unchanged.
        timing_scope_attributed!(
            timing_guard_frontend_module_semantic_total_2,
            crate::timing::TimingMetric::FrontendModuleSemanticTotal,
            module_context,
        );
        #[cfg(feature = "timers")]
        let semantic_result = compile_module(
            &compile_context,
            prepared.semantic,
            known_generated,
            module_context,
        );
        #[cfg(not(feature = "timers"))]
        let semantic_result = compile_module(&compile_context, prepared.semantic, known_generated);
        #[cfg(feature = "timers")]
        timing_guard_frontend_module_semantic_total_2.finish();
        let outcome = match semantic_result {
            Ok(ModuleCompilationOutcome::Success(compiled)) => {
                DirectoryModuleTaskOutcome::Success(compiled)
            }
            Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)) => {
                DirectoryModuleTaskOutcome::Diagnosed(diagnostics)
            }
            Err(error) => DirectoryModuleTaskOutcome::Infrastructure(error),
        };
        DirectoryModuleTaskResult {
            module_id,
            string_table_base_len: base_len,
            outcome,
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn compile_module_waves(
    context: BoundaryCompilationContext<'_>,
    graph: ProjectModuleGraph,
    module_waves: Vec<Vec<module_inventory::ModuleCompilationJob>>,
    check_only_jobs: Vec<module_inventory::CheckOnlyModuleCompilationJob>,
    provider_bindings: &[ResolvedDependencyEdge],
    source_package_dependencies: &[ResolvedSourcePackageDependency],
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
) -> Result<(CompiledGraphBoundary, Vec<CompilerMessages>), CompilerMessages> {
    let mut provider_store = ModuleArtifactStore::new(graph.nodes().len());
    let mut generated_store = BoundaryGeneratedFunctionStore::default();
    // The compiler resolves declaring generic templates through this registry, so it never reads a
    // live build store while semantic analysis runs. Completed packages seed it; each successful
    // module in this boundary extends it as it publishes.
    let mut provider_materialisations =
        seed_completed_package_materialisations(context.completed_packages)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // One direct lookup index per boundary so module binding never scans every provider edge,
    // source-package dependency or completed package for each retained dependency shell.
    let provider_binding_index = build_provider_binding_index(provider_bindings)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let source_package_dependency_index =
        build_source_package_dependency_index(&provider_binding_index, source_package_dependencies)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    // Canonical fact ownership is indexed once per boundary. Transient units borrow this index
    // instead of cloning and concatenating the canonical fact vector for every unit.
    let build_config_index = BuildConfigResolutionIndex::from_validated(
        &context.canonical_source_facts,
        &context.fixed_project_facts,
        &context.direct_project_facts,
    );

    // Index each consumer module's direct package dependencies once per boundary so readiness
    // walks only the packages that module actually depends on and never filters the full dependency
    // vector for every job.
    let module_package_dependencies = build_module_package_dependency_index(
        source_package_dependencies,
        context.completed_packages,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let mut diagnosed = Vec::new();
    let mut blocked = Vec::new();
    let mut transient_messages = Vec::new();
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
                    let package = context
                        .completed_packages
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
                                package.package_prefix()
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
            // This owner publishes each successful module transaction before starting the next
            // ModuleId, so duplicate requests in one ready wave materialise exactly once. File
            // preparation remains parallel inside each module. Semantic module-wave parallelism
            // remains a separate future phase because generated deltas currently commit
            // deterministically through this serial publication owner.
            let outcome = {
                let compile_context = DirectoryModuleCompileContext {
                    boundary: &context,
                    provider_store: &provider_store,
                    provider_materialisations: &provider_materialisations,
                    provider_bindings,
                    provider_binding_index: &provider_binding_index,
                    source_package_dependencies,
                    source_package_dependency_index: &source_package_dependency_index,
                };
                compile_context.compile(job, generated_store.known_generated())
            };
            match outcome.outcome {
                DirectoryModuleTaskOutcome::Success(compiled) => {
                    let compiled = *compiled;
                    let remap = string_table
                        .merge_delta_from(&compiled.string_table, outcome.string_table_base_len);
                    let ModuleSemanticResult {
                        mut module,
                        mut generated_delta,
                        resource_source_associations,
                        string_table: _,
                        public_interface,
                    } = compiled;
                    if !remap.is_identity() {
                        module.remap_string_ids(&remap);
                        generated_delta.remap_string_ids(&remap);
                    }
                    let artifact = CompiledModuleArtifact {
                        module,
                        interface: public_interface,
                    };
                    publish_module_and_generated(ModuleBoundaryPublication {
                        modules: &mut provider_store,
                        generated: &mut generated_store,
                        materialisations: &mut provider_materialisations,
                        resource_inputs,
                        module_id: outcome.module_id,
                        expected_origin: graph.node(outcome.module_id).stable_origin(),
                        artifact,
                        generated_delta,
                        resource_source_associations,
                    })
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
                DirectoryModuleTaskOutcome::Blocked => {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(format!(
                            "canonical ModuleId {} unexpectedly became transient-blocked",
                            outcome.module_id.index()
                        )),
                        string_table,
                    ));
                }
                DirectoryModuleTaskOutcome::Infrastructure(error) => {
                    return Err(CompilerMessages::from_error_ref(error, string_table));
                }
            }
        }
    }

    // Check-only units are semantically compiled after canonical publication, but their
    // successful artefacts, interfaces, generated deltas and resource associations are discarded.
    // Only their diagnostics/warnings cross the frontend result boundary.
    add_frontend_counter(
        FrontendCounter::ModuleCompilationSerialCount,
        check_only_jobs.len(),
    );
    for check_only_job in check_only_jobs {
        let outcome = {
            let compile_context = DirectoryModuleCompileContext {
                boundary: &context,
                provider_store: &provider_store,
                provider_materialisations: &provider_materialisations,
                provider_bindings,
                provider_binding_index: &provider_binding_index,
                source_package_dependencies,
                source_package_dependency_index: &source_package_dependency_index,
            };
            compile_context.compile_check_only(
                check_only_job,
                generated_store.known_generated(),
                &build_config_index,
            )
        };
        match outcome.outcome {
            DirectoryModuleTaskOutcome::Success(compiled) => {
                let ModuleSemanticResult {
                    module,
                    generated_delta,
                    string_table: module_string_table,
                    ..
                } = *compiled;
                let mut warnings = module.metadata.warnings;
                warnings.extend(
                    generated_delta
                        .records()
                        .iter()
                        .flat_map(|record| record.sidecar.module.metadata.warnings.iter().cloned()),
                );
                if !warnings.is_empty() {
                    let mut messages =
                        CompilerMessages::from_diagnostics(warnings, module_string_table);
                    let remap = string_table
                        .merge_delta_from(&messages.string_table, outcome.string_table_base_len);
                    if !remap.is_identity() {
                        messages.remap_string_ids(&remap);
                    }
                    transient_messages.push(messages);
                }
            }
            DirectoryModuleTaskOutcome::Diagnosed(diagnostics) => {
                let mut messages = diagnostics.into_messages();
                let remap = string_table
                    .merge_delta_from(&messages.string_table, outcome.string_table_base_len);
                if !remap.is_identity() {
                    messages.remap_string_ids(&remap);
                }
                transient_messages.push(messages);
            }
            DirectoryModuleTaskOutcome::Blocked => {
                // The failed canonical provider's own diagnostics remain authoritative; a
                // dependent check-only unit contributes no cascade diagnostics.
            }
            DirectoryModuleTaskOutcome::Infrastructure(error) => {
                return Err(CompilerMessages::from_error_ref(error, string_table));
            }
        }
    }

    let diagnosed_provider_exists = !diagnosed.is_empty()
        || context
            .completed_packages
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
        .map(|boundary| (boundary, transient_messages))
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}
/// Seed materialisation templates already published in a canonical boundary.
///
/// A deferred check-only pass cannot borrow the temporary registry used by canonical compilation,
/// so it reconstructs the immutable lookup from the retained successful artefacts. No transient
/// generated result is published into this registry.
fn seed_boundary_materialisations(
    registry: &mut ProviderMaterialisationRegistry,
    modules: &ModuleArtifactStore,
) -> Result<(), CompilerError> {
    for artifact in modules.successful_artefacts_in_module_id_order() {
        if let Some(context) = artifact.module.metadata.materialisation_context.as_ref() {
            registry.publish_context(context)?;
        }
    }
    Ok(())
}

/// Compile one boundary's transient jobs after its canonical artefacts are complete.
///
/// This is used for source packages after *all* source-package facades have published. Check-only
/// jobs can therefore consume any canonical module/package provider without adding package edges
/// or changing the retained boundary. Successful artefacts, generated deltas and resource
/// associations are dropped; only diagnostics and warnings are returned.
fn compile_check_only_jobs_after_canonical(
    context: BoundaryCompilationContext<'_>,
    provider_store: &ModuleArtifactStore,
    generated_store: &BoundaryGeneratedFunctionStore,
    check_only_jobs: Vec<module_inventory::CheckOnlyModuleCompilationJob>,
    provider_bindings: &[ResolvedDependencyEdge],
    source_package_dependencies: &[ResolvedSourcePackageDependency],
    string_table: &mut StringTable,
) -> Result<Vec<CompilerMessages>, CompilerMessages> {
    let mut provider_materialisations =
        seed_completed_package_materialisations(context.completed_packages)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    seed_boundary_materialisations(&mut provider_materialisations, provider_store)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let provider_binding_index = build_provider_binding_index(provider_bindings)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let source_package_dependency_index =
        build_source_package_dependency_index(&provider_binding_index, source_package_dependencies)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    // Reuse one borrowed canonical fact index for every deferred transient unit.
    let build_config_index = BuildConfigResolutionIndex::from_validated(
        &context.canonical_source_facts,
        &context.fixed_project_facts,
        &context.direct_project_facts,
    );

    add_frontend_counter(
        FrontendCounter::ModuleCompilationSerialCount,
        check_only_jobs.len(),
    );
    let mut transient_messages = Vec::new();
    for check_only_job in check_only_jobs {
        let outcome = {
            let compile_context = DirectoryModuleCompileContext {
                boundary: &context,
                provider_store,
                provider_materialisations: &provider_materialisations,
                provider_bindings,
                provider_binding_index: &provider_binding_index,
                source_package_dependencies,
                source_package_dependency_index: &source_package_dependency_index,
            };
            compile_context.compile_check_only(
                check_only_job,
                generated_store.known_generated(),
                &build_config_index,
            )
        };
        match outcome.outcome {
            DirectoryModuleTaskOutcome::Success(compiled) => {
                let ModuleSemanticResult {
                    module,
                    generated_delta,
                    string_table: module_string_table,
                    ..
                } = *compiled;
                let mut warnings = module.metadata.warnings;
                warnings.extend(
                    generated_delta
                        .records()
                        .iter()
                        .flat_map(|record| record.sidecar.module.metadata.warnings.iter().cloned()),
                );
                if !warnings.is_empty() {
                    let mut messages =
                        CompilerMessages::from_diagnostics(warnings, module_string_table);
                    let remap = string_table
                        .merge_delta_from(&messages.string_table, outcome.string_table_base_len);
                    if !remap.is_identity() {
                        messages.remap_string_ids(&remap);
                    }
                    transient_messages.push(messages);
                }
            }
            DirectoryModuleTaskOutcome::Diagnosed(diagnostics) => {
                let mut messages = diagnostics.into_messages();
                let remap = string_table
                    .merge_delta_from(&messages.string_table, outcome.string_table_base_len);
                if !remap.is_identity() {
                    messages.remap_string_ids(&remap);
                }
                transient_messages.push(messages);
            }
            DirectoryModuleTaskOutcome::Blocked => {
                // The failed canonical provider's own diagnostics remain authoritative; a
                // dependent check-only unit contributes no cascade diagnostics.
            }
            DirectoryModuleTaskOutcome::Infrastructure(error) => {
                return Err(CompilerMessages::from_error_ref(error, string_table));
            }
        }
    }

    Ok(transient_messages)
}

fn order_source_package_inventories(
    inventories: Vec<SourcePackageModuleInventory>,
    string_table: &StringTable,
) -> Result<Vec<SourcePackageModuleInventory>, CompilerMessages> {
    let package_prefixes = inventories
        .iter()
        .map(|inventory| inventory.dependency_prefix.clone())
        .collect::<Vec<_>>();
    // Only canonical source-package dependencies participate in package ordering. Check-only
    // bindings are transient semantic inputs and must not add package graph edges or make a
    // package appear cyclic; their jobs run after all canonical facades have published.
    let dependency_prefixes = inventories
        .iter()
        .map(|inventory| {
            let mut dependencies = inventory
                .schedule
                .canonical_source_package_dependencies()
                .iter()
                .map(|dependency| dependency.dependency_prefix.clone())
                .collect::<Vec<_>>();
            dependencies.sort();
            dependencies.dedup();
            dependencies
        })
        .collect::<Vec<_>>();

    let order = order_packages_by_dependency(&package_prefixes, &dependency_prefixes)
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
    package_prefixes: &[String],
    dependency_prefixes: &[Vec<String>],
) -> Result<Vec<usize>, CompilerError> {
    let package_count = package_prefixes.len();
    if dependency_prefixes.len() != package_count {
        return Err(CompilerError::compiler_error(format!(
            "package dependency schedule received {} packages but {} dependency entries",
            package_count,
            dependency_prefixes.len()
        )));
    }

    let mut index_by_prefix: FxHashMap<&str, usize> = FxHashMap::default();
    for (index, prefix) in package_prefixes.iter().enumerate() {
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
                        package_prefixes[index], dependency
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
            ready.push(std::cmp::Reverse((package_prefixes[index].as_str(), index)));
        }
    }

    let mut ordered = Vec::with_capacity(package_count);
    while let Some(std::cmp::Reverse((_, index))) = ready.pop() {
        ordered.push(index);
        for consumer_index in &consumer_lists[index] {
            indegree[*consumer_index] -= 1;
            if indegree[*consumer_index] == 0 {
                ready.push(std::cmp::Reverse((
                    package_prefixes[*consumer_index].as_str(),
                    *consumer_index,
                )));
            }
        }
    }

    if ordered.len() != package_count {
        let blocked = (0..package_count)
            .filter(|index| !ordered.contains(index))
            .map(|index| format!("@{}", package_prefixes[index]))
            .collect::<Vec<_>>();
        return Err(CompilerError::compiler_error(format!(
            "Source package dependency cycle detected; no package is ready among {}",
            blocked.join(", ")
        )));
    }

    Ok(ordered)
}

/// Discover all entry modules in a directory project and compile each one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_directory_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    validated_output_settings: Option<&ValidatedDirectoryOutputSettings>,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    string_table: &mut StringTable,
    build_config_inputs: &BuildConfigInputSet,
    mode: FrontendCompilationMode,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    // Directory inventory owns graph construction, source-package discovery,
    // and deterministic package ordering before any module semantics run.
    timing_scope!(
        timing_guard_stage0_directory_inventory,
        crate::timing::TimingMetric::Stage0DirectoryInventory
    );

    // 1. Setup path resolution based on config settings.
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
            return Err(error);
        }
    };
    let project_path_resolver = project_setup.resolver;
    let config_globals = builder_surface.config_globals().clone();

    // 2. Build every source-package inventory and the project inventory before semantic
    // compilation. Provider-backed discovery may extend the binding registry, so all boundaries
    // finish that serial mutation phase before the registry becomes the immutable frontend view.
    let mut external_imports = source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut builder_surface.binding_packages,
        providers: &builder_surface.external_import_providers,
        cache: &mut builder_surface.external_import_cache,
        resolution_table: &mut builder_surface.external_dependency_resolution_table,
    };
    let mut resource_inputs = ResourceInputRegistry::new();

    let mut source_package_inventories = Vec::new();
    for (dependency_prefix, package_index) in project_setup
        .module_namespace_set
        .source_package_boundaries()
    {
        // Register the package boundary before its inventory so inventory and compile
        // observations share one dense id for the human boundary total.
        #[cfg(feature = "timers")]
        let timing_boundary = crate::timing::register_timing_boundary(
            crate::timing::TimingBoundaryKind::SourcePackage,
            || format!("@{dependency_prefix}"),
        );
        let mut package_graph = ProjectModuleGraph::from_source_tree_index(package_index);
        let package_path_resolver = project_path_resolver.for_source_package_boundary(
            package_index.entry_root().to_path_buf(),
            package_index
                .module_identities()
                .derive_compilation_root_table(),
        );
        let package_resolution = DirectoryDependencyResolution::package(
            &project_setup.module_namespace_set,
            dependency_prefix,
            package_index,
        );
        timing_scope_attributed!(
            timing_guard_build_boundary_inventory_2,
            crate::timing::TimingMetric::BoundaryInventory,
            Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
        );
        let package_waves = match module_inventory::discover_all_modules_in_package_with_check_only(
            config,
            &package_path_resolver,
            &mut package_graph,
            style_directives,
            &mut external_imports,
            package_resolution,
            &mut resource_inputs,
            mode.includes_check_only(),
            string_table,
            #[cfg(feature = "timers")]
            timing_boundary,
        ) {
            Ok(module_waves) => module_waves,
            Err(messages) => {
                return Err(messages);
            }
        };
        // Merge canonical contract locations before any transient package job forks its string
        // table. Every later transient fact can then share this boundary prefix safely.
        let canonical_source_facts = config_boundary::source_contract_facts_from_module_waves(
            package_waves.waves(),
            string_table,
        );
        let root_module_id = package_index
            .module_identities()
            .module_id_for_directory(package_index.entry_root())
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "Source package @{dependency_prefix} has no module rooted at its indexed entry root"
                    )),
                    string_table,
                )
            })?;
        source_package_inventories.push(SourcePackageModuleInventory {
            dependency_prefix: dependency_prefix.to_owned(),
            package_identity: package_index.stable_package_identity().clone(),
            root_module_id,
            path_resolver: package_path_resolver,
            graph: package_graph,
            schedule: package_waves,
            canonical_source_facts,
            #[cfg(feature = "timers")]
            timing_boundary,
        });
    }

    // Register the main-project boundary before its inventory so its accumulated total is
    // attributed separately from every source package.
    #[cfg(feature = "timers")]
    let project_timing_boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || config.project_name.clone(),
    );

    let directory_dependency_resolution = DirectoryDependencyResolution::project(
        &project_setup.module_namespace_set,
        &project_setup.source_tree_index,
    );
    timing_scope_attributed!(
        timing_guard_build_boundary_inventory_3,
        crate::timing::TimingMetric::BoundaryInventory,
        Some(crate::timing::TimingContext::for_boundary(
            project_timing_boundary
        )),
    );
    let mut project_schedule =
        match module_inventory::discover_all_modules_in_project_with_check_only(
            config,
            &project_path_resolver,
            &mut project_setup.project_module_graph,
            style_directives,
            &mut external_imports,
            directory_dependency_resolution,
            &mut resource_inputs,
            mode.includes_check_only(),
            string_table,
            #[cfg(feature = "timers")]
            project_timing_boundary,
        ) {
            Ok(schedule) => schedule,
            Err(messages) => {
                return Err(messages);
            }
        };
    // Merge all canonical project contract locations before transient jobs fork their local
    // string-table base. Project fixed/direct fields are also materialized now so their locations
    // belong to the same inherited prefix used by every check-only job.
    let project_source_facts = config_boundary::source_contract_facts_from_module_waves(
        project_schedule.waves(),
        string_table,
    );
    let effective_project_fields = config_boundary::effective_project_fields(config, string_table)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let fixed_project_facts =
        config_boundary::fixed_project_contract_facts(&effective_project_fields);
    let direct_project_facts =
        config_boundary::direct_project_contract_facts(&effective_project_fields);
    let project_fallback = config.setting_location_or_config_file("project", string_table);
    // All canonical project and source-package inventories are complete now. Prepare transient
    // jobs only after that global provider-discovery barrier so each job forks final canonical
    // external package/cache/resolution state.
    if mode.includes_check_only() {
        for inventory in &mut source_package_inventories {
            let Some((_, package_index)) = project_setup
                .module_namespace_set
                .source_package_boundaries()
                .find(|(prefix, _)| *prefix == inventory.dependency_prefix.as_str())
            else {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "Source package @{} disappeared before deferred check-only preparation",
                        inventory.dependency_prefix
                    )),
                    string_table,
                ));
            };
            let package_resolution = DirectoryDependencyResolution::package(
                &project_setup.module_namespace_set,
                inventory.dependency_prefix.as_str(),
                package_index,
            );
            inventory.schedule.prepare_check_only_jobs(
                style_directives,
                &inventory.path_resolver,
                &mut external_imports,
                package_resolution,
                string_table,
            )?;
        }
    }
    if mode.includes_check_only() {
        project_schedule.prepare_check_only_jobs(
            style_directives,
            &project_path_resolver,
            &mut external_imports,
            directory_dependency_resolution,
            string_table,
        )?;
    }

    let (
        project_module_waves,
        project_provider_bindings,
        project_source_package_dependencies,
        project_check_only_jobs,
    ) = project_schedule.into_parts();
    let mut all_project_source_facts = project_source_facts.clone();
    if mode.includes_check_only() {
        all_project_source_facts.extend(
            config_boundary::source_contract_facts_from_check_only_jobs(
                &project_check_only_jobs,
                string_table,
            ),
        );
    }
    // Canonical resolution must use only canonical source facts, but explicit inputs are checked
    // against the full analyzed union after canonical values have validated successfully. This
    // lets a check-only-only name make an input known without retaining that transient contract.
    let canonical_project_inputs = config_boundary::filter_build_config_inputs_to_known_facts(
        build_config_inputs,
        &project_source_facts,
        &direct_project_facts,
    );
    let project_build_config_values = config_boundary::resolve_boundary_build_config(
        &project_source_facts,
        &fixed_project_facts,
        &direct_project_facts,
        &canonical_project_inputs,
        &config_globals,
        project_fallback.clone(),
        string_table,
    )?;
    if let Some(input) = config_boundary::first_unknown_build_config_input(
        build_config_inputs,
        &all_project_source_facts,
        &direct_project_facts,
    ) {
        return Err(config_boundary::build_config_resolution_messages(
            BuildConfigResolutionError::UnknownExplicitInput { input },
            project_fallback,
            string_table,
        ));
    }
    let project_globals = config_boundary::build_project_globals_interface(
        config,
        &effective_project_fields,
        string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    #[cfg(feature = "timers")]
    timing_guard_build_boundary_inventory_3.finish();
    let source_package_inventories =
        order_source_package_inventories(source_package_inventories, string_table)?;
    #[cfg(feature = "timers")]
    timing_guard_stage0_directory_inventory.finish();

    // Share the effective external package registry immutably across all boundary compilations;
    // the serial module scheduler can safely read the same Arc for every directory module.
    let external_packages = Arc::new(builder_surface.binding_packages.clone());

    // 3. Compile source packages in package-dependency order, then compile the project against
    // their immutable facade interfaces. Each boundary owns independent dense IDs, graphs and
    // provider stores; only the stable public interface crosses into a consuming boundary.
    timing_scope!(
        timing_guard_stage0_directory_compile,
        crate::timing::TimingMetric::Stage0DirectoryCompile
    );
    let mut completed_source_packages = CompletedSourcePackageRegistry::new();
    let mut transient_messages = Vec::new();
    let mut source_package_check_only_inventories = Vec::new();
    for inventory in source_package_inventories {
        let SourcePackageModuleInventory {
            package_identity,
            root_module_id,
            path_resolver,
            graph,
            schedule,
            canonical_source_facts: source_facts,
            dependency_prefix,
            #[cfg(feature = "timers")]
            timing_boundary,
        } = inventory;
        let (module_waves, provider_bindings, source_package_dependencies, check_only_jobs) =
            schedule.into_parts();
        let package_inputs = BuildConfigInputSet::new();
        let package_fallback = SourceLocation::from_path(path_resolver.entry_root(), string_table);
        let build_config_values = config_boundary::resolve_boundary_build_config(
            &source_facts,
            &[],
            &[],
            &package_inputs,
            &config_globals,
            package_fallback,
            string_table,
        )?;
        let deferred_path_resolver = path_resolver.clone();
        let deferred_build_config_values = build_config_values.clone();
        timing_scope_attributed!(
            timing_guard_build_boundary_compile,
            crate::timing::TimingMetric::BoundaryCompile,
            Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
        );
        // Canonical package compilation is deliberately independent of the transient lane. In
        // particular, no check-only job may publish an artefact or make a package ready for
        // dependency scheduling.
        let (boundary, mut package_transient_messages) = compile_module_waves(
            BoundaryCompilationContext::new(
                config,
                build_profile,
                &path_resolver,
                style_directives,
                &external_packages,
                builder_surface,
                &completed_source_packages,
                build_config_values,
                source_facts.clone(),
                BuildConfigInputSet::new(),
                config_globals.clone(),
                Vec::new(),
                Vec::new(),
                None,
            ),
            graph,
            module_waves,
            Vec::new(),
            &provider_bindings,
            &source_package_dependencies,
            &mut resource_inputs,
            string_table,
        )?;
        transient_messages.append(&mut package_transient_messages);
        let mut dependency_prefixes = Vec::new();
        let mut seen_dependency_prefixes = FxHashSet::default();
        for dependency in &source_package_dependencies {
            // Several modules may depend on the same provider. Publication records one direct
            // package edge, while module-level dependency bindings retain every consumer binding.
            if seen_dependency_prefixes.insert(dependency.dependency_prefix.clone()) {
                dependency_prefixes.push(dependency.dependency_prefix.clone());
            }
        }
        let package = CompiledSourcePackage {
            package_identity,
            root_module_id,
            boundary,
        };
        let publication = completed_source_packages
            .preflight(&package, &dependency_prefixes)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        completed_source_packages.reserve_commit(&publication);
        completed_source_packages.commit(publication, package);
        if mode.includes_check_only() && !check_only_jobs.is_empty() {
            source_package_check_only_inventories.push(SourcePackageCheckOnlyInventory {
                dependency_prefix,
                path_resolver: deferred_path_resolver,
                check_only_jobs,
                provider_bindings,
                source_package_dependencies,
                canonical_source_facts: source_facts,
                build_config_values: deferred_build_config_values,
            });
        }
    }

    completed_source_packages
        .validate_dependency_edges()
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    // Every canonical package facade is now published. Run the deferred transient package jobs
    // against those immutable boundaries so their package providers can never affect Kahn
    // ordering or surface as a readiness infrastructure failure.
    for inventory in source_package_check_only_inventories {
        let SourcePackageCheckOnlyInventory {
            dependency_prefix,
            path_resolver,
            check_only_jobs,
            provider_bindings,
            source_package_dependencies,
            canonical_source_facts,
            build_config_values,
        } = inventory;
        let package_id = completed_source_packages
            .by_prefix(dependency_prefix.as_str())
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "deferred check-only source package @{} was not published",
                        dependency_prefix
                    )),
                    string_table,
                )
            })?;
        let package = completed_source_packages
            .package(package_id)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        let package_transient_messages = compile_check_only_jobs_after_canonical(
            BoundaryCompilationContext::new(
                config,
                build_profile,
                &path_resolver,
                style_directives,
                &external_packages,
                builder_surface,
                &completed_source_packages,
                build_config_values,
                canonical_source_facts,
                BuildConfigInputSet::new(),
                config_globals.clone(),
                Vec::new(),
                Vec::new(),
                None,
            ),
            &package.boundary.modules,
            &package.boundary.generated,
            check_only_jobs,
            &provider_bindings,
            &source_package_dependencies,
            string_table,
        )?;
        transient_messages.extend(package_transient_messages);
    }

    timing_scope_attributed!(
        timing_guard_build_boundary_compile_2,
        crate::timing::TimingMetric::BoundaryCompile,
        Some(crate::timing::TimingContext::for_boundary(
            project_timing_boundary
        )),
    );
    let (project_boundary, mut project_transient_messages) = compile_module_waves(
        BoundaryCompilationContext::new(
            config,
            build_profile,
            &project_path_resolver,
            style_directives,
            &external_packages,
            builder_surface,
            &completed_source_packages,
            project_build_config_values,
            project_source_facts,
            build_config_inputs.clone(),
            config_globals.clone(),
            fixed_project_facts.clone(),
            direct_project_facts.clone(),
            project_globals.as_ref(),
        ),
        project_setup.project_module_graph,
        project_module_waves,
        project_check_only_jobs,
        &project_provider_bindings,
        &project_source_package_dependencies,
        &mut resource_inputs,
        string_table,
    )?;
    transient_messages.append(&mut project_transient_messages);
    #[cfg(feature = "timers")]
    timing_guard_build_boundary_compile_2.finish();
    #[cfg(feature = "timers")]
    timing_guard_stage0_directory_compile.finish();
    ProjectFrontendCompilation::new_with_transient_messages(
        project_boundary,
        completed_source_packages,
        resource_inputs,
        transient_messages,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}
