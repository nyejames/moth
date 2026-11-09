//! Canonical structural project module graph.
//!
//! WHAT: owns the deterministic project-module graph built directly from the Stage 0
//! [`SourceTreeIndex`]. Each canonical module becomes one node carrying its `ModuleId`,
//! stable origin, root role, root files, nearest structural parent and structural dependency
//! edges. The graph classifies `@entry.moth` entry candidates and the project package support
//! surface, encodes strict scoped-support visibility as a query, and exposes deterministic dependency-edge
//! insertion, an explicit construction-to-completion phase transition that freezes
//! provider/consumer adjacency into sorted `Vec<ModuleId>` storage, and topological compile waves
//! over those frozen provider-before-consumer edges. Source records and ownership remain in
//! `SourceTreeIndex`.
//! WHY: the compiler cannot schedule canonical modules until Stage 0 can distinguish normal
//! modules, support packages and their shared helper origins, and until dependency order can be
//! derived without a second filesystem traversal or a parallel identity/topology table. This
//! owner consumes the existing [`ModuleIdentityTable`] rather than recomputing it and resolves
//! source data through the retained index, so identity, ancestry and source ownership stay
//! single-owned.
//!
//! Reachable-file discovery resolves cross-module dependencies through the indexed namespace and
//! inserts provider-before-consumer edges directly by `ModuleId`, so dependency order is derived
//! from the canonical graph without another filesystem traversal or identity table.
//!
//! Production wiring: Stage 0 constructs the graph once from the [`SourceTreeIndex`] in
//! `project_roots` and retains it as the structural owner. `compile_waves` and `entry_modules`
//! drive deterministic entry selection in `module_inventory` and `build.rs`, so graph construction, wave
//! scheduling and dependency-edge insertion are genuine production paths. The
//! namespace builder consumes the scoped-support-visibility surface for project and package
//! boundaries.

#[cfg(test)]
use super::module_identity::ModuleIdentityRecord;
use super::module_identity::{ModuleId, ModuleIdentityTable};
use super::source_tree_index::SourceTreeIndex;

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{CompilerStage, DiagnosticBag, PreparedFailure};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootFiles, ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::FxHashMap;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Outcome of inserting one deterministic provider-before-consumer dependency edge.
///
/// WHAT: tells the caller whether a new edge was added or whether the edge was already present.
/// WHY: a duplicate edge does not change the dependency graph, so insertion is idempotent rather
///      than an error. Self-edges and out-of-range module IDs remain internal graph failures
///      reported through [`InfrastructureFailure`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DependencyEdgeOutcome {
    Inserted,
    AlreadyPresent,
}

/// The structural facts needed by the scoped-support visibility rule.
///
/// The project graph and package-boundary identity tables use one rule owner while retaining
/// their separate boundary-local identities and storage.
trait SupportVisibilityTopology {
    fn nearest_parent(&self, module_id: ModuleId) -> Option<ModuleId>;

    /// Whether this module occupies the normal consumer/owner position.
    ///
    /// WHAT: true when the module owns a normal or entry root, regardless of whether it also
    ///       owns a support root.
    /// WHY: an entry-only module has no normal public surface, but `@entry` still occupies the
    ///      normal owner/consumer position for scoped support visibility. It must never be
    ///      mistaken for a support-scoped consumer, and no normal interface is manufactured.
    fn occupies_normal_position(&self, module_id: ModuleId) -> bool;

    fn has_support_surface(&self, module_id: ModuleId) -> bool;
}

/// One canonical module node in the project module graph.
///
/// Nodes are stored in deterministic `ModuleId` order so the graph stays aligned with the
/// Stage 0 identity table. Each field is consumed from the existing owners (identity table and
/// source index) at construction time, not recomputed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectModuleGraphNode {
    module_id: ModuleId,
    shared_origin: StableModuleOriginIdentity,
    surface_origins: ModuleRootFiles<StableModuleOriginIdentity>,
    root_files: ModuleRootFiles<PathBuf>,
    nearest_parent: Option<ModuleId>,
}

impl ProjectModuleGraphNode {
    /// The dense build-local handle for this module.
    pub(crate) fn module_id(&self) -> ModuleId {
        self.module_id
    }

    // Structural node accessors. Each mirrors an identity fact the graph consumes for compile-wave
    // scheduling, entry selection or surface queries.
    /// The role-neutral origin shared by every helper file in this structural module.
    pub(crate) fn shared_origin(&self) -> &StableModuleOriginIdentity {
        &self.shared_origin
    }

