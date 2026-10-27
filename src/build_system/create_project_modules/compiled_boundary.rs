//! Retained project and source-package graph boundaries after frontend compilation.
//!
//! WHAT: owns the final frontend handoff shape: one [`CompiledGraphBoundary`] per project or
//! source-package graph, the typed [`CompiledSourcePackage`] wrapper that keeps package
//! identities beside their boundary-local dense handles, and the [`ProjectFrontendCompilation`]
//! that carries all boundaries plus diagnosed and blocked outcomes for `check`.
//! WHY: successful artefacts, the dense `ModuleId -> artefact` mapping, generated sidecars,
//! graph structure and per-module outcomes must survive together so `build`/`dev` can assemble
//! a success-only [`ProjectCompilation`](crate::build_system::build::ProjectCompilation) and
//! `check` can retain independent successful work beside user diagnostics without ever exposing
//! a partial linkable payload.
//! MUST NOT: concatenate package artefacts into one vector with a count, place build-local
//! `ModuleId` values inside [`CompiledModuleArtifact`], or mutate package root metadata.

use crate::build_system::build::ProjectAssemblyError;
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureErrors, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::{
    BoundaryAccumulator, SuccessfulWarningBoundary, SuccessfulWarningPosition,
};
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, PreparedFailure};
#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::instrumentation::MemoryLedgerSnapshot;
use crate::compiler_frontend::module_compilation::{CompiledModuleArtifact, Module};
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModuleRootRole, StablePackageIdentity,
};
#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::source::{FrozenIdentityContext, SourceDatabaseRetentionMetrics};
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceDatabase};
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::compiler_frontend::symbols::string_interning::{FrozenStringTable, StringTable};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::Arc;

use super::generated_store::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::MaterialisationContextLocation;
use super::module_artifact_store::{ModuleArtifactStore, ProviderSlot};
use super::module_identity::ModuleId;
use super::project_module_graph::ProjectModuleGraph;
use super::resource_inputs::ResourceInputRegistry;

struct PreparedAppendMetrics {
    diagnostic_count: usize,
    missing_source_error: bool,
    #[cfg(feature = "data_layout_memory_probe")]
    label_count: usize,
}

fn append_prepared_frontend_diagnostics(
    accumulator: &mut BoundaryAccumulator,
    mut batch: PreparedDiagnosticBatch,
    source: Option<(&Arc<SourceDatabase>, Option<StablePackageIdentity>)>,
    diagnostic_offset: usize,
    can_freeze: &mut bool,
) -> PreparedAppendMetrics {
    let diagnostic_count = batch.diagnostic_count();
    #[cfg(feature = "data_layout_memory_probe")]
    let label_count = batch.label_count();
    let requires_source = batch.requires_source_identity();

    if diagnostic_count == 0 {
        accumulator.append_prepared(batch, None);
        return PreparedAppendMetrics {
            diagnostic_count: 0,
            missing_source_error: false,
            #[cfg(feature = "data_layout_memory_probe")]
            label_count,
        };
    }

    let Some((source_database, domain)) = source else {
        *can_freeze = false;
        let missing_source_error = requires_source;
        if missing_source_error {
            let global_range = diagnostic_offset..diagnostic_offset + diagnostic_count;
            let error = InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!("diagnostic range {global_range:?} has no finalized source identity",),
            );
            accumulator.append_infrastructure(error.into());
        }
        accumulator.append_prepared(batch, None);
        return PreparedAppendMetrics {
            diagnostic_count,
            missing_source_error,
            #[cfg(feature = "data_layout_memory_probe")]
            label_count,
        };
    };
    if requires_source && let Some(identity) = domain.as_ref() {
        batch.attach_frozen_identity_handle_if_missing(FrozenIdentityHandle::for_domain(
            identity.clone(),
        ));
    }
    accumulator.append_prepared(batch, Some((Arc::clone(source_database), domain)));
    PreparedAppendMetrics {
        diagnostic_count,
        missing_source_error: false,
        #[cfg(feature = "data_layout_memory_probe")]
        label_count,
    }
}

#[derive(Debug)]
pub(crate) struct DiagnosedModule {
    pub(crate) module_id: ModuleId,
    pub(crate) diagnostics: PreparedDiagnosticBatch,
}

/// The direct provider that prevented one module from compiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BlockedProvider {
    Module(ModuleId),
    SourcePackage(StablePackageIdentity),
}

/// One module that was not semantically compiled because a required provider failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlockedModule {
    pub(crate) module_id: ModuleId,
    pub(crate) required_provider: BlockedProvider,
}

/// One complete project or source-package graph result retained after frontend compilation.
///
/// WHAT: pairs the frozen structural graph with the dense artefact store, the boundary's
/// generated-function store and the diagnosed/blocked outcome lanes.
/// WHY: entry selection, cross-boundary reachability, warning collection and `check` reporting
/// all need graph identity and successful artefacts to survive the handoff together.
pub(crate) struct CompiledGraphBoundary {
    pub(crate) structure: ProjectModuleGraph,
    pub(crate) modules: ModuleArtifactStore,
    pub(crate) generated: BoundaryGeneratedFunctionStore,
    pub(crate) diagnosed: Vec<DiagnosedModule>,
    pub(crate) blocked: Vec<BlockedModule>,
}

/// One final outcome lane claimed by a dense module slot during boundary validation.
///
/// WHAT: a compact per-slot marker proving that every diagnosed or blocked record owns exactly
///       one slot and that no record overlaps another lane. `ModuleId` is already a dense index,
///       so a vector replaces hashing and keeps first-error selection deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetainedOutcomeLane {
    None,
    Diagnosed,
    Blocked,
}

impl CompiledGraphBoundary {
    /// Sort diagnosed and blocked outcomes by dense `ModuleId`.
    ///
    /// Wave publication order is deterministic but may differ from `ModuleId` order; this
    /// finalization step keeps every boundary consumer's ordering stable.
    pub(crate) fn sort_outcomes(&mut self) {
        self.diagnosed
            .sort_by_key(|module| module.module_id.index());
        self.blocked.sort_by_key(|module| module.module_id.index());
    }

    /// Finalize one graph result before any consumer or registry may trust it.
    ///
    /// WHAT: the single authoritative completion transition. It sorts the diagnosed and blocked
    ///       lanes into deterministic `ModuleId` order and then runs the full outcome proof.
    /// WHY: directory compilation publishes source-package boundaries before the project
    ///       boundary exists, so a boundary must be provably complete before it becomes a
    ///       provider. Callers that later receive an already-finished boundary keep only
    ///       defensive validation.
    pub(crate) fn finish(mut self) -> Result<Self, InfrastructureFailure> {
        self.sort_outcomes();
        self.validate_invariants()?;
        Ok(self)
    }

