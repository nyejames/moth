//! Canonical structural project module graph.
//!
//! WHAT: owns the deterministic project-module graph built directly from the Stage 0
//! [`SourceTreeIndex`]. Each canonical module becomes one node carrying its `ModuleId`,
//! stable origin, root role, root directory/file, nearest structural parent, direct children
//! and structural dependency edges. The graph classifies normal entry candidates and the
//! optional project package facade, encodes strict scoped-support visibility as a query, and
//! exposes deterministic dependency-edge insertion, an explicit construction-to-completion
//! phase transition that freezes provider/consumer adjacency into sorted `Vec<ModuleId>` storage,
//! and topological compile waves over those frozen provider-before-consumer edges. Source records
//! and ownership remain in `SourceTreeIndex`.
//! WHY: the compiler cannot schedule canonical modules until Stage 0 can distinguish normal
//! modules, support packages and the optional facade, and until dependency order can be
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
//! drive deterministic entry selection in `module_inventory`, so graph construction, wave
//! scheduling and dependency-edge insertion are genuine production paths. The
//! namespace builder consumes the scoped-support-visibility surface for project and package
//! boundaries.

#[cfg(test)]
use super::module_identity::ModuleIdentityRecord;
use super::module_identity::{ModuleId, ModuleIdentityTable};
use super::source_tree_index::SourceTreeIndex;

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StableModuleOriginIdentity};

use rustc_hash::FxHashMap;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Outcome of inserting one deterministic provider-before-consumer dependency edge.
///
/// WHAT: tells the caller whether a new edge was added or whether the edge was already present.
/// WHY: a duplicate edge does not change the dependency graph, so insertion is idempotent rather
///      than an error. Self-edges and out-of-range module IDs remain internal graph failures
///      reported through [`CompilerError`].
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
    fn role(&self, module_id: ModuleId) -> ModuleRootRole;

    fn nearest_parent(&self, module_id: ModuleId) -> Option<ModuleId>;
}

/// One canonical module node in the project module graph.
///
/// Nodes are stored in deterministic `ModuleId` order so the graph stays aligned with the
/// Stage 0 identity table. Each field is consumed from the existing owners (identity table and
/// source index) at construction time, not recomputed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectModuleGraphNode {
    module_id: ModuleId,
    stable_origin: StableModuleOriginIdentity,
    role: ModuleRootRole,
    root_directory: std::path::PathBuf,
    root_file: std::path::PathBuf,
    nearest_parent: Option<ModuleId>,
    direct_children: Vec<ModuleId>,
}

impl ProjectModuleGraphNode {
    /// The dense build-local handle for this module.
    pub(crate) fn module_id(&self) -> ModuleId {
        self.module_id
    }

    // Structural node accessors. `module_id` and `root_file` are production-consumed
    // by compile-wave scheduling and entry selection; the remaining identity, ancestry and
    // source-set accessors expose data the graph carries for later provider-edge and source-set
    // consumers and are exercised by focused graph-invariant tests.
    /// The owned cross-build origin identity for this module.
    pub(crate) fn stable_origin(&self) -> &StableModuleOriginIdentity {
        &self.stable_origin
    }

    #[allow(dead_code)]
    /// The structural root role (`Normal`, `Support` or `ProjectPackageFacade`).
    pub(crate) fn role(&self) -> ModuleRootRole {
        self.role
    }

    #[allow(dead_code)]
    /// The canonical root directory that scopes this module.
    pub(crate) fn root_directory(&self) -> &Path {
        &self.root_directory
    }

    /// The canonical root file (`@*.moth` or `+*.moth`) that roots this module.
    pub(crate) fn root_file(&self) -> &Path {
        &self.root_file
    }

    #[allow(dead_code)]
    /// The nearest structural parent module, or `None` for the entry root and the facade.
    pub(crate) fn nearest_parent(&self) -> Option<ModuleId> {
        self.nearest_parent
    }