    /// The role-qualified origin for one present root surface.
    pub(crate) fn surface_origin(
        &self,
        role: ModuleRootRole,
    ) -> Option<&StableModuleOriginIdentity> {
        self.surface_origins.get(role)
    }

    /// Whether this structural module owns any root with the given surface role.
    pub(crate) fn has_surface(&self, role: ModuleRootRole) -> bool {
        self.surface_origin(role).is_some()
    }

    /// Whether a source origin belongs to this structural module.
    pub(crate) fn owns_origin(&self, origin: &StableModuleOriginIdentity) -> bool {
        &self.shared_origin == origin
            || self
                .surface_origins
                .present()
                .any(|(_, surface_origin)| surface_origin == origin)
    }

    /// The role of the package-facing public surface, absent for an entry-only module.
    pub(crate) fn public_surface_role(&self) -> Option<ModuleRootRole> {
        self.root_files.public_surface_role()
    }

    /// The stable origin of the package-facing public surface.
    pub(crate) fn public_surface_origin(&self) -> Option<&StableModuleOriginIdentity> {
        self.public_surface_role()
            .and_then(|role| self.surface_origin(role))
    }

    /// The cross-build origin of the preparation-anchor root.
    pub(crate) fn stable_origin(&self) -> &StableModuleOriginIdentity {
        self.surface_origins
            .preparation_anchor()
            .expect("a structural graph node has at least one root origin")
    }

    /// The exact optional normal and support origins one successful artefact must publish.
    ///
    /// Each entry mirrors the role-keyed root inventory, so a published surface must exist
    /// exactly when the node owns that facade root. An entry-only module returns two absences and
    /// publishes no interface.
    pub(crate) fn publication_surfaces(
        &self,
    ) -> (
        Option<&StableModuleOriginIdentity>,
        Option<&StableModuleOriginIdentity>,
    ) {
        (
            self.surface_origin(ModuleRootRole::Normal),
            self.surface_origin(ModuleRootRole::Support),
        )
    }

    /// The structural role of the preparation-anchor root.
    #[cfg(test)]
    pub(crate) fn role(&self) -> ModuleRootRole {
        self.root_files
            .anchor_role()
            .expect("a structural graph node has at least one root")
    }

    /// The module's fixed role-keyed root files.
    pub(crate) fn root_files(&self) -> &ModuleRootFiles<PathBuf> {
        &self.root_files
    }

    /// The canonical normal root file, when this module has one.
    pub(crate) fn normal_root_file(&self) -> Option<&Path> {
        self.root_files.normal.as_deref()
    }

    /// The canonical support root file, when this module has one.
    pub(crate) fn support_root_file(&self) -> Option<&Path> {
        self.root_files.support.as_deref()
    }

    /// The canonical preparation-anchor root file for focused graph tests.
    #[cfg(test)]
    pub(crate) fn root_file(&self) -> &Path {
        self.root_files
            .preparation_anchor()
            .map(PathBuf::as_path)
            .expect("a structural graph node has at least one root file")
    }
}

/// Provider and consumer adjacency held over the graph lifecycle.
///
/// WHAT: holds one live representation of provider/consumer adjacency at a time. During
///       construction, sorted `BTreeSet<ModuleId>` storage deduplicates incoming edges. After
///       completion, adjacency is converted to sorted `Vec<ModuleId>` storage and the
///       construction sets are dropped, so the graph keeps one complete adjacency representation.
/// WHY: the construction sets exist only for idempotent insertion. The frozen vectors are the
///      single adjacency consumed for indegree counting and compile-wave scheduling. Keeping both
///      directions in one state prevents phase or storage disagreement by construction.
enum ProjectModuleDependencies {
    UnderConstruction {
        dependency_providers: Vec<BTreeSet<ModuleId>>,
        provider_consumers: Vec<BTreeSet<ModuleId>>,
    },
    Frozen {
        dependency_providers: Vec<Vec<ModuleId>>,
        provider_consumers: Vec<Vec<ModuleId>>,
    },
}

/// The canonical structural project module graph for one build boundary.
///
/// Built directly from a [`SourceTreeIndex`] without filesystem IO or a second identity/topology
/// table. Nodes are stored in deterministic `ModuleId` order. Only a module with a project-local
/// `@entry.moth` is an entry candidate; API facade roots are never entries and a project-root
/// support surface is an ordinary structural module. Edge insertion and compile-wave scheduling
/// are separated by
/// one explicit [`ProjectModuleDependencies`] transition: edges are inserted while the graph is
/// under construction, then [`ProjectModuleGraph::complete`] freezes adjacency into sorted
/// `Vec<ModuleId>` storage before [`ProjectModuleGraph::compile_waves`] consumes it.
pub(crate) struct ProjectModuleGraph {
    nodes: Vec<ProjectModuleGraphNode>,
    entry_modules: Vec<ModuleId>,
    dependencies: ProjectModuleDependencies,
}