    /// Validate retained boundary invariants before any consumer trusts the dense lanes.
    ///
    /// WHAT: the single completion proof for one frontend graph result. It checks the complete
    ///       slot/lane bijection: graph nodes match store slots, every slot reached a final
    ///       outcome, successful slots reference an existing artefact row, and diagnosed and
    ///       blocked lanes hold exactly their slot's record without duplicates or overlap.
    /// WHY: dense lookups are only safe after this validation; invalid mappings must fail rather
    ///       than becoming absent artefacts. The success-only `ProjectCompilation` gate separately
    ///       adds the all-successful requirement.
    pub(crate) fn validate_invariants(&self) -> Result<(), InfrastructureFailure> {
        if self.structure.nodes().len() != self.modules.slot_count() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Graph retained {} nodes but module store has {} slots",
                    self.structure.nodes().len(),
                    self.modules.slot_count()
                ),
            ));
        }

        // Dense lane proof: every diagnosed or blocked record claims one distinct slot lane.
        // `ModuleId` is already a dense index, so a compact vector replaces hashing and keeps
        // first-error selection deterministic.
        let mut retained_lanes = vec![RetainedOutcomeLane::None; self.modules.slot_count()];
        for diagnosed in &self.diagnosed {
            let lane = retained_lanes
                .get_mut(diagnosed.module_id.index())
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "Diagnosed module {} is out of range for {} slots",
                            diagnosed.module_id.index(),
                            self.modules.slot_count()
                        ),
                    )
                })?;
            if *lane != RetainedOutcomeLane::None {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "ModuleId {} is both diagnosed and blocked or appears more than once in the diagnosed lane",
                        diagnosed.module_id.index()
                    ),
                ));
            }
            *lane = RetainedOutcomeLane::Diagnosed;
            if self.modules.slot(diagnosed.module_id)? != ProviderSlot::Diagnosed {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "Diagnosed module {} does not hold the diagnosed store slot",
                        diagnosed.module_id.index()
                    ),
                ));
            }
        }
        for blocked in &self.blocked {
            let lane = retained_lanes
                .get_mut(blocked.module_id.index())
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "Blocked module {} is out of range for {} slots",
                            blocked.module_id.index(),
                            self.modules.slot_count()
                        ),
                    )
                })?;
            if *lane != RetainedOutcomeLane::None {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "ModuleId {} is both diagnosed and blocked or appears more than once in the blocked lane",
                        blocked.module_id.index()
                    ),
                ));
            }
            *lane = RetainedOutcomeLane::Blocked;
            if self.modules.slot(blocked.module_id)? != ProviderSlot::Blocked {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "Blocked module {} does not hold the blocked store slot",
                        blocked.module_id.index()
                    ),
                ));
            }
        }

        // Walk the complete slot/lane bijection: every slot must be final, successful slots
        // must reference exactly one existing artefact row whose interface origin agrees with
        // the graph node, and every retained artefact row must be referenced by exactly one
        // slot. Dense row tracking proves the bijection without hashing.
        let mut row_owners = vec![None; self.modules.artifact_count()];
        for (index, retained_lane) in retained_lanes.iter().enumerate() {
            let module_id = ModuleId::from_index(index);
            match self.modules.slot(module_id)? {
                ProviderSlot::Unavailable => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!("ModuleId {index} never reached a completed outcome"),
                    ));
                }
                ProviderSlot::Successful(artifact_id) => {
                    let artifact = self.modules.artifact(module_id)?.ok_or_else(|| {
                        InfrastructureFailure::internal(
                            CompilerStage::ProjectBuildAssembly,
                            format!(
                                "ModuleId {index} references missing artifact row {}",
                                artifact_id.index()
                            ),
                        )
                    })?;
                    let row_count = row_owners.len();
                    let owner: &mut Option<ModuleId> = row_owners
                        .get_mut(artifact_id.index())
                        .ok_or_else(|| {
                            InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, format!(
                                "ModuleId {index} references artifact row {} outside the retained artefact lane of {} rows",
                                artifact_id.index(),
                                row_count
                            ))
                        })?;
                    if let Some(existing) = owner {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::ProjectBuildAssembly,
                            format!(
                                "Artifact row {} is referenced by both ModuleId {} and ModuleId {index}",
                                artifact_id.index(),
                                existing.index()
                            ),
                        ));
                    }
                    *owner = Some(module_id);
                    let node = &self.structure.nodes()[index];
                    if &artifact.interface.module_origin != node.stable_origin() {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::ProjectBuildAssembly,
                            format!(
                                "ModuleId {index} artefact interface origin {:?} disagrees with its graph node origin {:?}",
                                artifact.interface.module_origin,
                                node.stable_origin()
                            ),
                        ));
                    }
                }
                ProviderSlot::Diagnosed => {
                    if *retained_lane != RetainedOutcomeLane::Diagnosed {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::ProjectBuildAssembly,
                            format!(
                                "ModuleId {index} holds the diagnosed slot but has no diagnosed record"
                            ),
                        ));
                    }
                }
                ProviderSlot::Blocked => {
                    if *retained_lane != RetainedOutcomeLane::Blocked {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::ProjectBuildAssembly,
                            format!(
                                "ModuleId {index} holds the blocked slot but has no blocked record"
                            ),
                        ));
                    }
                }
            }
        }

        if let Some((row_index, None)) = row_owners
            .iter()
            .enumerate()
            .find(|(_, owner)| owner.is_none())
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!("Artifact row {row_index} is not referenced by any module slot"),
            ));
        }

        Ok(())
    }

    /// Require every slot to be successful and no diagnosed or blocked record to remain.
    ///
    /// WHAT: the success-only conversion used when assembling a linkable `ProjectCompilation`.
    ///       It runs one slot scan plus the two lane emptiness checks; finalization already
    ///       proved the full outcome bijection at `finish`.
    /// WHY: `build`/`dev` must never receive a diagnosed or blocked boundary, and the stricter
    ///       requirement should not re-run the full invariant proof once finalization is the
    ///       authoritative completion gate.
    pub(crate) fn require_all_successful(&self) -> Result<(), InfrastructureFailure> {
        if let Some(diagnosed) = self.diagnosed.first() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Project compilation received a boundary with diagnosed ModuleId {}",
                    diagnosed.module_id.index()
                ),
            ));
        }
        if let Some(blocked) = self.blocked.first() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Project compilation received a boundary with blocked ModuleId {}",
                    blocked.module_id.index()
                ),
            ));
        }
        self.modules.ensure_all_successful();
        Ok(())
    }

    /// Iterate every successful artefact in deterministic `ModuleId` order.
    pub(crate) fn successful_artefacts_in_module_id_order(
        &self,
    ) -> impl Iterator<Item = &CompiledModuleArtifact> + '_ {
        self.modules.successful_artefacts_in_module_id_order()
    }

    /// Base artefacts come first in `ModuleId` order, then generated sidecars in deterministic
    /// publication order. Their warning batches are retained separately from executable views.
    pub(crate) fn successful_module_views(&self) -> impl Iterator<Item = &Module> + '_ {
        self.successful_artefacts_in_module_id_order()
            .map(|artifact| &artifact.module)
            .chain(self.generated.sidecars().map(|sidecar| &sidecar.module))
    }

    /// Install the single boundary-owned identity pair after all canonical publications.
    ///
    /// WHAT: overwrites every base artefact and generated sidecar placeholder with one final
    ///       `Arc<PathTable>` and pairs the retained materialisation contexts with that table
    ///       plus one frozen requester-domain string table.
    /// WHY: a full table snapshot per successful module and a frozen table clone per context
    ///      would scale publication cost with the module count. The boundary tail builds each
    ///      once and shares it across every retained row; the local registry must already be
    ///      dropped so `Arc::make_mut` proves the retained contexts are the sole owners.
    ///
    /// Both tables are required together because every retained `PathId` resolves through the
    /// component IDs issued by the paired string table. A boundary with no materialisation
    /// contexts still installs the pair on its executable rows; the string table is a single
    /// bounded per-boundary owner, never a per-module snapshot.
    pub(crate) fn install_boundary_identity(
        &mut self,
        path_table: Arc<PathTable>,
        source_string_table: Arc<FrozenStringTable>,
    ) {
        self.modules.install_path_table(Arc::clone(&path_table));
        self.generated.install_path_table(Arc::clone(&path_table));
        self.modules
            .install_identity_tables(path_table, source_string_table);
    }
}

/// One compiled source-package boundary with its stable package identity.
///
/// The `package_identity` and `root_module_id` are the only cross-boundary handles; all other
/// dense values stay inside [`CompiledGraphBoundary`] so overlapping local `ModuleId` values
/// from different packages can never cross-address one another.
pub(crate) struct CompiledSourcePackage {
    pub(crate) package_identity: StablePackageIdentity,
    pub(crate) root_module_id: ModuleId,
    pub(crate) boundary: CompiledGraphBoundary,
}

impl CompiledSourcePackage {
    /// The `@`-stripped package prefix for this boundary.
    pub(crate) fn package_prefix(&self) -> &str {
        self.package_identity.name()
    }

