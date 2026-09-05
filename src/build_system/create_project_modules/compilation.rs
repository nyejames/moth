//! Single-file and directory frontend compilation.
//!
//! WHAT: compiles project modules through the frontend pipeline for single-file and directory entries.
//! WHY: keeps public entry contracts and high-level boundary orchestration here while focused
//!      compilation internals live in private child modules.
use crate::{timing_scope, timing_scope_attributed};

use crate::build_system::output::ValidatedDirectoryOutputSettings;
#[cfg(feature = "boracle")]
use crate::compiler_frontend::module_compilation::BoracleModuleInput;
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, GeneratedFunctionDelta, ProviderMaterialisationRegistry,
};

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::build_config::{
    BuildConfigContractFact, BuildConfigInputSet, BuildConfigResolutionError,
    ResolvedBuildConfigMap,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{
    StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::builder_surface::BuilderSurface;
use crate::projects::settings::Config;

use std::ffi::OsStr;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use super::FrontendCompilationMode;
use super::compiled_boundary::{
    CompiledSourcePackage, CompletedSourcePackageRegistry, PackageBoundaryId,
    ProjectFrontendCompilation,
};
use super::config_boundary;
use super::generated_store::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::ModuleArtifactStore;
use super::module_identity::ModuleId;
use super::module_inventory;
use super::module_namespace::DirectoryDependencyResolution;
use super::project_module_graph::ProjectModuleGraph;
use super::project_roots;
use super::resource_inputs::ResourceInputRegistry;
use super::source_discovery;
use super::source_discovery::{ResolvedDependencyEdge, ResolvedSourcePackageDependency};
use super::source_loading::load_registered_source_texts;

mod canonical;
mod deferred_check_only;
mod single_file;

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
    single_file::compile_single_file_frontend_with_inputs(
        config,
        build_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
        build_config_inputs,
        mode,
    )
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
    single_file::compile_single_file_boracle_frontend(
        config,
        build_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
    )
}

// -------------------------
//  Directory Compilation
// -------------------------

struct SourcePackageModuleInventory {
    dependency_prefix: String,
    package_identity: StablePackageIdentity,
    root_module_id: ModuleId,
    path_resolver: ProjectPathResolver,
    source_files: Arc<SourceDatabase>,
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
    source_files: Arc<SourceDatabase>,
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
    project_source_files: &mut Option<Arc<SourceDatabase>>,
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
    let project_registration_index = project_setup.source_tree_index.source_registration_index();
    let project_source_files =
        project_source_files.get_or_insert_with(|| Arc::new(SourceDatabase::empty()));
    let project_source_files_mut = Arc::get_mut(project_source_files).ok_or_else(|| {
        CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "project source database was shared before Stage 0 registration completed",
            ),
            string_table,
        )
    })?;
    project_source_files_mut
        .append_ordered_registration_index(
            &project_registration_index,
            project_path_resolver.entry_root(),
            Some(&project_path_resolver),
            string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    load_registered_source_texts(
        project_source_files_mut,
        &project_registration_index,
        string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let project_source_files = Arc::clone(project_source_files);
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
        let package_registration_index = package_index.source_registration_index();
        let mut package_source_files = SourceDatabase::from_ordered_registration_index(
            &package_registration_index,
            package_path_resolver.entry_root(),
            Some(&package_path_resolver),
            string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        load_registered_source_texts(
            &mut package_source_files,
            &package_registration_index,
            string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        let package_source_files = Arc::new(package_source_files);
        timing_scope_attributed!(
            timing_guard_build_boundary_inventory_2,
            crate::timing::TimingMetric::BoundaryInventory,
            Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
        );
        let package_waves = match module_inventory::discover_all_modules_in_package_with_check_only(
            config,
            &package_path_resolver,
            &package_source_files,
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
            source_files: package_source_files,
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
            &project_source_files,
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
                &inventory.source_files,
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
            &project_source_files,
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
            source_files,
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
        let (boundary, mut package_transient_messages) = canonical::compile_module_waves(
            canonical::BoundaryCompilationContext::new(
                config,
                build_profile,
                &path_resolver,
                Arc::clone(&source_files),
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
                source_files,
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
            source_files,
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
        let package_transient_messages =
            deferred_check_only::compile_check_only_jobs_after_canonical(
                canonical::BoundaryCompilationContext::new(
                    config,
                    build_profile,
                    &path_resolver,
                    Arc::clone(&source_files),
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
    let (project_boundary, mut project_transient_messages) = canonical::compile_module_waves(
        canonical::BoundaryCompilationContext::new(
            config,
            build_profile,
            &project_path_resolver,
            Arc::clone(&project_source_files),
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