impl ProjectModuleGraph {
    /// Build a synthetic graph from explicit standalone module roots.
    ///
    /// WHAT: creates one frozen node per `(stable origin, root file)` in the given order with
    /// dense `ModuleId` values starting at zero and no dependency edges. Each node's root file and
    /// surface origin occupy the slot named by the origin's role, so an explicitly selected
    /// synthetic entry publishes no facade interface. Only `Entry`-role origins are executable;
    /// a supplied `Normal` or `Support` origin stays an API-only facade.
    /// WHY: single-file compilation is a synthetic-module mode with no `SourceTreeIndex`, and
    /// focused tests need real graph boundaries without filesystem discovery. The graph keeps
    /// the same frozen adjacency contract as discovery-built graphs.
    pub(crate) fn from_standalone_roots(roots: Vec<(StableModuleOriginIdentity, PathBuf)>) -> Self {
        let node_count = roots.len();
        let nodes = roots
            .into_iter()
            .enumerate()
            .map(|(index, (stable_origin, root_file))| {
                let role = stable_origin.role();
                ProjectModuleGraphNode {
                    module_id: ModuleId::from_index(index),
                    surface_origins: ModuleRootFiles::single(role, stable_origin.clone()),
                    shared_origin: stable_origin,
                    root_files: ModuleRootFiles::single(role, root_file),
                    nearest_parent: None,
                }
            })
            .collect::<Vec<_>>();
        let entry_modules = nodes
            .iter()
            .filter(|node| node.shared_origin.role() == ModuleRootRole::Entry)
            .map(|node| node.module_id)
            .collect();

        Self {
            nodes,
            entry_modules,
            dependencies: ProjectModuleDependencies::Frozen {
                dependency_providers: vec![Vec::new(); node_count],
                provider_consumers: vec![Vec::new(); node_count],
            },
        }
    }

    /// Build the graph directly from the Stage 0 source-tree index.
    ///
    /// Consumes the index's identity table rather than recomputing it. Source ownership remains
    /// in the retained index and is not copied into graph nodes.
    /// Each module becomes one node in deterministic `ModuleId` order. Only a project-local
    /// `@entry.moth` is an executable entry; normal and support facades remain ordinary
    /// structural modules.
    pub(crate) fn from_source_tree_index(source_tree_index: &SourceTreeIndex) -> Self {
        let identities = source_tree_index.module_identities();
        let module_ids: Vec<ModuleId> = identities.module_ids().collect();
        let node_count = module_ids.len();

        let mut nodes = Vec::with_capacity(node_count);
        let mut entry_modules = Vec::new();

        for module_id in &module_ids {
            let record = identities.record(*module_id);
            let nearest_parent = identities.nearest_ancestor_module(*module_id);

            nodes.push(ProjectModuleGraphNode {
                module_id: *module_id,
                shared_origin: record.shared_origin().clone(),
                surface_origins: record.surface_origins().clone(),
                root_files: record.root_files().clone(),
                nearest_parent,
            });

            // Only a project-local `@entry.moth` is an executable entry. An API facade root
            // publishes a surface and never activates a start.
            if record.has_entry_root() {
                entry_modules.push(*module_id);
            }
        }

        // `entry_modules` is already in `ModuleId` order because `module_ids` iterates in
        // `ModuleId` order, but sort defensively so the contract does not depend on iteration
        // order.
        entry_modules.sort_by_key(|module_id| module_id.index());

        let dependency_providers = (0..node_count).map(|_| BTreeSet::new()).collect();
        let provider_consumers = (0..node_count).map(|_| BTreeSet::new()).collect();

        Self {
            nodes,
            entry_modules,
            dependencies: ProjectModuleDependencies::UnderConstruction {
                dependency_providers,
                provider_consumers,
            },
        }
    }