    /// Validate package identity and root ownership before registry publication.
    ///
    /// WHAT: proves the root `ModuleId` is in range, the root node belongs to this package's
    ///       stable identity, the root role is a normal API-compatible package entry root, the
    ///       root reached a final outcome, and a successful root interface agrees with the graph
    ///       node's origin. The boundary must already be finished (or be validated by this call
    ///       through the embedded boundary proof).
    /// WHY: `CompletedSourcePackageRegistry::publish` mutates registry state, so the package
    ///       row must be provably coherent before dependency edges and materialisation rows are
    ///       recorded beside it.
    pub(crate) fn validate(&self) -> Result<(), InfrastructureFailure> {
        self.boundary.validate_invariants()?;

        let root_index = self.root_module_id.index();
        let node = self
            .boundary
            .structure
            .nodes()
            .get(root_index)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "Source package @{} root ModuleId {} is out of range for {} graph nodes",
                        self.package_prefix(),
                        root_index,
                        self.boundary.structure.nodes().len()
                    ),
                )
            })?;
        if node.stable_origin().package() != &self.package_identity {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Source package @{} root node belongs to package {:?}, not {:?}",
                    self.package_prefix(),
                    node.stable_origin().package(),
                    self.package_identity
                ),
            ));
        }
        if node.role() != ModuleRootRole::Normal {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Source package @{} root module has role {:?}; packages require a normal entry-root module",
                    self.package_prefix(),
                    node.role()
                ),
            ));
        }

        match self.boundary.modules.slot(self.root_module_id)? {
            ProviderSlot::Successful(_) => {
                let interface = self
                    .boundary
                    .modules
                    .interface(self.root_module_id)?
                    .ok_or_else(|| {
                        InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, format!(
                            "Source package @{} root ModuleId {} holds a successful slot without a published interface",
                            self.package_prefix(),
                            root_index
                        ))
                    })?;
                if &interface.module_origin != node.stable_origin() {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "Source package @{} root interface origin {:?} disagrees with its graph node origin {:?}",
                            self.package_prefix(),
                            interface.module_origin,
                            node.stable_origin()
                        ),
                    ));
                }
            }
            ProviderSlot::Diagnosed | ProviderSlot::Blocked => {}
            ProviderSlot::Unavailable => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "Source package @{} root ModuleId {} never reached a completed outcome",
                        self.package_prefix(),
                        root_index
                    ),
                ));
            }
        }

        Ok(())
    }

    /// The root facade's publication slot inside this package's own store.
    pub(crate) fn root_slot(&self) -> Result<ProviderSlot, InfrastructureFailure> {
        self.boundary.modules.slot(self.root_module_id)
    }

    /// The completed facade interface for this package boundary.
    pub(crate) fn root_interface(&self) -> Result<&PublicSemanticInterface, InfrastructureFailure> {
        self.boundary
            .modules
            .interface(self.root_module_id)?
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "Source package @{} completed without a successful facade interface",
                        self.package_prefix()
                    ),
                )
            })
    }
}

/// Dense build-local identity of one completed source-package boundary.
///
/// WHAT: an operation-local handle into [`CompletedSourcePackageRegistry`]. It never enters a
///       compiled artefact or public semantic identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PackageBoundaryId(usize);

impl PackageBoundaryId {
    pub(crate) fn index(self) -> usize {
        self.0
    }

    pub(crate) fn from_index(index: usize) -> Self {
        Self(index)
    }
}

/// Exact cross-package location of one published materialisation template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PackageMaterialisationLocation {
    pub(crate) package_id: PackageBoundaryId,
    pub(crate) location: MaterialisationContextLocation,
}

/// One incrementally maintained registry of completed source-package boundaries.
///
/// WHAT: stores completed packages contiguously, resolves package prefixes to dense
///       [`PackageBoundaryId`] values, and records the package dependency graph (provider and
///       consumer edges) exactly once when each package publishes.
/// WHY: boundary compilation and module readiness checks both need package prefix lookup and
///      dependency adjacency; one registry keeps both in a single build-owned owner.
/// MUST NOT: store project modules, merge package artefacts into one vector, or expose
/// package-local dense handles as cross-boundary semantic identities.
pub(crate) struct CompletedSourcePackageRegistry {
    packages: Vec<CompiledSourcePackage>,
    by_prefix: FxHashMap<String, PackageBoundaryId>,
    provider_packages: Vec<Vec<PackageBoundaryId>>,
    consumer_packages: Vec<Vec<PackageBoundaryId>>,
    declarations_by_identity:
        FxHashMap<GeneratedDeclarationIdentity, PackageMaterialisationLocation>,
    /// Retained source databases aligned with `packages`, used for package warning rendering.
    source_databases: Vec<Option<Arc<SourceDatabase>>>,
    /// Every published package materialisation row in deterministic publication order.
    materialisation_rows: Vec<(GeneratedDeclarationIdentity, PackageMaterialisationLocation)>,
}

#[derive(Debug)]
pub(crate) struct SourcePackagePublication {
    package_id: PackageBoundaryId,
    prefix: String,
    resolved_providers: Vec<PackageBoundaryId>,
    materialisation_rows: Vec<(GeneratedDeclarationIdentity, PackageMaterialisationLocation)>,
}

impl CompletedSourcePackageRegistry {
    /// Diagnosed package boundaries retain their reports instead of exposing partial interfaces.
    pub(crate) fn has_diagnosed_or_blocked(&self) -> bool {
        self.iter().any(|package| {
            !package.boundary.diagnosed.is_empty() || !package.boundary.blocked.is_empty()
        })
    }

    pub(crate) fn new() -> Self {
        Self {
            packages: Vec::new(),
            by_prefix: FxHashMap::default(),
            provider_packages: Vec::new(),
            consumer_packages: Vec::new(),
            source_databases: Vec::new(),
            declarations_by_identity: FxHashMap::default(),
            materialisation_rows: Vec::new(),
        }
    }

