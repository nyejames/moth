//! Single-file and directory frontend compilation.
//!
//! WHAT: compiles project modules through the frontend pipeline for single-file and directory entries.
//! WHY: keeps public entry contracts and high-level boundary orchestration here while focused
//!      compilation internals live in private child modules.
use std::path::Path;

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::{timing_scope, timing_scope_attributed};

use crate::build_system::output::ValidatedDirectoryOutputSettings;
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, GeneratedFunctionDelta, ModuleSemanticResult,
    ProviderMaterialisationRegistry,
};

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
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{
    SourceCapacityError, SourceDatabase, SourceDatabaseBuilder, SourceDatabaseError, SourceId,
    SourceKind, SourceSpan,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::path_interner::{PathInternerBuilder, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::builder_surface::BuilderSurface;
use crate::builder_surface::SourceFileKind;
use crate::builder_surface::definition::CORE_SOURCE_PACKAGE_PREFIXES;
use crate::projects::settings::Config;

use std::collections::BTreeMap;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use super::FrontendCompilationMode;
use super::compiled_boundary::{
    BlockedProvider, CompiledSourcePackage, CompletedSourcePackageRegistry, PackageBoundaryId,
    ProjectDiagnosticBudget, ProjectFrontendCompilation, TransientPreparedBatch,
};
use super::config_boundary;
use super::facade_route_prepass;
use super::generated_store::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::{ModuleArtifactStore, ProviderSlot};
use super::module_identity::ModuleId;
use super::module_inventory;
use super::module_namespace::{
    DeferredPackageRouteCollision, DirectoryDependencyResolution, ModuleNamespaceSet,
    PackageRootFacadeSite, PackageRouteAttachment, PackageRouteDonorRequest,
    PackageRouteDonorRequests, PackageRouteSources, RoutePublicationFailure,
    deferred_package_collision_failure, resolve_deferred_package_route_site,
};
use super::project_module_graph::ProjectModuleGraph;
use super::project_roots;
use super::resource_inputs::ResourceInputRegistry;
use super::source_discovery;
use super::source_discovery::{
    ExternalImportDiscoveryState, ResolvedDependencyEdge, ResolvedSourcePackageDependency,
};
use super::source_loading::{SelectedSourceTextMap, SourceReadError, extract_source_code};
use super::source_tree_index::{ProjectConfigSource, SourceTreeProjectContext};
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;

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
    pub expected_normal_origin: Option<&'a StableModuleOriginIdentity>,
    pub expected_support_origin: Option<&'a StableModuleOriginIdentity>,
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
        expected_normal_origin,
        expected_support_origin,
        artifact,
        generated_delta,
        resource_source_associations,
    } = publication;
    // Every fallible check runs before anything mutates. The reservations and commits that follow
    // cannot fail, so a rejected publication leaves module, generated and resource registries
    // unchanged.
    let module_publication = modules.preflight_success(
        module_id,
        &artifact,
        expected_normal_origin,
        expected_support_origin,
    )?;
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
    expected_normal_origin: Option<&StableModuleOriginIdentity>,
    expected_support_origin: Option<&StableModuleOriginIdentity>,
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
        normal_interface,
        support_interface,
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
        expected_normal_origin,
        expected_support_origin,
        artifact: CompiledModuleArtifact {
            module,
            normal_interface,
            support_interface,
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

/// Add one newly published module's materialisation rows to the boundary registry.
///
/// WHY: later modules in this boundary materialise concrete generics and execute imported const
///      functions from their declaring module's validated context. The registry is the compiler's
///      immutable lookup for both; the store's own indexes stay behind for publication provenance
///      and duplicate detection.
fn publish_materialisation_templates(
    materialisations: &mut ProviderMaterialisationRegistry,
    artifact: &CompiledModuleArtifact,
) -> Result<(), InfrastructureFailure> {
    let Some(context) = artifact.module.metadata.materialisation_context.as_ref() else {
        return Ok(());
    };

    materialisations.publish_context(context)
}

/// Seed a boundary registry with every materialisation row completed source packages expose.
///
/// WHY: a project module may instantiate a package generic or fold a package const function, and
///      those packages finished before this boundary started. Const-only providers publish no
///      generic templates, so their retained bodies must be seeded through the same handoff.
///
/// The completed package registry already owns one immutable index of both row kinds. Borrowing
/// it keeps each boundary's seed O(1) in the number of completed packages; copying the rows here
/// would rehash the whole completed set once per boundary.
pub(crate) fn seed_completed_package_materialisations(
    completed_packages: &CompletedSourcePackageRegistry,
) -> ProviderMaterialisationRegistry {
    let mut registry = ProviderMaterialisationRegistry::default();
    registry.borrow_completed_index(completed_packages.materialisation_index());
    registry
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

/// Capture every prepared package's root facade while its source owner still exposes the slot.
///
/// WHAT: pairs each discovered package inventory with its indexed root facade span, physical path
///       and stable identity.
/// WHY: a project-local route may collide with a registered package prefix during project route
///      attachment. The package root facade's `SourceId` belongs to that package database, so the
///      report needs the authored site and identity before the project database can be consulted.
fn package_root_facade_sites(
    namespace_set: &ModuleNamespaceSet,
    inventories: &[SourcePackageModuleInventory],
) -> BTreeMap<String, PackageRootFacadeSite> {
    inventories
        .iter()
        .filter_map(|inventory| {
            let package_index =
                namespace_set.source_package_boundary(&inventory.dependency_prefix)?;
            PackageRootFacadeSite::from_package(
                package_index,
                inventory.package_identity.clone(),
                inventory.source_files.sources(),
            )
            .map(|site| (inventory.dependency_prefix.clone(), site))
        })
        .collect()
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

/// One package boundary retained as a cross-domain diagnostic report owner.
///
/// WHY: a cross-boundary route collision carries a package-local `$export` operand or root
///      facade span. That span belongs to the package source database and stable identity, so the
///      report needs both to render the site instead of reinterpreting the package `SourceId` in
///      the project DB.
struct BoundaryPackageDonor {
    source: Arc<SourceDatabase>,
    identity: StablePackageIdentity,
}

/// The finished report owners for a collision's primary and related sites.
///
/// A project collision needs only `related`; a collision between two registered packages needs
/// both. The sites are bounded by the diagnostic shape, so two named slots avoid a general
/// registry while keeping the rare failure type compact.
#[derive(Default)]
struct BoundaryPackageDonors {
    primary: Option<BoundaryPackageDonor>,
    related: Option<BoundaryPackageDonor>,
}

/// Typed boundary-orchestration failure retaining its source owner.
///
/// The terminal source owner is attached after finalization, when these prepared failures can be
/// paired with the database that owns their spans. Every failure belongs to that owner and is
/// reported in order. A cross-boundary route-collision label also retains its package owner as a
/// second frozen report domain, because its package-local `SourceId` must never be resolved in
/// the project database.
pub(super) struct BoundaryPremergeFailure {
    failures: Vec<PreparedFailure>,
    package_source: Option<Box<SourceDatabase>>,
    /// The finished package source owners a cross-boundary collision must render through.
    ///
    /// Boxed because this is the rare side of the failure type and the Err variant already sits
    /// near clippy's size boundary.
    package_donors: Option<Box<BoundaryPackageDonors>>,
    /// Requests for those owners, resolved in the remaining package inventories' own order.
    package_donor_requests: Option<Box<PackageRouteDonorRequests>>,
    /// Diagnosed failures that must render through a finished package database other than the
    /// primary `package_source`.
    package_failure_sources: Vec<(PreparedFailure, Arc<SourceDatabase>)>,
    /// Completed package report owners for diagnostics outside the route-collision shape.
    ///
    /// A single-file Boracle failure has no collision request, but its provider-owned primary or
    /// label still carries a named frozen handle. These donors register those identities before
    /// the report freezes.
    diagnostic_package_donors: Vec<BoundaryPackageDonor>,
}

impl BoundaryPremergeFailure {
    pub(super) fn project(failures: Vec<PreparedFailure>) -> Self {
        Self {
            failures,
            package_source: None,
            package_donors: None,
            package_donor_requests: None,
            package_failure_sources: Vec::new(),
            diagnostic_package_donors: Vec::new(),
        }
    }

    pub(super) fn package(failures: Vec<PreparedFailure>, source: SourceDatabase) -> Self {
        Self {
            failures,
            package_source: Some(Box::new(source)),
            package_donors: None,
            package_donor_requests: None,
            package_failure_sources: Vec::new(),
            diagnostic_package_donors: Vec::new(),
        }
    }

    /// Retain every completed package domain a diagnosed consumer may resolve through.
    #[cfg(feature = "boracle")]
    fn with_completed_package_donors(
        mut self,
        completed_packages: &CompletedSourcePackageRegistry,
    ) -> Self {
        self.diagnostic_package_donors = completed_packages
            .report_donors()
            .map(|(identity, source)| BoundaryPackageDonor {
                source: Arc::clone(source),
                identity: identity.clone(),
            })
            .collect();
        self
    }

    /// Record that a diagnosed collision needs package boundaries as foreign report domains.
    fn with_package_donor_requests(mut self, requests: PackageRouteDonorRequests) -> Self {
        self.package_donor_requests = Some(Box::new(requests));
        self
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

/// Register a package database and stable identity as a report domain without adding a diagnostic.
///
/// Frozen installation then resolves the diagnostic's named handles by identity instead of
/// reinterpreting their source IDs in the project database.
fn register_report_donor(accumulator: &mut BoundaryAccumulator, donor: BoundaryPackageDonor) {
    let donor_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::new(),
        StringTable::empty_unallocated(),
        None,
        None,
    );
    accumulator.append_prepared(donor_batch, Some((donor.source, Some(donor.identity))));
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
        package_donors,
        package_donor_requests,
        package_failure_sources,
        diagnostic_package_donors,
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
    for (failure, source) in package_failure_sources {
        append_prepared_failure(&mut accumulator, failure, Some(source));
    }
    if let Some(requests) = package_donor_requests {
        for (role, request) in [("primary", requests.primary), ("related", requests.related)]
            .into_iter()
            .filter_map(|(role, request)| request.map(|request| (role, request)))
        {
            // The reported collision still names a package `SourceId`. Without its finished
            // owner the site cannot be rendered; fail as an operational error rather than
            // silently dropping the site or reinterpreting the ID in the project database.
            accumulator.append_infrastructure(
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "route collision {role} package @{} never finalized its report source",
                        request.dependency_prefix
                    ),
                )
                .into(),
            );
        }
    }
    let can_freeze = package_donors
        .as_ref()
        .is_some_and(|donors| donors.primary.is_some() || donors.related.is_some())
        || !diagnostic_package_donors.is_empty();
    if let Some(donors) = package_donors {
        let BoundaryPackageDonors { primary, related } = *donors;
        for donor in [primary, related].into_iter().flatten() {
            register_report_donor(&mut accumulator, donor);
        }
    }
    for donor in diagnostic_package_donors {
        register_report_donor(&mut accumulator, donor);
    }
    // The accumulator must own the only source handles before it unwraps its source rows.
    drop(failure_source);
    warning_owner.merge_discovery_stops(accumulator.discovery_stops());
    let install = if can_freeze {
        InstallMode::Frozen
    } else {
        InstallMode::Unfrozen
    };
    accumulator.finish(install)
}
/// Append a boundary-wide finish cause, giving infrastructure precedence over diagnostics.
///
/// WHAT: drops the boundary's diagnosed batches and appends the finish cause after its existing
///       infrastructure causes.
/// WHY: once infrastructure exists the accumulator reports infrastructure alone, so a diagnosed
///      batch cannot appear beside it. Any package report owner those batches needed is released
///      with them.
pub(super) fn append_boundary_finish_failure(
    mut failure: BoundaryPremergeFailure,
    finish_error: InfrastructureFailure,
) -> BoundaryPremergeFailure {
    failure
        .failures
        .retain(|entry| matches!(entry, PreparedFailure::Infrastructure(_)));
    failure.failures.push(PreparedFailure::from(finish_error));
    failure.package_donors = None;
    failure.package_donor_requests = None;
    failure.diagnostic_package_donors.clear();
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

/// Finish the project source owner beside a facade-prepublish or namespace-building failure.
///
/// WHAT: retains the selected source snapshots the prepass loaded, adopts the span builders those
///       sources produced, finalizes every retained source and returns the failure in the project
///       premerge lane.
/// WHY: route prepublication runs after project source registration but before module discovery,
///       so its failures must leave the source owner in the same finished state as a failed
///       discovery. Reusing this shape keeps the boundary's one terminal freeze contract intact.
fn finalize_project_setup_failure(
    project_sources: &mut SourceDatabaseBuilder,
    selected_source_texts: &mut SelectedSourceTextMap,
    string_table: &mut StringTable,
    failure: PreparedFailure,
) -> BoundaryPremergeFailure {
    let selected_source_ids = selected_source_texts.selected_source_ids(project_sources.sources());
    let mut retain_path_fork = project_sources.sources().fork_path_interner();
    let retain_result =
        selected_source_texts.retain_into(project_sources.sources_mut(), &mut retain_path_fork);
    project_sources.adopt_pending_span_builders();
    let failure = match retain_result {
        Ok(()) => failure,
        Err(error) => error.admit_into_owner(Some(failure), string_table, &retain_path_fork),
    };
    let failure = match finalize_schedule_source_spans(project_sources, None, &selected_source_ids)
    {
        Ok(()) => failure,
        Err(error) => failure.with_infrastructure_cause(error),
    };
    BoundaryPremergeFailure::project(vec![failure])
}

/// Which authored site of a collision a finished package database resolves.
#[derive(Clone, Copy, Debug)]
enum BoundaryDonorRole {
    Primary,
    Related,
}

/// Take both roles' donor requests so an inventory can resolve every site it owns.
fn take_package_donor_requests(
    failure: &mut BoundaryPremergeFailure,
) -> Vec<(BoundaryDonorRole, PackageRouteDonorRequest)> {
    let Some(requests) = failure.package_donor_requests.take() else {
        return Vec::new();
    };
    [
        (BoundaryDonorRole::Primary, requests.primary),
        (BoundaryDonorRole::Related, requests.related),
    ]
    .into_iter()
    .filter_map(|(role, request)| request.map(|request| (role, request)))
    .collect()
}

/// Remove every donor request owned by `dependency_prefix`, preserving primary-before-related.
///
/// At most one prepared boundary owns each request, so the removed set is tiny and its order is
/// the reporting order from `take_package_donor_requests`.
fn take_matching_requests(
    requests: &mut Vec<(BoundaryDonorRole, PackageRouteDonorRequest)>,
    dependency_prefix: &str,
) -> Vec<(BoundaryDonorRole, PackageRouteDonorRequest)> {
    let mut matching = Vec::new();
    let mut index = 0;
    while index < requests.len() {
        if requests[index].1.dependency_prefix == dependency_prefix {
            matching.push(requests.remove(index));
        } else {
            index += 1;
        }
    }
    matching
}

fn attach_package_donor(
    failure: &mut BoundaryPremergeFailure,
    role: BoundaryDonorRole,
    request: PackageRouteDonorRequest,
    source: &Arc<SourceDatabase>,
) {
    let donors = failure
        .package_donors
        .get_or_insert_with(|| Box::new(BoundaryPackageDonors::default()));
    let donor = BoundaryPackageDonor {
        source: Arc::clone(source),
        identity: request.identity,
    };
    match role {
        BoundaryDonorRole::Primary => donors.primary = Some(donor),
        BoundaryDonorRole::Related => donors.related = Some(donor),
    }
}

/// Finish one colliding package boundary's source owner for a cross-domain diagnostic site.
///
/// WHY: package route operands and root facade sites carry package-local `SourceId` values. The
///      report must retain the package database and stable identity rather than translate that ID
///      through the project database. At project-namespace attachment the package inventories are
///      still builders, so this failure path finishes exactly the colliding boundaries while
///      leaving every other inventory to the normal prefix-ordered cleanup walk.
fn finish_package_donor(
    request: &PackageRouteDonorRequest,
    mut inventory: SourcePackageModuleInventory,
) -> Result<SourceDatabase, InfrastructureFailure> {
    debug_assert_eq!(inventory.dependency_prefix, request.dependency_prefix);
    if inventory.package_identity != request.identity {
        return Err(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "route collision package @{} changed its stable identity before report finalization",
                request.dependency_prefix
            ),
        ));
    }
    let provider_site = request
        .selected_site
        .as_deref()
        .map(|path| retain_package_donor_site(&mut inventory.source_files, path))
        .transpose()?
        .flatten();
    finalize_schedule_source_spans(
        &mut inventory.source_files,
        Some(&inventory.schedule),
        &inventory.selected_source_ids,
    )?;
    if let Some(source_id) = provider_site
        && !inventory.source_files.is_source_finalized(source_id)?
    {
        inventory.source_files.finish_source(source_id)?;
    }
    inventory.source_files.assemble_finalized_tables()?;
    inventory.source_files.finish()
}

/// Retain one namespace provider site in the package's own source database.
///
/// A provider visible in the package namespace need not be a selected module source, but the
/// frozen report needs a line index for its authored span. The slot already exists because the
/// package inventory registered every source; only its text is missing.
fn retain_package_donor_site(
    source_files: &mut SourceDatabaseBuilder,
    path: &Path,
) -> Result<Option<SourceId>, InfrastructureFailure> {
    let slot = source_files
        .sources()
        .get_by_canonical_path(path)
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "route collision provider site {} has no package source slot",
                    portable_path_text(path)
                ),
            )
        })?;
    let source_id = slot.id;
    let sources = source_files.sources();
    if sources.retained_text(source_id).is_some() {
        return Ok(Some(source_id));
    }
    // The related excerpt is optional report material: only a Pending slot is read here. A source
    // already recorded as invalid UTF-8, unreadable, or too large to address keeps the authored
    // collision and simply renders without an excerpt.
    if sources.is_source_non_utf8(source_id) {
        return Ok(None);
    }
    if sources.source_load_error(source_id).is_some() {
        return Ok(None);
    }
    if !sources.is_source_pending(source_id) {
        return Err(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "route collision provider site {} has no readable source state",
                portable_path_text(path)
            ),
        ));
    }
    let source = match extract_source_code(path) {
        Ok(source) => source,
        Err(SourceReadError::NonUtf8) => {
            source_files
                .sources_mut()
                .record_source_non_utf8(source_id)?;
            return Ok(None);
        }
        Err(SourceReadError::Infrastructure(error)) => return Err(error),
    };
    match source_files.sources_mut().retain_text(source_id, source) {
        Ok(()) => Ok(Some(source_id)),
        Err(SourceDatabaseError::Infrastructure(error)) => Err(error),
        Err(SourceDatabaseError::Capacity(SourceCapacityError::SourceSnapshotTooLarge)) => Ok(None),
        Err(SourceDatabaseError::Capacity(error)) => Err(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "route collision provider site {} exceeds source capacity: {error:?}",
                portable_path_text(path)
            ),
        )),
    }
}

