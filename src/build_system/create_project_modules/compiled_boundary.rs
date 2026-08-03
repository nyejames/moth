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
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use super::generated_worklist::BoundaryGeneratedFunctionStore;
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
            .chain(
                self.generated
                    .sidecars()
                    .iter()
                    .map(|sidecar| &sidecar.module),
            )
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
        package_index: usize,
        module_id: ModuleId,
    },
    GeneratedProject(usize),
    GeneratedSourcePackage {
        package_index: usize,
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
    source_packages: &'a [CompiledSourcePackage],
) -> Vec<(CompiledModuleRef, &'a Module)> {
    let mut views = Vec::new();

    for module_id in project
        .structure
        .nodes()
        .iter()
        .map(|node| node.module_id())
    {
        if let Some(artifact) = project.modules.artifact(module_id).ok().flatten() {
            views.push((CompiledModuleRef::Project(module_id), &artifact.module));
        }
    }

    for (package_index, package) in source_packages.iter().enumerate() {
        for module_id in package
            .boundary
            .structure
            .nodes()
            .iter()
            .map(|node| node.module_id())
        {
            if let Some(artifact) = package.boundary.modules.artifact(module_id).ok().flatten() {
                views.push((
                    CompiledModuleRef::SourcePackage {
                        package_index,
                        module_id,
                    },
                    &artifact.module,
                ));
            }
        }
    }

    for (sidecar_index, sidecar) in project.generated.sidecars().iter().enumerate() {
        views.push((
            CompiledModuleRef::GeneratedProject(sidecar_index),
            &sidecar.module,
        ));
    }

    for (package_index, package) in source_packages.iter().enumerate() {
        for (sidecar_index, sidecar) in package.boundary.generated.sidecars().iter().enumerate() {
            views.push((
                CompiledModuleRef::GeneratedSourcePackage {
                    package_index,
                    sidecar_index,
                },
                &sidecar.module,
            ));
        }
    }

    views
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
    pub(crate) source_packages: Vec<CompiledSourcePackage>,
}

impl ProjectFrontendCompilation {
    pub(crate) fn new(
        project: CompiledGraphBoundary,
        source_packages: Vec<CompiledSourcePackage>,
    ) -> Self {
        Self {
            project,
            source_packages,
        }
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
        for package in self.source_packages {
            for diagnosed in package.boundary.diagnosed {
                messages.append_messages_preserving_context(diagnosed.diagnostics.into_messages());
            }
        }

        messages
    }
}