    /// Publish one completed package boundary with its direct provider packages.
    ///
    /// WHAT: validates the package's finished boundary and root ownership, resolves every
    ///       dependency prefix once, rejects unknown or duplicate prefixes, and records both
    ///       dependency directions beside the package row. Every preflight runs before any
    ///       mutation so a failing publication leaves the registry unchanged.
    /// WHY: packages compile in dependency order, so every provider prefix must already have a
    ///      registry entry; a missing entry is a proven scheduling invariant failure. The
    ///      boundary must also be provably complete before later consumers resolve its facade,
    ///      slots or materialisation indexes.
    pub(crate) fn preflight(
        &self,
        package: &CompiledSourcePackage,
        dependency_prefixes: &[String],
    ) -> Result<SourcePackagePublication, InfrastructureFailure> {
        let prefix = package.package_prefix().to_owned();
        if self.by_prefix.contains_key(prefix.as_str()) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!("source package @{} completed more than once", prefix),
            ));
        }

        // Validate the finished boundary and package root before any mutation.
        package.validate()?;

        // Resolve every dependency prefix before any mutation so a failing publication leaves
        // the registry unchanged.
        let mut resolved_providers: Vec<PackageBoundaryId> =
            Vec::with_capacity(dependency_prefixes.len());
        let mut seen_providers: FxHashSet<PackageBoundaryId> = FxHashSet::default();
        for dependency_prefix in dependency_prefixes {
            let provider_id = self
                .by_prefix
                .get(dependency_prefix.as_str())
                .copied()
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "source package @{} depends on unindexed source package @{}",
                            prefix, dependency_prefix
                        ),
                    )
                })?;
            if !seen_providers.insert(provider_id) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "source package @{} lists provider package @{} more than once",
                        prefix, dependency_prefix
                    ),
                ));
            }
            resolved_providers.push(provider_id);
        }

        // Preflight materialisation rows against retained state before appending the package.
        let mut materialisation_rows = Vec::new();
        let mut seen_declarations = FxHashSet::default();
        for (identity, location) in package.boundary.modules.materialisation_locations() {
            if !seen_declarations.insert(identity.clone()) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "Generated declaration identity {:?} is duplicated inside source package @{}",
                        identity, prefix
                    ),
                ));
            }
            if let Some(existing) = self.declarations_by_identity.get(identity) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "Generated declaration identity {:?} was published by source packages @{} and @{}",
                        identity,
                        self.package(existing.package_id)?.package_prefix(),
                        prefix
                    ),
                ));
            }
            materialisation_rows.push((
                identity.clone(),
                PackageMaterialisationLocation {
                    package_id: PackageBoundaryId(self.packages.len()),
                    location,
                },
            ));
        }

        let package_id = PackageBoundaryId(self.packages.len());
        Ok(SourcePackagePublication {
            package_id,
            prefix,
            resolved_providers,
            materialisation_rows,
        })
    }

    pub(crate) fn commit(
        &mut self,
        publication: SourcePackagePublication,
        package: CompiledSourcePackage,
    ) {
        let SourcePackagePublication {
            package_id,
            prefix,
            resolved_providers,
            materialisation_rows,
        } = publication;

        self.packages.push(package);
        self.source_databases.push(None);
        // The provider vector was allocated while preflighting. Move it into the retained lane
        // so commit performs no capacity-bearing allocation after the first mutation.
        self.provider_packages.push(resolved_providers);
        self.consumer_packages.push(Vec::new());
        self.by_prefix.insert(prefix, package_id);
        for (identity, location) in &materialisation_rows {
            self.declarations_by_identity
                .insert(identity.clone(), *location);
            self.materialisation_rows
                .push((identity.clone(), *location));
        }
        for provider_index in 0..self.provider_packages[package_id.index()].len() {
            let provider_id = self.provider_packages[package_id.index()][provider_index];
            self.consumer_packages[provider_id.index()].push(package_id);
        }
    }

    /// Associate one published package with the source database that produced its diagnostics.
    ///
    /// Package results already live in this registry when this handoff occurs; keeping the Arc in
    /// the same owner lets render aggregation attach package warning ranges without reconstructing
    /// ownership from logical paths.
    pub(crate) fn set_source_database(
        &mut self,
        package_id: PackageBoundaryId,
        source_database: Arc<SourceDatabase>,
    ) -> Result<(), InfrastructureFailure> {
        let slot = self
            .source_databases
            .get_mut(package_id.index())
            .ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, format!(
                    "completed source package registry has no source database slot for boundary id {}",
                    package_id.index()
                ))
            })?;
        if slot.is_some() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "source package @{} source database was attached more than once",
                    self.package(package_id)?.package_prefix()
                ),
            ));
        }
        *slot = Some(source_database);
        Ok(())
    }

    /// Borrow the retained source database for one package boundary.
    ///
    /// Package IDs are dense publication indexes, so this lookup stays aligned with the package
    /// row and never reconstructs ownership from a diagnostic's logical path.
    pub(crate) fn source_database(
        &self,
        package_id: PackageBoundaryId,
    ) -> Option<&Arc<SourceDatabase>> {
        self.source_databases
            .get(package_id.index())
            .and_then(Option::as_ref)
    }

    pub(crate) fn reserve_commit(&mut self, publication: &SourcePackagePublication) {
        self.packages.reserve(1);
        self.source_databases.reserve(1);
        self.by_prefix.reserve(1);
        self.provider_packages.reserve(1);
        self.consumer_packages.reserve(1);
        self.declarations_by_identity
            .reserve(publication.materialisation_rows.len());
        self.materialisation_rows
            .reserve(publication.materialisation_rows.len());
        for provider_id in &publication.resolved_providers {
            self.consumer_packages[provider_id.index()].reserve(1);
        }
    }

    #[cfg(test)]
    pub(crate) fn publish(
        &mut self,
        package: CompiledSourcePackage,
        dependency_prefixes: &[String],
    ) -> Result<PackageBoundaryId, InfrastructureFailure> {
        let publication = self.preflight(&package, dependency_prefixes)?;
        let package_id = publication.package_id;
        self.reserve_commit(&publication);
        self.commit(publication, package);
        Ok(package_id)
    }

    pub(crate) fn by_prefix(&self, package_prefix: &str) -> Option<PackageBoundaryId> {
        self.by_prefix.get(package_prefix).copied()
    }

    pub(crate) fn package(
        &self,
        package_id: PackageBoundaryId,
    ) -> Result<&CompiledSourcePackage, InfrastructureFailure> {
        self.packages.get(package_id.0).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "completed source package registry has no package for boundary id {}",
                    package_id.0
                ),
            )
        })
    }

    /// Direct provider packages of one completed package, in dependency resolution order.
    pub(crate) fn provider_packages(
        &self,
        package_id: PackageBoundaryId,
    ) -> Result<&[PackageBoundaryId], InfrastructureFailure> {
        self.provider_packages
            .get(package_id.0)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, format!(
                    "completed source package registry has no provider edges for boundary id {}",
                    package_id.0
                ))
            })
    }

    /// Direct consumers of one completed package, in publication order.
    pub(crate) fn consumer_packages(
        &self,
        package_id: PackageBoundaryId,
    ) -> Result<&[PackageBoundaryId], InfrastructureFailure> {
        self.consumer_packages
            .get(package_id.0)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, format!(
                    "completed source package registry has no consumer edges for boundary id {}",
                    package_id.0
                ))
            })
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &CompiledSourcePackage> {
        self.packages.iter()
    }

    /// Number of completed source-package boundaries.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.packages.len()
    }

    /// Borrow one completed package by its dense publication index.
    #[cfg(test)]
    pub(crate) fn get(&self, index: usize) -> Option<&CompiledSourcePackage> {
        self.packages.get(index)
    }

    /// Iterate every cross-package materialisation location in deterministic package order.
    pub(crate) fn materialisation_locations(
        &self,
    ) -> impl Iterator<
        Item = (
            &GeneratedDeclarationIdentity,
            PackageMaterialisationLocation,
        ),
    > + '_ {
        self.materialisation_rows
            .iter()
            .map(|(identity, location)| (identity, *location))
    }

    /// Validate that every recorded dependency edge follows publication order.
    ///
    /// WHAT: each package's provider edges must point at earlier package IDs and its consumer
    ///       edges at later package IDs, proving the dense schedule published dependencies
    ///       before dependants.
    /// WHY: the registry records package dependency adjacency exactly once; this invariant
    ///       check keeps the recorded graph usable without re-deriving it from dependency vectors.
    pub(crate) fn validate_dependency_edges(&self) -> Result<(), InfrastructureFailure> {
        for package_id in 0..self.packages.len() {
            let package_id = PackageBoundaryId(package_id);
            for provider_id in self.provider_packages(package_id)? {
                if provider_id.index() >= package_id.index() {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "source package @{} lists provider package {} that did not publish first",
                        self.package(package_id)?.package_prefix(),
                        self.package(*provider_id)?.package_prefix()
                    );
                }
            }
            for consumer_id in self.consumer_packages(package_id)? {
                if consumer_id.index() <= package_id.index() {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "source package @{} lists consumer package {} that published before it",
                        self.package(package_id)?.package_prefix(),
                        self.package(*consumer_id)?.package_prefix()
                    );
                }
            }
        }

        Ok(())
    }

    /// Consume the registry, returning the completed packages in publication order.
    pub(crate) fn into_packages_with_source_databases(
        self,
    ) -> (Vec<CompiledSourcePackage>, Vec<Option<Arc<SourceDatabase>>>) {
        (self.packages, self.source_databases)
    }
}

/// Dense reference to one compiled module or generated sidecar inside a frontend outcome.
///
/// WHAT: a build-local handle used by entry assembly and reachability. Project and
/// source-package boundaries stay separate, so the same numeric `ModuleId` in two packages is
/// always disambiguated by the package index.
/// WHY: entry assembly must never resolve a package module through the project store or rely on
/// a flat vector layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum CompiledModuleRef {
    Project(ModuleId),
    SourcePackage {
        package_id: PackageBoundaryId,
        module_id: ModuleId,
    },
    GeneratedProject(usize),
    GeneratedSourcePackage {
        package_id: PackageBoundaryId,
        sidecar_index: usize,
    },
}

/// Collect every successful base module and generated sidecar with a stable deterministic ref.
///
/// Ordering is project base modules in `ModuleId` order, then source-package base modules in
/// package order, then generated sidecars per boundary. Construction callers use this only as a
/// transient view when building function-owner indexes; the retained boundaries stay separate.
pub(crate) fn compilation_module_views<'a>(
    project: &'a CompiledGraphBoundary,
    source_packages: &'a CompletedSourcePackageRegistry,
) -> Result<Vec<(CompiledModuleRef, &'a Module)>, InfrastructureFailure> {
    let mut views = Vec::new();

    for module_id in project
        .structure
        .nodes()
        .iter()
        .map(|node| node.module_id())
    {
        if let Some(artifact) = project.modules.artifact(module_id)? {
            views.push((CompiledModuleRef::Project(module_id), &artifact.module));
        }
    }

    for (package_index, package) in source_packages.iter().enumerate() {
        let package_id = PackageBoundaryId(package_index);
        for module_id in package
            .boundary
            .structure
            .nodes()
            .iter()
            .map(|node| node.module_id())
        {
            if let Some(artifact) = package.boundary.modules.artifact(module_id)? {
                views.push((
                    CompiledModuleRef::SourcePackage {
                        package_id,
                        module_id,
                    },
                    &artifact.module,
                ));
            }
        }
    }

    for (sidecar_index, sidecar) in project.generated.sidecars().enumerate() {
        views.push((
            CompiledModuleRef::GeneratedProject(sidecar_index),
            &sidecar.module,
        ));
    }

    for (package_index, package) in source_packages.iter().enumerate() {
        let package_id = PackageBoundaryId(package_index);
        for (sidecar_index, sidecar) in package.boundary.generated.sidecars().enumerate() {
            views.push((
                CompiledModuleRef::GeneratedSourcePackage {
                    package_id,
                    sidecar_index,
                },
                &sidecar.module,
            ));
        }
    }

    Ok(views)
}