    /// Build a graph from explicit identities for assembly tests.
    ///
    /// WHAT: supplies deterministic graph nodes without requiring source discovery to create a
    /// fixture. WHY: package assembly tests need to model a support surface and selected descendants while
    /// keeping their source artefacts synthetic and focused on liveness.
    #[cfg(test)]
    pub(crate) fn from_test_records(records: Vec<ModuleIdentityRecord>) -> Self {
        let identities = ModuleIdentityTable::from_records(records);
        let module_ids: Vec<ModuleId> = identities.module_ids().collect();
        let node_count = module_ids.len();
        let mut nodes = Vec::with_capacity(node_count);
        let mut entry_modules = Vec::new();

        for module_id in &module_ids {
            let record = identities.record(*module_id);
            nodes.push(ProjectModuleGraphNode {
                module_id: *module_id,
                shared_origin: record.shared_origin().clone(),
                surface_origins: record.surface_origins().clone(),
                root_files: record.root_files().clone(),
                nearest_parent: None,
            });

            if record.has_entry_root() {
                entry_modules.push(*module_id);
            }
        }

        Self {
            nodes,
            entry_modules,
            dependencies: ProjectModuleDependencies::Frozen {
                dependency_providers: vec![Vec::new(); node_count],
                provider_consumers: vec![Vec::new(); node_count],
            },
        }
    }

    /// The number of canonical module nodes in the graph.
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// All graph nodes in deterministic `ModuleId` order.
    pub(crate) fn nodes(&self) -> &[ProjectModuleGraphNode] {
        &self.nodes
    }
    /// The stable package identity shared by every module in this boundary.
    pub(crate) fn stable_package_identity(&self) -> Option<&StablePackageIdentity> {
        self.nodes
            .first()
            .map(|node| node.stable_origin().package())
    }

    /// The canonical node for one module identity.
    ///
    /// `module_id` must be a valid identity produced by the Stage 0 identity table.
    pub(crate) fn node(&self, module_id: ModuleId) -> &ProjectModuleGraphNode {
        &self.nodes[module_id.index()]
    }

    /// Entry-candidate modules in deterministic `ModuleId` order.
    ///
    /// A directory graph derives them only from present `@entry.moth` roots; the synthetic
    /// single-file graph contains exactly its explicitly selected root. API facade roots and
    /// separately compiled dependency entries are never candidates.
    pub(crate) fn entry_modules(&self) -> &[ModuleId] {
        &self.entry_modules
    }