fn finalize_remaining_source_package_inventories(
    mut failure: BoundaryPremergeFailure,
    inventories: Vec<SourcePackageModuleInventory>,
) -> BoundaryPremergeFailure {
    // Resolve collision donors during the existing prefix-ordered walk. An inventory is finished
    // once even when both sites belong to it, and a finish failure stays at that package's
    // position so an earlier inventory still owns the first cause.
    let mut requests = take_package_donor_requests(&mut failure);
    for mut inventory in inventories {
        let matching = take_matching_requests(&mut requests, &inventory.dependency_prefix);
        if !matching.is_empty() {
            let request = &matching[0].1;
            match finish_package_donor(request, inventory) {
                Ok(source) => {
                    let source = Arc::new(source);
                    for (role, request) in matching {
                        attach_package_donor(&mut failure, role, request, &source);
                    }
                }
                Err(error) => {
                    // A failed donor database cannot render its label. Infrastructure
                    // precedence already drops the diagnosed batch, so no site is silently
                    // reinterpreted through another domain.
                    failure = append_boundary_finish_failure(failure, error);
                    requests.clear();
                }
            }
            continue;
        }
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
    for (role, request) in requests {
        failure = append_boundary_finish_failure(
            failure,
            InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "route collision {role:?} package @{} was absent from the remaining inventory walk",
                    request.dependency_prefix
                ),
            ),
        );
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
        if !matches!(
            binding.provider_surface,
            crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal
                | crate::compiler_frontend::semantic_identity::ModuleRootRole::Support
        ) {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "ModuleId {} resolved a cross-module provider edge to a non-surface role {:?}",
                binding.consumer_module_id.index(),
                binding.provider_surface
            );
        }
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
        if let Some(previous_index) = index.insert(key, dependency_index) {
            // One authored clause names one package surface, so a duplicate consumer/shell key
            // means the same selection was resolved twice. Name both routes so the invariant
            // failure identifies which surface held the slot.
            let previous_route = &source_package_dependencies[previous_index].route;
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "ModuleId {} resolved dependency shell {:?} to more than one source-package dependency (routes {:?} and {:?})",
                package_dependency.consumer_module_id.index(),
                shell_id,
                previous_route.as_str(),
                package_dependency.route.as_str()
            );
        }
    }

    index
}