/// Move-only domain-tagged prepared batch lane for transient check-only diagnostics.
///
/// WHAT: retains a prepared diagnostic batch alongside the package domain that produced it
///       (`None` for project transients).
/// WHY: transient units have no graph slot, so their diagnostics cross into the final render
///      tail as move-only owners with domain tags and join the run through `append_prepared`.
#[derive(Debug)]
pub(crate) struct TransientPreparedBatch {
    pub(crate) package_id: Option<PackageBoundaryId>,
    pub(crate) batch: PreparedDiagnosticBatch,
}

impl TransientPreparedBatch {
    pub(crate) fn project(batch: PreparedDiagnosticBatch) -> Self {
        Self {
            package_id: None,
            batch,
        }
    }

    pub(crate) fn package(package_id: PackageBoundaryId, batch: PreparedDiagnosticBatch) -> Self {
        Self {
            package_id: Some(package_id),
            batch,
        }
    }
}
/// Storage retained by the final diagnostic report boundary.
///
/// The source fields count only frozen source owners reachable from the returned report.
/// Diagnostic fields count records and label slots, while `retained_identity_contexts` counts
/// distinct frozen identity allocations reachable through source rows or donor-only handles.
/// The `path_table_*` fields count distinct path-table owners reachable from that same report:
/// frozen identity contexts behind source rows or donor-only handles, explicit path rows, and
/// retained path tables. Successful-module executable views are deliberately not counted; a table
/// only reachable from executable rows without a diagnostic owner is not report-retained storage.
/// The `source_tokens_*`, construction, donor-snapshot and generic-owner fields are command-scoped
/// ledger observations and can remain nonzero for clean results even though report-retention
/// fields stay zero.
#[cfg(feature = "data_layout_memory_probe")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrozenRenderRetentionMetrics {
    pub(crate) source_snapshot_bytes: usize,
    pub(crate) source_text_capacity_bytes: usize,
    pub(crate) source_line_start_rows: usize,
    pub(crate) source_line_start_bytes: usize,
    pub(crate) source_line_start_capacity_bytes: usize,
    pub(crate) extended_span_rows: usize,
    pub(crate) extended_span_bytes: usize,
    pub(crate) extended_span_duplicate_rows: usize,
    pub(crate) source_identity_slots: usize,
    pub(crate) source_identity_slot_capacity_bytes: usize,
    pub(crate) source_provenance_field_bytes: usize,
    pub(crate) compilation_root_slots: usize,
    pub(crate) authored_physical_slots: usize,
    pub(crate) canonical_os_path_bytes: usize,
    pub(crate) loaded_source_records: usize,
    pub(crate) loaded_source_record_capacity_bytes: usize,
    pub(crate) diagnostic_records: usize,
    pub(crate) diagnostic_record_capacity_bytes: usize,
    pub(crate) diagnostic_label_slots: usize,
    pub(crate) diagnostic_extra_rows: usize,
    pub(crate) diagnostic_extra_capacity_bytes: usize,
    pub(crate) diagnostic_label_rows: usize,
    pub(crate) diagnostic_label_capacity_bytes: usize,
    pub(crate) diagnostic_label_data_rows: usize,
    pub(crate) diagnostic_label_data_capacity_bytes: usize,
    pub(crate) diagnostic_string_list_rows: usize,
    pub(crate) diagnostic_string_list_capacity_bytes: usize,
    pub(crate) diagnostic_substitution_rows: usize,
    pub(crate) diagnostic_substitution_capacity_bytes: usize,
    pub(crate) diagnostic_foreign_site_rows: usize,
    pub(crate) diagnostic_foreign_site_capacity_bytes: usize,
    pub(crate) type_display_record_rows: usize,
    pub(crate) type_display_record_capacity_bytes: usize,
    pub(crate) type_display_auxiliary_rows: usize,
    pub(crate) type_display_auxiliary_capacity_bytes: usize,
    pub(crate) diagnostic_report_count: usize,
    pub(crate) diagnostic_report_capacity_bytes: usize,
    pub(crate) diagnostic_run_count: usize,
    pub(crate) diagnostic_run_capacity_bytes: usize,
    pub(crate) diagnostic_foreign_context_slots: usize,
    pub(crate) diagnostic_foreign_context_capacity_bytes: usize,
    pub(crate) frozen_context_shell_bytes: usize,
    pub(crate) frozen_string_tables: usize,
    pub(crate) frozen_string_storage_bytes: usize,
    pub(crate) retained_identity_contexts: usize,
    /// Distinct path-table owners reachable from the returned diagnostic report.
    pub(crate) path_table_count: usize,
    /// Actual retained `PathNode` rows across those deduplicated path tables.
    pub(crate) path_table_node_rows: usize,
    /// Backing vector capacity bytes for those deduplicated path tables.
    pub(crate) path_table_storage_bytes: usize,
    /// Distinct canonical SourceTokens owners observed during this command.
    pub(crate) source_tokens_owners: usize,
    pub(crate) source_tokens_shape_bytes: usize,
    pub(crate) source_tokens_shape_capacity_bytes: usize,
    pub(crate) source_tokens_span_bytes: usize,
    pub(crate) source_tokens_span_capacity_bytes: usize,
    pub(crate) source_tokens_numeric_store_bytes: usize,
    pub(crate) source_tokens_numeric_store_capacity_bytes: usize,
    pub(crate) source_tokens_path_store_bytes: usize,
    pub(crate) source_tokens_path_store_capacity_bytes: usize,
    pub(crate) source_tokens_sequence_store_bytes: usize,
    pub(crate) source_tokens_sequence_store_capacity_bytes: usize,
    /// Cumulative builder shape/span capacities sampled at finish; not simultaneous live storage.
    pub(crate) transient_construction_buffer_bytes: usize,
    /// Largest individual builder buffer sample; a high-water observation, not an aggregate live
    /// allocation claim.
    pub(crate) transient_construction_peak_bytes: usize,
    pub(crate) donor_identity_string_tables: usize,
    pub(crate) donor_identity_string_storage_bytes: usize,
    pub(crate) donor_identity_path_tables: usize,
    pub(crate) mutable_string_table_samples: usize,
    pub(crate) mutable_string_table_slot_capacity_bytes: usize,
    pub(crate) mutable_string_table_local_payload_bytes: usize,
    pub(crate) mutable_string_table_reverse_capacity_entries: usize,
    pub(crate) mutable_string_table_inherited_prefix_rows: usize,
    pub(crate) mutable_string_table_inherited_prefix_slot_bytes: usize,
    pub(crate) mutable_string_table_inherited_reverse_capacity_entries: usize,
    pub(crate) mutable_string_table_peak_slot_capacity_bytes: usize,
    pub(crate) mutable_path_builder_samples: usize,
    pub(crate) mutable_path_table_storage_bytes: usize,
    pub(crate) mutable_path_builder_lookup_layers: usize,
    pub(crate) mutable_path_builder_lookup_capacity_entries: usize,
    pub(crate) mutable_path_builder_local_lookup_capacity_entries: usize,
    pub(crate) donor_identity_path_storage_bytes: usize,
    pub(crate) generic_source_tokens_owners: usize,
    pub(crate) generic_source_tokens_bytes: usize,
    pub(crate) generic_source_tokens_live_bytes: usize,
    pub(crate) generic_source_tokens_peak_bytes: usize,
    pub(crate) requester_remap_count: usize,
    pub(crate) requester_remap_used_bytes: usize,
    pub(crate) requester_remap_capacity_bytes: usize,
    pub(crate) string_merge_remap_ns: u64,
    pub(crate) path_merge_remap_ns: u64,
    pub(crate) observation_incomplete: bool,
}
#[cfg(feature = "data_layout_memory_probe")]
impl FrozenRenderRetentionMetrics {
    pub(crate) fn from_ledger(snapshot: MemoryLedgerSnapshot) -> Self {
        Self {
            source_tokens_owners: snapshot.source_tokens_owners,
            source_tokens_shape_bytes: snapshot.source_tokens_shape_bytes,
            source_tokens_shape_capacity_bytes: snapshot.source_tokens_shape_capacity_bytes,
            source_tokens_span_bytes: snapshot.source_tokens_span_bytes,
            source_tokens_span_capacity_bytes: snapshot.source_tokens_span_capacity_bytes,
            source_tokens_numeric_store_bytes: snapshot.source_tokens_numeric_store_bytes,
            source_tokens_numeric_store_capacity_bytes: snapshot
                .source_tokens_numeric_store_capacity_bytes,
            source_tokens_path_store_bytes: snapshot.source_tokens_path_store_bytes,
            source_tokens_path_store_capacity_bytes: snapshot
                .source_tokens_path_store_capacity_bytes,
            source_tokens_sequence_store_bytes: snapshot.source_tokens_sequence_store_bytes,
            source_tokens_sequence_store_capacity_bytes: snapshot
                .source_tokens_sequence_store_capacity_bytes,
            transient_construction_buffer_bytes: snapshot.transient_construction_buffer_bytes,
            transient_construction_peak_bytes: snapshot.transient_construction_peak_bytes,
            donor_identity_string_tables: snapshot.donor_identity_string_tables,
            donor_identity_string_storage_bytes: snapshot.donor_identity_string_storage_bytes,
            donor_identity_path_tables: snapshot.donor_identity_path_tables,
            mutable_string_table_samples: snapshot.mutable_string_table_samples,
            mutable_string_table_slot_capacity_bytes: snapshot
                .mutable_string_table_slot_capacity_bytes,
            mutable_string_table_local_payload_bytes: snapshot
                .mutable_string_table_local_payload_bytes,
            mutable_string_table_reverse_capacity_entries: snapshot
                .mutable_string_table_reverse_capacity_entries,
            mutable_string_table_inherited_prefix_rows: snapshot
                .mutable_string_table_inherited_prefix_rows,
            mutable_string_table_inherited_prefix_slot_bytes: snapshot
                .mutable_string_table_inherited_prefix_slot_bytes,
            mutable_string_table_inherited_reverse_capacity_entries: snapshot
                .mutable_string_table_inherited_reverse_capacity_entries,
            mutable_string_table_peak_slot_capacity_bytes: snapshot
                .mutable_string_table_peak_slot_capacity_bytes,
            mutable_path_builder_samples: snapshot.mutable_path_builder_samples,
            mutable_path_table_storage_bytes: snapshot.mutable_path_table_storage_bytes,
            mutable_path_builder_lookup_layers: snapshot.mutable_path_builder_lookup_layers,
            mutable_path_builder_lookup_capacity_entries: snapshot
                .mutable_path_builder_lookup_capacity_entries,
            mutable_path_builder_local_lookup_capacity_entries: snapshot
                .mutable_path_builder_local_lookup_capacity_entries,
            donor_identity_path_storage_bytes: snapshot.donor_identity_path_storage_bytes,
            generic_source_tokens_owners: snapshot.generic_source_tokens_owners,
            generic_source_tokens_bytes: snapshot.generic_source_tokens_bytes,
            generic_source_tokens_live_bytes: snapshot.generic_source_tokens_live_bytes,
            generic_source_tokens_peak_bytes: snapshot.generic_source_tokens_peak_bytes,
            requester_remap_count: snapshot.requester_remap_count,
            requester_remap_used_bytes: snapshot.requester_remap_used_bytes,
            requester_remap_capacity_bytes: snapshot.requester_remap_capacity_bytes,
            string_merge_remap_ns: snapshot.string_merge_remap_ns,
            path_merge_remap_ns: snapshot.path_merge_remap_ns,
            observation_incomplete: snapshot.observation_incomplete,
            ..Self::default()
        }
    }
    pub(crate) fn from_ledger_with_reports(
        snapshot: MemoryLedgerSnapshot,
        reports: &DiagnosticReportSet,
    ) -> Self {
        let mut retention = Self::from_ledger(snapshot);
        retention.add_report_storage(reports);
        retention
    }