    /// Completed source providers required before one module may bind dependencies.
    pub(crate) fn dependency_providers(
        &self,
        module_id: ModuleId,
    ) -> Result<&[ModuleId], InfrastructureFailure> {
        if !self.is_valid_module_id(module_id) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Project module graph received out-of-range ModuleId {} for provider lookup",
                    module_id.index()
                ),
            ));
        }

        let ProjectModuleDependencies::Frozen {
            dependency_providers,
            ..
        } = &self.dependencies
        else {
            return Err(Self::scheduling_before_completion_error());
        };

        Ok(&dependency_providers[module_id.index()])
    }

    /// The project-root support surface that assembles the external package facade.
    ///
    /// This is an ordinary structural module selected by its root location and surface, not a
    /// separate filesystem/graph node. Returns `None` when the selected boundary has no
    /// project-root support root.
    #[cfg(test)]
    pub(crate) fn project_package_surface(&self) -> Option<ModuleId> {
        self.project_package_surface_with_role().map(|(id, _)| id)
    }

    /// The project package module and the surface its external consumers must observe.
    pub(crate) fn project_package_surface_with_role(&self) -> Option<(ModuleId, ModuleRootRole)> {
        self.nodes
            .iter()
            .find(|node| {
                node.nearest_parent.is_none()
                    && node.support_root_file().is_some()
                    && node.stable_origin().logical_module_path().is_empty()
            })
            .and_then(|node| {
                node.public_surface_role()
                    .map(|role| (node.module_id, role))
            })
    }

    /// Scoped-support visibility query for one whole structural module.
    ///
    /// For a support package `S` whose nearest normal-position ancestor is `P`, `S` is visible
    /// to `P`, to normal-position descendants of `P` outside `S`'s private subtree, and to
    /// support facades in a strictly nested normal scope. A normal or entry root occupies the
    /// normal position, so an entry-only owner is a valid `P`. `S` is not visible above `P`,
    /// outside `P`'s subtree, to `S` itself, to `S`'s private descendants, or to another support
    /// module owned by `P`. Returns `false` when `support_id` is not a support module.
    pub(crate) fn is_support_visible_to_consumer(
        &self,
        support_id: ModuleId,
        consumer_id: ModuleId,
    ) -> bool {
        support_is_visible(self, support_id, consumer_id)
    }

    /// Scoped support visibility for one authored consumer surface.
    ///
    /// A paired structural module has both a normal and a support surface but one graph node.
    /// The node's primary role cannot distinguish an import authored in the support root, so
    /// callers pass the declaring source's role explicitly. An entry root is a normal-position
    /// consumer; a support root keeps the strict support-scope rule.
    pub(crate) fn is_support_visible_to_consumer_role(
        &self,
        support_id: ModuleId,
        consumer_id: ModuleId,
        consumer_role: ModuleRootRole,
    ) -> bool {
        support_is_visible_for_role(self, support_id, consumer_id, consumer_role)
    }

    /// Build a lookup from canonical source path to owning stable module origin from every
    /// module's owned source IDs.
    ///
    /// WHAT: the one production path that projects the retained central `SourceTreeIndex`
    ///       ownership authority into a canonical-path-to-StableModuleOriginIdentity map. The
    ///       enclosing project or package boundary uses this map once to build its shared
    ///       `SourceModuleOriginTable`. The graph carries no source records, so source ownership
    ///       stays single-owned by the index.
    /// WHY: the shared SourceModuleOriginTable must resolve each boundary source identity to its
    ///      owning stable module origin without a second filesystem traversal or a parallel
    ///      topology table. The index already carries every owned source record with its portable
    ///      logical identity, so this lookup is a direct projection, not a scan or guess.
    ///
    /// A canonical path owned by two modules, or an owned source whose logical identity module
    /// origin does not match its graph node origin, is a proven invariant violation surfaced
    /// through InfrastructureFailure rather than silently overwriting one origin.
    pub(crate) fn build_source_origin_lookup(
        &self,
        source_tree_index: &SourceTreeIndex,
    ) -> Result<FxHashMap<PathBuf, StableModuleOriginIdentity>, InfrastructureFailure> {
        let mut origins: FxHashMap<PathBuf, StableModuleOriginIdentity> = FxHashMap::default();

        for node in &self.nodes {
            for source_id in source_tree_index.owned_source_indices(node.module_id()) {
                let record = source_tree_index.source(*source_id);
                let Some(entry_origin) = record.logical_identity().module_origin() else {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "Project module graph owned source ID {} resolves to an unrooted record; \
                         an owned source record must carry an owned logical identity",
                            source_id.index(),
                        ),
                    ));
                };
                if !node.owns_origin(entry_origin) {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "Project module graph owned source entry {} has a stable identity module origin ({:?}) that does not belong to its containing structural module (shared {:?}, retained surface origins {:?})",
                            record.canonical_path().display(),
                            entry_origin,
                            node.shared_origin(),
                            [
                                node.surface_origin(ModuleRootRole::Normal),
                                node.surface_origin(ModuleRootRole::Support),
                                node.surface_origin(ModuleRootRole::Entry)
                            ],
                        ),
                    ));
                }
                let canonical_path = record.canonical_path().to_path_buf();
                if origins.contains_key(&canonical_path) {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "Source tree index owned source ID sets assign canonical path {} to multiple modules; each source file must have exactly one owning module",
                            canonical_path.display()
                        ),
                    ));
                }
                origins.insert(canonical_path, entry_origin.clone());
            }
        }

        Ok(origins)
    }

    /// Insert one deterministic provider-before-consumer dependency edge.
    ///
    /// The provider must compile before the consumer. Module IDs are validated and self-edges
    /// are rejected through an internal [`InfrastructureFailure`] without panicking. A duplicate edge
    /// is idempotent and reports [`DependencyEdgeOutcome::AlreadyPresent`] because it does not
    /// change the dependency graph.
    pub(crate) fn add_dependency_edge(
        &mut self,
        provider: ModuleId,
        consumer: ModuleId,
    ) -> Result<DependencyEdgeOutcome, InfrastructureFailure> {
        if !self.is_valid_module_id(provider) || !self.is_valid_module_id(consumer) {
            return Err(self.invalid_module_id_edge_error(provider, consumer));
        }

        if provider == consumer {
            return Err(self.self_edge_error(provider));
        }

        let ProjectModuleDependencies::UnderConstruction {
            dependency_providers,
            provider_consumers,
        } = &mut self.dependencies
        else {
            return Err(Self::mutation_after_completion_error());
        };

        if dependency_providers[consumer.index()].contains(&provider) {
            return Ok(DependencyEdgeOutcome::AlreadyPresent);
        }

        dependency_providers[consumer.index()].insert(provider);
        provider_consumers[provider.index()].insert(consumer);

        Ok(DependencyEdgeOutcome::Inserted)
    }

    /// Freeze provider and consumer adjacency into sorted `Vec<ModuleId>` storage.
    ///
    /// WHAT: the one-time construction-to-completion transition. Construction `BTreeSet` storage
    ///       is converted to sorted `Vec<ModuleId>` storage for both provider and consumer
    ///       adjacency in lockstep, and the dependency state becomes `Frozen`.
    /// WHY: compile-wave scheduling reads only the frozen adjacency so the graph keeps one
    ///      complete adjacency representation. Completing an already-completed graph is a
    ///      mutation after completion and reports an internal [`InfrastructureFailure`] rather than
    ///      silently no-op-ing, so the lifecycle transition stays a single explicit step.
    pub(crate) fn complete(&mut self) -> Result<(), InfrastructureFailure> {
        let (dependency_providers, provider_consumers) = match &mut self.dependencies {
            ProjectModuleDependencies::UnderConstruction {
                dependency_providers,
                provider_consumers,
            } => (
                std::mem::take(dependency_providers),
                std::mem::take(provider_consumers),
            ),
            ProjectModuleDependencies::Frozen { .. } => {
                return Err(Self::mutation_after_completion_error());
            }
        };

        let dependency_providers = dependency_providers
            .into_iter()
            .map(|providers| providers.into_iter().collect())
            .collect();
        let provider_consumers = provider_consumers
            .into_iter()
            .map(|consumers| consumers.into_iter().collect())
            .collect();
        self.dependencies = ProjectModuleDependencies::Frozen {
            dependency_providers,
            provider_consumers,
        };

        Ok(())
    }

    /// Whether a provider-before-consumer dependency edge is currently present.
    ///
    /// Reads adjacency in either lifecycle phase so the query stays valid after edge insertion
    /// (construction) and after the graph is completed for scheduling (frozen).
    #[cfg(test)]
    pub(crate) fn has_dependency_edge(&self, provider: ModuleId, consumer: ModuleId) -> bool {
        if !self.is_valid_module_id(provider) || !self.is_valid_module_id(consumer) {
            return false;
        }

        match &self.dependencies {
            ProjectModuleDependencies::UnderConstruction {
                dependency_providers,
                ..
            } => dependency_providers[consumer.index()].contains(&provider),
            ProjectModuleDependencies::Frozen {
                dependency_providers,
                ..
            } => dependency_providers[consumer.index()].contains(&provider),
        }
    }

    /// Deterministic topological compile waves over provider-before-consumer edges.
    ///
    /// Wave 0 contains every module with no outstanding providers. Each later wave contains
    /// modules whose providers all completed in earlier waves. Within a wave, modules are
    /// ordered by `ModuleId` so independent ready nodes keep one deterministic position. The
    /// project package support surface is ordered by its real edges, never by a hard-coded fake
    /// dependency.
    ///
    /// Dependency cycles are authored import cycles across modules or packages. They block the
    /// affected providers and report one `project_config::circular_dependency` diagnostic that
    /// names a module on the cycle.
    pub(crate) fn compile_waves(&self) -> Result<Vec<Vec<ModuleId>>, PreparedFailure> {
        let ProjectModuleDependencies::Frozen {
            dependency_providers,
            provider_consumers,
        } = &self.dependencies
        else {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Project module graph compile waves were requested before completion; the graph must complete and freeze its adjacency before compile-wave scheduling reads it"
            );
        };
        let node_count = self.node_count();
        let mut remaining_providers: Vec<usize> = dependency_providers
            .iter()
            .map(|providers| providers.len())
            .collect();

        // Wave 0: every module with no providers, in `ModuleId` order. Nodes are already stored
        // in `ModuleId` order, so iterating them preserves the deterministic wave position.
        let mut ready: Vec<ModuleId> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(index, _)| remaining_providers[*index] == 0)
            .map(|(_, node)| node.module_id())
            .collect();

        let mut waves: Vec<Vec<ModuleId>> = Vec::new();
        let mut processed = 0usize;

        while !ready.is_empty() {
            waves.push(ready.clone());

            let mut next_ready: Vec<ModuleId> = Vec::new();
            for provider in &ready {
                for consumer in provider_consumers[provider.index()].iter().copied() {
                    let outstanding = &mut remaining_providers[consumer.index()];
                    *outstanding -= 1;
                    if *outstanding == 0 {
                        next_ready.push(consumer);
                    }
                }
            }

            processed += ready.len();
            next_ready.sort_by_key(|module_id| module_id.index());
            ready = next_ready;
        }

        if processed < node_count {
            return Err(self.cycle_failure(&remaining_providers, dependency_providers));
        }

        Ok(waves)
    }

    /// Whether `module_id` is a valid graph identity.
    fn is_valid_module_id(&self, module_id: ModuleId) -> bool {
        module_id.index() < self.node_count()
    }

    /// Build an internal graph failure for mutating a completed graph.
    fn mutation_after_completion_error() -> InfrastructureFailure {
        InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            "Project module graph was mutated after completion; dependency edges must be inserted \
         before the graph completes and freezes its adjacency for compile-wave scheduling",
        )
    }

    /// Build an internal graph failure for scheduling before the graph completes.
    fn scheduling_before_completion_error() -> InfrastructureFailure {
        InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            "Project module graph compile waves were requested before completion; the graph must \
         complete and freeze its adjacency before compile-wave scheduling reads it",
        )
    }

    /// Build an internal graph failure for an out-of-range module ID supplied to edge insertion.
    fn invalid_module_id_edge_error(
        &self,
        provider: ModuleId,
        consumer: ModuleId,
    ) -> InfrastructureFailure {
        InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "Project module graph received a dependency edge with an out-of-range module ID: \
             provider index {} consumer index {} but the graph has {} modules",
                provider.index(),
                consumer.index(),
                self.node_count()
            ),
        )
    }

    /// Build an internal graph failure for a self-edge supplied to edge insertion.
    fn self_edge_error(&self, module_id: ModuleId) -> InfrastructureFailure {
        let origin = self.describe_module(module_id);
        InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "Project module graph received a self-dependency edge from module {origin}; a module \
             cannot be its own provider"
            ),
        )
    }

    /// Build the circular-dependency failure for modules the topological sweep left blocked.
    ///
    /// WHAT: names one module on the cycle in a `project_config::circular_dependency` diagnostic
    ///       carried in its own string and path tables.
    /// WHY: cycles come from authored cross-module and cross-package imports, so they are user
    ///      diagnostics rather than compiler bugs. The local tables keep the path pairable with
    ///      the diagnostic batch when it is appended to the build boundary.
    fn cycle_failure(
        &self,
        remaining_providers: &[usize],
        dependency_providers: &[Vec<ModuleId>],
    ) -> PreparedFailure {
        let cycle_index = module_on_dependency_cycle(remaining_providers, dependency_providers);
        let logical_path = self.nodes[cycle_index]
            .stable_origin()
            .logical_module_path();
        let mut strings = StringTable::new();
        let mut fork = PathInternerFork::empty();
        let Ok(path) = fork.try_intern_portable_path(logical_path, &mut strings) else {
            let mut bag = DiagnosticBag::new();
            bag.push(token_lexical::logical_path_capacity());
            return PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
                bag,
                StringTable::new(),
                None,
                None,
            ));
        };
        let mut bag = DiagnosticBag::new();
        bag.push(project_config::circular_dependency(path, None));
        PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
            bag,
            strings,
            None,
            Some(std::sync::Arc::new(fork.snapshot_table())),
        ))
    }

    /// A deterministic human-readable description of one module for internal graph failures.
    fn describe_module(&self, module_id: ModuleId) -> String {
        let node = &self.nodes[module_id.index()];
        format!(
            "{:?} {:?} (ModuleId {})",
            node.stable_origin().role(),
            node.stable_origin().logical_module_path(),
            module_id.index()
        )
    }
}