/// Index every consumer module's package providers once per boundary.
///
/// WHAT: groups each consumer module's authored source-package dependencies and, when the module
///       prepares a Moth template, the fixed template-prelude packages, as dense
///       [`PackageBoundaryId`] values sorted and deduplicated in package order.
/// WHY: readiness must walk only the current module's providers, so one grouped index keeps that
///      walk proportional to the direct inputs. Authored dependencies and the implicit prelude
///      follow the same classifier but arrive from different owners, so they stay separate
///      construction passes whose results join in this index rather than in the readiness loop.
///      The prelude providers are compiler policy and never appear in the authored dependency
///      vector. A `.mtf` authored inside one of the fixed prelude packages is unsupported content:
///      its own package cannot have completed yet, so that unresolved provider returns an internal
///      infrastructure failure naming the unfinished provider instead of a compiler bug. Sorting
///      by dense package identity also settles which failed provider a consumer reports first when
///      it observes more than one.
pub(crate) fn build_module_package_dependency_index(
    source_package_dependencies: &[ResolvedSourcePackageDependency],
    module_waves: &[Vec<module_inventory::ModuleCompilationJob>],
    completed_packages: &CompletedSourcePackageRegistry,
) -> Result<FxHashMap<ModuleId, Vec<PackageBoundaryId>>, InfrastructureFailure> {
    let mut dependencies: FxHashMap<ModuleId, Vec<PackageBoundaryId>> = FxHashMap::default();

    // Authored dependencies resolve through the prefix discovery retained on the graph edge.
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

    // Implicit prelude dependencies come from compiler policy, not the authored dependency vector.
    for job in module_waves.iter().flatten() {
        if !job.prepared.contains_moth_template {
            continue;
        }
        for prefix in CORE_SOURCE_PACKAGE_PREFIXES {
            let Some(package_id) = completed_packages.by_prefix(prefix) else {
                // A Moth template authored inside one installed prelude root cannot bind while
                // that root is still compiling: its own package is not published yet and the
                // fixed provider set has no ordering edge between the two roots. That package
                // content is unsupported rather than a proven-impossible state.
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "Moth template ModuleId {} cannot bind the fixed template prelude: Moth \
                         templates inside a fixed prelude package are unsupported, and prelude \
                         package @{} has not completed",
                        job.module_id.index(),
                        prefix
                    ),
                ));
            };
            dependencies
                .entry(job.module_id)
                .or_default()
                .push(package_id);
        }
    }

    for package_ids in dependencies.values_mut() {
        package_ids.sort_unstable();
        package_ids.dedup();
    }

    Ok(dependencies)
}

/// Classify one consumer's already-resolved source-package providers for readiness.
///
/// WHAT: returns the first diagnosed or blocked provider package, and raises the canonical
///       readiness invariant when a provider never completed its facade while the lane is open.
/// WHY: authored package dependencies and the implicit `.mtf` prelude providers share one rule.
///      A failed provider must block its consumer so the provider's diagnostic stays the primary
///      result instead of the consumer failing on a missing facade.
pub(crate) fn classify_blocked_package_provider(
    completed_packages: &CompletedSourcePackageRegistry,
    consumer_module_id: ModuleId,
    package_ids: impl IntoIterator<Item = PackageBoundaryId>,
) -> Result<Option<BlockedProvider>, InfrastructureFailure> {
    for package_id in package_ids {
        let package = completed_packages.package(package_id)?;
        match package.root_slot()? {
            ProviderSlot::Successful(_) => {}
            ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                return Ok(Some(BlockedProvider::SourcePackage(
                    package.package_identity.clone(),
                )));
            }
            ProviderSlot::Unavailable => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "ModuleId {} became ready before source package @{} completed its facade",
                        consumer_module_id.index(),
                        package.package_prefix()
                    ),
                ));
            }
        }
    }

    Ok(None)
}

fn inventory_contains_moth_template(inventory: &SourcePackageModuleInventory) -> bool {
    inventory.source_files.sources().iter().any(|slot| {
        matches!(
            slot.kind,
            Some(SourceKind::Compiler(SourceFileKind::MothTemplate))
        )
    })
}

