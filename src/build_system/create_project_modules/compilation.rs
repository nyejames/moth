//! Single-file and directory frontend compilation.
//!
//! WHAT: compiles project modules through the frontend pipeline for single-file and directory entries.
//! WHY: keeps public entry contracts and high-level boundary orchestration here while focused
//!      compilation internals live in private child modules.
use crate::compiler_frontend::compiler_messages::CompilerStage;
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
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureErrors, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::{
    BoundaryAccumulator, SuccessfulWarningBoundary, SuccessfulWarningPosition,
    SuccessfulWarningSink,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, PreparedFailure, SourceSpanCapacityResource,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{
    StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{
    SourceDatabase, SourceDatabaseBuilder, SourceDatabaseError, SourceId,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::path_interner::{PathInternerBuilder, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::builder_surface::BuilderSurface;
use crate::projects::settings::Config;

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use super::FrontendCompilationMode;
use super::compiled_boundary::{
    CompiledSourcePackage, CompletedSourcePackageRegistry, PackageBoundaryId,
    ProjectFrontendCompilation, TransientPreparedBatch,
};
use super::config_boundary;
use super::generated_store::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::ModuleArtifactStore;
use super::module_identity::ModuleId;
use super::module_inventory;
use super::module_namespace::{DirectoryDependencyResolution, ModuleNamespaceSet};
use super::project_module_graph::ProjectModuleGraph;
use super::project_roots;
use super::resource_inputs::ResourceInputRegistry;
use super::source_discovery;
use super::source_discovery::{
    ExternalImportDiscoveryState, ResolvedDependencyEdge, ResolvedSourcePackageDependency,
};
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
) -> Result<(), InfrastructureFailure> {
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
#[allow(
    clippy::too_many_arguments,
    reason = "publication keeps mutable module, generated-function, materialisation and resource owners, module identity and origin, the compiled result and transient warning bags, string/path base lengths, the command warning sink, and mutable boundary string/path tables separate"
)]
pub(super) fn publish_compiled_module(
    modules: &mut ModuleArtifactStore,
    generated: &mut BoundaryGeneratedFunctionStore,
    materialisations: &mut ProviderMaterialisationRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    module_id: ModuleId,
    expected_origin: &StableModuleOriginIdentity,
    compiled: ModuleSemanticResult,
    mut warnings: DiagnosticBag,
    mut generated_warning_bags: Vec<DiagnosticBag>,
    string_table_base_len: usize,
    path_base_len: usize,
    string_table: &mut StringTable,
    path_interner: &mut PathInternerBuilder,
    warning_sink: Option<&mut SuccessfulWarningSink<'_>>,
    warning_boundary: SuccessfulWarningBoundary,
) -> Result<(), PreparedFailure> {
    if compiled.generated_delta.records().len() != generated_warning_bags.len() {
        return Err(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            "generated sidecar warning count disagrees with completed records",
        )
        .into());
    }
    let remap = string_table.merge_delta_from(&compiled.string_table, string_table_base_len);
    debug_assert_eq!(compiled.path_fork.base_len(), path_base_len);
    // Authored exhaustion during module publication is a deterministic source-capacity
    // rejection: the module's own compiled paths are the authored entries that exhausted the
    // table. Structural merge mismatches stay infrastructure.
    let path_remap = path_interner
        .merge_delta_from(&compiled.path_fork, &remap)
        .map_err(|error| match error {
            crate::compiler_frontend::symbols::path_interner::PathInternError::TableFull => {
                let batch = PreparedDiagnosticBatch::capture_from_bag(
                    DiagnosticBag::from(token_lexical::source_span_capacity(
                        0,
                        u32::MAX,
                        SourceSpanCapacityResource::LogicalPath,
                        None,
                    )),
                    string_table.take_for_handoff(),
                    Some(&compiled.module.executable.type_environment),
                    None,
                );
                PreparedFailure::Diagnosed(batch)
            }
            error => PreparedFailure::Infrastructure(
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!("module path merge failed: {error:?}"),
                )
                .into(),
            ),
        })?;
    let ModuleSemanticResult {
        mut module,
        mut generated_delta,
        resource_source_associations,
        string_table: _,
        path_fork: _,
        public_interface,
    } = compiled;
    if !path_remap.is_identity() {
        module.remap_path_ids(&path_remap);
        generated_delta.remap_path_ids(&path_remap);
        warnings.remap_path_ids(&path_remap);
        for warning_bag in &mut generated_warning_bags {
            warning_bag.remap_path_ids(&path_remap);
        }
    }
    let mut module_warning_batch = None;
    let mut sidecar_warning_batches = Vec::new();
    if warning_sink.is_some() {
        let module_warnings = warnings.into_diagnostics();
        let generated_records = generated_delta.into_records();
        let sidecar_publication_base = generated.published_record_count();
        // `merge_delta_from` has already rebased local strings into the command-owned root table.
        // Capture typed facts while each owning environment is live, then stage by module and
        // sidecar publication order so task completion order cannot affect report bytes.
        let capture_rebased_warnings = |diagnostics, environment: &TypeEnvironment| {
            let mut batch = PreparedDiagnosticBatch::capture_from_bag(
                DiagnosticBag::from_diagnostics(diagnostics),
                StringTable::empty_unallocated(),
                Some(environment),
                None,
            );
            batch.rebase_strings(&remap);
            batch
        };
        module_warning_batch = (!module_warnings.is_empty()).then(|| {
            capture_rebased_warnings(module_warnings, &module.executable.type_environment)
        });
        for (index, (record, warning_bag)) in generated_records
            .iter()
            .zip(generated_warning_bags)
            .enumerate()
        {
            let diagnostics = warning_bag.into_diagnostics();
            if !diagnostics.is_empty() {
                sidecar_warning_batches.push((
                    sidecar_publication_base + index,
                    capture_rebased_warnings(
                        diagnostics,
                        &record.sidecar.module.executable.type_environment,
                    ),
                ));
            }
        }
        generated_delta = GeneratedFunctionDelta::from_records(generated_records);
    }
    // The executable lanes keep their construction placeholders; the single boundary-owned path
    // table is installed once after canonical publication. Prepared warning facts already use
    // root StringIds and their empty batch tables avoid another merge at the render boundary.
    if !remap.is_identity() {
        module.remap_string_ids(&remap);
        generated_delta.remap_string_ids(&remap);
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
    .map_err(PreparedFailure::from)?;
    if let Some(warning_sink) = warning_sink {
        if let Some(batch) = module_warning_batch {
            warning_sink.append(
                warning_boundary,
                SuccessfulWarningPosition::BaseModule(module_id.index()),
                batch,
            );
        }
        for (publication_index, batch) in sidecar_warning_batches {
            warning_sink.append(
                warning_boundary,
                SuccessfulWarningPosition::GeneratedSidecar(publication_index),
                batch,
            );
        }
    }
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
) -> Result<(), InfrastructureFailure> {
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
) -> Result<ProviderMaterialisationRegistry, InfrastructureFailure> {
    let mut registry = ProviderMaterialisationRegistry::default();
    let mut rows = Vec::new();
    for (identity, location) in completed_packages.materialisation_locations() {
        let package = completed_packages.package(location.package_id)?;
        let context = package
            .boundary
            .modules
            .materialisation_context_at(location.location);
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
        // Completed packages finished before this boundary compiled, so their retained
        // identity domains differ from this requester's fork. Marking every seed row
        // cross-boundary keeps the consuming requester re-interning by spelling.
        registry.publish(identity, context, template_index, true)?;
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

pub(super) struct SourcePackageModuleInventory {
    dependency_prefix: String,
    package_identity: StablePackageIdentity,
    root_module_id: ModuleId,
    path_resolver: ProjectPathResolver,
    source_files: SourceDatabaseBuilder,
    path_interner: PathInternerBuilder,
    graph: ProjectModuleGraph,
    schedule: module_inventory::ModuleCompilationSchedule,
    selected_source_ids: Vec<SourceId>,
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
    batches: Vec<PreparedDiagnosticBatch>,
}

/// Typed boundary-orchestration failure retaining its source owner.
///
/// The terminal source owner is attached after finalization, when these prepared failures can be
/// paired with the database that owns their spans. Every failure belongs to that owner and is
/// reported in order.
pub(super) struct BoundaryPremergeFailure {
    failures: Vec<PreparedFailure>,
    package_source: Option<Box<SourceDatabase>>,
}

impl BoundaryPremergeFailure {
    pub(super) fn project(failures: Vec<PreparedFailure>) -> Self {
        Self {
            failures,
            package_source: None,
        }
    }

    pub(super) fn package(failures: Vec<PreparedFailure>, source: SourceDatabase) -> Self {
        Self {
            failures,
            package_source: Some(Box::new(source)),
        }
    }
}

impl From<PreparedFailure> for BoundaryPremergeFailure {
    fn from(failure: PreparedFailure) -> Self {
        Self::project(vec![failure])
    }
}

impl From<InfrastructureFailure> for BoundaryPremergeFailure {
    fn from(error: InfrastructureFailure) -> Self {
        Self::project(vec![PreparedFailure::from(error)])
    }
}

impl From<SourceDatabaseError> for BoundaryPremergeFailure {
    fn from(error: SourceDatabaseError) -> Self {
        Self::from(PreparedFailure::from(error))
    }
}

fn append_prepared_failure(
    accumulator: &mut BoundaryAccumulator,
    failure: PreparedFailure,
    source: Option<Arc<SourceDatabase>>,
) {
    match failure {
        PreparedFailure::Diagnosed(batch) => {
            accumulator.append_prepared(batch, source.map(|source| (source, None)));
        }
        PreparedFailure::Infrastructure(errors) => {
            accumulator.append_infrastructure(errors);
        }
    }
}

/// Convert one typed boundary failure after its last source owner has finalized.
pub(super) fn premerge_failure_to_reports(
    failure: BoundaryPremergeFailure,
    warning_owner: &mut BoundaryAccumulator,
    project_source: Option<Arc<SourceDatabase>>,
) -> Result<DiagnosticReportSet, InfrastructureErrors> {
    let BoundaryPremergeFailure {
        failures,
        package_source,
    } = failure;
    let package_source = package_source.map(|source| Arc::new(*source));
    let failure_source = package_source.or_else(|| project_source.as_ref().map(Arc::clone));
    warning_owner.discard_successful_warnings();
    let mut accumulator = std::mem::replace(warning_owner, BoundaryAccumulator::new());
    if let Some(source) = project_source {
        accumulator.set_project_source_database(source);
    }
    for failure in failures {
        append_prepared_failure(&mut accumulator, failure, failure_source.clone());
    }
    // The accumulator must own the only source handles before it unwraps its source rows.
    drop(failure_source);
    accumulator.finish(InstallMode::Unfrozen)
}
/// Append a boundary-wide finish cause, giving infrastructure precedence over diagnostics.
///
/// WHAT: drops the boundary's diagnosed batches and appends the finish cause after its existing
///       infrastructure causes.
/// WHY: once infrastructure exists the accumulator reports infrastructure alone, so a diagnosed
///      batch cannot appear beside it.
pub(super) fn append_boundary_finish_failure(
    mut failure: BoundaryPremergeFailure,
    finish_error: InfrastructureFailure,
) -> BoundaryPremergeFailure {
    failure
        .failures
        .retain(|entry| matches!(entry, PreparedFailure::Infrastructure(_)));
    failure.failures.push(PreparedFailure::from(finish_error));
    failure
}

// Intermediate orchestration stays typed until the terminal wrapper has finalized the relevant
// source owner. That wrapper returns the `CompilationResult` with its frozen report set.

/// Finish a package source owner beside a typed failure.
///
/// WHAT: moves the builder's finished database onto a package-scoped failure. When
///       finalization itself fails there is no snapshot to attach, so the infrastructure failure
///       takes precedence: a diagnosed batch and its table are dropped, while an existing
///       infrastructure aggregate retains its causes and appends the finish failure.
/// WHY: the builder is consumed by the failed finish, so no diagnostic snapshot can be trusted
///      for the result. Returning infrastructure directly preserves all operational causes
///      without cloning any diagnostic batch or string table.
fn finalize_package_failure(
    failure: PreparedFailure,
    mut builder: SourceDatabaseBuilder,
) -> BoundaryPremergeFailure {
    if let Err(assembly_error) = builder.assemble_finalized_tables() {
        let failure = failure.with_infrastructure_cause(assembly_error);
        return BoundaryPremergeFailure::project(vec![failure]);
    }
    match builder.finish() {
        Ok(source) => BoundaryPremergeFailure::package(vec![failure], source),
        Err(finish_error) => {
            let failure = failure.with_infrastructure_cause(finish_error);
            BoundaryPremergeFailure::project(vec![failure])
        }
    }
}

/// Finalize any loaded source builders not already frozen by their owning preparation outcome.
///
/// Candidate identities include registered but unselected records, so only sources whose retained
/// snapshots exist have a span table to finalize. The source-set combines module candidates with
/// selected content sources that are not module candidates.
fn finalize_schedule_source_spans(
    source_files: &mut SourceDatabaseBuilder,
    schedule: Option<&module_inventory::ModuleCompilationSchedule>,
    selected_source_ids: &[SourceId],
) -> Result<(), InfrastructureFailure> {
    let candidate_source_ids = schedule
        .into_iter()
        .flat_map(module_inventory::ModuleCompilationSchedule::candidate_source_ids)
        .chain(selected_source_ids.iter().copied());

    for source_id in candidate_source_ids {
        if source_files.sources().retained_text(source_id).is_none()
            || source_files.is_source_finalized(source_id)?
        {
            continue;
        }
        source_files.finish_source(source_id)?;
    }
    Ok(())
}

fn finalize_remaining_source_package_inventories(
    mut failure: BoundaryPremergeFailure,
    inventories: Vec<SourcePackageModuleInventory>,
) -> BoundaryPremergeFailure {
    for mut inventory in inventories {
        if let Err(error) = finalize_schedule_source_spans(
            &mut inventory.source_files,
            Some(&inventory.schedule),
            &inventory.selected_source_ids,
        ) {
            failure = append_boundary_finish_failure(failure, error);
            continue;
        }
        if let Err(error) = inventory.source_files.assemble_finalized_tables() {
            failure = append_boundary_finish_failure(failure, error);
            continue;
        }
        if let Err(error) = inventory.source_files.finish() {
            failure = append_boundary_finish_failure(failure, error);
        }
    }
    failure
}

pub(super) fn finalize_source_package_inventories(
    inventories: &mut Vec<SourcePackageModuleInventory>,
) -> Result<(), BoundaryPremergeFailure> {
    for index in 0..inventories.len() {
        let finalization = {
            let inventory = &mut inventories[index];
            finalize_schedule_source_spans(
                &mut inventory.source_files,
                Some(&inventory.schedule),
                &inventory.selected_source_ids,
            )
        };
        if let Err(error) = finalization {
            let inventory = inventories.swap_remove(index);
            let failure =
                finalize_package_failure(PreparedFailure::from(error), inventory.source_files);
            return Err(finalize_remaining_source_package_inventories(
                failure,
                std::mem::take(inventories),
            ));
        }
    }
    Ok(())
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
) -> FxHashMap<(ModuleId, DependencyShellId), usize> {
    let mut index = FxHashMap::default();
    for (binding_index, binding) in provider_bindings.iter().enumerate() {
        let shell_id = binding.dependency_shell_id;
        let key = (binding.consumer_module_id, shell_id);
        if index.insert(key, binding_index).is_some() {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "ModuleId {} resolved dependency shell {:?} to more than one provider edge",
                binding.consumer_module_id.index(),
                shell_id
            );
        }
    }

    index
}

/// Index every resolved source-package dependency once by consumer module and retained shell.
pub(crate) fn build_source_package_dependency_index(
    provider_binding_index: &FxHashMap<(ModuleId, DependencyShellId), usize>,
    source_package_dependencies: &[ResolvedSourcePackageDependency],
) -> FxHashMap<(ModuleId, DependencyShellId), usize> {
    let mut index = FxHashMap::default();
    for (dependency_index, package_dependency) in source_package_dependencies.iter().enumerate() {
        let shell_id = package_dependency.dependency_shell_id;
        let key = (package_dependency.consumer_module_id, shell_id);
        if provider_binding_index.contains_key(&key) {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "ModuleId {} resolved dependency shell {:?} to both a provider module and a source package",
                package_dependency.consumer_module_id.index(),
                shell_id
            );
        }
        if index.insert(key, dependency_index).is_some() {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "ModuleId {} resolved dependency shell {:?} to more than one source-package dependency",
                package_dependency.consumer_module_id.index(),
                shell_id
            );
        }
    }

    index
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
) -> FxHashMap<ModuleId, Vec<PackageBoundaryId>> {
    let mut dependencies: FxHashMap<ModuleId, Vec<PackageBoundaryId>> = FxHashMap::default();

    for package_dependency in source_package_dependencies {
        let package_id = completed_packages
            .by_prefix(package_dependency.dependency_prefix.as_str())
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "ModuleId {} depends on unindexed source package @{}",
                    package_dependency.consumer_module_id.index(),
                    package_dependency.dependency_prefix
                )
            });
        dependencies
            .entry(package_dependency.consumer_module_id)
            .or_default()
            .push(package_id);
    }

    for package_ids in dependencies.values_mut() {
        package_ids.sort_unstable();
        package_ids.dedup();
    }

    dependencies
}