/// Names one module on a dependency cycle left unscheduled by the topological sweep.
///
/// WHAT: walks from the first blocked module through blocked providers until one repeats.
/// WHY: the first blocked module may only depend on a cycle, so its index is not a witness.
///      Every blocked module keeps a blocked provider, so the walk cannot stop early, and the
///      repeated module is on the cycle.
fn module_on_dependency_cycle(
    remaining_providers: &[usize],
    dependency_providers: &[Vec<ModuleId>],
) -> usize {
    let blocked = |index: usize| remaining_providers[index] > 0;
    let mut visited = vec![false; remaining_providers.len()];
    let mut current = remaining_providers
        .iter()
        .position(|remaining| *remaining > 0)
        .expect("a module cycle leaves at least one blocked module");
    while !visited[current] {
        visited[current] = true;
        current = dependency_providers[current]
            .iter()
            .map(|provider| provider.index())
            .find(|&provider| blocked(provider))
            .expect("every blocked module has a blocked provider");
    }
    current
}

impl SupportVisibilityTopology for ProjectModuleGraph {
    fn nearest_parent(&self, module_id: ModuleId) -> Option<ModuleId> {
        self.nodes[module_id.index()].nearest_parent
    }

    fn occupies_normal_position(&self, module_id: ModuleId) -> bool {
        let node = &self.nodes[module_id.index()];
        node.normal_root_file().is_some() || node.has_surface(ModuleRootRole::Entry)
    }