    #[allow(dead_code)]
    /// Direct child modules whose nearest structural parent is this module, in `ModuleId` order.
    pub(crate) fn direct_children(&self) -> &[ModuleId] {
        &self.direct_children
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
/// table. Nodes are stored in deterministic `ModuleId` order. Normal modules are entry
/// candidates; support roots are never entries; the optional project package facade is a node
/// outside the normal ancestry tree. Edge insertion and compile-wave scheduling are separated by
/// one explicit [`ProjectModuleDependencies`] transition: edges are inserted while the graph is
/// under construction, then [`ProjectModuleGraph::complete`] freezes adjacency into sorted
/// `Vec<ModuleId>` storage before [`ProjectModuleGraph::compile_waves`] consumes it.
pub(crate) struct ProjectModuleGraph {
    nodes: Vec<ProjectModuleGraphNode>,
    entry_modules: Vec<ModuleId>,
    facade: Option<ModuleId>,
    dependencies: ProjectModuleDependencies,
    // Retained authored source location for each inserted provider-before-consumer edge, keyed
    // by the (provider, consumer) `ModuleId` pair. Only the first observation in deterministic
    // merge order is retained; duplicate observations are idempotent for the edge and never
    // overwrite the retained location. Source locations are never used for edge identity.
    edge_source_locations: BTreeMap<(ModuleId, ModuleId), SourceLocation>,
}

impl ProjectModuleGraph {
    /// Build a synthetic graph from explicit normal module roots.
    ///
    /// WHAT: creates one frozen `Normal` node per `(stable origin, root directory, root file)`
    /// in the given order with dense `ModuleId` values starting at zero and no dependency edges.
    /// WHY: single-file compilation is a synthetic-module mode with no `SourceTreeIndex`, and
    /// focused tests need real graph boundaries without filesystem discovery. The graph keeps
    /// the same frozen adjacency contract as discovery-built graphs.
    pub(crate) fn from_normal_roots(
        roots: Vec<(StableModuleOriginIdentity, PathBuf, PathBuf)>,
    ) -> Self {
        let node_count = roots.len();
        let nodes = roots
            .into_iter()
            .enumerate()
            .map(
                |(index, (stable_origin, root_directory, root_file))| ProjectModuleGraphNode {
                    module_id: ModuleId::from_index(index),
                    stable_origin,
                    role: ModuleRootRole::Normal,
                    root_directory,
                    root_file,
                    nearest_parent: None,
                    direct_children: Vec::new(),
                },
            )
            .collect::<Vec<_>>();
        let entry_modules = nodes.iter().map(|node| node.module_id).collect();

        Self {
            nodes,
            entry_modules,
            facade: None,
            dependencies: ProjectModuleDependencies::Frozen {
                dependency_providers: vec![Vec::new(); node_count],
                provider_consumers: vec![Vec::new(); node_count],
            },
            edge_source_locations: BTreeMap::new(),
        }
    }

    /// Build the graph directly from the Stage 0 source-tree index.
    ///
    /// Consumes the index's identity table rather than recomputing it. Source ownership remains
    /// in the retained index and is not copied into graph nodes.
    /// Each module becomes one node in deterministic `ModuleId` order. Normal modules are
    /// classified as entry candidates; the optional project package facade is recorded as a
    /// node outside the normal ancestry tree.
    pub(crate) fn from_source_tree_index(source_tree_index: &SourceTreeIndex) -> Self {
        let identities = source_tree_index.module_identities();
        let module_ids: Vec<ModuleId> = identities.module_ids().collect();
        let node_count = module_ids.len();

        let mut nodes = Vec::with_capacity(node_count);
        let mut entry_modules = Vec::new();
        let mut facade = None;

        for module_id in &module_ids {
            let record = identities.record(*module_id);
            let nearest_parent = identities.nearest_ancestor_module(*module_id);
            let direct_children = identities.direct_child_modules(*module_id).to_vec();

            nodes.push(ProjectModuleGraphNode {
                module_id: *module_id,
                stable_origin: record.stable_origin().clone(),
                role: record.role(),
                root_directory: record.root_directory().to_path_buf(),
                root_file: record.root_file().to_path_buf(),
                nearest_parent,
                direct_children,
            });

            match record.role() {
                ModuleRootRole::Normal => entry_modules.push(*module_id),
                ModuleRootRole::ProjectPackageFacade => facade = Some(*module_id),
                ModuleRootRole::Support => {}
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
            facade,
            dependencies: ProjectModuleDependencies::UnderConstruction {
                dependency_providers,
                provider_consumers,
            },
            edge_source_locations: BTreeMap::new(),
        }
    }

    /// Build a graph from explicit identities for assembly tests.
    ///
    /// WHAT: supplies deterministic graph nodes without requiring source discovery to create a
    /// fixture. WHY: package assembly tests need to model a facade and selected descendants while
    /// keeping their source artefacts synthetic and focused on liveness.
    #[cfg(test)]
    pub(crate) fn from_test_records(records: Vec<ModuleIdentityRecord>) -> Self {
        let identities = ModuleIdentityTable::from_records(records);
        let module_ids: Vec<ModuleId> = identities.module_ids().collect();
        let node_count = module_ids.len();
        let mut nodes = Vec::with_capacity(node_count);
        let mut entry_modules = Vec::new();
        let mut facade = None;

        for module_id in &module_ids {
            let record = identities.record(*module_id);
            nodes.push(ProjectModuleGraphNode {
                module_id: *module_id,
                stable_origin: record.stable_origin().clone(),
                role: record.role(),
                root_directory: record.root_directory().to_path_buf(),
                root_file: record.root_file().to_path_buf(),
                nearest_parent: None,
                direct_children: Vec::new(),
            });

            match record.role() {
                ModuleRootRole::Normal => entry_modules.push(*module_id),
                ModuleRootRole::ProjectPackageFacade => facade = Some(*module_id),
                ModuleRootRole::Support => {}
            }
        }

        Self {
            nodes,
            entry_modules,
            facade,
            dependencies: ProjectModuleDependencies::Frozen {
                dependency_providers: vec![Vec::new(); node_count],
                provider_consumers: vec![Vec::new(); node_count],
            },
            edge_source_locations: BTreeMap::new(),
        }
    }

    /// The number of canonical module nodes in the graph.
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[allow(dead_code)]
    /// All graph nodes in deterministic `ModuleId` order.
    pub(crate) fn nodes(&self) -> &[ProjectModuleGraphNode] {
        &self.nodes
    }

    /// The canonical node for one module identity.
    ///
    /// `module_id` must be a valid identity produced by the Stage 0 identity table.
    pub(crate) fn node(&self, module_id: ModuleId) -> &ProjectModuleGraphNode {
        &self.nodes[module_id.index()]
    }

    /// Normal entry-candidate modules in deterministic `ModuleId` order.
    ///
    /// Support roots and the project package facade are never entry candidates.
    pub(crate) fn entry_modules(&self) -> &[ModuleId] {
        &self.entry_modules
    }

    /// Completed source providers required before one module may bind dependencies.
    pub(crate) fn dependency_providers(
        &self,
        module_id: ModuleId,
    ) -> Result<&[ModuleId], CompilerError> {
        if !self.is_valid_module_id(module_id) {
            return Err(CompilerError::compiler_error(format!(
                "Project module graph received out-of-range ModuleId {} for provider lookup",
                module_id.index()
            )));
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

    #[allow(dead_code)]
    /// The optional project package facade module, outside the normal ancestry tree.
    pub(crate) fn facade(&self) -> Option<ModuleId> {
        self.facade
    }

    /// Strict scoped-support visibility query.
    ///
    /// For a support package `S` whose nearest normal ancestor is `P`, `S` is visible to `P`, to
    /// normal descendants of `P` outside `S`'s private subtree, and to support facades in a
    /// strictly nested normal scope. It is not visible above `P`, outside `P`'s subtree, to `S`
    /// itself, to `S`'s private descendants, or to another support module owned by `P`. Returns
    /// `false` when `support_id` is not a support module.
    pub(crate) fn is_support_visible_to_consumer(
        &self,
        support_id: ModuleId,
        consumer_id: ModuleId,
    ) -> bool {
        support_is_visible(self, support_id, consumer_id)
    }

    /// Build a lookup from canonical source path to owning stable module origin from every
    /// module's owned source IDs.
    ///
    /// WHAT: the one production path that projects the retained central `SourceTreeIndex`
    ///       ownership authority into a canonical-path-to-StableModuleOriginIdentity map
    ///       consumed by directory-module preparation to build the per-module
    ///       SourceModuleOriginTable. The graph carries no source records; it reads the index
    ///       directly so source ownership stays single-owned by the index.
    /// WHY: the SourceModuleOriginTable must resolve each prepared source file to its
    ///      owning stable module origin without a second filesystem traversal or a parallel
    ///      topology table. The index already carries every owned source record with its
    ///      portable logical identity, so this lookup is a direct projection, not a scan or
    ///      guess.
    ///
    /// A canonical path owned by two modules, or an owned source whose logical identity module
    /// origin does not match its graph node origin, is a proven invariant violation surfaced
    /// through CompilerError rather than silently overwriting one origin.
    pub(crate) fn build_source_origin_lookup(
        &self,
        source_tree_index: &SourceTreeIndex,
    ) -> Result<FxHashMap<std::path::PathBuf, StableModuleOriginIdentity>, CompilerError> {
        let mut origins: FxHashMap<std::path::PathBuf, StableModuleOriginIdentity> =
            FxHashMap::default();

        for node in &self.nodes {
            let node_origin = node.stable_origin();
            for source_id in source_tree_index.owned_source_ids(node.module_id()) {
                let record = source_tree_index.source(*source_id);
                let Some(entry_origin) = record.logical_identity().module_origin() else {
                    return Err(CompilerError::compiler_error(format!(
                        "Project module graph owned source ID {} resolves to an unrooted record; \
                         an owned source record must carry an owned logical identity",
                        source_id.index(),
                    )));
                };
                if entry_origin != node_origin {
                    return Err(CompilerError::compiler_error(format!(
                        "Project module graph owned source entry {} has a stable identity module origin ({:?}) that does not match its containing graph node origin ({:?})",
                        record.canonical_path().display(),
                        entry_origin,
                        node_origin,
                    )));
                }
                let canonical_path = record.canonical_path().to_path_buf();
                if origins.contains_key(&canonical_path) {
                    return Err(CompilerError::compiler_error(format!(
                        "Source tree index owned source ID sets assign canonical path {} to multiple modules; each source file must have exactly one owning module",
                        canonical_path.display()
                    )));
                }
                origins.insert(canonical_path, entry_origin.clone());
            }
        }

        Ok(origins)
    }

    /// Insert one deterministic provider-before-consumer dependency edge.
    ///
    /// The provider must compile before the consumer. Module IDs are validated and self-edges
    /// are rejected through an internal [`CompilerError`] without panicking. A duplicate edge
    /// is idempotent and reports [`DependencyEdgeOutcome::AlreadyPresent`] because it does not
    /// change the dependency graph.
    pub(crate) fn add_dependency_edge(
        &mut self,
        provider: ModuleId,
        consumer: ModuleId,
    ) -> Result<DependencyEdgeOutcome, CompilerError> {
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
    ///       adjacency in lockstep, and the dependency state becomes `Frozen`. The retained
    ///       authored edge locations are unaffected.
    /// WHY: compile-wave scheduling reads only the frozen adjacency so the graph keeps one
    ///      complete adjacency representation. Completing an already-completed graph is a
    ///      mutation after completion and reports an internal [`CompilerError`] rather than
    ///      silently no-op-ing, so the lifecycle transition stays a single explicit step.
    pub(crate) fn complete(&mut self) -> Result<(), CompilerError> {
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

    /// Insert one resolved structural dependency edge and retain its authored location.
    ///
    /// WHAT: the production edge-insertion path maps already-resolved `ModuleId`
    ///       identities to the low-level [`add_dependency_edge`] inserter and, for a newly
    ///       inserted edge, retains the exact authored `SourceLocation` carried by the
    ///       dependency reference. Duplicate observations are idempotent for the edge and never
    ///       overwrite the retained location; source locations are never used for edge identity.
    /// WHY: the namespace resolves dependencies to `ModuleId` directly and then calls this method so
    ///      the graph stays the single owner of both edge adjacency and retained provenance.
    pub(crate) fn add_resolved_dependency_edge(
        &mut self,
        provider: ModuleId,
        consumer: ModuleId,
        authored_location: SourceLocation,
    ) -> Result<DependencyEdgeOutcome, CompilerError> {
        let outcome = self.add_dependency_edge(provider, consumer)?;
        if outcome == DependencyEdgeOutcome::Inserted {
            self.edge_source_locations
                .insert((provider, consumer), authored_location);
        }
        Ok(outcome)
    }

    /// The retained authored source location for one provider-before-consumer edge, if present.
    ///
    /// Focused graph-invariant tests use this to verify that exact authored source locations
    /// survive direct edge insertion.
    #[cfg(test)]
    pub(crate) fn edge_source_location(
        &self,
        provider: ModuleId,
        consumer: ModuleId,
    ) -> Option<&SourceLocation> {
        self.edge_source_locations.get(&(provider, consumer))
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
    /// optional project package facade is ordered by its real edges, never by a hard-coded fake
    /// dependency.
    ///
    /// A defensive cycle returns an internal [`CompilerError`] naming the modules left blocked by
    /// cyclic dependencies in deterministic `ModuleId` order.
    pub(crate) fn compile_waves(&self) -> Result<Vec<Vec<ModuleId>>, CompilerError> {
        let ProjectModuleDependencies::Frozen {
            dependency_providers,
            provider_consumers,
        } = &self.dependencies
        else {
            return Err(Self::scheduling_before_completion_error());
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
            return Err(self.cycle_error(&remaining_providers));
        }

        Ok(waves)
    }

    /// Whether `module_id` is a valid graph identity.
    fn is_valid_module_id(&self, module_id: ModuleId) -> bool {
        module_id.index() < self.node_count()
    }

    /// Build an internal graph failure for mutating a completed graph.
    fn mutation_after_completion_error() -> CompilerError {
        CompilerError::compiler_error(
            "Project module graph was mutated after completion; dependency edges must be inserted \
             before the graph completes and freezes its adjacency for compile-wave scheduling",
        )
    }

    /// Build an internal graph failure for scheduling before the graph completes.
    fn scheduling_before_completion_error() -> CompilerError {
        CompilerError::compiler_error(
            "Project module graph compile waves were requested before completion; the graph must \
             complete and freeze its adjacency before compile-wave scheduling reads it",
        )
    }

    /// Build an internal graph failure for an out-of-range module ID supplied to edge insertion.
    fn invalid_module_id_edge_error(
        &self,
        provider: ModuleId,
        consumer: ModuleId,
    ) -> CompilerError {
        CompilerError::compiler_error(format!(
            "Project module graph received a dependency edge with an out-of-range module ID: \
             provider index {} consumer index {} but the graph has {} modules",
            provider.index(),
            consumer.index(),
            self.node_count()
        ))
    }

    /// Build an internal graph failure for a self-edge supplied to edge insertion.
    fn self_edge_error(&self, module_id: ModuleId) -> CompilerError {
        let origin = self.describe_module(module_id);
        CompilerError::compiler_error(format!(
            "Project module graph received a self-dependency edge from module {origin}; a module \
             cannot be its own provider"
        ))
    }

    /// Build an internal graph failure for a dependency cycle, naming every module still blocked
    /// by cyclic dependencies in deterministic `ModuleId` order.
    fn cycle_error(&self, remaining_providers: &[usize]) -> CompilerError {
        let blocked: Vec<String> = remaining_providers
            .iter()
            .enumerate()
            .filter_map(|(index, remaining)| {
                if *remaining > 0 {
                    Some(self.describe_module(self.nodes[index].module_id()))
                } else {
                    None
                }
            })
            .collect();

        CompilerError::compiler_error(format!(
            "Project module dependency cycle detected; {} module(s) remain blocked: {}",
            blocked.len(),
            blocked.join(", ")
        ))
    }

    /// A deterministic human-readable description of one module for internal graph failures.
    fn describe_module(&self, module_id: ModuleId) -> String {
        let node = &self.nodes[module_id.index()];
        format!(
            "{:?} {:?} (ModuleId {})",
            node.role,
            node.stable_origin.logical_module_path(),
            module_id.index()
        )
    }
}

impl SupportVisibilityTopology for ProjectModuleGraph {
    fn role(&self, module_id: ModuleId) -> ModuleRootRole {
        self.nodes[module_id.index()].role
    }

    fn nearest_parent(&self, module_id: ModuleId) -> Option<ModuleId> {
        self.nodes[module_id.index()].nearest_parent
    }
}

impl SupportVisibilityTopology for ModuleIdentityTable {
    fn role(&self, module_id: ModuleId) -> ModuleRootRole {
        self.record(module_id).role()
    }

    fn nearest_parent(&self, module_id: ModuleId) -> Option<ModuleId> {
        self.nearest_ancestor_module(module_id)
    }
}

/// Apply the project graph's scoped-support visibility rule to one package identity table.
///
/// Package boundaries don't yet retain their own graph, so namespace construction consumes the
/// same structural rule directly over the package's boundary-local identity table.
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
    if topology.role(support_id) != ModuleRootRole::Support {
        return false;
    }

    let Some(owner) = nearest_normal_ancestor(topology, support_id) else {
        return false;
    };

    match topology.role(consumer_id) {
        ModuleRootRole::Normal => {
            consumer_id == owner
                || (is_ancestor_of(topology, owner, consumer_id)
                    && !is_ancestor_of(topology, support_id, consumer_id))
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

        ModuleRootRole::ProjectPackageFacade => false,
    }
}

fn nearest_normal_ancestor(
    topology: &impl SupportVisibilityTopology,
    module_id: ModuleId,
) -> Option<ModuleId> {
    let mut current = topology.nearest_parent(module_id);

    while let Some(parent) = current {
        if topology.role(parent) == ModuleRootRole::Normal {
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