/// Adds the compiler-owned ordering edges from template-bearing packages to the prelude packages.
///
/// WHAT: a source package that contains at least one `.mtf` source depends on every official Core
///       prelude package before its module boundary can bind the flat helper prelude.
/// WHY: authored package edges alone do not order the prelude packages; without these edges a
///      package such as `@docs` can publish before `@core/html` or `@core/css`, and prelude
///      interface lookup then fails even though the builder registered the official packages.
///
/// A prelude package is never given a self edge, and the prelude packages never depend on each
/// other: one fixed set of compiler-owned roots has no ordering relation of its own. A `.mtf`
/// authored inside a prelude package would therefore gain no edge to its companion. That case
/// cannot occur: both roots are compiler-shipped `Core`/`MothSource` packages whose committed
/// source trees contain no Moth template, and a builder or project package cannot replace them.
pub(crate) fn add_template_prelude_ordering_dependencies(
    package_prefixes: &[String],
    contains_moth_template: &[bool],
    dependencies: &mut [Vec<String>],
) {
    debug_assert_eq!(package_prefixes.len(), contains_moth_template.len());
    debug_assert_eq!(package_prefixes.len(), dependencies.len());
    let prelude_prefixes = CORE_SOURCE_PACKAGE_PREFIXES
        .into_iter()
        .filter(|prelude| package_prefixes.iter().any(|prefix| prefix == prelude))
        .collect::<Vec<_>>();
    if prelude_prefixes.is_empty() {
        return;
    }
    for ((prefix, has_template), dependencies) in package_prefixes
        .iter()
        .zip(contains_moth_template)
        .zip(dependencies)
    {
        // The fixed prelude packages order as one compiler-owned set: a template authored inside
        // one of them gains no self edge and no edge to the other. The canonical lookup reports
        // its missing companion instead of inventing a package cycle.
        if !has_template || prelude_prefixes.contains(&prefix.as_str()) {
            continue;
        }
        for prelude in &prelude_prefixes {
            if !dependencies.iter().any(|dependency| dependency == prelude) {
                dependencies.push((*prelude).to_owned());
            }
        }
    }
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
    let mut dependency_prefixes = inventories
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
    let contains_moth_template = inventories
        .iter()
        .map(inventory_contains_moth_template)
        .collect::<Vec<_>>();
    add_template_prelude_ordering_dependencies(
        &package_prefixes,
        &contains_moth_template,
        &mut dependency_prefixes,
    );

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

/// One registered package boundary whose facades are prepared and routes published.
///
/// WHAT: owns the package-local source owner, graph, path/string domains, prepared facade cache
///       and shared origin table between route prepublication and the module walk.
/// WHY: all package boundaries must publish their root facade routes before any package or
///      project consumer resolves a nested route. Splitting preparation from discovery keeps that
///      ordering to one pass over the registered prefixes and lets each walk adopt the exact
///      facade preparation instead of parsing it again.
struct SourcePackageBoundaryPreparation {
    dependency_prefix: String,
    package_identity: StablePackageIdentity,
    root_module_id: ModuleId,
    path_resolver: ProjectPathResolver,
    source_files: SourceDatabaseBuilder,
    path_interner: PathInternerBuilder,
    graph: ProjectModuleGraph,
    prepared_facades: facade_route_prepass::FacadePreparationCache,
    source_module_origins:
        Arc<crate::compiler_frontend::source_module_origin::SourceModuleOriginTable>,
    selected_source_texts: SelectedSourceTextMap,
    /// A prepass or route-publication failure reported when this boundary's walk turn arrives.
    prepass_failure: Option<PreparedFailure>,
    /// Root facade `$export` operands captured when this boundary's own attachment stopped.
    ///
    /// WHY: a later publisher may collide with one of these authored claims after this
    ///      boundary's attachment failed, so the failed routes stay out of import resolution
    ///      but remain available as the competing diagnostic site.
    failed_root_routes: Option<BTreeMap<String, SourceSpan>>,
    #[cfg(feature = "timers")]
    timing_boundary: crate::timing::TimingBoundaryId,
}

/// A cross-package collision detected before discovery but reported at its boundary's turn.
struct PendingRouteCollision {
    /// Preparation index of the first boundary whose route claims participate.
    preparation_index: usize,
    failure: PreparedFailure,
    package_donors: Option<Box<PackageRouteDonorRequests>>,
}

struct PreparedSourcePackageBoundaries {
    preparations: Vec<SourcePackageBoundaryPreparation>,
    /// Cross-package claim collision, reported at the earliest affected prefix's turn.
    route_collision: Option<PendingRouteCollision>,
    /// Package-internal prefix collision, reported at the publisher's turn as HEAD did.
    ///
    /// The competing site is resolved only after this collision's turn wins. Resolving a losing
    /// collision would read a provider that HEAD never reads, and a cached oversized snapshot can
    /// then fail an earlier winner's cleanup.
    package_collision: Option<DeferredPackageRouteCollision>,
}

/// Build the terminal failure for a collision detected before discovery.
fn pending_route_collision_failure(pending: PendingRouteCollision) -> BoundaryPremergeFailure {
    let PendingRouteCollision {
        failure: prepared,
        package_donors,
        ..
    } = pending;
    let failure = BoundaryPremergeFailure::project(vec![prepared]);
    match package_donors {
        Some(requests) => failure.with_package_donor_requests(*requests),
        None => failure,
    }
}

/// Prepare every registered package facade and publish its declared routes.
///
/// WHAT: builds each boundary's Stage 0 source owner and graph, runs the same facade route prepass
///       the project boundary uses, then inserts the boundary's route namespace entries and
///       retains its root support facade routes for external selection.
/// WHY: package-internal consumers resolve routes through their own namespaces, and external
///      consumers select the exact published root surface. Preparing all boundaries before any
///      package walk also makes a cross-package route reference independent of registration order.
fn prepare_source_package_boundaries(
    config: &Config,
    namespace_set: &mut ModuleNamespaceSet,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
) -> Result<PreparedSourcePackageBoundaries, BoundaryPremergeFailure> {
    let package_prefixes = namespace_set.source_package_prefixes();
    let mut preparations = Vec::with_capacity(package_prefixes.len());
    // Only the first attached prefix collision can report: HEAD stopped at it, so later
    // collisions are never read. One slot avoids retaining dead records.
    let mut deferred_package_collision: Option<DeferredPackageRouteCollision> = None;

    for dependency_prefix in package_prefixes {
        // Register the package boundary before its inventory so inventory and compile
        // observations share one dense id for the human boundary total.
        #[cfg(feature = "timers")]
        let timing_boundary = crate::timing::register_timing_boundary(
            crate::timing::TimingBoundaryKind::SourcePackage,
            || format!("@{dependency_prefix}"),
        );
        let package_index = namespace_set
            .source_package_boundary(&dependency_prefix)
            .expect("a prefix snapshot names one registered package boundary");
        let package_graph = ProjectModuleGraph::from_source_tree_index(package_index);
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
        let mut package_selected_source_texts = SelectedSourceTextMap::for_package_boundary(
            package_index.package_set_membership(),
            &dependency_prefix,
        );
        let path_interner = package_sources.sources_mut().fork_path_builder();
        timing_scope_attributed!(
            timing_guard_package_route_inventory,
            crate::timing::TimingMetric::BoundaryInventory,
            Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
        );
        let prepass_result = {
            let (package_source_files, mut package_source_spans) = package_sources.split();
            facade_route_prepass::prepublish_facade_routes(
                package_index,
                &package_graph,
                package_source_files,
                style_directives,
                config.frontend_options().max_errors_per_module,
                facade_route_prepass::FacadeDiagnosticOwner::BoundaryWalk,
                &mut package_source_spans,
                &mut package_selected_source_texts,
                string_table,
                &path_interner,
                #[cfg(feature = "timers")]
                timing_boundary,
            )
        };
        let prepass = match prepass_result {
            Ok(prepass) => prepass,
            Err(failure) => {
                #[cfg(feature = "timers")]
                timing_guard_package_route_inventory.finish();
                return Err(finalize_failed_package_preparation(
                    failure,
                    package_sources,
                    &mut package_selected_source_texts,
                    string_table,
                ));
            }
        };
        let facade_route_prepass::FacadeRoutePrepass {
            routes,
            cache,
            source_module_origins,
            unavailable_facades,
            failure: mut prepass_failure,
        } = prepass;
        // Derive every stable boundary handle before the mutable route publication borrow.
        let package_identity = package_index.stable_package_identity().clone();
        let root_module_id = package_index
            .module_identities()
            .module_id_for_directory(package_index.entry_root())
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Source package has no module rooted at its indexed entry root"
                )
            });
        let mut publication_failed = false;
        if prepass_failure.is_none() {
            match namespace_set.attach_package_facade_routes(
                &dependency_prefix,
                &routes,
                package_sources.sources(),
                &mut package_selected_source_texts,
            ) {
                Ok(PackageRouteAttachment::Published) => {}
                Ok(PackageRouteAttachment::Deferred(collision)) => {
                    // The routes stopped at a registered package overlap. The boundary remains
                    // unavailable to consumers, but the report waits until every package claim
                    // exists so the competing authored site can be named.
                    publication_failed = true;
                    // HEAD reported only the first attached collision; later records cannot report.
                    if deferred_package_collision.is_none() {
                        deferred_package_collision = Some(collision);
                    }
                }
                Err(failure) => {
                    publication_failed = true;
                    prepass_failure = Some(failure);
                }
            }
        }
        #[cfg(feature = "timers")]
        timing_guard_package_route_inventory.finish();
        // The external registered prefix is unavailable only when boundary-wide publication
        // failed or its root support facade failed. A child facade's diagnostic stays scoped to
        // that internal facade key, so an external consumer of an unrelated undeclared path still
        // receives the route-miss diagnostic instead of being masked by the child's failure.
        let root_support_unavailable = unavailable_facades.iter().any(|(module_id, surface)| {
            *module_id == root_module_id && *surface == ModuleRootRole::Support
        });
        // A prepass that stopped on a later facade never attaches its captured root claims, so
        // they are absent from published routes. Keep them for a deferred collision's diagnostic
        // lookup only; import resolution never reads this map.
        let root_routes_unpublished = prepass_failure.is_some() || publication_failed;
        let failed_root_routes = root_routes_unpublished.then(|| {
            routes
                .routes(root_module_id, ModuleRootRole::Support)
                .iter()
                .map(|claim| (claim.route.as_str().to_owned(), claim.span))
                .collect()
        });
        if prepass_failure.is_some() || publication_failed || root_support_unavailable {
            namespace_set.mark_source_package_unavailable(&dependency_prefix);
        }
        if !unavailable_facades.is_empty() {
            namespace_set
                .mark_package_facades_unavailable(&dependency_prefix, &unavailable_facades);
        }
        preparations.push(SourcePackageBoundaryPreparation {
            dependency_prefix,
            package_identity,
            root_module_id,
            path_resolver: package_path_resolver,
            source_files: package_sources,
            path_interner,
            graph: package_graph,
            prepared_facades: cache,
            source_module_origins,
            selected_source_texts: package_selected_source_texts,
            prepass_failure,
            failed_root_routes,
            #[cfg(feature = "timers")]
            timing_boundary,
        });
    }

    // Detect every package claim before a consumer can resolve an ambiguous route to one site,
    // but report it from the prefix-ordered walk: an earlier boundary's discovery diagnostic and
    // a later boundary's prepass failure must retain their turns. A package set with no routes
    // skips the source map and every folded collision index.
    let package_claim_collision = if namespace_set.publishes_package_routes() {
        let validation = {
            let package_sources = preparations
                .iter()
                .map(|preparation| {
                    (
                        preparation.dependency_prefix.as_str(),
                        preparation.source_files.sources().as_ref(),
                    )
                })
                .collect::<PackageRouteSources<'_>>();
            namespace_set.validate_package_route_collisions(&package_sources)
        };
        validation.err()
    } else {
        None
    };

    // The first attached prefix collision is reported at its publisher's turn, matching HEAD.
    // Its competing site is resolved only when that turn wins; later attachments never report
    // because HEAD stopped at the first collision.
    let package_collision = deferred_package_collision;
    let route_collision = package_claim_collision.map(|publication| {
        let preparation_index = earliest_preparation_turn(
            &preparations,
            publication.affected_prefixes.iter().map(String::as_str),
        );
        let RoutePublicationFailure {
            failure,
            package_donors,
            ..
        } = publication;
        PendingRouteCollision {
            preparation_index,
            failure,
            package_donors,
        }
    });

    Ok(PreparedSourcePackageBoundaries {
        preparations,
        route_collision,
        package_collision,
    })
}

