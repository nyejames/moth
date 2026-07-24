//! Canonical structural project module graph.
//!
//! WHAT: owns the deterministic project-module graph built directly from the Stage 0
//! [`SourceTreeIndex`]. Each canonical module becomes one node carrying its `ModuleId`,
//! stable origin, root role, root directory/file, nearest structural parent, direct children
//! and owned source set. The graph classifies normal entry candidates and the optional project
//! package facade, encodes strict scoped-support visibility as a query, and exposes
//! deterministic dependency-edge insertion plus topological compile waves over
//! provider-before-consumer edges.
//! WHY: the compiler cannot schedule canonical modules until Stage 0 can distinguish normal
//! modules, support packages and the optional facade, and until dependency order can be
//! derived without a second filesystem traversal or a parallel identity/topology table. This
//! owner consumes the existing [`ModuleIdentityTable`] and owned source sets rather than
//! recomputing them, so identity, ancestry and source ownership stay single-owned.
//!
//! Edge insertion is narrow and production-consumed by Phase 5b: reachable-file discovery
//! retains one local structural dependency fact per cross-module import resolution and the
//! inventory merge maps the fact's canonical roots through this graph before inserting
//! provider-before-consumer edges, so dependency order is derived without a second filesystem
//! traversal or a parallel identity/topology table.
//!
//! Production wiring: Stage 0 constructs the graph once from the [`SourceTreeIndex`] in
//! `project_roots` and retains it as the structural owner. `compile_waves` and `entry_modules`
//! drive deterministic entry selection in `module_inventory`, so graph construction, wave
//! scheduling and dependency-edge insertion are genuine production paths. The
//! scoped-support-visibility surface remains a future consumer and carries a narrowly scoped
//! dead-code allowance until a later slice exercises support-package visibility.

use super::module_identity::ModuleId;
use super::source_tree_index::{OwnedSourceSet, SourceTreeIndex};

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StableModuleOriginIdentity};

use rustc_hash::FxHashMap;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

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
    owned_source_set: OwnedSourceSet,
}

impl ProjectModuleGraphNode {
    /// The dense build-local handle for this module.
    pub(crate) fn module_id(&self) -> ModuleId {
        self.module_id
    }

    // Phase 5b/future-consumer node accessors. `module_id` and `root_file` are production-consumed
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

    /// The canonical root file (`#*.moth` or `+*.moth`) that roots this module.
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

    /// The deterministic owned supported-source set for this module.
    pub(crate) fn owned_source_set(&self) -> &OwnedSourceSet {
        &self.owned_source_set
    }
}

/// The canonical structural project module graph for one build boundary.
///
/// Built directly from a [`SourceTreeIndex`] without filesystem IO or a second identity/topology
/// table. Nodes are stored in deterministic `ModuleId` order. Normal modules are entry
/// candidates; support roots are never entries; the optional project package facade is a node
/// outside the normal ancestry tree.
pub(crate) struct ProjectModuleGraph {
    nodes: Vec<ProjectModuleGraphNode>,
    entry_modules: Vec<ModuleId>,
    facade: Option<ModuleId>,
    // Per-consumer provider sets: the providers each consumer must compile after. Used for
    // indegree counting and idempotent duplicate detection.
    dependency_providers: Vec<BTreeSet<ModuleId>>,
    // Per-provider consumer sets: the consumers that depend on each provider. Used for wave
    // traversal. Maintained in lockstep with `dependency_providers`.
    provider_consumers: Vec<BTreeSet<ModuleId>>,
    // Canonical module root directory to `ModuleId` lookup, owned by the graph so the Phase 5b
    // dependency-fact merge can resolve retained canonical roots to graph identities without
    // recreating the identity table or scanning the filesystem.
    root_directory_to_module_id: FxHashMap<std::path::PathBuf, ModuleId>,
    // Retained authored source location for each inserted provider-before-consumer edge, keyed
    // by the (provider, consumer) `ModuleId` pair. Only the first observation in deterministic
    // merge order is retained; duplicate observations are idempotent for the edge and never
    // overwrite the retained location. Source locations are never used for edge identity.
    edge_source_locations: BTreeMap<(ModuleId, ModuleId), SourceLocation>,
}

