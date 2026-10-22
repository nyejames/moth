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
    BoundaryFailure, CompilerError, CompilerMessages, RenderSourceContext,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticBag};
#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::instrumentation::MemoryLedgerSnapshot;
use crate::compiler_frontend::module_compilation::{CompiledModuleArtifact, Module};
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModuleRootRole, StablePackageIdentity,
};
use crate::compiler_frontend::source::SourceDatabase;
#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::source::{FrozenIdentityContext, SourceDatabaseRetentionMetrics};
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::compiler_frontend::symbols::string_interning::{FrozenStringTable, StringTable};
use rustc_hash::{FxHashMap, FxHashSet};
use std::ops::Range;
use std::sync::Arc;

use super::generated_store::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::MaterialisationContextLocation;
use super::module_artifact_store::{ModuleArtifactStore, ProviderSlot};
use super::module_identity::ModuleId;
use super::project_module_graph::ProjectModuleGraph;
use super::resource_inputs::ResourceInputRegistry;
/// One prepared successful-warning batch and its source owner, if the batch came from a package.
pub(crate) struct SuccessfulWarningBatch {
    pub(crate) batch: PreparedDiagnosticBatch,
    pub(crate) source: Option<(Arc<SourceDatabase>, Option<StablePackageIdentity>)>,
}

fn install_frontend_source_context(
    messages: &mut CompilerMessages,
    local_range: Range<usize>,
    diagnostic_offset: usize,
    source: Option<&Arc<SourceDatabase>>,
    domain: Option<StablePackageIdentity>,
    can_freeze: &mut bool,
) {
    if local_range.is_empty() {
        return;
    }
    if let Some(source) = source {
        messages.install_source_contexts(
            [RenderSourceContext {
                diagnostic_range: local_range,
                source_database: Arc::clone(source),
                domain,
            }],
            0,
        );
    } else {
        let global_range =
            (diagnostic_offset + local_range.start)..(diagnostic_offset + local_range.end);
        if diagnostic_range_requires_source_identity(messages.diagnostic_slice(), &local_range) {
            messages.set_infrastructure_error(CompilerError::compiler_error(format!(
                "diagnostic range {global_range:?} has no finalized source identity",
            )));
        }
        *can_freeze = false;
    }
}

#[cfg(feature = "data_layout_memory_probe")]
fn record_frontend_diagnostic_metrics(
    messages: &CompilerMessages,
    diagnostic_records: &mut usize,
    diagnostic_label_slots: &mut usize,
) {
    *diagnostic_records += messages.diagnostics.len();
    *diagnostic_label_slots += messages
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.labels.len())
        .sum::<usize>();
}
fn diagnostic_range_requires_source_identity(
    diagnostics: &[CompilerDiagnostic],
    range: &std::ops::Range<usize>,
) -> bool {
    diagnostics[range.clone()].iter().any(|diagnostic| {
        diagnostic.primary_span.is_some()
            || diagnostic.labels.iter().any(|label| label.span.is_some())
    })
}
struct PreparedAppendMetrics {
    diagnostic_count: usize,
    missing_source_error: bool,
    #[cfg(feature = "data_layout_memory_probe")]
    label_count: usize,
}