/// The earliest preparation turn among the boundaries that participate in one collision.
///
/// WHY: `preparations` is built in package-prefix order, so its position is the walk turn. A
/// prefix that no preparation owns is already an inconsistent boundary set and reports at the
/// first turn instead of deferring into discovery.
fn earliest_preparation_turn<'a>(
    preparations: &[SourcePackageBoundaryPreparation],
    prefixes: impl IntoIterator<Item = &'a str>,
) -> usize {
    prefixes
        .into_iter()
        .filter_map(|prefix| {
            preparations
                .iter()
                .position(|preparation| preparation.dependency_prefix == prefix)
        })
        .min()
        .unwrap_or(0)
}

/// The source owner that can supply one deferred collision's competing root facade.
///
/// WHY: the provider may be the current publisher (a route under its own registered prefix), a
///      later preparation, or an already-walked inventory. The site is read from the owning
///      boundary's index and source database without translating its ids.
fn provider_root_facade(
    namespace_set: &ModuleNamespaceSet,
    current: &SourcePackageBoundaryPreparation,
    remaining: &[SourcePackageBoundaryPreparation],
    inventories: &[SourcePackageModuleInventory],
    provider_prefix: &str,
) -> Option<PackageRootFacadeSite> {
    let package_index = namespace_set.source_package_boundary(provider_prefix)?;
    let preparation = if current.dependency_prefix == provider_prefix {
        Some(current)
    } else {
        remaining
            .iter()
            .find(|preparation| preparation.dependency_prefix == provider_prefix)
    };
    if let Some(preparation) = preparation {
        return PackageRootFacadeSite::from_package(
            package_index,
            preparation.package_identity.clone(),
            preparation.source_files.sources(),
        );
    }
    inventories
        .iter()
        .find(|inventory| inventory.dependency_prefix == provider_prefix)
        .and_then(|inventory| {
            PackageRootFacadeSite::from_package(
                package_index,
                inventory.package_identity.clone(),
                inventory.source_files.sources(),
            )
        })
}

/// Root claims the provider captured before its prepass stopped.
///
/// Only the current/later preparations can carry them; an earlier provider that reached the walk
/// published its routes, so its claims live in the namespace set instead.
fn provider_failed_root_routes<'a>(
    current: &'a SourcePackageBoundaryPreparation,
    remaining: &'a [SourcePackageBoundaryPreparation],
    provider_prefix: &str,
) -> Option<&'a BTreeMap<String, SourceSpan>> {
    if current.dependency_prefix == provider_prefix {
        return current.failed_root_routes.as_ref();
    }
    remaining
        .iter()
        .find(|preparation| preparation.dependency_prefix == provider_prefix)
        .and_then(|preparation| preparation.failed_root_routes.as_ref())
}

/// Report one deferred package-prefix collision now that its publisher owns the walk turn.
///
/// WHAT: resolves the provider's authored site, finalizes only the provider donor the label
///       needs, confirms that donor has a line index for the related span, and then builds the
///       two-site report or the publisher-only fallback before finalizing the primary donor.
/// WHY: resolving before this turn can read a losing collision's provider, and an earlier
///      winner's cleanup would then reject that cached oversized snapshot as a syntax failure.
///      Finalizing the related donor first proves the label can render instead of leaving a
///      dangling site or silently switching the primary to its canonical-host spelling.
fn report_package_prefix_collision_at_turn(
    namespace_set: &ModuleNamespaceSet,
    mut inventories: Vec<SourcePackageModuleInventory>,
    current: SourcePackageBoundaryPreparation,
    mut remaining: Vec<SourcePackageBoundaryPreparation>,
    deferred: DeferredPackageRouteCollision,
    string_table: &mut StringTable,
) -> BoundaryPremergeFailure {
    let publisher_prefix = deferred.publisher_prefix.clone();
    let provider_prefix = deferred.provider_prefix.clone();
    // A route under the publisher's own registered prefix is a self-collision. HEAD reported it
    // eagerly from the publisher's own source context as one site, so the deferred report must
    // keep that shape instead of labelling the publisher's root facade as a competing provider.
    let self_provider = provider_prefix == publisher_prefix;
    let provider_root_facade = provider_root_facade(
        namespace_set,
        &current,
        &remaining,
        &inventories,
        &provider_prefix,
    );
    let provider_authored_routes =
        provider_failed_root_routes(&current, &remaining, &provider_prefix);
    let mut site = match resolve_deferred_package_route_site(
        namespace_set,
        deferred,
        provider_root_facade,
        provider_authored_routes,
    ) {
        Ok(site) => site,
        Err(failure) => return boundary_from_route_failure(failure),
    };
    let mut current = Some(current);
    let mut related_donor = match (site.related_request.take(), site.related_span) {
        (Some(request), Some(span)) => {
            let source = finalize_provider_donor(
                &request,
                &provider_prefix,
                &mut current,
                &mut remaining,
                &mut inventories,
                string_table,
            );
            source.map(|source| (request, source, span))
        }
        _ => None,
    };
    let related_renderable = !self_provider
        && related_donor.as_ref().is_some_and(|(_, source, span)| {
            // A retained snapshot owns a line index; a pending, non-UTF-8, failed or capacity-
            // rejected slot does not. Checking the retained text keeps this available outside
            // test builds.
            source.retained_text(span.source()).is_some()
        });
    let RoutePublicationFailure {
        failure,
        package_donors,
        ..
    } = deferred_package_collision_failure(&site, related_renderable);
    let mut boundary = BoundaryPremergeFailure::project(vec![failure]);
    let primary_request = package_donors
        .and_then(|requests| requests.primary)
        .expect("a deferred collision always requests its publisher donor");
    let related_source = if related_renderable {
        let (request, source, _) = related_donor
            .take()
            .expect("a renderable related site owns a finalized donor");
        attach_package_donor(&mut boundary, BoundaryDonorRole::Related, request, &source);
        Some(source)
    } else {
        None
    };

    let Some(current) = current else {
        // The provider was the publisher itself: a package route under its own registered
        // prefix. Its single finalized database must render the primary site too.
        let source = related_source.or_else(|| related_donor.map(|(_, source, _)| source));
        let Some(source) = source else {
            return BoundaryPremergeFailure::from(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "route collision in @{publisher_prefix} lost its finalized publisher source"
                ),
            ));
        };
        // A capacity-rejected or unreadable publisher snapshot still owns a partially retained
        // database. HEAD reported the collision without a primary excerpt; the diagnostic keeps
        // the authored span and renders without it instead of failing as an internal error.
        attach_package_donor(
            &mut boundary,
            BoundaryDonorRole::Primary,
            primary_request,
            &source,
        );
        return boundary;
    };
    boundary = boundary.with_package_donor_requests(PackageRouteDonorRequests {
        primary: Some(primary_request),
        related: None,
    });
    finalize_package_collision_preparations(
        boundary,
        std::iter::once(current).chain(remaining),
        string_table,
    )
}