fn order_source_package_inventories(
    inventories: Vec<SourcePackageModuleInventory>,
) -> Result<Vec<SourcePackageModuleInventory>, BoundaryPremergeFailure> {
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

    let order = match order_packages_by_dependency(&package_prefixes, &dependency_prefixes) {
        Ok(order) => order,
        Err(error) => {
            let failure = BoundaryPremergeFailure::from(error);
            return Err(finalize_remaining_source_package_inventories(
                failure,
                inventories,
            ));
        }
    };
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
///      dependency sets per pass; cycles are user-authored config diagnostics.
pub(crate) fn order_packages_by_dependency(
    package_prefixes: &[String],
    dependency_prefixes: &[Vec<String>],
) -> Result<Vec<usize>, PreparedFailure> {
    let package_count = package_prefixes.len();
    if dependency_prefixes.len() != package_count {
        crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "package dependency schedule received {} packages but {} dependency entries",
            package_count,
            dependency_prefixes.len()
        );
    }

    let mut index_by_prefix: FxHashMap<&str, usize> = FxHashMap::default();
    for (index, prefix) in package_prefixes.iter().enumerate() {
        if index_by_prefix.insert(prefix.as_str(), index).is_some() {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "source package @{} appears more than once in the package inventory",
                prefix
            );
        }
    }

    // Build the deterministic dense dependency graph once: package -> direct consumers and the
    // indegree of each package over its provider edges.
    let mut consumer_lists: Vec<Vec<usize>> = vec![Vec::new(); package_count];
    let mut indegree: Vec<usize> = vec![0; package_count];
    for (index, dependencies) in dependency_prefixes.iter().enumerate() {
        let mut seen_providers: FxHashSet<usize> = FxHashSet::default();
        for dependency in dependencies {
            let Some(provider_index) = index_by_prefix.get(dependency.as_str()).copied() else {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Source package @{} depends on unindexed source package @{}",
                    package_prefixes[index],
                    dependency
                );
            };
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
        let blocked_index = package_on_dependency_cycle(&indegree, &consumer_lists);
        let mut strings = StringTable::new();
        let mut fork = PathInternerFork::empty();
        let path = fork
            .try_intern_portable_path(&package_prefixes[blocked_index], &mut strings)
            .map_err(|_| {
                let mut bag = DiagnosticBag::new();
                bag.push(token_lexical::logical_path_capacity());
                PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
                    bag,
                    StringTable::new(),
                    None,
                    None,
                ))
            })?;
        let mut bag = DiagnosticBag::new();
        bag.push(project_config::circular_dependency(path, None));
        return Err(PreparedFailure::Diagnosed(
            PreparedDiagnosticBatch::capture_from_bag(
                bag,
                strings,
                None,
                Some(std::sync::Arc::new(fork.snapshot_table())),
            ),
        ));
    }

    Ok(ordered)
}