    fn has_support_surface(&self, module_id: ModuleId) -> bool {
        self.nodes[module_id.index()].support_root_file().is_some()
    }
}

impl SupportVisibilityTopology for ModuleIdentityTable {
    fn nearest_parent(&self, module_id: ModuleId) -> Option<ModuleId> {
        self.nearest_ancestor_module(module_id)
    }

    fn occupies_normal_position(&self, module_id: ModuleId) -> bool {
        let record = self.record(module_id);
        record.has_normal_root() || record.has_entry_root()
    }

    fn has_support_surface(&self, module_id: ModuleId) -> bool {
        self.record(module_id).support_root_file().is_some()
    }
}

/// Apply the project graph's scoped-support visibility rule to one package identity table.
///
/// Namespace construction has the package-boundary identity table, not its compiled graph, so it
/// applies the same structural rule directly over those boundary-local identities.
pub(super) fn is_support_visible_in_identity_table(
    identities: &ModuleIdentityTable,
    support_id: ModuleId,
    consumer_id: ModuleId,
) -> bool {
    support_is_visible(identities, support_id, consumer_id)
}

fn support_is_visible(
    topology: &impl SupportVisibilityTopology,
    support_id: ModuleId,
    consumer_id: ModuleId,
) -> bool {
    support_is_visible_for_role(
        topology,
        support_id,
        consumer_id,
        module_consumer_role(topology, consumer_id),
    )
}