    fn add_source(&mut self, metrics: SourceDatabaseRetentionMetrics) {
        self.source_snapshot_bytes += metrics.source_snapshot_bytes;
        self.source_text_capacity_bytes += metrics.source_text_capacity_bytes;
        self.source_line_start_rows += metrics.source_line_start_rows;
        self.source_line_start_bytes += metrics.source_line_start_bytes;
        self.extended_span_rows += metrics.extended_span_rows;
        self.source_line_start_capacity_bytes += metrics.source_line_start_capacity_bytes;
        self.extended_span_bytes += metrics.extended_span_bytes;
        self.extended_span_duplicate_rows += metrics.extended_span_duplicate_rows;
        self.source_identity_slots += metrics.source_identity_slots;
        self.source_identity_slot_capacity_bytes += metrics.source_identity_slot_capacity_bytes;
        self.source_provenance_field_bytes += metrics.source_provenance_field_bytes;
        self.compilation_root_slots += metrics.compilation_root_slots;
        self.authored_physical_slots += metrics.authored_physical_slots;
        self.canonical_os_path_bytes += metrics.canonical_os_path_bytes;
        self.loaded_source_records += metrics.loaded_source_records;
        self.loaded_source_record_capacity_bytes += metrics.loaded_source_record_capacity_bytes;
    }

