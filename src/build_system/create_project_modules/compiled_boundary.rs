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
    GeneratedDeclarationIdentity, StablePackageIdentity,
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

    /// Validate retained boundary invariants before any consumer trusts the dense lanes.
    ///
    /// WHAT: checks graph node count against store slot count and every diagnosed and blocked
    ///       module against its own store slot.
    /// WHY: dense lookups are only safe after this validation; an invalid mapping must surface
    ///       as `CompilerError` instead of becoming an absent artefact later. The success-only
    ///       `ProjectCompilation` gate separately rejects unresolved slots.
    pub(crate) fn validate_invariants(&self) -> Result<(), CompilerError> {
        if self.structure.nodes().len() != self.modules.slot_count() {
            return Err(CompilerError::compiler_error(format!(
                "Graph retained {} nodes but module store has {} slots",
                self.structure.nodes().len(),
                self.modules.slot_count()
            )));
        }

        for diagnosed in &self.diagnosed {
            if self.modules.slot(diagnosed.module_id)? != ProviderSlot::Diagnosed {
                return Err(CompilerError::compiler_error(format!(
                    "Diagnosed module {} does not hold the diagnosed store slot",
                    diagnosed.module_id.index()
                )));
            }
        }
        for blocked in &self.blocked {
            if self.modules.slot(blocked.module_id)? != ProviderSlot::Blocked {
                return Err(CompilerError::compiler_error(format!(
                    "Blocked module {} does not hold the blocked store slot",
                    blocked.module_id.index()
                )));
            }
        }

        Ok(())
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
}

impl CompletedSourcePackageRegistry {
    pub(crate) fn new() -> Self {
        Self {
            packages: Vec::new(),
            by_prefix: FxHashMap::default(),
            provider_packages: Vec::new(),
            consumer_packages: Vec::new(),
            declarations_by_identity: FxHashMap::default(),
        }
    }

    /// Publish one completed package boundary with its direct provider packages.
    ///
    /// WHAT: resolves every dependency prefix once, rejects unknown or duplicate prefixes, and
    ///       records both dependency directions beside the package row.
    /// WHY: packages compile in dependency order, so every provider prefix must already have a
    ///      registry entry; a missing entry is a proven scheduling invariant failure.
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

        let package_id = PackageBoundaryId(self.packages.len());
        self.register_package_materialisations(&package, package_id)?;
        self.packages.push(package);
        self.provider_packages.push(Vec::new());
        self.consumer_packages.push(Vec::new());
        self.by_prefix.insert(prefix.clone(), package_id);

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
                continue;
            }
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
        self.declarations_by_identity
            .iter()
            .map(|(identity, location)| (identity, *location))
    }

    fn register_package_materialisations(
        &mut self,
        package: &CompiledSourcePackage,
        package_id: PackageBoundaryId,
    ) -> Result<(), CompilerError> {
        for (identity, location) in package.boundary.modules.materialisation_locations() {
            if let Some(existing) = self.declarations_by_identity.get(identity) {
                return Err(CompilerError::compiler_error(format!(
                    "Generated declaration identity {:?} was published by source packages @{} and @{}",
                    identity,
                    self.package(existing.package_id)?.import_prefix(),
                    package.import_prefix()
                )));
            }
            self.declarations_by_identity.insert(
                identity.clone(),
                PackageMaterialisationLocation {
                    package_id,
                    location,
                },
            );
        }
        Ok(())
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

        for (identity, project_location) in project.modules.materialisation_locations() {
            if let Some(package_location) = source_packages.materialisation_location_for(identity) {
                return Err(CompilerError::compiler_error(format!(
                    "Generated declaration identity {:?} is published by both project module {} and source package @{}",
                    identity,
                    project_location.artifact_id.index(),
                    source_packages
                        .package(package_location.package_id)?
                        .import_prefix()
                )));
            }
        }
        for (identity, package_location) in source_packages.materialisation_locations() {
            if project
                .modules
                .materialisation_context_for(identity)?
                .is_some()
            {
                return Err(CompilerError::compiler_error(format!(
                    "Generated declaration identity {:?} is published by both source package @{} and project module",
                    identity,
                    source_packages
                        .package(package_location.package_id)?
                        .import_prefix()
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
