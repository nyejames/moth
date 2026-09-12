//! Single-file and directory frontend compilation.
//!
//! WHAT: compiles project modules through the frontend pipeline for single-file and directory entries.
//! WHY: keeps public entry contracts and high-level boundary orchestration here while focused
//!      compilation internals live in private child modules.
use crate::{timing_scope, timing_scope_attributed};

use crate::build_system::output::ValidatedDirectoryOutputSettings;
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, GeneratedFunctionDelta, ModuleSemanticResult,
    ProviderMaterialisationRegistry,
};

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::build_config::{
    BuildConfigContractFact, BuildConfigInputSet, BuildConfigResolutionError,
    ResolvedBuildConfigMap,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::{PremergeDiagnosticBatch, PremergeFailure};
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{
    StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{SourceDatabase, SourceDatabaseBuilder};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::symbols::path_interner::PathInternerBuilder;

use crate::builder_surface::BuilderSurface;
use crate::projects::settings::Config;

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use super::FrontendCompilationMode;
use super::compiled_boundary::{
    CompiledSourcePackage, CompletedSourcePackageRegistry, PackageBoundaryId,
    ProjectFrontendCompilation, TransientPremergeBatch,
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
use super::source_loading::SelectedSourceTextMap;

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

/// Publish one successful compiler result through the shared boundary tail.
///
/// WHAT: merges the module-local string-table delta, remaps the module and generated records when
///       needed, and publishes the complete module result and its sidecars.
/// WHY: the build-system contract makes publication atomic, so this is the single
///      merge-remap-publish tail shared by canonical directory and synthetic single-file lanes.
#[allow(clippy::too_many_arguments)]
pub(super) fn publish_compiled_module(
    modules: &mut ModuleArtifactStore,
    generated: &mut BoundaryGeneratedFunctionStore,
    materialisations: &mut ProviderMaterialisationRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    module_id: ModuleId,
    expected_origin: &StableModuleOriginIdentity,
    compiled: ModuleSemanticResult,
    string_table_base_len: usize,
    path_base_len: usize,
    string_table: &mut StringTable,
    path_interner: &mut PathInternerBuilder,
) -> Result<(), CompilerError> {
    let remap = string_table.merge_delta_from(&compiled.string_table, string_table_base_len);
    debug_assert_eq!(compiled.path_fork.base_len(), path_base_len);
    let path_remap = path_interner
        .merge_delta_from(&compiled.path_fork, &remap)
        .map_err(|error| {
            CompilerError::compiler_error(format!("module path merge failed: {error:?}"))
        })?;
    let ModuleSemanticResult {
        mut module,
        mut generated_delta,
        resource_source_associations,
        string_table: _,
        path_fork: _,
        public_interface,
    } = compiled;
    // Path IDs become boundary identities at this merge point. Retain an immutable snapshot on
    // the executable lane so backend/render consumers can resolve names without a live fork.
    let merged_path_table = std::sync::Arc::new(path_interner.paths().clone());
    // Published templates retain the complete identity tables that issued their semantic IDs. A
    // later project/package boundary can rebase those tables into its requester fork before
    // materialising an imported generic.
    if let Some(context) = &mut module.metadata.materialisation_context {
        let source_string_table = std::sync::Arc::new(string_table.clone().freeze());
        std::sync::Arc::make_mut(context).install_identity_tables(
            Arc::clone(&merged_path_table),
            source_string_table,
        );
    }
    module.executable.path_table = std::sync::Arc::clone(&merged_path_table);
    generated_delta.install_path_table(merged_path_table);
    if !remap.is_identity() {
        module.remap_string_ids(&remap);
        generated_delta.remap_string_ids(&remap);
    }
    if !path_remap.is_identity() {
        module.remap_path_ids(&path_remap);
        generated_delta.remap_path_ids(&path_remap);
    }
    publish_module_and_generated(ModuleBoundaryPublication {
        modules,
        generated,
        materialisations,
        resource_inputs,
        module_id,
        expected_origin,
        artifact: CompiledModuleArtifact {
            module,
            interface: public_interface,
        },
        generated_delta,
        resource_source_associations,
    })
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

#[cfg(feature = "boracle")]
pub(crate) use single_file::compile_single_file_boracle_frontend;
/// Compile a single `.moth` file as its own module.
pub(crate) use single_file::compile_single_file_frontend_with_inputs;

// -------------------------
//  Directory Compilation
// -------------------------

struct SourcePackageModuleInventory {
    dependency_prefix: String,
    package_identity: StablePackageIdentity,
    root_module_id: ModuleId,
    path_resolver: ProjectPathResolver,
    source_files: SourceDatabaseBuilder,
    path_interner: PathInternerBuilder,
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
    source_files: SourceDatabaseBuilder,
    path_interner: PathInternerBuilder,
    check_only_jobs: Vec<module_inventory::CheckOnlyModuleCompilationJob>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    source_package_dependencies: Vec<ResolvedSourcePackageDependency>,
    /// Canonical source contracts used to resolve each deferred job independently.
    canonical_source_facts: Vec<BuildConfigContractFact>,
    build_config_values: ResolvedBuildConfigMap,
    /// Canonical batches retained until the package source finalizes and converts each
    /// batch exactly once with its snapshot.
    batches: Vec<PremergeDiagnosticBatch>,
}

/// Typed directory-orchestration failure retaining its source owner.
///
/// WHAT: pairs the premerge lane with the finished package database when the failure is
///       package-scoped, so the single public tail can attach the correct snapshot once.
/// WHY: package failures must render against their own snapshot while project failures render
///      against the project database finalized by the outer tail; carrying the owner here keeps
///      every intermediate site typed until that deliberate final conversion.
struct DirectoryPremergeFailure {
    failure: Box<PremergeFailure>,
    package_source: Option<Box<SourceDatabase>>,
}

impl DirectoryPremergeFailure {
    fn project(failure: PremergeFailure) -> Self {
        Self {
            failure: Box::new(failure),
            package_source: None,
        }
    }

    fn package(failure: PremergeFailure, source: SourceDatabase) -> Self {
        Self {
            failure: Box::new(failure),
            package_source: Some(Box::new(source)),
        }
    }
}

impl From<PremergeFailure> for DirectoryPremergeFailure {
    fn from(failure: PremergeFailure) -> Self {
        Self::project(failure)
    }
}

impl From<CompilerError> for DirectoryPremergeFailure {
    fn from(error: CompilerError) -> Self {
        Self::project(PremergeFailure::Infrastructure(error))
    }
}

// NOTE: no `From<CompilerMessages>` here by design. Stage 0 stays in the typed premerge lane;
// the public tail owns the single final vessel conversion.

/// Finish a package source owner beside a typed failure.
///
/// WHAT: moves the builder's finished database onto a package-scoped failure. When
///       finalization itself fails there is no snapshot to attach, so the original failure
///       stays authoritative and the finish failure is surfaced deterministically beside it:
///       a diagnosed batch keeps its diagnostics and table by move and carries the finish
///       failure on the mixed outer infrastructure lane, while an infrastructure error keeps
///       its type and metadata and chains the finish message.
/// WHY: the builder is consumed by the failed finish, yet the original failure still owns
///      diagnostics and tables no later stage can rebuild. Dropping either side would lose
///      the user's failure or the freeze failure silently; both stay observable in the one
///      project-scoped failure without cloning any diagnostic batch or string table.
fn finalize_package_failure(
    failure: PremergeFailure,
    builder: SourceDatabaseBuilder,
) -> DirectoryPremergeFailure {
    match builder.finish() {
        Ok(source) => DirectoryPremergeFailure::package(failure, source),
        Err(finish_error) => {
            DirectoryPremergeFailure::project(append_finish_failure(failure, finish_error))
        }
    }
}
/// Chain a failed source finalization beside an existing typed failure.
///
/// WHAT: keeps the semantic failure authoritative without cloning any diagnostic batch
///       or string table: a diagnosed batch keeps its diagnostics by move and carries the
///       finish failure on the mixed outer infrastructure lane, while an infrastructure
///       error keeps its type and metadata and chains the finish message behind
///       its own.
/// WHY: the builder is consumed by the failed finish, yet the original failure still
///      owns diagnostics and tables no later stage can rebuild. Every Stage 0 tail
///      shares this so a finish failure never replaces the failure it raced with.
pub(super) fn append_finish_failure(
    failure: PremergeFailure,
    finish_error: CompilerError,
) -> PremergeFailure {
    match failure {
        PremergeFailure::Diagnosed(batch) => PremergeFailure::Mixed {
            batch,
            error: finish_error,
        },
        PremergeFailure::Mixed {
            batch,
            error: original,
        } => {
            // Both sides stay observable: the original render identity wins, and the
            // finish message chains deterministically behind the original message.
            let mut chained = original;
            chained.msg = format!(
                "{original_msg}; package source finalization also failed: {finish_msg}",
                original_msg = chained.msg,
                finish_msg = finish_error.msg,
            );
            for (key, value) in finish_error.metadata {
                chained.metadata.entry(key).or_insert(value);
            }
            PremergeFailure::Mixed {
                batch,
                error: chained,
            }
        }
        PremergeFailure::Infrastructure(mut original) => {
            // Both sides stay observable: the original failure remains authoritative, and
            // the finish message chains deterministically behind the original message.
            original.msg = format!(
                "{original_msg}; package source finalization also failed: {finish_msg}",
                original_msg = original.msg,
                finish_msg = finish_error.msg,
            );
            for (key, value) in finish_error.metadata {
                original.metadata.entry(key).or_insert(value);
            }
            PremergeFailure::Infrastructure(original)
        }
    }
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
) -> Result<Vec<SourcePackageModuleInventory>, CompilerError> {
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

    let order = order_packages_by_dependency(&package_prefixes, &dependency_prefixes)?;
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
///
/// WHAT: owns the single final conversion from the premerge lane into the boundary vessel.
/// WHY: every intermediate directory site stays typed until here, so source attachment and
///      string-table finalization happen exactly once at this outer tail.
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
    match compile_directory_frontend_in_premerge_lane(
        config,
        build_profile,
        validated_output_settings,
        style_directives,
        builder_surface,
        string_table,
        project_source_files,
        build_config_inputs,
        mode,
    ) {
        Ok(compilation) => Ok(compilation),
        Err(directory_failure) => {
            if let Some(package_source) = directory_failure.package_source {
                return Err((*directory_failure.failure)
                    .into_messages_with_source(string_table, Arc::new(*package_source)));
            }
            let mut messages = (*directory_failure.failure).into_messages(string_table);
            if messages.source_database_for_diagnostic(0).is_none()
                && let Some(project_source) = project_source_files.as_ref()
            {
                messages.set_source_database(Arc::clone(project_source));
            }
            Err(messages)
        }
    }
}

/// Directory orchestration in the premerge lane.
///
/// WHAT: runs package/project discovery, source loading, deferred check-only preparation,
///       provider publication and graph validation with typed failures, retaining a finished
///       package snapshot beside package-scoped failures.
/// WHY: the public wrapper above owns the single final vessel conversion; this lane never
///      constructs it early and never clones a diagnostic table solely to transport a failure.
#[allow(clippy::too_many_arguments)]
fn compile_directory_frontend_in_premerge_lane(
    config: &Config,
    build_profile: FrontendBuildProfile,
    validated_output_settings: Option<&ValidatedDirectoryOutputSettings>,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    string_table: &mut StringTable,
    project_source_files: &mut Option<Arc<SourceDatabase>>,
    build_config_inputs: &BuildConfigInputSet,
    mode: FrontendCompilationMode,
) -> Result<ProjectFrontendCompilation, DirectoryPremergeFailure> {
    // Directory inventory owns graph construction, source-package discovery,
    // and deterministic package ordering before any module semantics run.
    timing_scope!(
        timing_guard_stage0_directory_inventory,
        crate::timing::TimingMetric::Stage0DirectoryInventory
    );

    // 1. Setup path resolution based on config settings.
    // `build_project_path_resolver_with_index` returns the typed premerge lane; retain it
    // beside the finished source owner and convert once at the outer tail.
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
        Err(failure) => {
            return Err(DirectoryPremergeFailure::project(failure));
        }
    };
    let project_path_resolver = project_setup.resolver;
    let project_registration_index = project_setup.source_tree_index.source_registration_index();
    let cached_source = match project_source_files.take() {
        Some(cached) => match Arc::try_unwrap(cached) {
            Ok(database) => database,
            Err(_) => {
                return Err(DirectoryPremergeFailure::project(
                    PremergeFailure::Infrastructure(CompilerError::compiler_error(
                        "project source database was unexpectedly shared before registration",
                    )),
                ));
            }
        },
        None => SourceDatabase::empty(),
    };
    let mut project_sources = SourceDatabaseBuilder::new(cached_source);
    let result = (|| -> Result<ProjectFrontendCompilation, DirectoryPremergeFailure> {
        project_sources
            .sources_mut()
            .append_ordered_registration_index(
                &project_registration_index,
                project_path_resolver.entry_root(),
                Some(&project_path_resolver),
                string_table,
            )?;
        let (project_source_files, mut project_source_spans) = project_sources.split();
        let mut project_path_interner = project_source_files.clone_path_builder();
        let project_path_fork = project_path_interner.fork_source().fork_for_module();
        let mut selected_source_texts = SelectedSourceTextMap::default();
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
            let package_registration_index = package_index.source_registration_index();
            let package_source_files = SourceDatabase::from_ordered_registration_index(
                &package_registration_index,
                package_path_resolver.entry_root(),
                Some(&package_path_resolver),
                string_table,
            )?;
            let mut package_sources = SourceDatabaseBuilder::new(package_source_files);
            let mut package_selected_source_texts = SelectedSourceTextMap::default();
            let (package_source_files, mut package_source_spans) = package_sources.split();
            let mut path_interner = package_source_files.clone_path_builder();
            let package_path_fork = path_interner.fork_source().fork_for_module();
            let package_resolution = DirectoryDependencyResolution::package(
                &project_setup.module_namespace_set,
                dependency_prefix,
                package_index,
                &package_path_fork,
            );
            timing_scope_attributed!(
                timing_guard_build_boundary_inventory_2,
                crate::timing::TimingMetric::BoundaryInventory,
                Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
            );
            let package_waves_result =
                module_inventory::discover_all_modules_in_package_with_check_only(
                    config,
                    &package_path_resolver,
                    package_source_files,
                    &mut package_source_spans,
                    &mut package_graph,
                    style_directives,
                    &mut external_imports,
                    package_resolution,
                    &mut resource_inputs,
                    mode.includes_check_only(),
                    &mut package_selected_source_texts,
                    string_table,
                    &mut path_interner,
                    #[cfg(feature = "timers")]
                    timing_boundary,
                );
            let _ = package_source_spans;
            let package_retain_result =
                package_selected_source_texts.retain_into(package_sources.sources_mut());
            package_sources.adopt_pending_span_builders();
            if let Err(error) = package_retain_result {
                return Err(finalize_package_failure(
                    PremergeFailure::Infrastructure(error),
                    package_sources,
                ));
            }
            let package_waves = match package_waves_result {
                Ok(module_waves) => module_waves,
                Err(failure) => {
                    return Err(finalize_package_failure(failure, package_sources));
                }
            };
            // Merge canonical contract spans before any transient package job forks its string
            // table. Every later transient fact can then share this boundary prefix safely.
            let canonical_source_facts = config_boundary::source_contract_facts_from_module_waves(
                package_waves.waves(),
                string_table,
            );
            let Some(root_module_id) = package_index
                .module_identities()
                .module_id_for_directory(package_index.entry_root())
            else {
                return Err(finalize_package_failure(
                    PremergeFailure::Infrastructure(CompilerError::compiler_error(format!(
                        "Source package @{dependency_prefix} has no module rooted at its indexed entry root"
                    ))),
                    package_sources,
                ));
            };
            source_package_inventories.push(SourcePackageModuleInventory {
                dependency_prefix: dependency_prefix.to_owned(),
                package_identity: package_index.stable_package_identity().clone(),
                root_module_id,
                path_resolver: package_path_resolver,
                source_files: package_sources,
                path_interner,
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
            &project_path_fork,
        );
        timing_scope_attributed!(
            timing_guard_build_boundary_inventory_3,
            crate::timing::TimingMetric::BoundaryInventory,
            Some(crate::timing::TimingContext::for_boundary(
                project_timing_boundary
            )),
        );
        let project_schedule_result =
            module_inventory::discover_all_modules_in_project_with_check_only(
                config,
                &project_path_resolver,
                project_source_files,
                &mut project_source_spans,
                &mut project_setup.project_module_graph,
                style_directives,
                &mut external_imports,
                directory_dependency_resolution,
                &mut resource_inputs,
                mode.includes_check_only(),
                &mut selected_source_texts,
                string_table,
                &mut project_path_interner,
                #[cfg(feature = "timers")]
                project_timing_boundary,
            );
        let mut project_schedule = match project_schedule_result {
            Ok(schedule) => schedule,
            Err(failure) => {
                let _ = project_source_spans;
                let retain_result =
                    selected_source_texts.retain_into(project_sources.sources_mut());
                project_sources.adopt_pending_span_builders();
                let failure = match retain_result {
                    Ok(()) => failure,
                    Err(error) => append_finish_failure(failure, error),
                };
                return Err(DirectoryPremergeFailure::project(failure));
            }
        };
        let effective_project_fields =
            config_boundary::effective_project_fields(config, string_table)?;
        let fixed_project_facts =
            config_boundary::fixed_project_contract_facts(&effective_project_fields);
        let direct_project_facts =
            config_boundary::direct_project_contract_facts(&effective_project_fields);
        let project_fallback = config.setting_span("project");
        // All canonical project and source-package inventories are complete now. Prepare transient
        // jobs only after that global provider-discovery barrier so each job forks final canonical
        // external package/cache/resolution state.
        if mode.includes_check_only() {
            for index in 0..source_package_inventories.len() {
                let inventory = &mut source_package_inventories[index];
                let Some((_, package_index)) = project_setup
                    .module_namespace_set
                    .source_package_boundaries()
                    .find(|(prefix, _)| *prefix == inventory.dependency_prefix.as_str())
                else {
                    return Err(CompilerError::compiler_error(format!(
                        "Source package @{} disappeared before deferred check-only preparation",
                        inventory.dependency_prefix
                    ))
                    .into());
                };
                let package_path_fork = inventory.path_interner.fork_source().fork_for_module();
                let package_resolution = DirectoryDependencyResolution::package(
                    &project_setup.module_namespace_set,
                    inventory.dependency_prefix.as_str(),
                    package_index,
                    &package_path_fork,
                );
                let (_, mut source_spans) = inventory.source_files.split();
                let mut selected_source_texts = SelectedSourceTextMap::default();
                let preparation_result = inventory.schedule.prepare_check_only_jobs(
                    style_directives,
                    &mut source_spans,
                    &inventory.path_resolver,
                    &mut external_imports,
                    package_resolution,
                    string_table,
                    &mut inventory.path_interner,
                    &mut selected_source_texts,
                );
                let _ = source_spans;
                let retain_result =
                    selected_source_texts.retain_into(inventory.source_files.sources_mut());
                inventory.source_files.adopt_pending_span_builders();
                match (preparation_result, retain_result) {
                    (Ok(()), Ok(())) => {}
                    (Ok(()), Err(error)) => {
                        let inventory = source_package_inventories.swap_remove(index);
                        return Err(finalize_package_failure(
                            PremergeFailure::Infrastructure(error),
                            inventory.source_files,
                        ));
                    }
                    (Err(failure), Ok(())) => {
                        let inventory = source_package_inventories.swap_remove(index);
                        return Err(finalize_package_failure(failure, inventory.source_files));
                    }
                    (Err(failure), Err(error)) => {
                        let inventory = source_package_inventories.swap_remove(index);
                        return Err(finalize_package_failure(
                            append_finish_failure(failure, error),
                            inventory.source_files,
                        ));
                    }
                }
            }
        }
        let project_check_only_result = if mode.includes_check_only() {
            project_schedule.prepare_check_only_jobs(
                style_directives,
                &mut project_source_spans,
                &project_path_resolver,
                &mut external_imports,
                directory_dependency_resolution,
                string_table,
                &mut project_path_interner,
                &mut selected_source_texts,
            )
        } else {
            Ok(())
        };
        let _ = project_source_spans;
        let project_retain_result =
            selected_source_texts.retain_into(project_sources.sources_mut());
        project_sources.adopt_pending_span_builders();
        match (project_check_only_result, project_retain_result) {
            (Ok(()), Ok(())) => {}
            (Ok(()), Err(error)) => {
                return Err(DirectoryPremergeFailure::project(
                    PremergeFailure::Infrastructure(error),
                ));
            }
            (Err(failure), Ok(())) => {
                return Err(DirectoryPremergeFailure::project(failure));
            }
            (Err(failure), Err(error)) => {
                return Err(DirectoryPremergeFailure::project(append_finish_failure(
                    failure, error,
                )));
            }
        }
        let project_source_files = Arc::clone(project_sources.sources());

        let (
            project_module_waves,
            project_provider_bindings,
            project_source_package_dependencies,
            project_check_only_jobs,
        ) = project_schedule.into_parts();
        let project_source_facts = config_boundary::source_contract_facts_from_module_waves(
            &project_module_waves,
            string_table,
        );
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
            project_fallback,
            string_table,
        )
        .map_err(DirectoryPremergeFailure::project)?;
        if let Some(input) = config_boundary::first_unknown_build_config_input(
            build_config_inputs,
            &all_project_source_facts,
            &direct_project_facts,
        ) {
            // Diagnosed config failures move the boundary table into the batch; this
            // path terminates, so no clone is needed to carry the diagnostic.
            let failure = config_boundary::build_config_resolution_failure(
                BuildConfigResolutionError::UnknownExplicitInput { input },
                project_fallback,
                string_table,
            );
            return Err(DirectoryPremergeFailure::project(failure));
        }
        let project_globals = config_boundary::build_project_globals_interface(
            config,
            &effective_project_fields,
            string_table,
        )?;
        let source_package_inventories =
            order_source_package_inventories(source_package_inventories)?;
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
        let mut transient_batches: Vec<TransientPremergeBatch> = Vec::new();
        let mut source_package_check_only_inventories = Vec::new();
        for inventory in source_package_inventories {
            let SourcePackageModuleInventory {
                package_identity,
                root_module_id,
                path_resolver,
                mut source_files,
                mut path_interner,
                graph,
                schedule,
                canonical_source_facts: source_facts,
                dependency_prefix,
                #[cfg(feature = "timers")]
                timing_boundary,
            } = inventory;
            let (module_waves, provider_bindings, source_package_dependencies, check_only_jobs) =
                schedule.into_parts();
            let result: Result<_, PremergeFailure> = (|| {
                let package_inputs = BuildConfigInputSet::new();
                let package_fallback_span = None;
                let build_config_values = config_boundary::resolve_boundary_build_config(
                    &source_facts,
                    &[],
                    &[],
                    &package_inputs,
                    &config_globals,
                    package_fallback_span,
                    string_table,
                )?;
                let deferred_build_config_values = build_config_values.clone();
                timing_scope_attributed!(
                    timing_guard_build_boundary_compile,
                    crate::timing::TimingMetric::BoundaryCompile,
                    Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
                );
                // Canonical package compilation is deliberately independent of the transient lane.
                // The typed lane returns batches; conversion happens once at the package
                // source-owner tail below.
                let (boundary, package_transient_batches) =
                    canonical::compile_module_waves_in_premerge_lane(
                        canonical::BoundaryCompilationContext::new(
                            config,
                            build_profile,
                            &path_resolver,
                            Arc::clone(source_files.sources()),
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
                        &mut path_interner,
                    )?;
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
                let publication =
                    completed_source_packages.preflight(&package, &dependency_prefixes)?;
                completed_source_packages.reserve_commit(&publication);
                completed_source_packages.commit(publication, package);
                Ok((deferred_build_config_values, package_transient_batches))
            })();
            let (build_config_values, batches) = match result {
                Ok(value) => value,
                Err(failure) => {
                    source_files.sources_mut().adopt_path_builder(path_interner);
                    return Err(finalize_package_failure(failure, source_files));
                }
            };
            source_package_check_only_inventories.push(SourcePackageCheckOnlyInventory {
                dependency_prefix,
                path_resolver,
                source_files,
                check_only_jobs,
                provider_bindings,
                source_package_dependencies,
                canonical_source_facts: source_facts,
                build_config_values,
                batches,
                path_interner,
            });
        }
        completed_source_packages.validate_dependency_edges()?;
        // Every canonical package facade is now published. Run the deferred transient package jobs
        // against those immutable boundaries so their package providers can never affect Kahn
        // ordering or surface as a readiness infrastructure failure.
        for inventory in source_package_check_only_inventories {
            let SourcePackageCheckOnlyInventory {
                dependency_prefix,
                mut source_files,
                path_resolver,
                check_only_jobs,
                provider_bindings,
                source_package_dependencies,
                canonical_source_facts,
                build_config_values,
                mut batches,
                mut path_interner,
            } = inventory;
            let result: Result<_, PremergeFailure> = (|| {
                let package_id = completed_source_packages
                    .by_prefix(dependency_prefix.as_str())
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "deferred check-only source package @{} was not published",
                            dependency_prefix
                        ))
                    })?;
                let package = completed_source_packages.package(package_id)?;
                let check_only_batches =
                    deferred_check_only::compile_check_only_jobs_after_canonical(
                        canonical::BoundaryCompilationContext::new(
                            config,
                            build_profile,
                            &path_resolver,
                            Arc::clone(source_files.sources()),
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
                        &mut path_interner,
                    )?;
                Ok((package_id, check_only_batches))
            })();
            // Finish the package source owner beside the deferred result so success batches
            // and failures convert/attach exactly once with this snapshot. A finished
            // source keeps a failure package-scoped; a failed finish keeps the deferred
            // failure authoritative and chains the finish failure beside it.
            source_files.sources_mut().adopt_path_builder(path_interner);
            let finish_outcome = source_files.finish();
            let (result, finalized_source) = match (result, finish_outcome) {
                (result, Ok(finished)) => (result, finished),
                (Ok(_), Err(finish_error)) => {
                    return Err(DirectoryPremergeFailure::project(
                        PremergeFailure::Infrastructure(finish_error),
                    ));
                }
                (Err(failure), Err(finish_error)) => {
                    return Err(DirectoryPremergeFailure::project(append_finish_failure(
                        failure,
                        finish_error,
                    )));
                }
            };
            let finalized = Arc::new(finalized_source);
            let (package_id, check_only_batches) = match result {
                Ok(value) => value,
                Err(failure) => {
                    let database = Arc::try_unwrap(finalized).map_err(|_| {
                        CompilerError::compiler_error(
                            "package source database was unexpectedly shared before failure attach",
                        )
                    })?;
                    return Err(DirectoryPremergeFailure::package(failure, database));
                }
            };
            // Publish the package snapshot for later frozen-identity extraction. The
            // registry holds the sole owner; transient batches retain only their local
            // tables and domain tags until the final render tail merges them exactly once.
            completed_source_packages.set_source_database(package_id, finalized)?;
            batches.extend(check_only_batches);
            for batch in batches {
                transient_batches.push(TransientPremergeBatch::package(package_id, batch));
            }
        }

        timing_scope_attributed!(
            timing_guard_build_boundary_compile_2,
            crate::timing::TimingMetric::BoundaryCompile,
            Some(crate::timing::TimingContext::for_boundary(
                project_timing_boundary
            )),
        );
        let (project_boundary, project_batches) = canonical::compile_module_waves_in_premerge_lane(
            canonical::BoundaryCompilationContext::new(
                config,
                build_profile,
                &project_path_resolver,
                project_source_files,
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
            &mut project_path_interner,
        )?;
        for batch in project_batches {
            transient_batches.push(TransientPremergeBatch::project(batch));
        }
        #[cfg(feature = "timers")]
        timing_guard_build_boundary_compile_2.finish();
        #[cfg(feature = "timers")]
        timing_guard_stage0_directory_compile.finish();

        project_sources
            .sources_mut()
            .adopt_path_builder(project_path_interner);
        Ok(ProjectFrontendCompilation::new_with_transient_messages(
            project_boundary,
            completed_source_packages,
            resource_inputs,
            transient_batches,
        )?)
    })();
    let finish_outcome = project_sources.finish();
    // Finalize the project source owner beside the semantic result. A finished source keeps
    // current attachment behavior; a failed finish has no snapshot, so the semantic
    // failure stays authoritative and the finish failure chains beside it for the
    // public tail to render. A successful result with a failed finish surfaces only
    // the finish infrastructure error.
    let (result, finalized) = match (result, finish_outcome) {
        (result, Ok(finished)) => (result, Arc::new(finished)),
        (Ok(_), Err(finish_error)) => {
            return Err(DirectoryPremergeFailure::project(
                PremergeFailure::Infrastructure(finish_error),
            ));
        }
        (Err(directory_failure), Err(finish_error)) => {
            let combined = append_finish_failure(*directory_failure.failure, finish_error);
            return Err(DirectoryPremergeFailure {
                failure: Box::new(combined),
                package_source: directory_failure.package_source,
            });
        }
    };
    *project_source_files = Some(finalized);
    result
}