    fn add_report_storage(&mut self, reports: &DiagnosticReportSet) {
        let report_metrics = reports.memory_ledger_metrics();
        self.diagnostic_records = report_metrics.record_rows;
        self.diagnostic_record_capacity_bytes = report_metrics.record_capacity_bytes;
        self.diagnostic_label_slots = report_metrics.label_rows;
        self.diagnostic_extra_rows = report_metrics.extra_rows;
        self.diagnostic_extra_capacity_bytes = report_metrics.extra_capacity_bytes;
        self.diagnostic_label_rows = report_metrics.label_rows;
        self.diagnostic_label_capacity_bytes = report_metrics.label_capacity_bytes;
        self.diagnostic_label_data_rows = report_metrics.label_data_rows;
        self.diagnostic_label_data_capacity_bytes = report_metrics.label_data_capacity_bytes;
        self.diagnostic_string_list_rows = report_metrics.string_list_rows;
        self.diagnostic_string_list_capacity_bytes = report_metrics.string_list_capacity_bytes;
        self.diagnostic_substitution_rows = report_metrics.substitution_rows;
        self.diagnostic_substitution_capacity_bytes = report_metrics.substitution_capacity_bytes;
        self.diagnostic_foreign_site_rows = report_metrics.foreign_site_rows;
        self.diagnostic_foreign_site_capacity_bytes = report_metrics.foreign_site_capacity_bytes;
        self.type_display_record_rows = report_metrics.type_display_record_rows;
        self.type_display_record_capacity_bytes = report_metrics.type_display_record_capacity_bytes;
        self.type_display_auxiliary_rows = report_metrics.type_display_auxiliary_rows;
        self.type_display_auxiliary_capacity_bytes =
            report_metrics.type_display_auxiliary_capacity_bytes;
        self.diagnostic_report_count = report_metrics.report_rows;
        self.diagnostic_report_capacity_bytes = report_metrics.report_capacity_bytes;
        self.diagnostic_run_count = report_metrics.run_rows;
        self.diagnostic_run_capacity_bytes = report_metrics.run_capacity_bytes;
        self.diagnostic_foreign_context_slots = report_metrics.foreign_context_rows;
        self.diagnostic_foreign_context_capacity_bytes =
            report_metrics.foreign_context_capacity_bytes;

        for (index, identity) in reports.contexts().enumerate() {
            let identity_address = identity as *const FrozenIdentityContext as usize;
            let already_counted = reports.contexts().take(index).any(|previous| {
                previous as *const FrozenIdentityContext as usize == identity_address
            });
            if !already_counted {
                self.add_source(identity.retention_metrics());
                self.retained_identity_contexts += 1;
                self.frozen_context_shell_bytes += std::mem::size_of::<FrozenIdentityContext>();
            }

            let strings = identity.strings();
            let string_address = strings as *const FrozenStringTable as usize;
            let string_already_counted = reports.contexts().take(index).any(|previous| {
                previous.strings() as *const FrozenStringTable as usize == string_address
            });
            if !string_already_counted {
                self.frozen_string_tables += 1;
                self.frozen_string_storage_bytes += strings.storage_bytes();
            }

            let paths = identity.paths();
            let path_address = paths as *const PathTable as usize;
            let path_already_counted = reports
                .contexts()
                .take(index)
                .any(|previous| previous.paths() as *const PathTable as usize == path_address);
            if !path_already_counted {
                self.path_table_count += 1;
                self.path_table_node_rows += paths.len();
                self.path_table_storage_bytes += paths.storage_bytes();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn from_reports(reports: &DiagnosticReportSet) -> Self {
        let mut retention = Self::default();
        retention.add_report_storage(reports);
        retention
    }
}

#[cfg(not(feature = "data_layout_memory_probe"))]
type FrozenRenderRetentionMetrics = ();

/// Typed frontend outcome carrying every project and source-package graph boundary.
///
/// WHAT: the `Ok` payload of [`compile_project_frontend`](super::compile_project_frontend).
/// It retains successful boundaries beside diagnosed and blocked outcomes so `check` and
/// benchmark tooling never lose independent work to an aggregate `Err`.
/// WHY: user diagnostics are retained data, not an infrastructure failure. Only
/// [`InfrastructureFailure`] aborts compilation.
pub(crate) struct ProjectFrontendCompilation {
    pub(crate) project: CompiledGraphBoundary,
    pub(crate) source_packages: CompletedSourcePackageRegistry,
    /// Build-only physical resource inputs and missing-target watches discovered before AST.
    pub(crate) resource_inputs: ResourceInputRegistry,
    /// Diagnostics and warnings from transient check-only units, which have no canonical graph slot.
    ///
    /// WHAT: move-only premerge batches tagged by domain. They stay retained until the canonical
    ///       tail can append them after project and package graph diagnostics.
    pub(crate) transient_batches: Vec<TransientPreparedBatch>,
    /// Project source identity retained by the no-input frontend seam for its caller to pass into
    /// the frozen render tail.
    ///
    /// Command-owned callers provide this owner directly; retaining it here only keeps the
    /// config-free seam move-only until its caller invokes the canonical frozen renderer.
    pub(crate) project_source_database: Option<Arc<SourceDatabase>>,
}

impl ProjectFrontendCompilation {
    pub(crate) fn new(
        project: CompiledGraphBoundary,
        source_packages: CompletedSourcePackageRegistry,
        resource_inputs: ResourceInputRegistry,
        transient_batches: Vec<TransientPreparedBatch>,
    ) -> Result<Self, InfrastructureFailure> {
        project.validate_invariants()?;
        source_packages.validate_dependency_edges()?;
        resource_inputs.validate()?;
        for package in source_packages.iter() {
            package.validate()?;
        }

        // The project store and package registry already enforce uniqueness inside their own
        // lanes, so the final handoff only needs to prove that one generated declaration
        // identity is never published by both a project boundary and a source package. Iterate
        // the package index directly and allocate no owner strings unless a collision exists.
        let mut package_owners =
            FxHashMap::<&GeneratedDeclarationIdentity, PackageBoundaryId>::default();
        for (identity, location) in source_packages.materialisation_locations() {
            package_owners.insert(identity, location.package_id);
        }
        for (identity, location) in project.modules.materialisation_locations() {
            if let Some(package_id) = package_owners.get(identity) {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Generated declaration identity {:?} is published by both project module {} and source package @{}",
                    identity,
                    location.artifact_id.index(),
                    source_packages.package(*package_id)?.package_prefix()
                );
            }
        }

        Ok(Self {
            project,
            source_packages,
            resource_inputs,
            transient_batches,
            project_source_database: None,
        })
    }

    /// Whether any boundary contains a diagnosed or blocked module.
    ///
    /// `build` and `dev` use this gate before assembling a success-only [`ProjectCompilation`]
    /// while `check` renders the retained outcomes instead.
    pub(crate) fn has_diagnosed_or_blocked(&self) -> bool {
        !self.project.diagnosed.is_empty()
            || !self.project.blocked.is_empty()
            || self
                .transient_batches
                .iter()
                .any(|transient| transient.batch.has_errors())
            || self.source_packages.has_diagnosed_or_blocked()
    }

    /// Iterate every successful module view (base artefacts and generated sidecars) across all
    /// boundaries in deterministic order.
    #[cfg(test)]
    pub(crate) fn successful_module_views(&self) -> impl Iterator<Item = &Module> + '_ {
        self.project.successful_module_views().chain(
            self.source_packages
                .iter()
                .flat_map(|package| package.boundary.successful_module_views()),
        )
    }

    /// Convert diagnosed graph work at the owning frontend boundary. Successful warnings remain
    /// staged in the command-owned accumulator until its report owner releases the source database.
    pub(crate) fn into_compilation_result(
        mut self,
        warning_owner: &mut BoundaryAccumulator,
        project_source: Option<Arc<SourceDatabase>>,
        trailing_facade_failure: Option<ProjectAssemblyError>,
    ) -> CompilationResult<Self> {
        if self.has_diagnosed_or_blocked() || trailing_facade_failure.is_some() {
            let reports = self.into_diagnostic_reports(
                warning_owner,
                project_source,
                trailing_facade_failure,
            )?;
            return Ok(CompilationOutcome::Diagnosed(reports));
        }

        let project_source = project_source.or_else(|| self.project_source_database.clone());
        self.project_source_database = project_source;
        self.stage_transient_warnings(warning_owner)?;
        self.retain_successful_warning_identity_domains(warning_owner);
        Ok(CompilationOutcome::Success {
            value: self,
            warnings: None,
        })
    }
    fn retain_successful_warning_identity_domains(&self, warning_owner: &mut BoundaryAccumulator) {
        if let (Some(identity), Some(source_database)) = (
            self.project.structure.stable_package_identity(),
            self.project_source_database.as_ref(),
        ) {
            warning_owner.retain_successful_warning_identity_domain(identity, source_database);
        }
        for (package, source_database) in self
            .source_packages
            .packages
            .iter()
            .zip(&self.source_packages.source_databases)
        {
            if let Some(source_database) = source_database.as_ref() {
                warning_owner.retain_successful_warning_identity_domain(
                    &package.package_identity,
                    source_database,
                );
            }
        }
    }

    fn stage_transient_warnings(
        &mut self,
        warning_owner: &mut BoundaryAccumulator,
    ) -> Result<(), InfrastructureErrors> {
        let transient_batches = std::mem::take(&mut self.transient_batches);
        if transient_batches
            .iter()
            .all(|transient| transient.batch.diagnostic_count() == 0)
        {
            return Ok(());
        }

        let (_, mut warning_sink) = warning_owner.compiler_parts_mut();
        for transient in transient_batches {
            if transient.batch.diagnostic_count() == 0 {
                continue;
            }
            let Some(package_id) = transient.package_id else {
                warning_sink.append(
                    SuccessfulWarningBoundary::Project,
                    SuccessfulWarningPosition::CommandTail,
                    transient.batch,
                );
                continue;
            };

            let package = self.source_packages.package(package_id)?;
            let source_database = self
                .source_packages
                .source_database(package_id)
                .cloned()
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "check-only warning package @{} has no finalized source database",
                            package.package_prefix()
                        ),
                    )
                })?;
            let boundary = SuccessfulWarningBoundary::SourcePackage(package_id.index());
            warning_sink.set_source_package_identity(boundary, package.package_identity.clone());
            warning_sink.append(
                boundary,
                SuccessfulWarningPosition::CommandTail,
                transient.batch,
            );
            warning_sink.commit_boundary(boundary, Some(source_database));
        }
        Ok(())
    }

    /// Freeze only diagnosed graph work. Committed success warnings never share this report set.
    pub(crate) fn into_diagnostic_reports(
        self,
        warning_owner: &mut BoundaryAccumulator,
        project_source: Option<Arc<SourceDatabase>>,
        trailing_facade_failure: Option<ProjectAssemblyError>,
    ) -> Result<DiagnosticReportSet, InfrastructureErrors> {
        self.into_diagnostic_reports_with_retention(
            warning_owner,
            project_source,
            trailing_facade_failure,
        )
        .0
    }

    fn into_diagnostic_reports_with_retention(
        self,
        warning_owner: &mut BoundaryAccumulator,
        project_source: Option<Arc<SourceDatabase>>,
        trailing_facade_failure: Option<ProjectAssemblyError>,
    ) -> (
        Result<DiagnosticReportSet, InfrastructureErrors>,
        FrozenRenderRetentionMetrics,
    ) {
        let Self {
            mut project,
            source_packages,
            transient_batches,
            project_source_database,
            ..
        } = self;
        let project_diagnosed = std::mem::take(&mut project.diagnosed);
        let (mut source_packages, package_source_arcs) =
            source_packages.into_packages_with_source_databases();
        #[cfg(feature = "data_layout_memory_probe")]
        {
            project.modules.record_memory_ledger();
            for package in &source_packages {
                package.boundary.modules.record_memory_ledger();
            }
        }
        #[cfg(feature = "data_layout_memory_probe")]
        let ledger_snapshot = crate::compiler_frontend::instrumentation::snapshot_memory_ledger();

        let project_domain = project.structure.stable_package_identity().cloned();
        let project_source = project_source.or(project_source_database);
        let trailing_facade_failure = trailing_facade_failure.map(|failure| {
            let diagnostic_strings = DiagnosticStringBuilder::new(warning_owner.string_table_mut());
            failure.into_prepared_failure(&diagnostic_strings)
        });
        warning_owner.discard_successful_warnings();
        let mut accumulator = std::mem::replace(warning_owner, BoundaryAccumulator::new());
        if let Some(source) = project_source.as_ref() {
            accumulator.set_project_source_database(Arc::clone(source));
        }
        let mut diagnostic_count = accumulator.diagnostic_count();
        let mut can_freeze = true;
        let mut saw_infrastructure = false;
        #[cfg(feature = "data_layout_memory_probe")]
        let (mut diagnostic_records, mut diagnostic_label_slots) =
            (diagnostic_count, accumulator.prepared_label_count());

        // Diagnosed graph modules are already in canonical ModuleId order.
        for diagnosed in project_diagnosed {
            let metrics = append_prepared_frontend_diagnostics(
                &mut accumulator,
                diagnosed.diagnostics,
                project_source.as_ref().map(|source| (source, None)),
                diagnostic_count,
                &mut can_freeze,
            );
            #[cfg(feature = "data_layout_memory_probe")]
            {
                diagnostic_records += metrics.diagnostic_count;
                diagnostic_label_slots += metrics.label_count;
            }
            diagnostic_count += metrics.diagnostic_count;
            saw_infrastructure |= metrics.missing_source_error;
        }
        for (package_index, package) in source_packages.iter_mut().enumerate() {
            let source = package_source_arcs
                .get(package_index)
                .unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "diagnostic range references missing source package identity index {package_index}"
                    )
                })
                .as_ref();
            let domain = Some(package.package_identity.clone());
            for diagnosed in std::mem::take(&mut package.boundary.diagnosed) {
                let metrics = append_prepared_frontend_diagnostics(
                    &mut accumulator,
                    diagnosed.diagnostics,
                    source.map(|source| (source, domain.clone())),
                    diagnostic_count,
                    &mut can_freeze,
                );
                #[cfg(feature = "data_layout_memory_probe")]
                {
                    diagnostic_records += metrics.diagnostic_count;
                    diagnostic_label_slots += metrics.label_count;
                }
                diagnostic_count += metrics.diagnostic_count;
                saw_infrastructure |= metrics.missing_source_error;
            }
        }
        for transient in transient_batches {
            if !transient.batch.has_errors() {
                continue;
            }
            let (source, domain) = match transient.package_id {
                Some(package_id) => {
                    let package_index = package_id.index();
                    let package = source_packages.get(package_index).unwrap_or_else(|| {
                        crate::compiler_bug!(
                            CompilerStage::ProjectBuildAssembly,
                            "diagnostic range references missing source package identity index {package_index}"
                        )
                    });
                    let source = package_source_arcs
                        .get(package_index)
                        .unwrap_or_else(|| {
                            crate::compiler_bug!(
                                CompilerStage::ProjectBuildAssembly,
                                "diagnostic range references missing source package identity index {package_index}"
                            )
                        })
                        .as_ref();
                    let domain = Some(package.package_identity.clone());
                    (source, domain)
                }
                None => (project_source.as_ref(), None),
            };
            let metrics = append_prepared_frontend_diagnostics(
                &mut accumulator,
                transient.batch,
                source.map(|source| (source, domain)),
                diagnostic_count,
                &mut can_freeze,
            );
            #[cfg(feature = "data_layout_memory_probe")]
            {
                diagnostic_records += metrics.diagnostic_count;
                diagnostic_label_slots += metrics.label_count;
            }
            diagnostic_count += metrics.diagnostic_count;
            saw_infrastructure |= metrics.missing_source_error;
        }
        if let Some(failure) = trailing_facade_failure {
            match failure {
                PreparedFailure::Diagnosed(batch) => {
                    let metrics = append_prepared_frontend_diagnostics(
                        &mut accumulator,
                        batch,
                        project_source.as_ref().map(|source| (source, None)),
                        diagnostic_count,
                        &mut can_freeze,
                    );
                    #[cfg(feature = "data_layout_memory_probe")]
                    {
                        diagnostic_records += metrics.diagnostic_count;
                        diagnostic_label_slots += metrics.label_count;
                    }
                    diagnostic_count += metrics.diagnostic_count;
                    saw_infrastructure |= metrics.missing_source_error;
                }
                PreparedFailure::Infrastructure(errors) => {
                    accumulator.append_infrastructure(errors);
                    saw_infrastructure = true;
                }
            }
        }
        // Keep the project source inventory available as the identityless fallback for package
        // diagnostics whose source IDs collide outside their package identity handles.
        if let Some(source) = project_source.as_ref() {
            accumulator.set_project_source_database(Arc::clone(source));
        }

        #[cfg(feature = "data_layout_memory_probe")]
        let retention = {
            let mut retention = FrozenRenderRetentionMetrics::from_ledger(ledger_snapshot);
            retention.diagnostic_records = diagnostic_records;
            retention.diagnostic_label_slots = diagnostic_label_slots;
            retention
        };
        #[cfg(not(feature = "data_layout_memory_probe"))]
        let retention = ();

        // A clean result has no source-backed render work. Freeze an empty run so the common
        // success path retains no caller string table, source snapshot, path trie or extended-span
        // table solely for an empty report set.
        if diagnostic_count == 0 && !saw_infrastructure {
            drop(project_source);
            drop(package_source_arcs);
            drop(source_packages);
            drop(project);
            let messages = accumulator.finish(InstallMode::Frozen);
            return (messages, retention);
        }

        // Preserve donor rows after every diagnostic range, including identities referenced only
        // by a foreign frozen handle. Empty rows do not change diagnostic pairing.
        if let Some(source) = project_source.as_ref() {
            let donor = PreparedDiagnosticBatch::capture_from_bag(
                DiagnosticBag::new(),
                StringTable::empty_unallocated(),
                None,
                None,
            );
            accumulator.append_prepared(donor, Some((Arc::clone(source), project_domain.clone())));
        }
        for (package, source) in source_packages.iter().zip(&package_source_arcs) {
            if let Some(source) = source {
                let donor = PreparedDiagnosticBatch::capture_from_bag(
                    DiagnosticBag::new(),
                    StringTable::empty_unallocated(),
                    None,
                    None,
                );
                accumulator.append_prepared(
                    donor,
                    Some((Arc::clone(source), Some(package.package_identity.clone()))),
                );
            }
        }

        // The accumulator must own the only source handles before it unwraps its source rows.
        drop(project_source);
        drop(package_source_arcs);
        drop(source_packages);
        drop(project);

        let install = if can_freeze {
            InstallMode::Frozen
        } else {
            InstallMode::Unfrozen
        };
        let messages = accumulator.finish(install);
        (messages, retention)
    }
}