impl ProjectModuleGraph {
    /// Build the graph directly from the Stage 0 source-tree index.
    ///
    /// Consumes the index's identity table and owned source sets rather than recomputing them.
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
            let owned_source_set = source_tree_index.owned_source_set(*module_id).clone();

            nodes.push(ProjectModuleGraphNode {
                module_id: *module_id,
                stable_origin: record.stable_origin().clone(),
                role: record.role(),
                root_directory: record.root_directory().to_path_buf(),
                root_file: record.root_file().to_path_buf(),
                nearest_parent,
                direct_children,
                owned_source_set,
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

        let mut root_directory_to_module_id = FxHashMap::default();
        for node in &nodes {
            root_directory_to_module_id.insert(node.root_directory.clone(), node.module_id);
        }

        Self {
            nodes,
            entry_modules,
            facade,
            dependency_providers,
            provider_consumers,
            root_directory_to_module_id,
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
    #[allow(dead_code)]
    pub(crate) fn is_support_visible_to_consumer(
        &self,
        support_id: ModuleId,
        consumer_id: ModuleId,
    ) -> bool {
        if self.role(support_id) != ModuleRootRole::Support {
            return false;
        }
        let Some(owner) = self.nearest_normal_ancestor(support_id) else {
            return false;
        };

        match self.role(consumer_id) {
            ModuleRootRole::Normal => {
                // The owner and normal modules in its remaining subtree consume this package.
                // Normal descendants inside the package's private subtree cannot import their
                // own facade.
                consumer_id == owner
                    || (self.is_ancestor_of(owner, consumer_id)
                        && !self.is_ancestor_of(support_id, consumer_id))
            }

            ModuleRootRole::Support => {
                // A support facade may import packages from a strictly outer scope. Same-scope
                // support siblings and support facades inside this package's private subtree stay
                // isolated.
                let Some(consumer_owner) = self.nearest_normal_ancestor(consumer_id) else {
                    return false;
                };

                consumer_id != support_id
                    && consumer_owner != owner
                    && self.is_ancestor_of(owner, consumer_owner)
                    && !self.is_ancestor_of(support_id, consumer_id)
            }

            ModuleRootRole::ProjectPackageFacade => false,
        }
    }

    /// The canonical `ModuleId` whose root directory matches `root_directory`, or `None` when
    /// the root is not a project module graph node.
    ///
    /// WHAT: the graph-owned canonical-root-to-`ModuleId` mapping consumed by the Phase 5b
    ///       dependency-fact merge. Registered source-package roots outside the project graph
    ///       are intentionally absent and are ignored by the caller before edge insertion.
    /// WHY: edge insertion must not recreate the `ModuleIdentityTable` or scan the filesystem;
    ///      the graph already carries every canonical root directory as a node field.
    pub(crate) fn module_id_for_root_directory(&self, root_directory: &Path) -> Option<ModuleId> {
        self.root_directory_to_module_id
            .get(root_directory)
            .copied()
    }

    /// Build a lookup from canonical source path to owning stable module origin from every
    /// module's owned source set.
    ///
    /// WHAT: the one production path that materializes the graph's OwnedSourceSet ownership
    ///       authority into a canonical-path-to-StableModuleOriginIdentity map consumed by
    ///       directory-module preparation to build the per-module SourceModuleOriginTable.
    /// WHY: the SourceModuleOriginTable must resolve each prepared source file to its
    ///      graph-owned origin without a second filesystem traversal or a parallel topology
    ///      table. The graph already carries every owned source entry with its stable identity,
    ///      so this lookup is a direct projection, not a scan or guess.
    ///
    /// A canonical path owned by two modules is a proven graph-construction invariant violation
    /// surfaced through CompilerError rather than silently overwriting one origin.
    pub(crate) fn build_source_origin_lookup(
        &self,
    ) -> Result<FxHashMap<std::path::PathBuf, StableModuleOriginIdentity>, CompilerError> {
        let mut origins: FxHashMap<std::path::PathBuf, StableModuleOriginIdentity> =
            FxHashMap::default();

        for node in &self.nodes {
            let node_origin = node.stable_origin();
            for entry in node.owned_source_set().entries() {
                let entry_origin = entry.stable_identity().module_origin();
                if entry_origin != node_origin {
                    return Err(CompilerError::compiler_error(format!(
                        "Project module graph owned source entry {} has a stable identity module origin ({:?}) that does not match its containing graph node origin ({:?})",
                        entry.canonical_path().display(),
                        entry_origin,
                        node_origin,
                    )));
                }
                let canonical_path = entry.canonical_path().to_path_buf();
                if origins.contains_key(&canonical_path) {
                    return Err(CompilerError::compiler_error(format!(
                        "Project module graph owned source sets assign canonical path {} to multiple modules; each source file must have exactly one owning module",
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

        if self.dependency_providers[consumer.index()].contains(&provider) {
            return Ok(DependencyEdgeOutcome::AlreadyPresent);
        }

        self.dependency_providers[consumer.index()].insert(provider);
        self.provider_consumers[provider.index()].insert(consumer);

        Ok(DependencyEdgeOutcome::Inserted)
    }

    /// Insert one resolved local structural dependency edge and retain its authored location.
    ///
    /// WHAT: the Phase 5b production edge-insertion path. Maps already-resolved `ModuleId`
    ///       identities to the low-level [`add_dependency_edge`] inserter and, for a newly
    ///       inserted edge, retains the exact authored `SourceLocation` carried by the
    ///       dependency fact. Duplicate observations are idempotent for the edge and never
    ///       overwrite the retained location; source locations are never used for edge identity.
    /// WHY: the inventory merge resolves canonical roots to `ModuleId` through
    ///      [`module_id_for_root_directory`] and then calls this method so the graph stays the
    ///      single owner of both edge adjacency and the retained dependency-fact provenance.
    pub(crate) fn add_local_structural_dependency_edge(
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
    /// survive the Phase 5b dependency-fact merge.
    #[cfg(test)]
    pub(crate) fn edge_source_location(
        &self,
        provider: ModuleId,
        consumer: ModuleId,
    ) -> Option<&SourceLocation> {
        self.edge_source_locations.get(&(provider, consumer))
    }

    /// Whether a provider-before-consumer dependency edge is currently present.
    #[allow(dead_code)]
    pub(crate) fn has_dependency_edge(&self, provider: ModuleId, consumer: ModuleId) -> bool {
        self.dependency_providers
            .get(consumer.index())
            .is_some_and(|providers| providers.contains(&provider))
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
        let node_count = self.node_count();
        let mut remaining_providers: Vec<usize> = self
            .dependency_providers
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
                for consumer in self.provider_consumers[provider.index()].iter() {
                    let outstanding = &mut remaining_providers[consumer.index()];
                    *outstanding -= 1;
                    if *outstanding == 0 {
                        next_ready.push(*consumer);
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

    #[allow(dead_code)]
    /// The structural root role for one module.
    fn role(&self, module_id: ModuleId) -> ModuleRootRole {
        self.nodes[module_id.index()].role
    }

    #[allow(dead_code)]
    /// Whether `ancestor` is the nearest parent, or a transitive nearest-parent, of
    /// `descendant`.
    fn is_ancestor_of(&self, ancestor: ModuleId, descendant: ModuleId) -> bool {
        let mut current = self.nodes[descendant.index()].nearest_parent;
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self.nodes[parent.index()].nearest_parent;
        }
        false
    }

    /// The nearest normal ancestor of `module_id`, walking past intervening support modules.
    ///
    /// Returns `None` for the entry root, the facade and any support module with no enclosing
    /// normal module.
    #[allow(dead_code)]
    fn nearest_normal_ancestor(&self, module_id: ModuleId) -> Option<ModuleId> {
        let mut current = self.nodes[module_id.index()].nearest_parent;
        while let Some(parent) = current {
            if self.nodes[parent.index()].role == ModuleRootRole::Normal {
                return Some(parent);
            }
            current = self.nodes[parent.index()].nearest_parent;
        }
        None
    }

    #[allow(dead_code)]
    /// Whether `module_id` is a valid graph identity.
    fn is_valid_module_id(&self, module_id: ModuleId) -> bool {
        module_id.index() < self.node_count()
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
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