/// Finish the provider owner named by one deferred collision's related donor request.
///
/// WHAT: takes the provider's preparation or inventory, finishes only that source owner, and
///       returns its database. The caller checks the related span's line index before attaching.
/// WHY: the donor must exist before the report shape is chosen; a capacity-rejected or unreadable
///      snapshot yields no line index and the caller reports the publisher alone.
fn finalize_provider_donor(
    request: &PackageRouteDonorRequest,
    provider_prefix: &str,
    current: &mut Option<SourcePackageBoundaryPreparation>,
    remaining: &mut Vec<SourcePackageBoundaryPreparation>,
    inventories: &mut Vec<SourcePackageModuleInventory>,
    string_table: &mut StringTable,
) -> Option<Arc<SourceDatabase>> {
    if current
        .as_ref()
        .is_some_and(|preparation| preparation.dependency_prefix == provider_prefix)
    {
        let preparation = current
            .take()
            .expect("the provider is the current preparation");
        return finish_prepared_owner_for_site(preparation, request, string_table);
    }
    if let Some(position) = remaining
        .iter()
        .position(|preparation| preparation.dependency_prefix == provider_prefix)
    {
        let preparation = remaining.remove(position);
        return finish_prepared_owner_for_site(preparation, request, string_table);
    }
    if let Some(position) = inventories
        .iter()
        .position(|inventory| inventory.dependency_prefix == provider_prefix)
    {
        let inventory = inventories.remove(position);
        return finish_package_donor(request, inventory).ok().map(Arc::new);
    }
    None
}

/// Finish one preparation's source owner for a collision donor site.
fn finish_prepared_owner_for_site(
    preparation: SourcePackageBoundaryPreparation,
    request: &PackageRouteDonorRequest,
    string_table: &mut StringTable,
) -> Option<Arc<SourceDatabase>> {
    let SourcePackageBoundaryPreparation {
        source_files,
        selected_source_texts,
        ..
    } = preparation;
    match finish_prepared_source_owner(
        source_files,
        selected_source_texts,
        request.selected_site.as_deref(),
        string_table,
    ) {
        Ok(source) => Some(Arc::new(source)),
        Err(PreparedSourceOwnerFailure { source, .. }) => source.map(|source| Arc::new(*source)),
    }
}

/// Convert an unindexed collision failure back to the boundary failure shape.
fn boundary_from_route_failure(failure: RoutePublicationFailure) -> BoundaryPremergeFailure {
    let RoutePublicationFailure {
        failure: prepared,
        package_donors,
        ..
    } = failure;
    let boundary = BoundaryPremergeFailure::project(vec![prepared]);
    match package_donors {
        Some(requests) => boundary.with_package_donor_requests(*requests),
        None => boundary,
    }
}

/// Finish every package preparation after a route collision was detected before discovery.
///
/// WHAT: retains each remaining preparation's selected snapshots, finalizes its source table and,
///       for a requested donor boundary, keeps that database as a report domain.
/// WHY: the collision must be reported before a package consumer's import resolution can reduce
///      it to the importer span. Package builders still own unretained facade snapshots here, so
///      the donor path performs the same retention/finalization tail a failed discovery would.
fn finalize_route_collision_preparations(
    mut failure: BoundaryPremergeFailure,
    preparations: impl IntoIterator<Item = SourcePackageBoundaryPreparation>,
    string_table: &mut StringTable,
) -> BoundaryPremergeFailure {
    let mut requests = take_package_donor_requests(&mut failure);
    for preparation in preparations {
        let SourcePackageBoundaryPreparation {
            dependency_prefix,
            source_files,
            selected_source_texts,
            prepass_failure,
            ..
        } = preparation;
        // A later boundary consumed by cleanup may already carry a stored infrastructure cause.
        // Keep it dominant instead of silently dropping it with the unconsumed diagnostic.
        let stored_infrastructure =
            matches!(&prepass_failure, Some(PreparedFailure::Infrastructure(_)));
        if let Some(PreparedFailure::Infrastructure(errors)) = prepass_failure {
            failure = append_cleanup_failure(failure, PreparedFailure::Infrastructure(errors));
        }
        let matching = take_matching_requests(&mut requests, &dependency_prefix);
        let selected_site = matching
            .first()
            .and_then(|(_, request)| request.selected_site.as_deref());
        match finish_prepared_source_owner(
            source_files,
            selected_source_texts,
            selected_site,
            string_table,
        ) {
            Ok(source) => {
                if !matching.is_empty() {
                    let source = Arc::new(source);
                    for (role, request) in matching {
                        attach_package_donor(&mut failure, role, request, &source);
                    }
                }
            }
            Err(PreparedSourceOwnerFailure {
                failure: prepared,
                source,
            }) => match source {
                Some(source) => {
                    // The partially retained package owner still supplies the exact database for
                    // its own diagnostic and, when requested, the collision donor identity. Its
                    // SourceIds therefore never fall back to the project owner.
                    let source = Arc::new(*source);
                    if !matching.is_empty() {
                        for (role, request) in matching {
                            attach_package_donor(&mut failure, role, request, &source);
                        }
                    }
                    match prepared {
                        PreparedFailure::Diagnosed(batch) => {
                            failure
                                .package_failure_sources
                                .push((PreparedFailure::Diagnosed(batch), source));
                        }
                        PreparedFailure::Infrastructure(errors) => {
                            // A stored prepass infrastructure cause already names this failed
                            // read. Re-appending the same source-slot error would print it as an
                            // additional cause of one operational failure.
                            if !stored_infrastructure {
                                failure
                                    .failures
                                    .push(PreparedFailure::Infrastructure(errors));
                            }
                        }
                    }
                }
                None => {
                    // Without a finished owner, a failed donor cannot install the handle that
                    // names its identity. Release the partial donor set so frozen installation
                    // never sees a handle whose domain is absent.
                    if !matching.is_empty() {
                        requests.clear();
                        failure.package_donors = None;
                    }
                    let repeats_stored_infrastructure = stored_infrastructure
                        && matches!(&prepared, PreparedFailure::Infrastructure(_));
                    if !repeats_stored_infrastructure {
                        failure = append_cleanup_failure(failure, prepared);
                    }
                }
            },
        }
    }
    // Claim collisions report at their earliest affected prefix, and that prefix is one of the
    // requested donors, so no already-walked inventory can match a request here.
    for (role, request) in requests {
        failure = append_boundary_finish_failure(
            failure,
            InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "route collision {role:?} package @{} was absent from the prepared package set",
                    request.dependency_prefix
                ),
            ),
        );
    }
    failure
}

/// Finish only the source owners one package prefix collision's label names.
///
/// WHAT: resolves the primary and related donor requests from the current/remaining preparations
///       and drops every other preparation without finalizing it.
/// WHY: HEAD reported this collision as the publisher's own prepass failure and returned before a
///      later boundary's unreadable source could be promoted. The label still needs both authored
///      databases, so only those owners are finished; their own stored failure does not replace
///      the collision.
fn finalize_package_collision_preparations(
    mut failure: BoundaryPremergeFailure,
    preparations: impl IntoIterator<Item = SourcePackageBoundaryPreparation>,
    string_table: &mut StringTable,
) -> BoundaryPremergeFailure {
    let mut requests = take_package_donor_requests(&mut failure);
    for preparation in preparations {
        let matching = take_matching_requests(&mut requests, &preparation.dependency_prefix);
        if matching.is_empty() {
            continue;
        }
        let SourcePackageBoundaryPreparation {
            source_files,
            selected_source_texts,
            ..
        } = preparation;
        let selected_site = matching
            .first()
            .and_then(|(_, request)| request.selected_site.as_deref());
        match finish_prepared_source_owner(
            source_files,
            selected_source_texts,
            selected_site,
            string_table,
        ) {
            Ok(source) => {
                let source = Arc::new(source);
                for (role, request) in matching {
                    attach_package_donor(&mut failure, role, request, &source);
                }
            }
            Err(PreparedSourceOwnerFailure {
                source: Some(source),
                ..
            }) => {
                // A partially retained owner still supplies the database its span resolves in.
                let source = Arc::new(*source);
                for (role, request) in matching {
                    attach_package_donor(&mut failure, role, request, &source);
                }
            }
            Err(PreparedSourceOwnerFailure { source: None, .. }) => {
                // No finished database can render this label. Keep the request pending so the
                // boundary fails closed instead of reinterpreting the site in another domain.
                requests.extend(matching);
            }
        }
    }
    for (role, request) in requests {
        failure = append_boundary_finish_failure(
            failure,
            InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "route collision {role:?} package @{} was absent from the prepared package set",
                    request.dependency_prefix
                ),
            ),
        );
    }
    failure
}

/// A late preparation failure plus its own finished package owner when that owner can be built.
struct PreparedSourceOwnerFailure {
    failure: PreparedFailure,
    source: Option<Box<SourceDatabase>>,
}

impl PreparedSourceOwnerFailure {
    fn without_owner(error: InfrastructureFailure) -> Self {
        Self {
            failure: PreparedFailure::from(error),
            source: None,
        }
    }
}