/// The consumer role a whole structural module's namespace is built for.
///
/// WHAT: a module with a normal or entry root is built as a normal-position consumer; a
///       support-only module is built as a strict support consumer.
/// WHY: the namespace is module-wide, but a paired module holds sources with different authored
///      roles. Building it for the normal position exposes every provider a normal or entry
///      source may reach, and the module-inventory import guard then rechecks each authored
///      source's role so a co-located support facade gains nothing.
fn module_consumer_role(
    topology: &impl SupportVisibilityTopology,
    module_id: ModuleId,
) -> ModuleRootRole {
    if topology.occupies_normal_position(module_id) {
        ModuleRootRole::Normal
    } else {
        // Every structural module holds at least one root, so a module outside the normal
        // position is support-only and consumes at the strict support scope.
        ModuleRootRole::Support
    }
}

fn support_is_visible_for_role(
    topology: &impl SupportVisibilityTopology,
    support_id: ModuleId,
    consumer_id: ModuleId,
    consumer_role: ModuleRootRole,
) -> bool {
    if !topology.has_support_surface(support_id) {
        return false;
    }

    let Some(owner) = nearest_normal_ancestor(topology, support_id) else {
        return false;
    };

    match consumer_role {
        // An entry root publishes no normal surface but is a normal-position consumer: it sees
        // the support packages its normal position can reach.
        ModuleRootRole::Normal | ModuleRootRole::Entry => {
            consumer_id != support_id
                && (consumer_id == owner
                    || (is_ancestor_of(topology, owner, consumer_id)
                        && !is_ancestor_of(topology, support_id, consumer_id)))
        }

        ModuleRootRole::Support => {
            let Some(consumer_owner) = nearest_normal_ancestor(topology, consumer_id) else {
                return false;
            };

            consumer_id != support_id
                && consumer_owner != owner
                && is_ancestor_of(topology, owner, consumer_owner)
                && !is_ancestor_of(topology, support_id, consumer_id)
        }

        ModuleRootRole::Shared | ModuleRootRole::SyntheticProjectContext => false,
    }
}

fn nearest_normal_ancestor(
    topology: &impl SupportVisibilityTopology,
    module_id: ModuleId,
) -> Option<ModuleId> {
    let mut current = topology.nearest_parent(module_id);

    while let Some(parent) = current {
        if topology.occupies_normal_position(parent) {
            return Some(parent);
        }
        current = topology.nearest_parent(parent);
    }

    None
}

fn is_ancestor_of(
    topology: &impl SupportVisibilityTopology,
    ancestor: ModuleId,
    descendant: ModuleId,
) -> bool {
    let mut current = topology.nearest_parent(descendant);

    while let Some(parent) = current {
        if parent == ancestor {
            return true;
        }
        current = topology.nearest_parent(parent);
    }

    false
}