/// Names one package on a dependency cycle left unordered by the topological sweep.
///
/// WHAT: walks from the first blocked package through blocked providers until one repeats.
/// WHY: the first blocked package may only depend on a cycle, so its index is not a witness.
///      Every blocked package keeps a blocked provider, so the walk cannot stop early, and the
///      repeated package is on the cycle.
fn package_on_dependency_cycle(indegree: &[usize], consumer_lists: &[Vec<usize>]) -> usize {
    let blocked = |index: usize| indegree[index] > 0;
    let mut visited = vec![false; indegree.len()];
    let mut current = indegree
        .iter()
        .position(|remaining| *remaining > 0)
        .expect("a package cycle leaves at least one blocked inventory");
    while !visited[current] {
        visited[current] = true;
        current = (0..indegree.len())
            .find(|&provider| blocked(provider) && consumer_lists[provider].contains(&current))
            .expect("every blocked package has a blocked provider");
    }
    current
}

/// Discover each registered source-backed package within its own indexed boundary.
///
/// WHAT: builds the package-local graph, source owner and canonical/check-only schedule using the
///       already completed namespace index and shared provider-discovery state.
/// WHY: directory and synthetic consumers must use the same Stage 0 package boundary path; only
///      the consumer's source discovery differs.
#[allow(
    clippy::too_many_arguments,
    reason = "package discovery keeps immutable boundary inputs and independently borrowed import/resource/string services explicit"
)]
pub(super) fn discover_source_package_inventories(
    config: &Config,
    namespace_set: &ModuleNamespaceSet,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
    mode: FrontendCompilationMode,
) -> Result<Vec<SourcePackageModuleInventory>, BoundaryPremergeFailure> {
    let mut inventories = Vec::new();
    for (dependency_prefix, package_index) in namespace_set.source_package_boundaries() {
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
        let path_interner = package_sources.sources_mut().fork_path_builder();
        let (package_source_files, mut package_source_spans) = package_sources.split();
        let package_path_fork = path_interner.fork_source().fork_for_module();
        let package_resolution = DirectoryDependencyResolution::package(
            namespace_set,
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
                external_imports,
                package_resolution,
                resource_inputs,
                mode.includes_check_only(),
                &mut package_selected_source_texts,
                string_table,
                &path_interner,
                #[cfg(feature = "timers")]
                timing_boundary,
            );
        let (package_waves, package_failure) = match package_waves_result {
            Ok(module_waves) => (Some(module_waves), None),
            Err(failure) => (None, Some(failure)),
        };
        let _ = package_source_spans;
        let package_selected_source_ids =
            package_selected_source_texts.selected_source_ids(package_sources.sources());
        let mut package_retain_path_fork = package_sources.sources().fork_path_interner();
        let package_retain_result = package_selected_source_texts
            .retain_into(package_sources.sources_mut(), &mut package_retain_path_fork);
        package_sources.adopt_pending_span_builders();
        if let Err(error) = package_retain_result {
            let failure =
                error.admit_into_owner(package_failure, string_table, &package_retain_path_fork);
            let failure = match finalize_schedule_source_spans(
                &mut package_sources,
                package_waves.as_ref(),
                &package_selected_source_ids,
            ) {
                Ok(()) => failure,
                Err(finish_error) => failure.with_infrastructure_cause(finish_error),
            };
            return Err(finalize_package_failure(failure, package_sources));
        }
        let Some(package_waves) = package_waves else {
            let failure = package_failure
                .expect("a failed package discovery must retain its prepared failure");
            let failure = match finalize_schedule_source_spans(
                &mut package_sources,
                None,
                &package_selected_source_ids,
            ) {
                Ok(()) => failure,
                Err(finish_error) => failure.with_infrastructure_cause(finish_error),
            };
            return Err(finalize_package_failure(failure, package_sources));
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
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Source package @{dependency_prefix} has no module rooted at its indexed entry root"
            );
        };
        inventories.push(SourcePackageModuleInventory {
            dependency_prefix: dependency_prefix.to_owned(),
            package_identity: package_index.stable_package_identity().clone(),
            root_module_id,
            path_resolver: package_path_resolver,
            source_files: package_sources,
            path_interner,
            graph: package_graph,
            schedule: package_waves,
            selected_source_ids: package_selected_source_ids,
            canonical_source_facts,
            #[cfg(feature = "timers")]
            timing_boundary,
        });
    }

    Ok(inventories)
}

