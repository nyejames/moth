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

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::module_diagnostics::ModuleDiagnostics;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
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

use super::compiled_boundary::{
    BlockedModule, BlockedProvider, CompiledGraphBoundary, CompiledSourcePackage,
    CompletedSourcePackageRegistry, DiagnosedModule, PackageBoundaryId, ProjectFrontendCompilation,
};
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

/// Compile a single `.moth` file as its own module.
pub(crate) fn compile_single_file_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    match compile_single_file_frontend_with_target(
        config,
        build_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
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

fn compile_single_file_frontend_with_target(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
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

    // Semantic compilation is one compiler service call. A synthetic single-file module has no
    // completed providers, so it binds against empty provider and materialisation views.
    let source_provider_dependencies = SourceProviderDependencySet::default();
    let mut provider_materialisations = ProviderMaterialisationRegistry::default();
    let mut generated_store = BoundaryGeneratedFunctionStore::default();
    let compile_context = ModuleCompilationContext {
        options: config.frontend_options(),
        build_profile,
        project_path_resolver: Some(project_path_resolver),
        style_directives,
        external_packages: Arc::clone(&external_packages),
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
    implicit_template_package_ids: Vec<PackageBoundaryId>,
}

impl<'a> BoundaryCompilationContext<'a> {
    fn new(
        config: &'a Config,
        build_profile: FrontendBuildProfile,
        project_path_resolver: &'a ProjectPathResolver,
        style_directives: &'a StyleDirectiveRegistry,
        external_packages: &'a Arc<ExternalPackageRegistry>,
        builder_surface: &'a BuilderSurface,
        completed_packages: &'a CompletedSourcePackageRegistry,
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
    module_waves: Vec<Vec<module_inventory::ModuleCompilationJob>>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    source_package_dependencies: Vec<ResolvedSourcePackageDependency>,
    #[cfg(feature = "timers")]
    timing_boundary: crate::timing::TimingBoundaryId,
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

impl<'boundary, 'services> DirectoryModuleCompileContext<'boundary, 'services> {
    /// Build the per-module provider input set by direct retained-shell lookup.
    ///
    /// WHAT: resolves every retained dependency shell through the boundary indexes built once per
    ///       graph, so binding never scans all edges, all source-package dependencies or all
    ///       completed packages for each shell.
    fn build_source_provider_dependencies(
        &self,
        consumer_module_id: ModuleId,
        prepared: &PreparedModule,
    ) -> Result<SourceProviderDependencySet<'boundary>, CompilerError> {
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

        // Semantic compilation is provider-dependent, so every required provider interface must
        // already be published before this call. The stage sequence behind it belongs to
        // `compile_module`; Stage 0 only guarantees the inputs.
        let source_provider_dependencies =
            match self.build_source_provider_dependencies(module_id, &prepared) {
                Ok(dependencies) => dependencies,
                Err(error) => {
                    return DirectoryModuleTaskResult {
                        module_id,
                        string_table_base_len: base_len,
                        outcome: DirectoryModuleTaskOutcome::Infrastructure(error),
                    };
                }
            };
        let compile_context = ModuleCompilationContext {
            options: self.boundary.config.frontend_options(),
            build_profile: self.boundary.build_profile,
            project_path_resolver: Some(self.boundary.project_path_resolver.clone()),
            style_directives: self.boundary.style_directives,
            external_packages: Arc::clone(self.boundary.external_packages),
            external_dependency_resolution_table: &self
                .boundary
                .builder_surface
                .external_dependency_resolution_table,
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
fn compile_module_waves(
    context: BoundaryCompilationContext<'_>,
    graph: ProjectModuleGraph,
    module_waves: Vec<Vec<module_inventory::ModuleCompilationJob>>,
    provider_bindings: &[ResolvedDependencyEdge],
    source_package_dependencies: &[ResolvedSourcePackageDependency],
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
) -> Result<CompiledGraphBoundary, CompilerMessages> {
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
                DirectoryModuleTaskOutcome::Infrastructure(error) => {
                    return Err(CompilerMessages::from_error_ref(error, string_table));
                }
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
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}

fn order_source_package_inventories(
    inventories: Vec<SourcePackageModuleInventory>,
    string_table: &StringTable,
) -> Result<Vec<SourcePackageModuleInventory>, CompilerMessages> {
    let package_prefixes = inventories
        .iter()
        .map(|inventory| inventory.dependency_prefix.clone())
        .collect::<Vec<_>>();
    let dependency_prefixes = inventories
        .iter()
        .map(|inventory| {
            inventory
                .source_package_dependencies
                .iter()
                .map(|dependency| dependency.dependency_prefix.clone())
                .collect::<Vec<_>>()
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
pub(crate) fn compile_directory_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    validated_output_settings: Option<&ValidatedDirectoryOutputSettings>,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    string_table: &mut StringTable,
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
        let package_waves = match module_inventory::discover_all_modules_in_package(
            config,
            &package_path_resolver,
            &mut package_graph,
            style_directives,
            &mut external_imports,
            package_resolution,
            &mut resource_inputs,
            string_table,
            #[cfg(feature = "timers")]
            timing_boundary,
        ) {
            Ok(module_waves) => module_waves,
            Err(messages) => {
                return Err(messages);
            }
        };
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
        let (module_waves, provider_bindings, source_package_dependencies) =
            package_waves.into_parts();

        source_package_inventories.push(SourcePackageModuleInventory {
            dependency_prefix: dependency_prefix.to_owned(),
            package_identity: package_index.stable_package_identity().clone(),
            root_module_id,
            path_resolver: package_path_resolver,
            graph: package_graph,
            module_waves,
            provider_bindings,
            source_package_dependencies,
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
    let module_waves = match module_inventory::discover_all_modules_in_project(
        config,
        &project_path_resolver,
        &mut project_setup.project_module_graph,
        style_directives,
        &mut external_imports,
        directory_dependency_resolution,
        &mut resource_inputs,
        string_table,
        #[cfg(feature = "timers")]
        project_timing_boundary,
    ) {
        Ok(module_waves) => module_waves,
        Err(messages) => {
            return Err(messages);
        }
    };
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
    for inventory in source_package_inventories {
        let SourcePackageModuleInventory {
            package_identity,
            root_module_id,
            path_resolver,
            graph,
            module_waves,
            provider_bindings,
            source_package_dependencies,
            dependency_prefix: _,
            #[cfg(feature = "timers")]
            timing_boundary,
        } = inventory;
        timing_scope_attributed!(
            timing_guard_build_boundary_compile,
            crate::timing::TimingMetric::BoundaryCompile,
            Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
        );
        let compiled = compile_module_waves(
            BoundaryCompilationContext::new(
                config,
                build_profile,
                &path_resolver,
                style_directives,
                &external_packages,
                builder_surface,
                &completed_source_packages,
            ),
            graph,
            module_waves,
            &provider_bindings,
            &source_package_dependencies,
            &mut resource_inputs,
            string_table,
        );
        let boundary = compiled?;
        #[cfg(feature = "timers")]
        timing_guard_build_boundary_compile.finish();
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
    }

    completed_source_packages
        .validate_dependency_edges()
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let (project_module_waves, project_provider_bindings, project_source_package_dependencies) =
        module_waves.into_parts();
    timing_scope_attributed!(
        timing_guard_build_boundary_compile_2,
        crate::timing::TimingMetric::BoundaryCompile,
        Some(crate::timing::TimingContext::for_boundary(
            project_timing_boundary
        )),
    );
    let compiled_project = compile_module_waves(
        BoundaryCompilationContext::new(
            config,
            build_profile,
            &project_path_resolver,
            style_directives,
            &external_packages,
            builder_surface,
            &completed_source_packages,
        ),
        project_setup.project_module_graph,
        project_module_waves,
        &project_provider_bindings,
        &project_source_package_dependencies,
        &mut resource_inputs,
        string_table,
    );
    let project_boundary = compiled_project?;
    #[cfg(feature = "timers")]
    timing_guard_build_boundary_compile_2.finish();
    #[cfg(feature = "timers")]
    timing_guard_stage0_directory_compile.finish();

    ProjectFrontendCompilation::new(project_boundary, completed_source_packages, resource_inputs)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}