/// Finish a partially retained package owner so its diagnostic never borrows the project database.
fn finish_source_owner_in_failure(
    failure: PreparedFailure,
    mut source_files: SourceDatabaseBuilder,
    selected_source_ids: &[SourceId],
    selected_site: Option<&Path>,
    capacity_rejected_source_ids: &[SourceId],
) -> PreparedSourceOwnerFailure {
    let failure = match finalize_schedule_source_spans(&mut source_files, None, selected_source_ids)
    {
        Ok(()) => failure,
        Err(error) => failure.with_infrastructure_cause(error),
    };
    // The requested donor site may be an ordinary unselected provider. The successful path
    // retains it after selected snapshots; the cleanup path must do the same before freezing the
    // package database, or the related label would name the right owner without a line index.
    if let Some(site) = selected_site {
        let donor_result = (|| -> Result<(), InfrastructureFailure> {
            // A selected donor snapshot rejected by the size policy stays Pending with its
            // authored content still on disk. Re-reading it here cannot retain the snapshot and
            // would repeat the same failed read, so the optional excerpt is skipped instead.
            let donor_is_capacity_rejected = source_files
                .sources()
                .get_by_canonical_path(site)
                .is_some_and(|record| capacity_rejected_source_ids.contains(&record.id));
            if donor_is_capacity_rejected {
                return Ok(());
            }
            let Some(source_id) = retain_package_donor_site(&mut source_files, site)? else {
                return Ok(());
            };
            if !source_files.is_source_finalized(source_id)? {
                source_files.finish_source(source_id)?;
            }
            Ok(())
        })();
        if let Err(error) = donor_result {
            return PreparedSourceOwnerFailure {
                failure: failure.with_infrastructure_cause(error),
                source: None,
            };
        }
    }
    if let Err(error) = source_files.assemble_finalized_tables() {
        return PreparedSourceOwnerFailure {
            failure: failure.with_infrastructure_cause(error),
            source: None,
        };
    }
    match source_files.finish() {
        Ok(source) => PreparedSourceOwnerFailure {
            failure,
            source: Some(Box::new(source)),
        },
        Err(error) => PreparedSourceOwnerFailure {
            failure: failure.with_infrastructure_cause(error),
            source: None,
        },
    }
}

/// Finalize one prepared source owner, including any donor site that discovery had not loaded.
fn finish_prepared_source_owner(
    mut source_files: SourceDatabaseBuilder,
    mut selected_source_texts: SelectedSourceTextMap,
    selected_site: Option<&Path>,
    string_table: &mut StringTable,
) -> Result<SourceDatabase, PreparedSourceOwnerFailure> {
    let selected_source_ids = selected_source_texts.selected_source_ids(source_files.sources());
    let mut retain_path_fork = source_files.sources().fork_path_interner();
    let retain_result =
        selected_source_texts.retain_into(source_files.sources_mut(), &mut retain_path_fork);
    if retain_result.is_err() {
        source_files.discard_pending_span_builders_for_capacity_rejected_sources(
            selected_source_texts.capacity_rejected_source_ids(),
        );
    }
    source_files.adopt_pending_span_builders();
    if let Err(error) = retain_result {
        let failure = error.into_failure_with_root_string_table(
            string_table,
            Arc::new(retain_path_fork.shared_table()),
        );
        return Err(finish_source_owner_in_failure(
            failure,
            source_files,
            &selected_source_ids,
            selected_site,
            selected_source_texts.capacity_rejected_source_ids(),
        ));
    }
    if let Err(error) =
        finalize_schedule_source_spans(&mut source_files, None, &selected_source_ids)
    {
        return Err(PreparedSourceOwnerFailure::without_owner(error));
    }
    if let Some(site) = selected_site {
        let source_id = retain_package_donor_site(&mut source_files, site)
            .map_err(PreparedSourceOwnerFailure::without_owner)?;
        if let Some(source_id) = source_id
            && !source_files
                .is_source_finalized(source_id)
                .map_err(PreparedSourceOwnerFailure::without_owner)?
        {
            source_files
                .finish_source(source_id)
                .map_err(PreparedSourceOwnerFailure::without_owner)?;
        }
    }
    source_files
        .assemble_finalized_tables()
        .map_err(PreparedSourceOwnerFailure::without_owner)?;
    source_files
        .finish()
        .map_err(PreparedSourceOwnerFailure::without_owner)
}

/// Append a late preparation failure without discarding the route-collision ambiguity.
fn append_cleanup_failure(
    mut failure: BoundaryPremergeFailure,
    prepared: PreparedFailure,
) -> BoundaryPremergeFailure {
    match prepared {
        PreparedFailure::Infrastructure(errors) => {
            failure
                .failures
                .retain(|entry| matches!(entry, PreparedFailure::Infrastructure(_)));
            failure
                .failures
                .push(PreparedFailure::Infrastructure(errors));
        }
        PreparedFailure::Diagnosed(batch) => {
            failure.failures.push(PreparedFailure::Diagnosed(batch))
        }
    }
    failure
}

/// Finish one package source owner beside a facade-prepass or route-publication failure.
///
/// WHY: both failures happen after sources were registered but before the walk's normal failure
///      tail. Retaining the selected snapshots and finalizing loaded spans keeps the package
///      diagnostic source owner in the same state a failed discovery would leave it.
fn finalize_failed_package_preparation(
    failure: PreparedFailure,
    mut package_sources: SourceDatabaseBuilder,
    package_selected_source_texts: &mut SelectedSourceTextMap,
    string_table: &mut StringTable,
) -> BoundaryPremergeFailure {
    let selected_source_ids =
        package_selected_source_texts.selected_source_ids(package_sources.sources());
    let mut retain_path_fork = package_sources.sources().fork_path_interner();
    let retain_result = package_selected_source_texts
        .retain_into(package_sources.sources_mut(), &mut retain_path_fork);
    if retain_result.is_err() {
        package_sources.discard_pending_span_builders_for_capacity_rejected_sources(
            package_selected_source_texts.capacity_rejected_source_ids(),
        );
    }
    package_sources.adopt_pending_span_builders();
    let failure = match retain_result {
        Ok(()) => failure,
        Err(error) => error.admit_into_owner(Some(failure), string_table, &retain_path_fork),
    };
    let failure =
        match finalize_schedule_source_spans(&mut package_sources, None, &selected_source_ids) {
            Ok(()) => failure,
            Err(finish_error) => failure.with_infrastructure_cause(finish_error),
        };
    finalize_package_failure(failure, package_sources)
}