/// Prepare package check-only jobs after every canonical package/provider discovery is complete.
///
/// Check-only package dependencies remain outside canonical ordering. Preparing them after the
/// discovery barrier gives each transient job the final provider registry without making its
/// dependencies affect which canonical facades publish first.
pub(super) fn prepare_source_package_check_only_jobs(
    inventories: &mut Vec<SourcePackageModuleInventory>,
    namespace_set: &ModuleNamespaceSet,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    string_table: &mut StringTable,
) -> Result<(), BoundaryPremergeFailure> {
    for index in 0..inventories.len() {
        let inventory = &mut inventories[index];
        let Some((_, package_index)) = namespace_set
            .source_package_boundaries()
            .find(|(prefix, _)| *prefix == inventory.dependency_prefix.as_str())
        else {
            let failure = BoundaryPremergeFailure::from(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Source package @{} disappeared before deferred check-only preparation",
                    inventory.dependency_prefix
                ),
            ));
            return Err(finalize_remaining_source_package_inventories(
                failure,
                std::mem::take(inventories),
            ));
        };
        let package_path_fork = inventory.path_interner.fork_source().fork_for_module();
        let package_resolution = DirectoryDependencyResolution::package(
            namespace_set,
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
            external_imports,
            package_resolution,
            string_table,
            &inventory.path_interner,
            &mut selected_source_texts,
        );
        let _ = source_spans;
        let check_only_source_ids =
            selected_source_texts.selected_source_ids(inventory.source_files.sources());
        inventory.selected_source_ids.extend(check_only_source_ids);
        let mut retain_path_fork = inventory.source_files.sources().fork_path_interner();
        let retain_result = selected_source_texts
            .retain_into(inventory.source_files.sources_mut(), &mut retain_path_fork);
        if retain_result.is_ok() {
            inventory.source_files.adopt_pending_span_builders();
        }
        let owner = preparation_result.err();
        let owner = match retain_result {
            Ok(()) => owner,
            Err(error) => Some(error.admit_into_owner(owner, string_table, &retain_path_fork)),
        };
        let Some(failure) = owner else {
            continue;
        };
        let mut inventory = inventories.swap_remove(index);
        let failure = match finalize_schedule_source_spans(
            &mut inventory.source_files,
            Some(&inventory.schedule),
            &inventory.selected_source_ids,
        ) {
            Ok(()) => failure,
            Err(finish_error) => failure.with_infrastructure_cause(finish_error),
        };
        let failure = finalize_package_failure(failure, inventory.source_files);
        return Err(finalize_remaining_source_package_inventories(
            failure,
            std::mem::take(inventories),
        ));
    }

    Ok(())
}

