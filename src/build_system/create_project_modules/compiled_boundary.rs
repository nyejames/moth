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

use crate::build_system::build::{CompiledModuleArtifact, Module};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::module_diagnostics::ModuleDiagnostics;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModuleRootRole, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::{FxHashMap, FxHashSet};

use super::generated_worklist::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::MaterialisationContextLocation;
use super::module_artifact_store::{ModuleArtifactStore, ProviderSlot};
use super::module_identity::ModuleId;
use super::project_module_graph::ProjectModuleGraph;

/// One diagnosed module retained at the graph boundary.
///
/// The dense build-local `ModuleId` is retained only for deterministic ordering and outcome
/// queries inside the owning boundary; it never enters a [`CompiledModuleArtifact`].
#[derive(Debug)]
pub(crate) struct DiagnosedModule {
    pub(crate) module_id: ModuleId,
    pub(crate) diagnostics: ModuleDiagnostics,
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

    /// Iterate every successful module view including generated sidecars.
    ///
    /// Base artefacts come first in `ModuleId` order, then generated sidecars in deterministic
    /// publication order. Warnings produced during generic materialisation live in sidecar
    /// metadata, so boundary consumers must visit both lanes.
    pub(crate) fn successful_module_views(&self) -> impl Iterator<Item = &Module> + '_ {
        self.successful_artefacts_in_module_id_order()
            .map(|artifact| &artifact.module)
            .chain(self.generated.sidecars().map(|sidecar| &sidecar.module))
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
    /// The `@`-stripped import spelling for this package boundary.
    pub(crate) fn import_prefix(&self) -> &str {
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
                    self.import_prefix(),
                    root_index,
                    self.boundary.structure.nodes().len()
                ))
            })?;
        if node.stable_origin().package() != &self.package_identity {
            return Err(CompilerError::compiler_error(format!(
                "Source package @{} root node belongs to package {:?}, not {:?}",
                self.import_prefix(),
                node.stable_origin().package(),
                self.package_identity
            )));
        }
        if node.role() != ModuleRootRole::Normal {
            return Err(CompilerError::compiler_error(format!(
                "Source package @{} root module has role {:?}; packages require a normal entry-root module",
                self.import_prefix(),
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
                            self.import_prefix(),
                            root_index
                        ))
                    })?;
                if &interface.module_origin != node.stable_origin() {
                    return Err(CompilerError::compiler_error(format!(
                        "Source package @{} root interface origin {:?} disagrees with its graph node origin {:?}",
                        self.import_prefix(),
                        interface.module_origin,
                        node.stable_origin()
                    )));
                }
            }
            ProviderSlot::Diagnosed | ProviderSlot::Blocked => {}
            ProviderSlot::Unavailable => {
                return Err(CompilerError::compiler_error(format!(
                    "Source package @{} root ModuleId {} never reached a completed outcome",
                    self.import_prefix(),
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
                    self.import_prefix()
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
}

/// Exact cross-package location of one published materialisation template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PackageMaterialisationLocation {
    pub(crate) package_id: PackageBoundaryId,
    pub(crate) location: MaterialisationContextLocation,
}

/// One incrementally maintained registry of completed source-package boundaries.
///
/// WHAT: stores completed packages contiguously, resolves import prefixes to dense
///       [`PackageBoundaryId`] values, and records the package dependency graph (provider and
///       consumer edges) exactly once when each package publishes.
/// WHY: every boundary compilation and module readiness check previously rebuilt prefix maps or
///      filtered the full import vector per module; one registry keeps package indexing and
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
    /// Every published package materialisation row in deterministic publication order.
    materialisation_rows: Vec<(GeneratedDeclarationIdentity, PackageMaterialisationLocation)>,
}

impl CompletedSourcePackageRegistry {
    pub(crate) fn new() -> Self {
        Self {
            packages: Vec::new(),
            by_prefix: FxHashMap::default(),
            provider_packages: Vec::new(),
            consumer_packages: Vec::new(),
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
    pub(crate) fn publish(
        &mut self,
        package: CompiledSourcePackage,
        dependency_prefixes: &[String],
    ) -> Result<PackageBoundaryId, CompilerError> {
        let prefix = package.import_prefix().to_owned();
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
            if seen_providers.insert(provider_id) {
                resolved_providers.push(provider_id);
            }
        }

        // Preflight materialisation rows against retained state before appending the package.
        let mut materialisation_rows = Vec::new();
        for (identity, location) in package.boundary.modules.materialisation_locations() {
            if let Some(existing) = self.declarations_by_identity.get(identity) {
                return Err(CompilerError::compiler_error(format!(
                    "Generated declaration identity {:?} was published by source packages @{} and @{}",
                    identity,
                    self.package(existing.package_id)?.import_prefix(),
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
        self.packages.push(package);
        self.provider_packages.push(Vec::new());
        self.consumer_packages.push(Vec::new());
        self.by_prefix.insert(prefix.clone(), package_id);
        for (identity, location) in &materialisation_rows {
            self.declarations_by_identity
                .insert(identity.clone(), *location);
            self.materialisation_rows
                .push((identity.clone(), *location));
        }
        for provider_id in resolved_providers {
            self.provider_packages[package_id.0].push(provider_id);
            self.consumer_packages[provider_id.0].push(package_id);
        }

        Ok(package_id)
    }

    pub(crate) fn by_prefix(&self, import_prefix: &str) -> Option<PackageBoundaryId> {
        self.by_prefix.get(import_prefix).copied()
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

    /// Resolve one published materialisation template across every completed package boundary.
    pub(crate) fn materialisation_location_for(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Option<PackageMaterialisationLocation> {
        self.declarations_by_identity.get(identity).copied()
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
    ///       check keeps the recorded graph usable without re-deriving it from import vectors.
    pub(crate) fn validate_dependency_edges(&self) -> Result<(), CompilerError> {
        for package_id in 0..self.packages.len() {
            let package_id = PackageBoundaryId(package_id);
            for provider_id in self.provider_packages(package_id)? {
                if provider_id.index() >= package_id.index() {
                    return Err(CompilerError::compiler_error(format!(
                        "source package @{} lists provider package {} that did not publish first",
                        self.package(package_id)?.import_prefix(),
                        self.package(*provider_id)?.import_prefix()
                    )));
                }
            }
            for consumer_id in self.consumer_packages(package_id)? {
                if consumer_id.index() <= package_id.index() {
                    return Err(CompilerError::compiler_error(format!(
                        "source package @{} lists consumer package {} that published before it",
                        self.package(package_id)?.import_prefix(),
                        self.package(*consumer_id)?.import_prefix()
                    )));
                }
            }
        }

        Ok(())
    }

    /// Consume the registry, returning the completed packages in publication order.
    pub(crate) fn into_packages(self) -> Vec<CompiledSourcePackage> {
        self.packages
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
}

impl ProjectFrontendCompilation {
    pub(crate) fn new(
        project: CompiledGraphBoundary,
        source_packages: CompletedSourcePackageRegistry,
    ) -> Result<Self, CompilerError> {
        project.validate_invariants()?;
        source_packages.validate_dependency_edges()?;
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
                    source_packages.package(*package_id)?.import_prefix()
                )));
            }
        }

        Ok(Self {
            project,
            source_packages,
        })
    }

    /// Whether any boundary contains a diagnosed or blocked module.
    ///
    /// `build` and `dev` use this gate before assembling a success-only [`ProjectCompilation`]
    /// while `check` renders the retained outcomes instead.
    pub(crate) fn has_diagnosed_or_blocked(&self) -> bool {
        !self.project.diagnosed.is_empty()
            || !self.project.blocked.is_empty()
            || self.source_packages.iter().any(|package| {
                !package.boundary.diagnosed.is_empty() || !package.boundary.blocked.is_empty()
            })
    }

    /// Iterate every successful module view (base artefacts and generated sidecars) across all
    /// boundaries in deterministic order.
    pub(crate) fn successful_module_views(&self) -> impl Iterator<Item = &Module> + '_ {
        self.project.successful_module_views().chain(
            self.source_packages
                .iter()
                .flat_map(|package| package.boundary.successful_module_views()),
        )
    }

    /// Build the render-boundary message set for this outcome.
    ///
    /// Warnings from every successful boundary are retained first, then diagnosed module
    /// messages in deterministic `ModuleId` order. Blocked modules produce no cascade
    /// diagnostics and are not rendered.
    pub(crate) fn into_render_messages(self, string_table: &mut StringTable) -> CompilerMessages {
        let warnings = self
            .successful_module_views()
            .flat_map(|module| module.metadata.warnings.iter().cloned())
            .collect::<Vec<_>>();
        let mut messages = CompilerMessages::from_diagnostics(warnings, string_table.clone());

        for diagnosed in self.project.diagnosed {
            messages.append_messages_preserving_context(diagnosed.diagnostics.into_messages());
        }
        let source_packages = self.source_packages.into_packages();
        for package in source_packages {
            for diagnosed in package.boundary.diagnosed {
                messages.append_messages_preserving_context(diagnosed.diagnostics.into_messages());
            }
        }

        messages
    }
}