fn append_prepared_frontend_diagnostics(
    accumulator: &mut BoundaryAccumulator,
    batch: PreparedDiagnosticBatch,
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
            let error = CompilerError::compiler_error(format!(
                "diagnostic range {global_range:?} has no finalized source identity",
            ));
            accumulator.append(CompilerMessages::from_error(error, StringTable::new()));
        }
        accumulator.append_prepared(batch, None);
        return PreparedAppendMetrics {
            diagnostic_count,
            missing_source_error,
            #[cfg(feature = "data_layout_memory_probe")]
            label_count,
        };
    };

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
    pub(crate) fn finish(mut self) -> Result<Self, CompilerError> {
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
    /// WHY: dense lookups are only safe after this validation; an invalid mapping must surface
    ///       as `CompilerError` instead of becoming an absent artefact later. The success-only
    ///       `ProjectCompilation` gate separately adds the all-successful requirement.
    pub(crate) fn validate_invariants(&self) -> Result<(), CompilerError> {
        if self.structure.nodes().len() != self.modules.slot_count() {
            return Err(CompilerError::compiler_error(format!(
                "Graph retained {} nodes but module store has {} slots",
                self.structure.nodes().len(),
                self.modules.slot_count()
            )));
        }

        // Dense lane proof: every diagnosed or blocked record claims one distinct slot lane.
        // `ModuleId` is already a dense index, so a compact vector replaces hashing and keeps
        // first-error selection deterministic.
        let mut retained_lanes = vec![RetainedOutcomeLane::None; self.modules.slot_count()];
        for diagnosed in &self.diagnosed {
            let lane = retained_lanes
                .get_mut(diagnosed.module_id.index())
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Diagnosed module {} is out of range for {} slots",
                        diagnosed.module_id.index(),
                        self.modules.slot_count()
                    ))
                })?;
            if *lane != RetainedOutcomeLane::None {
                return Err(CompilerError::compiler_error(format!(
                    "ModuleId {} is both diagnosed and blocked or appears more than once in the diagnosed lane",
                    diagnosed.module_id.index()
                )));
            }
            *lane = RetainedOutcomeLane::Diagnosed;
            if self.modules.slot(diagnosed.module_id)? != ProviderSlot::Diagnosed {
                return Err(CompilerError::compiler_error(format!(
                    "Diagnosed module {} does not hold the diagnosed store slot",
                    diagnosed.module_id.index()
                )));
            }
        }
        for blocked in &self.blocked {
            let lane = retained_lanes
                .get_mut(blocked.module_id.index())
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Blocked module {} is out of range for {} slots",
                        blocked.module_id.index(),
                        self.modules.slot_count()
                    ))
                })?;
            if *lane != RetainedOutcomeLane::None {
                return Err(CompilerError::compiler_error(format!(
                    "ModuleId {} is both diagnosed and blocked or appears more than once in the blocked lane",
                    blocked.module_id.index()
                )));
            }
            *lane = RetainedOutcomeLane::Blocked;
            if self.modules.slot(blocked.module_id)? != ProviderSlot::Blocked {
                return Err(CompilerError::compiler_error(format!(
                    "Blocked module {} does not hold the blocked store slot",
                    blocked.module_id.index()
                )));
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
                    return Err(CompilerError::compiler_error(format!(
                        "ModuleId {index} never reached a completed outcome"
                    )));
                }
                ProviderSlot::Successful(artifact_id) => {
                    let artifact = self.modules.artifact(module_id)?.ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "ModuleId {index} references missing artifact row {}",
                            artifact_id.index()
                        ))
                    })?;
                    let row_count = row_owners.len();
                    let owner: &mut Option<ModuleId> = row_owners
                        .get_mut(artifact_id.index())
                        .ok_or_else(|| {
                            CompilerError::compiler_error(format!(
                                "ModuleId {index} references artifact row {} outside the retained artefact lane of {} rows",
                                artifact_id.index(),
                                row_count
                            ))
                        })?;
                    if let Some(existing) = owner {
                        return Err(CompilerError::compiler_error(format!(
                            "Artifact row {} is referenced by both ModuleId {} and ModuleId {index}",
                            artifact_id.index(),
                            existing.index()
                        )));
                    }
                    *owner = Some(module_id);
                    let node = &self.structure.nodes()[index];
                    if &artifact.interface.module_origin != node.stable_origin() {
                        return Err(CompilerError::compiler_error(format!(
                            "ModuleId {index} artefact interface origin {:?} disagrees with its graph node origin {:?}",
                            artifact.interface.module_origin,
                            node.stable_origin()
                        )));
                    }
                }
                ProviderSlot::Diagnosed => {
                    if *retained_lane != RetainedOutcomeLane::Diagnosed {
                        return Err(CompilerError::compiler_error(format!(
                            "ModuleId {index} holds the diagnosed slot but has no diagnosed record"
                        )));
                    }
                }
                ProviderSlot::Blocked => {
                    if *retained_lane != RetainedOutcomeLane::Blocked {
                        return Err(CompilerError::compiler_error(format!(
                            "ModuleId {index} holds the blocked slot but has no blocked record"
                        )));
                    }
                }
            }
        }

        if let Some((row_index, None)) = row_owners
            .iter()
            .enumerate()
            .find(|(_, owner)| owner.is_none())
        {
            return Err(CompilerError::compiler_error(format!(
                "Artifact row {row_index} is not referenced by any module slot"
            )));
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
    pub(crate) fn require_all_successful(&self) -> Result<(), CompilerError> {
        if let Some(diagnosed) = self.diagnosed.first() {
            return Err(CompilerError::compiler_error(format!(
                "Project compilation received a boundary with diagnosed ModuleId {}",
                diagnosed.module_id.index()
            )));
        }
        if let Some(blocked) = self.blocked.first() {
            return Err(CompilerError::compiler_error(format!(
                "Project compilation received a boundary with blocked ModuleId {}",
                blocked.module_id.index()
            )));
        }
        self.modules.ensure_all_successful()
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

    /// Move prepared successful-module warnings in module-id then generated-publication order.
    pub(crate) fn take_successful_warnings(&mut self) -> Vec<SuccessfulWarningBatch> {
        let batches = self.modules.take_successful_warnings();
        let uncaptured_sidecar_warnings = self.generated.take_sidecar_warnings();
        assert!(
            uncaptured_sidecar_warnings.is_empty(),
            "generated sidecar warnings must be captured when their module is published",
        );
        batches
            .into_iter()
            .map(|batch| SuccessfulWarningBatch {
                batch,
                source: None,
            })
            .collect()
    }

    /// Install the single boundary-owned identity pair after all canonical publications.
    ///
    /// WHAT: overwrites every base artefact and generated sidecar placeholder with one final
    ///       `Arc<PathTable>` and pairs the retained materialisation contexts with that table
    ///       plus one frozen requester-domain string table.
    /// WHY: per-module publication used to take one full table snapshot per successful module
    ///      and one frozen table clone per context. The boundary tail builds each once and
    ///      shares it across every retained row; the local registry must already be dropped so
    ///      `Arc::make_mut` proves the retained contexts are the sole owners.
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
    pub(crate) fn validate(&self) -> Result<(), CompilerError> {
        self.boundary.validate_invariants()?;

        let root_index = self.root_module_id.index();
        let node = self
            .boundary
            .structure
            .nodes()
            .get(root_index)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Source package @{} root ModuleId {} is out of range for {} graph nodes",
                    self.package_prefix(),
                    root_index,
                    self.boundary.structure.nodes().len()
                ))
            })?;
        if node.stable_origin().package() != &self.package_identity {
            return Err(CompilerError::compiler_error(format!(
                "Source package @{} root node belongs to package {:?}, not {:?}",
                self.package_prefix(),
                node.stable_origin().package(),
                self.package_identity
            )));
        }
        if node.role() != ModuleRootRole::Normal {
            return Err(CompilerError::compiler_error(format!(
                "Source package @{} root module has role {:?}; packages require a normal entry-root module",
                self.package_prefix(),
                node.role()
            )));
        }

        match self.boundary.modules.slot(self.root_module_id)? {
            ProviderSlot::Successful(_) => {
                let interface = self
                    .boundary
                    .modules
                    .interface(self.root_module_id)?
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Source package @{} root ModuleId {} holds a successful slot without a published interface",
                            self.package_prefix(),
                            root_index
                        ))
                    })?;
                if &interface.module_origin != node.stable_origin() {
                    return Err(CompilerError::compiler_error(format!(
                        "Source package @{} root interface origin {:?} disagrees with its graph node origin {:?}",
                        self.package_prefix(),
                        interface.module_origin,
                        node.stable_origin()
                    )));
                }
            }
            ProviderSlot::Diagnosed | ProviderSlot::Blocked => {}
            ProviderSlot::Unavailable => {
                return Err(CompilerError::compiler_error(format!(
                    "Source package @{} root ModuleId {} never reached a completed outcome",
                    self.package_prefix(),
                    root_index
                )));
            }
        }

        Ok(())
    }

    /// The root facade's publication slot inside this package's own store.
    pub(crate) fn root_slot(&self) -> Result<ProviderSlot, CompilerError> {
        self.boundary.modules.slot(self.root_module_id)
    }

    /// The completed facade interface for this package boundary.
    pub(crate) fn root_interface(&self) -> Result<&PublicSemanticInterface, CompilerError> {
        self.boundary
            .modules
            .interface(self.root_module_id)?
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Source package @{} completed without a successful facade interface",
                    self.package_prefix()
                ))
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
/// WHY: every boundary compilation and module readiness check previously rebuilt prefix maps or
///      filtered the full dependency vector per module; one registry keeps package indexing and
///      dependency adjacency in a single build-owned owner.
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
    ) -> Result<SourcePackagePublication, CompilerError> {
        let prefix = package.package_prefix().to_owned();
        if self.by_prefix.contains_key(prefix.as_str()) {
            return Err(CompilerError::compiler_error(format!(
                "source package @{} completed more than once",
                prefix
            )));
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
                    CompilerError::compiler_error(format!(
                        "source package @{} depends on unindexed source package @{}",
                        prefix, dependency_prefix
                    ))
                })?;
            if !seen_providers.insert(provider_id) {
                return Err(CompilerError::compiler_error(format!(
                    "source package @{} lists provider package @{} more than once",
                    prefix, dependency_prefix
                )));
            }
            resolved_providers.push(provider_id);
        }

        // Preflight materialisation rows against retained state before appending the package.
        let mut materialisation_rows = Vec::new();
        let mut seen_declarations = FxHashSet::default();
        for (identity, location) in package.boundary.modules.materialisation_locations() {
            if !seen_declarations.insert(identity.clone()) {
                return Err(CompilerError::compiler_error(format!(
                    "Generated declaration identity {:?} is duplicated inside source package @{}",
                    identity, prefix
                )));
            }
            if let Some(existing) = self.declarations_by_identity.get(identity) {
                return Err(CompilerError::compiler_error(format!(
                    "Generated declaration identity {:?} was published by source packages @{} and @{}",
                    identity,
                    self.package(existing.package_id)?.package_prefix(),
                    prefix
                )));
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
    ) -> Result<(), CompilerError> {
        let slot = self
            .source_databases
            .get_mut(package_id.index())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "completed source package registry has no source database slot for boundary id {}",
                    package_id.index()
                ))
            })?;
        if slot.is_some() {
            return Err(CompilerError::compiler_error(format!(
                "source package @{} source database was attached more than once",
                self.package(package_id)?.package_prefix()
            )));
        }
        *slot = Some(source_database);
        Ok(())
    }

    #[cfg(test)]
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
    /// Move package warning batches with the source owner of their publishing package.
    pub(crate) fn append_successful_warnings(
        &mut self,
        warnings: &mut Vec<SuccessfulWarningBatch>,
    ) {
        for package_index in 0..self.packages.len() {
            let package = &mut self.packages[package_index];
            let source = self
                .source_databases
                .get_mut(package_index)
                .and_then(Option::take)
                .map(|database| (database, Some(package.package_identity.clone())));
            warnings.extend(package.boundary.take_successful_warnings().into_iter().map(
                |mut batch| {
                    batch.source = source.clone();
                    batch
                },
            ));
        }
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
    ) -> Result<PackageBoundaryId, CompilerError> {
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
    ) -> Result<&CompiledSourcePackage, CompilerError> {
        self.packages.get(package_id.0).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "completed source package registry has no package for boundary id {}",
                package_id.0
            ))
        })
    }

    /// Direct provider packages of one completed package, in dependency resolution order.
    pub(crate) fn provider_packages(
        &self,
        package_id: PackageBoundaryId,
    ) -> Result<&[PackageBoundaryId], CompilerError> {
        self.provider_packages
            .get(package_id.0)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "completed source package registry has no provider edges for boundary id {}",
                    package_id.0
                ))
            })
    }

    /// Direct consumers of one completed package, in publication order.
    pub(crate) fn consumer_packages(
        &self,
        package_id: PackageBoundaryId,
    ) -> Result<&[PackageBoundaryId], CompilerError> {
        self.consumer_packages
            .get(package_id.0)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
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
    pub(crate) fn validate_dependency_edges(&self) -> Result<(), CompilerError> {
        for package_id in 0..self.packages.len() {
            let package_id = PackageBoundaryId(package_id);
            for provider_id in self.provider_packages(package_id)? {
                if provider_id.index() >= package_id.index() {
                    return Err(CompilerError::compiler_error(format!(
                        "source package @{} lists provider package {} that did not publish first",
                        self.package(package_id)?.package_prefix(),
                        self.package(*provider_id)?.package_prefix()
                    )));
                }
            }
            for consumer_id in self.consumer_packages(package_id)? {
                if consumer_id.index() <= package_id.index() {
                    return Err(CompilerError::compiler_error(format!(
                        "source package @{} lists consumer package {} that published before it",
                        self.package(package_id)?.package_prefix(),
                        self.package(*consumer_id)?.package_prefix()
                    )));
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
) -> Result<Vec<(CompiledModuleRef, &'a Module)>, CompilerError> {
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
/// Storage retained by the final diagnostic render boundary.
///
/// The source fields count only frozen source owners reachable from the returned diagnostic
/// vessel. Diagnostic fields count records and label slots, while `retained_identity_contexts`
/// counts distinct frozen identity allocations reachable through range rows or donor-only handles.
/// The `path_table_*` fields count distinct path-table owners reachable from that same returned
/// vessel: the frozen identity contexts behind range rows or donor-only handles, explicit
/// `RenderPathContext` tables, and any transitional source-context table still present after
/// freezing. Successful-module executable views are deliberately not counted; a table only
/// reachable from executable rows without a diagnostic owner is not report-retained storage.
/// The `source_tokens_*`, construction, donor-snapshot and generic-owner fields are command-scoped
/// ledger observations and can remain nonzero for clean results even though report-retention
/// fields stay zero.
#[cfg(feature = "data_layout_memory_probe")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrozenRenderRetentionMetrics {
    pub(crate) source_snapshot_bytes: usize,
    pub(crate) extended_span_rows: usize,
    pub(crate) source_identity_slots: usize,
    pub(crate) diagnostic_records: usize,
    pub(crate) diagnostic_label_slots: usize,
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
    pub(crate) cutover_adapter_bytes: usize,
    pub(crate) donor_identity_string_tables: usize,
    pub(crate) donor_identity_string_storage_bytes: usize,
    pub(crate) donor_identity_path_tables: usize,
    pub(crate) donor_identity_path_storage_bytes: usize,
    pub(crate) generic_source_tokens_owners: usize,
    pub(crate) generic_source_tokens_bytes: usize,
    pub(crate) generic_source_tokens_live_bytes: usize,
    pub(crate) generic_source_tokens_peak_bytes: usize,
    pub(crate) requester_remap_count: usize,
    pub(crate) requester_remap_used_bytes: usize,
    pub(crate) requester_remap_capacity_bytes: usize,
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
            cutover_adapter_bytes: snapshot.cutover_adapter_bytes,
            donor_identity_string_tables: snapshot.donor_identity_string_tables,
            donor_identity_string_storage_bytes: snapshot.donor_identity_string_storage_bytes,
            donor_identity_path_tables: snapshot.donor_identity_path_tables,
            donor_identity_path_storage_bytes: snapshot.donor_identity_path_storage_bytes,
            generic_source_tokens_owners: snapshot.generic_source_tokens_owners,
            generic_source_tokens_bytes: snapshot.generic_source_tokens_bytes,
            generic_source_tokens_live_bytes: snapshot.generic_source_tokens_live_bytes,
            generic_source_tokens_peak_bytes: snapshot.generic_source_tokens_peak_bytes,
            requester_remap_count: snapshot.requester_remap_count,
            requester_remap_used_bytes: snapshot.requester_remap_used_bytes,
            requester_remap_capacity_bytes: snapshot.requester_remap_capacity_bytes,
            observation_incomplete: snapshot.observation_incomplete,
            ..Self::default()
        }
    }

    fn add_source(&mut self, metrics: SourceDatabaseRetentionMetrics) {
        self.source_snapshot_bytes += metrics.source_snapshot_bytes;
        self.extended_span_rows += metrics.extended_span_rows;
        self.source_identity_slots += metrics.source_identity_slots;
    }

    /// Count only the identity and source owners the converted boundary failure still reaches.
    ///
    /// WHAT: walks each report's frozen identity context, which owns its frozen source database,
    ///       then the infrastructure causes' identity contexts and live source databases. Each
    ///       owner is counted once, by pointer.
    /// WHY: the returned failure is the retention owner after the build boundary drops, so the
    ///      probe must describe only the owners its renderers can still dereference.
    fn add_report_source_owners(&mut self, failure: &BoundaryFailure) {
        let mut identities = FxHashSet::<*const FrozenIdentityContext>::default();
        let mut add_identity = |identity: &FrozenIdentityContext| {
            if identities.insert(identity as *const FrozenIdentityContext) {
                self.add_source(identity.retention_metrics());
            }
        };
        for identity in failure.reports.contexts() {
            add_identity(identity);
        }
        let causes = failure.infrastructure_errors();
        if let Some(errors) = causes {
            for identity in errors.frozen_identities() {
                add_identity(identity);
            }
        }
        self.retained_identity_contexts = identities.len();
        let mut live_sources = FxHashSet::<*const SourceDatabase>::default();
        if let Some(errors) = causes {
            for source in errors.source_databases() {
                if live_sources.insert(Arc::as_ptr(source)) {
                    self.add_source(source.retention_metrics());
                }
            }
        }
    }

    /// Count distinct path-table owners reachable from the converted boundary failure.
    ///
    /// WHAT: walks each report's frozen identity path table, then the infrastructure causes'
    ///       identity path tables and live source path tables. Each table is counted once, by
    ///       pointer.
    /// WHY: the returned failure is the retention owner after the build boundary drops; counting
    ///      executable-row tables instead would describe storage the returned failure cannot
    ///      reach, including all-diagnosed results with no successful module at all.
    fn add_report_path_tables(&mut self, failure: &BoundaryFailure) {
        let mut seen = FxHashSet::<usize>::default();
        let mut add = |path_table: &PathTable| {
            let address = path_table as *const PathTable as usize;
            if seen.insert(address) {
                self.path_table_count += 1;
                self.path_table_node_rows += path_table.len();
                self.path_table_storage_bytes += path_table.storage_bytes();
            }
        };
        for context in failure.reports.contexts() {
            add(context.paths());
        }
        if let Some(errors) = failure.infrastructure_errors() {
            for identity in errors.frozen_identities() {
                add(identity.paths());
            }
            for source in errors.source_databases() {
                add(source.paths());
            }
        }
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
/// [`CompilerError`] aborts compilation.
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
    ) -> Result<Self, CompilerError> {
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
                return Err(CompilerError::compiler_error(format!(
                    "Generated declaration identity {:?} is published by both project module {} and source package @{}",
                    identity,
                    location.artifact_id.index(),
                    source_packages.package(*package_id)?.package_prefix()
                )));
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

    /// Freeze this outcome directly into its canonical diagnostic boundary.
    ///
    /// Parts are appended in the historical stream order. Source rows are paired as each owner
    /// becomes known, and the accumulator makes the run-wide install decision at the sole freeze.
    pub(crate) fn into_boundary_failure(
        self,
        string_table: &mut StringTable,
        project_source: Option<Arc<SourceDatabase>>,
        trailing_facade_failure: Option<ProjectAssemblyError>,
    ) -> BoundaryFailure {
        self.into_boundary_failure_with_retention(
            string_table,
            project_source,
            trailing_facade_failure,
        )
        .0
    }

    /// Freeze this outcome once and count the report owners retained by the probe.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn into_boundary_failure_and_metrics(
        self,
        string_table: &mut StringTable,
        project_source: Option<Arc<SourceDatabase>>,
        trailing_facade_failure: Option<ProjectAssemblyError>,
    ) -> (BoundaryFailure, FrozenRenderRetentionMetrics) {
        let (failure, mut retention) = self.into_boundary_failure_with_retention(
            string_table,
            project_source,
            trailing_facade_failure,
        );
        retention.add_report_source_owners(&failure);
        retention.add_report_path_tables(&failure);
        (failure, retention)
    }

    fn into_boundary_failure_with_retention(
        self,
        string_table: &mut StringTable,
        project_source: Option<Arc<SourceDatabase>>,
        trailing_facade_failure: Option<ProjectAssemblyError>,
    ) -> (BoundaryFailure, FrozenRenderRetentionMetrics) {
        let Self {
            mut project,
            source_packages,
            transient_batches,
            project_source_database,
            ..
        } = self;
        let project_warnings = project.take_successful_warnings();
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
        let mut accumulator = BoundaryAccumulator::new();
        let mut diagnostic_count = 0;
        let mut can_freeze = true;
        let mut saw_infrastructure = false;
        #[cfg(feature = "data_layout_memory_probe")]
        let (mut diagnostic_records, mut diagnostic_label_slots) = (0, 0);

        // Keep project warnings before package warnings. Each batch already owns its captured
        // identity snapshot and is appended directly to the root table.
        let root_batch = PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::new(),
            std::mem::take(string_table),
            None,
            false,
            None,
        );
        accumulator.append_prepared(root_batch, None);
        for warning in project_warnings {
            let source = warning
                .source
                .as_ref()
                .map(|(source, domain)| (source, domain.clone()))
                .or_else(|| project_source.as_ref().map(|source| (source, None)));
            let metrics = append_prepared_frontend_diagnostics(
                &mut accumulator,
                warning.batch,
                source,
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
                .and_then(Option::as_ref);
            let domain = Some(package.package_identity.clone());
            for warning in package.boundary.take_successful_warnings() {
                let metrics = append_prepared_frontend_diagnostics(
                    &mut accumulator,
                    warning.batch,
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
                    panic!(
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
            let (source, domain) = match transient.package_id {
                Some(package_id) => {
                    let package_index = package_id.index();
                    let package = source_packages.get(package_index).unwrap_or_else(|| {
                        panic!(
                            "diagnostic range references missing source package identity index {package_index}"
                        )
                    });
                    let source = package_source_arcs
                        .get(package_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "diagnostic range references missing source package identity index {package_index}"
                            )
                        })
                        .as_ref();
                    (source, Some(package.package_identity.clone()))
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
        if let Some(facade_failure) = trailing_facade_failure {
            let mut messages = facade_failure.into_owned_messages();
            if let (Some(errors), Some(source)) = (
                messages.infrastructure_errors.as_mut(),
                project_source.as_ref(),
            ) {
                errors.set_source_database(Arc::clone(source));
            }
            let end = messages.diagnostics.len();
            install_frontend_source_context(
                &mut messages,
                0..end,
                diagnostic_count,
                project_source.as_ref(),
                None,
                &mut can_freeze,
            );
            #[cfg(feature = "data_layout_memory_probe")]
            record_frontend_diagnostic_metrics(
                &messages,
                &mut diagnostic_records,
                &mut diagnostic_label_slots,
            );
            diagnostic_count += end;
            saw_infrastructure |= messages.infrastructure_errors.is_some();
            accumulator.append(messages);
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
            return (
                BoundaryAccumulator::new().finish(InstallMode::Frozen),
                retention,
            );
        }

        // Preserve donor rows after every diagnostic range, including identities referenced only
        // by a foreign frozen handle. Empty rows do not change diagnostic pairing.
        let mut donor_messages = CompilerMessages::empty(StringTable::new());
        if let Some(source) = project_source.as_ref() {
            donor_messages.install_source_contexts(
                [RenderSourceContext {
                    diagnostic_range: 0..0,
                    source_database: Arc::clone(source),
                    domain: project_domain,
                }],
                0,
            );
        }
        for (package, source) in source_packages.iter().zip(&package_source_arcs) {
            if let Some(source) = source {
                donor_messages.install_source_contexts(
                    [RenderSourceContext {
                        diagnostic_range: 0..0,
                        source_database: Arc::clone(source),
                        domain: Some(package.package_identity.clone()),
                    }],
                    0,
                );
            }
        }
        accumulator.append(donor_messages);

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
        (accumulator.finish(install), retention)
    }
}