pub(super) fn commit_successful_source_package_warnings(
    warning_sink: &mut SuccessfulWarningSink<'_>,
    package_index: usize,
    source_database: &Arc<SourceDatabase>,
) {
    warning_sink.commit_boundary(
        SuccessfulWarningBoundary::SourcePackage(package_index),
        Some(Arc::clone(source_database)),
    );
}

/// Compile source packages in provider order and publish their completed facades.
///
/// The package lane owns canonical publication, deferred check-only work and the package source
/// snapshots needed by later consumers. It returns only after all package owners are finalized.
#[allow(
    clippy::too_many_arguments,
    reason = "package publication borrows compilation settings, builder services and mutable resource/string owners separately"
)]
pub(super) fn compile_source_package_inventories(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &BuilderSurface,
    external_packages: &Arc<ExternalPackageRegistry>,
    inventories: Vec<SourcePackageModuleInventory>,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
    mut warning_sink: Option<&mut SuccessfulWarningSink<'_>>,
) -> Result<(CompletedSourcePackageRegistry, Vec<TransientPreparedBatch>), BoundaryPremergeFailure>
{
    let inventories = order_source_package_inventories(inventories)?;
    let config_globals = builder_surface.config_globals().clone();
    let mut completed_source_packages = CompletedSourcePackageRegistry::new();
    let mut transient_batches = Vec::new();
    let mut check_only_inventories = Vec::new();

    // Publish every canonical package before compiling transient package jobs. This keeps
    // check-only dependencies out of canonical package ordering and gives each job the complete
    // immutable facade registry.
    for (package_index, inventory) in inventories.into_iter().enumerate() {
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
            ..
        } = inventory;
        let (module_waves, provider_bindings, source_package_dependencies, check_only_jobs) =
            schedule.into_parts();
        if let Some(warning_sink) = warning_sink.as_deref_mut() {
            warning_sink.set_source_package_identity(
                SuccessfulWarningBoundary::SourcePackage(package_index),
                package_identity.clone(),
            );
        }
        let result: Result<_, PreparedFailure> = (|| {
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
            // Canonical package compilation stays independent of the transient lane. Convert its
            // typed batches once at the package source-owner tail below.
            let (boundary, package_transient_batches) =
                canonical::compile_module_waves_in_premerge_lane(
                    canonical::BoundaryCompilationContext::new(
                        config,
                        build_profile,
                        &path_resolver,
                        Arc::clone(source_files.sources()),
                        style_directives,
                        external_packages,
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
                    resource_inputs,
                    string_table,
                    &mut path_interner,
                    warning_sink.as_deref_mut(),
                    SuccessfulWarningBoundary::SourcePackage(package_index),
                )?;
            let mut dependency_prefixes = Vec::new();
            let mut seen_dependency_prefixes = FxHashSet::default();
            for dependency in &source_package_dependencies {
                // Several modules may depend on the same provider. Publication records one direct
                // package edge, while module-level bindings retain every consumer binding.
                if seen_dependency_prefixes.insert(dependency.dependency_prefix.clone()) {
                    dependency_prefixes.push(dependency.dependency_prefix.clone());
                }
            }
            // The package boundary owns its identity pair from here on: canonical publications
            // share one final table across base artefacts, generated sidecars and retained contexts.
            // Later boundaries seed from these contexts and mark them cross-boundary, so this
            // install must precede that seed and no re-install is possible.
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
        check_only_inventories.push(SourcePackageCheckOnlyInventory {
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
    for inventory in check_only_inventories {
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
            path_interner,
        } = inventory;
        let result: Result<_, PreparedFailure> = (|| {
            let package_id = completed_source_packages
                .by_prefix(dependency_prefix.as_str())
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "deferred check-only source package @{} was not published",
                            dependency_prefix
                        ),
                    )
                })?;
            let package = completed_source_packages.package(package_id)?;
            let check_only_batches = deferred_check_only::compile_check_only_jobs_after_canonical(
                canonical::BoundaryCompilationContext::new(
                    config,
                    build_profile,
                    &path_resolver,
                    Arc::clone(source_files.sources()),
                    style_directives,
                    external_packages,
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
                &path_interner,
            )?;
            Ok((package_id, check_only_batches))
        })();
        // Assemble tables frozen at each package source's last preparation producer, then consume
        // the source owner after all compilation views have dropped.
        source_files.sources_mut().adopt_path_builder(path_interner);
        let finish_outcome = source_files
            .assemble_finalized_tables()
            .and_then(|()| source_files.finish());
        let (result, finalized_source) = match (result, finish_outcome) {
            (result, Ok(finished)) => (result, finished),
            (Ok(_), Err(finish_error)) => {
                return Err(BoundaryPremergeFailure::project(vec![
                    PreparedFailure::from(finish_error),
                ]));
            }
            (Err(failure), Err(finish_error)) => {
                return Err(BoundaryPremergeFailure::project(vec![
                    failure.with_infrastructure_cause(finish_error),
                ]));
            }
        };
        let finalized = Arc::new(finalized_source);
        let (package_id, check_only_batches) = match result {
            Ok(value) => value,
            Err(failure) => {
                let database = Arc::try_unwrap(finalized).map_err(|_| {
                    InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        "package source database was unexpectedly shared before failure attach",
                    )
                })?;
                return Err(BoundaryPremergeFailure::package(vec![failure], database));
            }
        };
        if let Some(warning_sink) = warning_sink.as_deref_mut() {
            commit_successful_source_package_warnings(warning_sink, package_id.index(), &finalized);
        }
        completed_source_packages.set_source_database(package_id, finalized)?;
        batches.extend(check_only_batches);
        for batch in batches {
            transient_batches.push(TransientPreparedBatch::package(package_id, batch));
        }
    }

    Ok((completed_source_packages, transient_batches))
}