/// Discover each registered source-backed package within its own indexed boundary.
///
/// WHAT: walks each prepared package boundary's modules and assigns the canonical/check-only
///       schedule using the already completed namespace index and shared provider-discovery state.
/// WHY: directory and synthetic consumers must use the same Stage 0 package boundary path; only
///      the consumer's source discovery differs.
#[allow(
    clippy::too_many_arguments,
    reason = "package discovery keeps immutable boundary inputs and independently borrowed import/resource/string services explicit"
)]
pub(super) fn discover_source_package_inventories(
    config: &Config,
    namespace_set: &mut ModuleNamespaceSet,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
    mode: FrontendCompilationMode,
) -> Result<Vec<SourcePackageModuleInventory>, BoundaryPremergeFailure> {
    let PreparedSourcePackageBoundaries {
        preparations,
        mut route_collision,
        mut package_collision,
    } = prepare_source_package_boundaries(
        config,
        namespace_set,
        project_path_resolver,
        style_directives,
        string_table,
    )?;
    let mut inventories = Vec::with_capacity(preparations.len());
    let mut preparations = preparations.into_iter().enumerate();
    for (index, preparation) in preparations.by_ref() {
        // A deferred prefix collision is the publisher's own prepass failure at this turn, so a
        // claim collision at the same turn is skipped exactly as HEAD skipped it.
        let own_collision_at_turn = package_collision
            .as_ref()
            .is_some_and(|collision| collision.publisher_prefix == preparation.dependency_prefix);
        // The current boundary's own prepass failure owns this turn. Reporting a route collision
        // first would replace, for example, the facade's MOTH-RULE-0092 case conflict with a
        // downstream collision that HEAD reported only after the boundary failure.
        if preparation.prepass_failure.is_none()
            && !own_collision_at_turn
            && route_collision
                .as_ref()
                .is_some_and(|pending| pending.preparation_index == index)
        {
            let pending = route_collision
                .take()
                .expect("the collision still belongs to this preparation turn");
            return Err(finalize_route_collision_preparations(
                pending_route_collision_failure(pending),
                std::iter::once(preparation).chain(preparations.map(|(_, next)| next)),
                string_table,
            ));
        }
        if own_collision_at_turn {
            let deferred = package_collision
                .take()
                .expect("the publisher collision still belongs to this preparation turn");
            let remaining = preparations.map(|(_, next)| next).collect();
            return Err(report_package_prefix_collision_at_turn(
                namespace_set,
                std::mem::take(&mut inventories),
                preparation,
                remaining,
                deferred,
                string_table,
            ));
        }
        let SourcePackageBoundaryPreparation {
            dependency_prefix,
            package_identity,
            root_module_id,
            path_resolver: package_path_resolver,
            source_files: mut package_sources,
            path_interner,
            graph: mut package_graph,
            mut prepared_facades,
            source_module_origins,
            selected_source_texts: mut package_selected_source_texts,
            prepass_failure,
            failed_root_routes: _,
            #[cfg(feature = "timers")]
            timing_boundary,
        } = preparation;
        // A boundary's prepass failure is reported here, at the turn its ordinary walk would
        // take. Earlier boundaries have already walked and can fail first with their own
        // discovery diagnostics, preserving package-prefix order across the preparation barrier.
        if let Some(failure) = prepass_failure {
            return Err(finalize_failed_package_preparation(
                failure,
                package_sources,
                &mut package_selected_source_texts,
                string_table,
            ));
        }
        let package_index = namespace_set
            .source_package_boundary(&dependency_prefix)
            .expect("a prepared package boundary remains registered");
        let (package_source_files, mut package_source_spans) = package_sources.split();
        let package_path_fork = path_interner.fork_source().fork_for_module();
        let package_resolution = DirectoryDependencyResolution::package(
            namespace_set,
            &dependency_prefix,
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
                &mut prepared_facades,
                Some(source_module_origins),
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
        inventories.push(SourcePackageModuleInventory {
            dependency_prefix,
            package_identity,
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
    if let Some(pending) = route_collision {
        return Err(finalize_route_collision_preparations(
            pending_route_collision_failure(pending),
            std::iter::empty::<SourcePackageBoundaryPreparation>(),
            string_table,
        ));
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
        let mut selected_source_texts = SelectedSourceTextMap::for_package_boundary(
            package_index.package_set_membership(),
            inventory.dependency_prefix.as_str(),
        );
        let preparation_result = inventory.schedule.prepare_check_only_jobs(
            style_directives,
            &mut source_spans,
            &inventory.path_resolver,
            external_imports,
            package_resolution,
            &inventory.graph,
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
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &BuilderSurface,
    external_packages: &Arc<ExternalPackageRegistry>,
    inventories: Vec<SourcePackageModuleInventory>,
    budget: &mut ProjectDiagnosticBudget,
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
        if budget.is_exhausted() {
            // The invoking command's threshold stops new compilation work. Each remaining package
            // inventory was discovered but never compiled, so it has no boundary to record and its
            // source owner is simply dropped as unvisited work.
            canonical::refuse_scheduling_admission(budget, warning_sink.as_deref_mut());
            break;
        }
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
                    budget,
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
            // Deferred check-only units keep sharing their canonical owner's allowance across
            // package transitions; the owner's retained errors reset only with the request.
            let mut retained_module_errors = if check_only_jobs.is_empty() {
                FxHashMap::default()
            } else {
                canonical::retained_module_error_counts(&package.boundary.diagnosed)
            };
            let check_only_batches = deferred_check_only::compile_check_only_jobs_after_canonical(
                canonical::BoundaryCompilationContext::new(
                    config,
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
                    None,
                ),
                &package.boundary.modules,
                &package.boundary.generated,
                check_only_jobs,
                &provider_bindings,
                &source_package_dependencies,
                &mut retained_module_errors,
                budget,
                warning_sink.as_deref_mut(),
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
    reason = "directory compilation keeps configuration, validated output settings, directives, mutable builder, warning/report, source, build-config inputs and compilation mode separate"
)]
pub(crate) fn compile_directory_frontend(
    config: &Config,
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
///
/// Visible to `create_project_modules` so boundary tests can inspect the retained graph before
/// the terminal wrapper converts a diagnosed result into reports.
#[allow(
    clippy::too_many_arguments,
    reason = "premerge directory orchestration keeps configuration, output settings, directives, mutable builder/string/source owners, build-config inputs and mode separate"
)]
pub(super) fn compile_directory_frontend_in_premerge_lane(
    config: &Config,
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
    // Resolve the bootstrap config identity through the builder that owns it. Discovery excludes
    // this canonical target from ordinary-source admission.
    let project_config_source = project_sources
        .config_source_id()
        .filter(|source_id| project_sources.sources().get(*source_id).is_some())
        .map(|source_id| ProjectConfigSource {
            source_id,
            source_files: project_sources.sources().as_ref(),
        });
    let mut project_setup = match project_roots::build_project_path_resolver_with_index(
        config,
        SourceTreeProjectContext {
            validated_output_settings,
            config_source: project_config_source,
        },
        &builder_surface.source_packages,
        &builder_surface.source_file_kinds,
        &builder_surface.external_import_providers,
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

        // 1. Build package namespaces and every source-package inventory first. Provider-backed
        // discovery may extend the binding registry, so every boundary must finish that serial
        // mutation phase before the registry becomes the immutable frontend view. Keeping package
        // inventories before any project facade preparation also preserves the established
        // package-before-project failure ordering.
        let mut module_namespace_set = match ModuleNamespaceSet::for_source_packages(
            project_setup.source_package_boundary_indexes,
            &builder_surface.binding_packages,
            string_table,
        ) {
            Ok(namespace_set) => namespace_set,
            Err(failure) => {
                return Err(finalize_project_setup_failure(
                    &mut project_sources,
                    &mut selected_source_texts,
                    string_table,
                    failure,
                ));
            }
        };

        let mut external_imports = source_discovery::ExternalImportDiscoveryState {
            external_packages: &mut builder_surface.binding_packages,
            providers: &builder_surface.external_import_providers,
            cache: &mut builder_surface.external_import_cache,
            resolution_table: &mut builder_surface.external_dependency_resolution_table,
        };
        let mut resource_inputs = ResourceInputRegistry::new();

        let mut source_package_inventories = discover_source_package_inventories(
            config,
            &mut module_namespace_set,
            &project_path_resolver,
            style_directives,
            &mut external_imports,
            &mut resource_inputs,
            string_table,
            mode,
        )?;

        // Register the main-project boundary before facade preparation so facade read/tokenize
        // time is attributed to `frontend.prepare` and its module, not lost outside the boundary.
        #[cfg(feature = "timers")]
        let project_timing_boundary = crate::timing::register_timing_boundary(
            crate::timing::TimingBoundaryKind::MainProject,
            || config.project_name.clone(),
        );
        // 2. Prepare every project facade root once and publish its declared public routes before
        // project consumer namespaces exist. Project namespace attachment, including reserved-root
        // validation, therefore runs after package inventories and after this prepass; project
        // inventory is discovered afterwards. The preparation is provider-independent, and its
        // cache is adopted by the module walk below so the facade is never tokenized or parsed a
        // second time. The prepass and namespace attachment are inventory work for this boundary,
        // not an untimed preamble.
        timing_scope_attributed!(
            timing_guard_project_route_inventory,
            crate::timing::TimingMetric::BoundaryInventory,
            Some(crate::timing::TimingContext::for_boundary(
                project_timing_boundary
            )),
        );
        let mut facade_route_prepass = match facade_route_prepass::prepublish_facade_routes(
            &project_setup.source_tree_index,
            &project_setup.project_module_graph,
            project_source_files,
            style_directives,
            config.frontend_options().max_errors_per_module,
            facade_route_prepass::FacadeDiagnosticOwner::Prepass,
            &mut project_source_spans,
            &mut selected_source_texts,
            string_table,
            &project_path_interner,
            #[cfg(feature = "timers")]
            project_timing_boundary,
        ) {
            Ok(prepass) => prepass,
            Err(failure) => {
                #[cfg(feature = "timers")]
                timing_guard_project_route_inventory.finish();
                let failure = finalize_project_setup_failure(
                    &mut project_sources,
                    &mut selected_source_texts,
                    string_table,
                    failure,
                );
                return Err(finalize_remaining_source_package_inventories(
                    failure,
                    source_package_inventories,
                ));
            }
        };
        // Consumer namespaces now combine the structural index with those published routes.
        // Descendant package paths stay private and a route never falls through to a physical
        // folder. Cross-package claims were already validated before project preparation.
        let package_root_facades =
            package_root_facade_sites(&module_namespace_set, &source_package_inventories);
        if let Err(failure) = module_namespace_set.attach_project_namespaces(
            &project_setup.source_tree_index,
            Some(project_source_files),
            &package_root_facades,
            Some(&mut selected_source_texts),
            &project_setup.project_module_graph,
            &facade_route_prepass.routes,
            string_table,
        ) {
            #[cfg(feature = "timers")]
            timing_guard_project_route_inventory.finish();
            let RoutePublicationFailure {
                failure,
                package_donors,
                affected_prefixes: _,
            } = failure;
            let failure = finalize_project_setup_failure(
                &mut project_sources,
                &mut selected_source_texts,
                string_table,
                failure,
            );
            let failure = match package_donors {
                Some(requests) => failure.with_package_donor_requests(*requests),
                None => failure,
            };
            return Err(finalize_remaining_source_package_inventories(
                failure,
                source_package_inventories,
            ));
        }
        #[cfg(feature = "timers")]
        timing_guard_project_route_inventory.finish();

        // The project boundary was registered before facade preparation; its accumulated total
        // remains separate from every source package.
        let directory_dependency_resolution = DirectoryDependencyResolution::project(
            &module_namespace_set,
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
                &mut facade_route_prepass.cache,
                Some(Arc::clone(&facade_route_prepass.source_module_origins)),
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
                &module_namespace_set,
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
                &project_setup.project_module_graph,
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
            &[],
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
        // One command-owned project budget spans every selected source package and the project
        // graph. It charges each returned diagnosed batch once and a package transition never
        // resets its total.
        let mut project_budget = ProjectDiagnosticBudget::from_config(config);
        let (completed_source_packages, mut transient_batches) =
            compile_source_package_inventories(
                config,
                style_directives,
                builder_surface,
                &external_packages,
                source_package_inventories,
                &mut project_budget,
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
            &mut project_budget,
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
            project_path_resolver.entry_root().to_path_buf(),
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