/// Discover all entry modules in a directory project and compile each one.
///
/// WHAT: owns the single final conversion from the premerge lane into an explicit outcome.
/// WHY: every intermediate directory site stays typed until here, so source attachment and
///      terminal diagnostic finalization happen exactly once at this outer tail.
#[allow(
    clippy::too_many_arguments,
    reason = "directory compilation keeps configuration, build profile, validated output settings, directives, mutable builder, warning/report, source, build-config inputs and compilation mode separate"
)]
pub(crate) fn compile_directory_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    validated_output_settings: Option<&ValidatedDirectoryOutputSettings>,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    warning_owner: &mut BoundaryAccumulator,
    project_source_files: &mut Option<Arc<SourceDatabase>>,
    project_source_builder: &mut Option<SourceDatabaseBuilder>,
    build_config_inputs: &BuildConfigInputSet,
    mode: FrontendCompilationMode,
) -> CompilationResult<ProjectFrontendCompilation> {
    let result = {
        let (string_table, mut warning_sink) = warning_owner.compiler_parts_mut();
        compile_directory_frontend_in_premerge_lane(
            config,
            build_profile,
            validated_output_settings,
            style_directives,
            builder_surface,
            string_table,
            &mut warning_sink,
            project_source_files,
            project_source_builder,
            build_config_inputs,
            mode,
        )
    };
    match result {
        Ok(compilation) => {
            compilation.into_compilation_result(warning_owner, project_source_files.take(), None)
        }
        Err(directory_failure) => match premerge_failure_to_reports(
            directory_failure,
            warning_owner,
            project_source_files.take(),
        ) {
            Ok(reports) => Ok(CompilationOutcome::Diagnosed(reports)),
            Err(errors) => Err(errors),
        },
    }
}

/// Directory orchestration in the premerge lane.
///
/// WHAT: runs package/project discovery, source loading, deferred check-only preparation,
///       provider publication and graph validation with typed failures, retaining a finished
///       package snapshot beside package-scoped failures.
/// WHY: the terminal wrapper above owns the single final boundary freeze; this lane keeps
///      prepared failures typed and avoids cloning a diagnostic table solely to transport them.
#[allow(
    clippy::too_many_arguments,
    reason = "premerge directory orchestration keeps configuration, profile, output settings, directives, mutable builder/string/source owners, build-config inputs and mode separate"
)]
fn compile_directory_frontend_in_premerge_lane(
    config: &Config,
    build_profile: FrontendBuildProfile,
    validated_output_settings: Option<&ValidatedDirectoryOutputSettings>,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    string_table: &mut StringTable,
    warning_sink: &mut SuccessfulWarningSink<'_>,
    project_source_files: &mut Option<Arc<SourceDatabase>>,
    project_source_builder: &mut Option<SourceDatabaseBuilder>,
    build_config_inputs: &BuildConfigInputSet,
    mode: FrontendCompilationMode,
) -> Result<ProjectFrontendCompilation, BoundaryPremergeFailure> {
    // Directory inventory owns graph construction, source-package discovery,
    // and deterministic package ordering before any module semantics run.
    timing_scope!(
        timing_guard_stage0_directory_inventory,
        crate::timing::TimingMetric::Stage0DirectoryInventory
    );

    let mut project_sources = project_source_builder
        .take()
        .unwrap_or_else(|| SourceDatabaseBuilder::new(SourceDatabase::empty()));
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
            let failure = BoundaryPremergeFailure::project(vec![failure]);
            let finish_outcome = project_sources
                .assemble_finalized_tables()
                .and_then(|()| project_sources.finish());
            return match finish_outcome {
                Ok(sources) => {
                    *project_source_files = Some(Arc::new(sources));
                    Err(failure)
                }
                Err(finish_error) => Err(append_boundary_finish_failure(failure, finish_error)),
            };
        }
    };
    let project_path_resolver = project_setup.resolver;
    let project_registration_index = project_setup.source_tree_index.source_registration_index();
    let result = (|| -> Result<ProjectFrontendCompilation, BoundaryPremergeFailure> {
        project_sources
            .sources_mut()
            .append_ordered_registration_index(
                &project_registration_index,
                project_path_resolver.entry_root(),
                Some(&project_path_resolver),
                string_table,
            )?;
        let mut project_path_interner = project_sources.sources_mut().fork_path_builder();
        let (project_source_files, mut project_source_spans) = project_sources.split();
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

        let mut source_package_inventories = discover_source_package_inventories(
            config,
            &project_setup.module_namespace_set,
            &project_path_resolver,
            style_directives,
            &mut external_imports,
            &mut resource_inputs,
            string_table,
            mode,
        )?;

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
                &project_path_interner,
                #[cfg(feature = "timers")]
                project_timing_boundary,
            );
        let mut project_schedule = match project_schedule_result {
            Ok(schedule) => schedule,
            Err(failure) => {
                let _ = project_source_spans;
                let project_selected_source_ids =
                    selected_source_texts.selected_source_ids(project_sources.sources());
                let mut project_retain_path_fork = project_sources.sources().fork_path_interner();
                let retain_result = selected_source_texts
                    .retain_into(project_sources.sources_mut(), &mut project_retain_path_fork);
                project_sources.adopt_pending_span_builders();
                let failure = match retain_result {
                    Ok(()) => failure,
                    Err(error) => error.admit_into_owner(
                        Some(failure),
                        string_table,
                        &project_retain_path_fork,
                    ),
                };
                let failure = match finalize_schedule_source_spans(
                    &mut project_sources,
                    None,
                    &project_selected_source_ids,
                ) {
                    Ok(()) => failure,
                    Err(error) => failure.with_infrastructure_cause(error),
                };
                let failure = BoundaryPremergeFailure::project(vec![failure]);
                return Err(finalize_remaining_source_package_inventories(
                    failure,
                    source_package_inventories,
                ));
            }
        };
        let _ = project_source_spans;
        let project_selected_source_ids =
            selected_source_texts.selected_source_ids(project_sources.sources());
        let mut project_retain_path_fork = project_sources.sources().fork_path_interner();
        let project_retain_result = selected_source_texts
            .retain_into(project_sources.sources_mut(), &mut project_retain_path_fork);
        project_sources.adopt_pending_span_builders();
        if let Err(error) = project_retain_result {
            let failure = error.admit_into_owner(None, string_table, &project_retain_path_fork);
            let failure = match finalize_schedule_source_spans(
                &mut project_sources,
                Some(&project_schedule),
                &project_selected_source_ids,
            ) {
                Ok(()) => failure,
                Err(finish_error) => failure.with_infrastructure_cause(finish_error),
            };
            return Err(BoundaryPremergeFailure::project(vec![failure]));
        }

        // All canonical project and source-package inventories are complete now. Prepare transient
        // jobs only after that global provider-discovery barrier so each job forks final canonical
        // external package/cache/resolution state.
        if mode.includes_check_only()
            && let Err(failure) = prepare_source_package_check_only_jobs(
                &mut source_package_inventories,
                &project_setup.module_namespace_set,
                style_directives,
                &mut external_imports,
                string_table,
            )
        {
            return Err(
                match finalize_schedule_source_spans(
                    &mut project_sources,
                    Some(&project_schedule),
                    &project_selected_source_ids,
                ) {
                    Ok(()) => failure,
                    Err(error) => append_boundary_finish_failure(failure, error),
                },
            );
        }
        if let Err(failure) = finalize_source_package_inventories(&mut source_package_inventories) {
            return Err(
                match finalize_schedule_source_spans(
                    &mut project_sources,
                    Some(&project_schedule),
                    &project_selected_source_ids,
                ) {
                    Ok(()) => failure,
                    Err(error) => append_boundary_finish_failure(failure, error),
                },
            );
        }

        let (_, mut project_source_spans) = project_sources.split();
        let mut project_check_only_source_texts = SelectedSourceTextMap::default();
        let project_check_only_result = if mode.includes_check_only() {
            project_schedule.prepare_check_only_jobs(
                style_directives,
                &mut project_source_spans,
                &project_path_resolver,
                &mut external_imports,
                directory_dependency_resolution,
                string_table,
                &project_path_interner,
                &mut project_check_only_source_texts,
            )
        } else {
            Ok(())
        };
        let _ = project_source_spans;
        let project_check_only_source_ids =
            project_check_only_source_texts.selected_source_ids(project_sources.sources());
        let mut project_check_only_path_fork = project_sources.sources().fork_path_interner();
        let project_check_only_retain_result = project_check_only_source_texts.retain_into(
            project_sources.sources_mut(),
            &mut project_check_only_path_fork,
        );
        project_sources.adopt_pending_span_builders();
        let mut all_project_source_ids = project_selected_source_ids;
        all_project_source_ids.extend(project_check_only_source_ids);
        let owner = project_check_only_result.err();
        let owner = match project_check_only_retain_result {
            Ok(()) => owner,
            Err(error) => {
                Some(error.admit_into_owner(owner, string_table, &project_check_only_path_fork))
            }
        };
        if let Some(failure) = owner {
            let failure = match finalize_schedule_source_spans(
                &mut project_sources,
                Some(&project_schedule),
                &all_project_source_ids,
            ) {
                Ok(()) => failure,
                Err(error) => failure.with_infrastructure_cause(error),
            };
            return Err(BoundaryPremergeFailure::project(vec![failure]));
        }
        if let Err(error) = finalize_schedule_source_spans(
            &mut project_sources,
            Some(&project_schedule),
            &all_project_source_ids,
        ) {
            return Err(BoundaryPremergeFailure::project(vec![
                PreparedFailure::from(error),
            ]));
        }

        let effective_project_fields =
            config_boundary::effective_project_fields(config, string_table)?;
        let fixed_project_facts =
            config_boundary::fixed_project_contract_facts(&effective_project_fields);
        let direct_project_facts = config_boundary::input_contract_facts(&effective_project_fields);
        let project_fallback = config.setting_span("project");
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
        .map_err(|failure| BoundaryPremergeFailure::project(vec![failure]))?;
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
            return Err(BoundaryPremergeFailure::project(vec![failure]));
        }
        let project_globals =
            config_boundary::build_project_globals_interface(config, &effective_project_fields)?;
        let external_packages = Arc::new(builder_surface.binding_packages.clone());
        #[cfg(feature = "timers")]
        timing_guard_stage0_directory_inventory.finish();

        // Compile every registered source package through the same boundary path used by
        // synthetic consumers, then compile this project's modules against the published facades.
        timing_scope!(
            timing_guard_stage0_directory_compile,
            crate::timing::TimingMetric::Stage0DirectoryCompile
        );
        let (completed_source_packages, mut transient_batches) =
            compile_source_package_inventories(
                config,
                build_profile,
                style_directives,
                builder_surface,
                &external_packages,
                source_package_inventories,
                &mut resource_inputs,
                string_table,
                Some(warning_sink),
            )?;

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
            Some(warning_sink),
            SuccessfulWarningBoundary::Project,
        )?;
        for batch in project_batches {
            transient_batches.push(TransientPreparedBatch::project(batch));
        }
        #[cfg(feature = "timers")]
        timing_guard_build_boundary_compile_2.finish();
        #[cfg(feature = "timers")]
        timing_guard_stage0_directory_compile.finish();

        project_sources
            .sources_mut()
            .adopt_path_builder(project_path_interner);
        Ok(ProjectFrontendCompilation::new(
            project_boundary,
            completed_source_packages,
            resource_inputs,
            transient_batches,
        )?)
    })();
    let finish_outcome = project_sources
        .assemble_finalized_tables()
        .and_then(|()| project_sources.finish());
    // Finalize the project source owner beside the semantic result. A finished source keeps
    // current attachment behavior; a failed finish has no snapshot, so the semantic
    // failure stays authoritative and the finish failure chains beside it for the
    // public tail to render. A successful result with a failed finish surfaces only
    // the finish infrastructure error.
    let (result, finalized) = match (result, finish_outcome) {
        (result, Ok(finished)) => (result, Arc::new(finished)),
        (Ok(_), Err(finish_error)) => {
            return Err(BoundaryPremergeFailure::project(vec![
                PreparedFailure::from(finish_error),
            ]));
        }
        (Err(directory_failure), Err(finish_error)) => {
            return Err(append_boundary_finish_failure(
                directory_failure,
                finish_error,
            ));
        }
    };
    *project_source_files = Some(finalized);
    result
}
