//! Directory-project module inventory assembly.
//!
//! WHAT: turns every canonical project module graph node into a `ModuleCompilationJob`
//! records carrying their graph-assigned stable module origin and all transitively reachable
//! input files.
//! WHY: module inventory is the Stage 0 bridge between the structural graph and frontend
//! compilation. The graph-owned `StableModuleOriginIdentity` travels with each module so semantic
//! compilation receives a canonical identity instead of reconstructing one from an entry path.
//! Entry root paths and deterministic compile-wave grouping come from the graph's compile waves so
//! entry classification has one owner; semantic module jobs are scheduled serially while each
//! job may parallelize its own file preparation. Root setup and source-backed package validation
//! live in sibling modules.

use crate::builder_surface::SourceFileKind;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    InvalidConfigReason, PremergeDiagnosticBatch, PremergeFailure,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::parse_file_headers::FileRole;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::source::{
    SourceDatabase, SourceId as CompilerSourceId, SourceSpanBuilders,
};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::string_interning::{StringTable, StringTableForkSource};
use crate::compiler_frontend::symbols::path_interner::{PathInternerBuilder, PathInternerForkSource};
use crate::projects::settings::Config;
use rustc_hash::FxHashMap;

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::file_reference_resolution::FileReferenceResolver;
use super::module_identity::ModuleId;
use super::module_namespace::{DirectoryDependencyResolution, ResolvedDependency};
use super::module_preparation::{ModulePreparationContext, RegisteredModuleSources};
use super::prepared_module::PreparedModule;
use super::project_module_graph::ProjectModuleGraph;
use super::project_structure_diagnostics::{config_diagnostic, path_id};
use super::resource_inputs::ResourceInputRegistry;
use super::source_discovery::{
    ExternalImportDiscoveryState, ResolvedDependencyEdge, ResolvedSourcePackageDependency,
    StructuralProviderAction, prepare_owned_source_input, resolve_structural_provider_reference,
};
use super::source_loading::SelectedSourceTextMap;
use super::source_tree_index::{
    SourceClassification, SourceOwnership, SourceRecordIndex, SourceTreeIndex,
};

/// One normal entry module seed carrying its graph-assigned `ModuleId` and canonical root file.
///
/// WHAT: discovery seeds entry modules in deterministic `ModuleId` order. The `ModuleId` travels
///       through serial discovery so the deterministic compile-wave reorder can match by identity
///       rather than re-deriving identity from a root path, and so the graph-owned
///       `StableModuleOriginIdentity` is preserved for each discovered module.
/// WHY: the graph owns the canonical origin identity; discovery must not reconstruct it. Carrying
///      the dense `ModuleId` keeps the graph as the single identity owner through reorder.
struct ModuleEntrySeed {
    module_id: ModuleId,
    entry_path: PathBuf,
}

/// Discovery-internal inventory carrying the graph-assigned `ModuleId` through serial discovery so
/// the compile-wave reorder can match by identity.
///
/// The graph-owned `StableModuleOriginIdentity` is attached once, after reorder, when each draft
/// is lifted to the consumer-facing [`ModuleCompilationJob`].
struct ModuleCompilationJobDraft {
    module_id: ModuleId,
    string_table_base_len: usize,
    path_base_len: usize,
    prepared: PreparedModule,
    #[cfg(feature = "timers")]
    timing_module_key: crate::timing::TimingModuleKey,
}

/// One graph-owned module job with its stable origin and prepared semantic inputs.
///
/// Carries both graph identities through the build-owned scheduling boundary. `ModuleId` remains
/// the dense project-local job and merge-order key; `StableModuleOriginIdentity` is the portable
/// semantic identity consumed by the frontend and public interface.
pub(crate) struct ModuleCompilationJob {
    /// Dense project-local identity used only by build-system scheduling and result storage.
    pub(crate) module_id: ModuleId,
    /// The graph-assigned cross-build origin identity for this canonical module.
    #[cfg(test)]
    pub(crate) stable_origin: StableModuleOriginIdentity,
    pub(crate) string_table_base_len: usize,
    /// Prefix length used when merging this job's local path fork.
    pub(crate) path_base_len: usize,
    pub(crate) prepared: PreparedModule,
    #[cfg(feature = "timers")]
    pub(crate) timing_module_key: crate::timing::TimingModuleKey,
}
/// Deferred preparation identity for one owned, unselected `.moth` source.
///
/// Discovery records these descriptors while canonical sources still mutate the shared external
/// provider state. The descriptors are prepared only after every canonical boundary has finished
struct CheckOnlyModuleSpec {
    owner_module_id: ModuleId,
    source_index: SourceRecordIndex,
    candidate_source_indices: Vec<SourceRecordIndex>,
    stable_origin: StableModuleOriginIdentity,
}

/// One transient semantic unit for one owned, unselected `.moth` source.
///
/// Check-only jobs deliberately carry no graph publication state. The owner module ID lets the
/// frontend reuse the canonical boundary context. Provider bindings and source-package dependencies
/// are resolved into this job-local metadata without inserting graph edges. The prepared payload
/// carries the job-local file-reference table and resource IDs; both are discarded with the
/// transient result.
pub(crate) struct CheckOnlyModuleCompilationJob {
    /// Canonical module that owns this transient source.
    pub(crate) owner_module_id: ModuleId,
    /// Prefix length used when merging this job's local string table.
    pub(crate) string_table_base_len: usize,
    /// Prefix length used when merging this job's local path fork.
    pub(crate) path_base_len: usize,
    /// Provider-module bindings resolved for retained clauses in this source only.
    pub(crate) provider_bindings: Vec<CheckOnlyProviderBinding>,
    /// Source-package bindings resolved for retained clauses in this source only.
    pub(crate) source_package_dependencies: Vec<CheckOnlySourcePackageDependency>,
    /// Provider registry snapshot isolated to this transient job.
    ///
    /// Provider-backed imports may create packages while this source is prepared. The snapshot
    /// starts from canonical discovery state but is never written back to the builder surface.
    pub(crate) external_packages: Arc<ExternalPackageRegistry>,
    /// Provider resolution rows isolated to this transient job.
    pub(crate) external_dependency_resolution_table: ExternalImportResolutionTable,
    /// Provider-independent prepared semantic payload for the transient source.
    pub(crate) prepared: PreparedModule,
}

/// One transient authored provider clause bound to a canonical module interface.
///
/// This is intentionally not a `ResolvedDependencyEdge`: check-only resolution must not carry
/// graph insertion spans or be mistaken for a canonical graph edge.
#[derive(Clone, Debug)]
pub(crate) struct CheckOnlyProviderBinding {
    pub(crate) dependency_shell_id: DependencyShellId,
    pub(crate) provider_module_id: ModuleId,
}
pub(crate) struct CheckOnlySourcePackageDependency {
    pub(crate) dependency_shell_id: DependencyShellId,
    pub(crate) dependency_prefix: String,
}
/// The check-only jobs stay in a separate lane so no caller can accidentally publish their
/// interfaces, generated functions, resource associations, graph edges or backend roots.
struct ModuleCompilationJobBatch {
    drafts: Vec<ModuleCompilationJobDraft>,
    resolved_edges: Vec<ResolvedDependencyEdge>,
    source_package_dependencies: Vec<ResolvedSourcePackageDependency>,
    check_only_specs: Vec<CheckOnlyModuleSpec>,
}

fn resolve_directory_dependency_path(
    directory_dependency_resolution: DirectoryDependencyResolution<'_>,
    provider: &RetainedDependencyPath,
    source_path: &Path,
    string_table: &mut StringTable,
) -> Result<ResolvedDependency, PremergeFailure> {
    directory_dependency_resolution
        .resolve_dependency(provider, source_path, string_table)
        .map_err(|diagnostic| {
            // Move the local table into the batch; the caller aborts discovery on this
            // diagnosed path, so no clone is needed to carry the diagnostic.
            let table = std::mem::take(string_table);
            PremergeFailure::Diagnosed(PremergeDiagnosticBatch::from_diagnostic(diagnostic, table))
        })
}
/// Borrowed Stage 0 services and live source spans for one serial discovery pass.
struct ModuleDiscoveryContext<'a, 'sources> {
    project_path_resolver: &'a ProjectPathResolver,
    style_directives: &'a StyleDirectiveRegistry,
    source_spans: &'a mut SourceSpanBuilders<'sources>,
    /// Snapshots selected by the reachability walk and retained after the walk completes.
    selected_source_texts: &'a mut SelectedSourceTextMap,
    directory_dependency_resolution: DirectoryDependencyResolution<'a>,
    project_module_graph: &'a ProjectModuleGraph,
    /// One boundary-owned source-origin table shared by every prepared module and check-only
    /// source in this project or package boundary.
    source_module_origins: Arc<SourceModuleOriginTable>,
}
/// Normal entry modules grouped by the populated graph's compile waves.
///
/// WHAT: owns the wave-preserving data contract between module inventory and directory
///       semantic compilation. Each wave holds every canonical module job in one retained graph
///       dependency wave, preserving the populated graph's dependency ordering and deterministic
///       `ModuleId` order. This schedule feeds the provider-store
///       scheduler; completed waves publish immutable interfaces before later consumers bind.
/// WHY: preserving wave boundaries lets the directory compiler execute provider-dependent semantic
///      jobs in dependency order while retaining one deterministic scheduling contract. File
///      preparation may use Rayon inside each job, but semantic module jobs remain serial until a
///      separate parallel-wave phase changes the publication protocol.
pub(crate) struct ModuleCompilationSchedule {
    waves: Vec<Vec<ModuleCompilationJob>>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    source_package_dependencies: Vec<ResolvedSourcePackageDependency>,
    /// Shared immutable source origins for this project or package boundary.
    source_module_origins: Arc<SourceModuleOriginTable>,
    /// Transient check-only units, kept separate from canonical publication lanes.
    check_only_jobs: Vec<CheckOnlyModuleCompilationJob>,
    /// Deferred transient source descriptors awaiting the final canonical provider state.
    check_only_specs: Vec<CheckOnlyModuleSpec>,
}

impl ModuleCompilationSchedule {
    /// Read-only access to the compile waves in deterministic graph order.
    ///
    /// Each wave is non-empty and contains every canonical module assigned to the graph wave.
    pub(crate) fn waves(&self) -> &[Vec<ModuleCompilationJob>] {
        &self.waves
    }
    pub(crate) fn canonical_source_package_dependencies(
        &self,
    ) -> &[ResolvedSourcePackageDependency] {
        &self.source_package_dependencies
    }

    /// Prepare deferred transient jobs from the final canonical provider state.
    ///
    /// Canonical discovery for every project and source-package boundary must complete before
    /// this method is called. Each job receives the same boundary source database, live span
    /// builder view and source-origin table while transient provider mutations remain isolated
    /// to the job. Failures travel in the premerge lane; the final boundary owns the single
    /// vessel conversion.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_check_only_jobs(
        &mut self,
        style_directives: &StyleDirectiveRegistry,
        source_spans: &mut SourceSpanBuilders<'_>,
        project_path_resolver: &ProjectPathResolver,
        external_imports: &mut ExternalImportDiscoveryState<'_>,
        directory_dependency_resolution: DirectoryDependencyResolution<'_>,
        string_table: &mut StringTable,
        path_interner: &mut PathInternerBuilder,
        selected_source_texts: &mut SelectedSourceTextMap,
    ) -> Result<(), PremergeFailure> {
        let specs = std::mem::take(&mut self.check_only_specs);
        if specs.is_empty() {
            return Ok(());
        }
        let preparation_context = ModulePreparationContext {
            source_files: source_spans.sources(),
            style_directives,
            project_path_resolver: Some(project_path_resolver.clone()),
        };
        let fork_source = string_table.fork_source();
        let path_fork_source = path_interner.fork_source();
        for spec in specs {
            let CheckOnlyModuleSpec {
                owner_module_id,
                source_index,
                candidate_source_indices,
                stable_origin,
            } = spec;
            self.check_only_jobs.push(prepare_check_only_module(
                owner_module_id,
                source_index,
                &candidate_source_indices,
                directory_dependency_resolution.source_tree_index(),
                style_directives,
                &preparation_context,
                source_spans,
                project_path_resolver,
                external_imports,
                directory_dependency_resolution,
                Arc::clone(&self.source_module_origins),
                stable_origin,
                &fork_source,
                &path_fork_source,
                selected_source_texts,
            )?);
        }
        Ok(())
    }

    /// Consume the schedule while retaining the separate transient check-only lane.
    ///
    /// The first three values are the canonical graph publication lanes. Check-only jobs are
    /// returned separately and must never be inserted into those lanes or the project graph.
    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<Vec<ModuleCompilationJob>>,
        Vec<ResolvedDependencyEdge>,
        Vec<ResolvedSourcePackageDependency>,
        Vec<CheckOnlyModuleCompilationJob>,
    ) {
        (
            self.waves,
            self.provider_bindings,
            self.source_package_dependencies,
            self.check_only_jobs,
        )
    }
}

/// Discovers one compilation job for every canonical module in the directory project.
///
/// Root files are seeded from every graph node in deterministic `ModuleId`
/// order. Reachable-file discovery retains direct `ModuleId` edges for cross-module dependencies;
/// after discovery completes the edges enter the graph as provider-before-consumer order, and the
/// returned modules are ordered by the populated graph's compile waves. The directory compiler
/// consumes these waves sequentially; each job may use Rayon for file preparation, but semantic
/// module publication remains serial. Only normal roots remain entry candidates, but support and
/// facade roots now own API-only semantic jobs. A defensive
/// graph cycle, a missing project-local root or a graph/inventory disagreement surfaces as a
/// typed premerge failure without panicking; the final boundary owns the vessel conversion.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn discover_all_modules_in_project(
    config: &Config,
    project_path_resolver: &ProjectPathResolver,
    source_files: &SourceDatabase,
    source_spans: &mut SourceSpanBuilders<'_>,
    project_module_graph: &mut ProjectModuleGraph,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_dependency_resolution: DirectoryDependencyResolution<'_>,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
    path_interner: &mut PathInternerBuilder,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<ModuleCompilationSchedule, PremergeFailure> {
    let mut selected_source_texts = SelectedSourceTextMap::default();
    discover_all_modules_in_project_with_check_only(
        config,
        project_path_resolver,
        source_files,
        source_spans,
        project_module_graph,
        style_directives,
        external_imports,
        directory_dependency_resolution,
        resource_inputs,
        false,
        &mut selected_source_texts,
        string_table,
        path_interner,
        #[cfg(feature = "timers")]
        timing_boundary,
    )
}

/// Discover a project inventory and optionally prepare transient check-only units.
///
/// The default project discovery path remains canonical-only. Check mode opts in explicitly so
#[allow(clippy::too_many_arguments)]
pub(crate) fn discover_all_modules_in_project_with_check_only(
    config: &Config,
    project_path_resolver: &ProjectPathResolver,
    source_files: &SourceDatabase,
    source_spans: &mut SourceSpanBuilders<'_>,
    project_module_graph: &mut ProjectModuleGraph,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_dependency_resolution: DirectoryDependencyResolution<'_>,
    resource_inputs: &mut ResourceInputRegistry,
    include_check_only: bool,
    selected_source_texts: &mut SelectedSourceTextMap,
    string_table: &mut StringTable,
    path_interner: &mut PathInternerBuilder,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<ModuleCompilationSchedule, PremergeFailure> {
    discover_all_modules_in_boundary(
        config,
        project_path_resolver,
        source_files,
        source_spans,
        project_module_graph,
        style_directives,
        external_imports,
        directory_dependency_resolution,
        resource_inputs,
        true,
        include_check_only,
        selected_source_texts,
        string_table,
        path_interner,
        #[cfg(feature = "timers")]
        timing_boundary,
    )
}
/// Discover a source-package inventory and optionally prepare transient check-only units.
///
/// Source packages use the same explicit opt-in as the project boundary; their canonical graph
#[allow(clippy::too_many_arguments)]
pub(crate) fn discover_all_modules_in_package_with_check_only(
    config: &Config,
    project_path_resolver: &ProjectPathResolver,
    source_files: &SourceDatabase,
    source_spans: &mut SourceSpanBuilders<'_>,
    package_module_graph: &mut ProjectModuleGraph,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_dependency_resolution: DirectoryDependencyResolution<'_>,
    resource_inputs: &mut ResourceInputRegistry,
    include_check_only: bool,
    selected_source_texts: &mut SelectedSourceTextMap,
    string_table: &mut StringTable,
    path_interner: &mut PathInternerBuilder,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<ModuleCompilationSchedule, PremergeFailure> {
    discover_all_modules_in_boundary(
        config,
        project_path_resolver,
        source_files,
        source_spans,
        package_module_graph,
        style_directives,
        external_imports,
        directory_dependency_resolution,
        resource_inputs,
        false,
        include_check_only,
        selected_source_texts,
        string_table,
        path_interner,
        #[cfg(feature = "timers")]
        timing_boundary,
    )
}

#[allow(clippy::too_many_arguments)]
fn discover_all_modules_in_boundary(
    config: &Config,
    project_path_resolver: &ProjectPathResolver,
    source_files: &SourceDatabase,
    source_spans: &mut SourceSpanBuilders<'_>,
    project_module_graph: &mut ProjectModuleGraph,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_dependency_resolution: DirectoryDependencyResolution<'_>,
    resource_inputs: &mut ResourceInputRegistry,
    require_normal_entry: bool,
    include_check_only: bool,
    selected_source_texts: &mut SelectedSourceTextMap,
    string_table: &mut StringTable,
    path_interner: &mut PathInternerBuilder,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<ModuleCompilationSchedule, PremergeFailure> {
    let seeds = module_seeds_in_module_id_order(project_module_graph);
    let source_origin_lookup = project_module_graph
        .build_source_origin_lookup(directory_dependency_resolution.source_tree_index())?;
    // Construct one immutable source-origin table for this project or package boundary. Prepared
    // module payloads retain only cloned `Arc` handles, never per-module origin rows.
    let source_module_origins = Arc::new(SourceModuleOriginTable::from_graph_ownership(
        source_files,
        &source_origin_lookup,
    ));
    drop(source_origin_lookup);

    if require_normal_entry && project_module_graph.entry_modules().is_empty() {
        // Move the local table into the diagnosed batch; the final boundary owns the single
        // vessel conversion.
        let diagnostic = config_diagnostic(
            config,
            "entry_root",
            InvalidConfigReason::NoRootModuleEntries {
                entry_root: path_id(project_path_resolver.entry_root(), string_table),
            },
            string_table,
        );
        let table = std::mem::take(string_table);
        return Err(PremergeFailure::Diagnosed(
            PremergeDiagnosticBatch::from_diagnostic(diagnostic, table),
        ));
    }

    let ModuleCompilationJobBatch {
        drafts,
        resolved_edges,
        source_package_dependencies,
        check_only_specs,
    } = discover_modules_serial_provider_capable(
        &seeds,
        ModuleDiscoveryContext {
            project_path_resolver,
            style_directives,
            source_spans,
            selected_source_texts,
            directory_dependency_resolution,
            project_module_graph,
            source_module_origins: Arc::clone(&source_module_origins),
        },
        external_imports,
        resource_inputs,
        include_check_only,
        string_table,
        path_interner,
        #[cfg(feature = "timers")]
        timing_boundary,
    )?;

    // Insert the resolved dependency edges directly by ModuleId before the graph completes.
    // Edges are idempotent, so duplicate retained dependency shells collapse without changing the
    // graph.
    insert_resolved_dependency_edges(project_module_graph, &resolved_edges)?;

    // Freeze the graph's adjacency into sorted `Vec<ModuleId>` storage before compile waves are
    // computed. The no-edge production graph also completes here so scheduling always reads one
    // frozen adjacency. Mutation or scheduling in an invalid phase is an internal `CompilerError`.
    project_module_graph.complete()?;

    // Order the discovered modules by the completed graph's compile waves so providers precede
    // consumers in the returned inventory waves. Discovery seeded entries in `ModuleId` order;
    // this groups the result into dependency-ordered compile waves without re-running discovery.
    // Every canonical module appears in exactly one wave, and the directory compiler consumes one
    // wave at a time; per-module file preparation owns any internal parallelism.
    let schedule = order_discovered_modules_by_compile_waves(
        project_module_graph,
        drafts,
        resolved_edges,
        source_package_dependencies,
        check_only_specs,
        source_module_origins,
    )?;
    Ok(schedule)
}

/// Deterministic canonical module seeds in `ModuleId` order, for discovery seeding.
///
/// Maps every graph node to its `ModuleId` and canonical root file in `ModuleId` order. Compile
/// waves are not consulted here: dependency edges are inserted only after discovery completes,
/// so seeding uses the stable identity order. The
/// `ModuleId` is carried through discovery so the compile-wave reorder matches by identity and
/// the graph-owned `StableModuleOriginIdentity` is preserved without re-deriving it from a path.
fn module_seeds_in_module_id_order(
    project_module_graph: &ProjectModuleGraph,
) -> Vec<ModuleEntrySeed> {
    project_module_graph
        .nodes()
        .iter()
        .map(|node| ModuleEntrySeed {
            module_id: node.module_id(),
            entry_path: node.root_file().to_path_buf(),
        })
        .collect()
}
/// Select the owned, unselected Moth sources for one canonical module.
///
/// `candidate_source_ids` is the exact semantic candidate vector used by the canonical BFS and
/// `selected_source_ids` is its final queued set. Filtering in that order preserves the index's
/// deterministic module-relative path ordering while excluding templates, Markdown, provider-owned
/// records and any source outside the owner boundary. In particular, an unrooted record can never
/// enter a transient unit because it cannot satisfy the explicit ownership check.
fn classify_check_only_source_indices(
    module_id: ModuleId,
    candidate_source_indices: &[SourceRecordIndex],
    selected_source_indices: &BTreeSet<SourceRecordIndex>,
    source_tree_index: &super::source_tree_index::SourceTreeIndex,
) -> Vec<SourceRecordIndex> {
    candidate_source_indices
        .iter()
        .copied()
        .filter(|source_index| !selected_source_indices.contains(source_index))
        .filter(|source_index| {
            let source = source_tree_index.source(*source_index);
            matches!(
                source.classification(),
                SourceClassification::CompilerSemantic(SourceFileKind::Moth)
            ) && matches!(
                source.ownership(),
                SourceOwnership::Owned(owner) if owner == module_id
            )
        })
        .collect()
}
/// Convert the ordered Stage 0 source rows into compiler identities owned by this boundary.
///
/// Source rows are Stage 0 handles rather than compiler identities. Resolve each row's canonical
/// path through the boundary database so a source registered ahead of the rows cannot shift their
/// identities.
fn compiler_source_ids_for_indices(
    source_indices: &[SourceRecordIndex],
    source_tree_index: &SourceTreeIndex,
    source_files: &SourceDatabase,
) -> Result<Vec<CompilerSourceId>, CompilerError> {
    source_indices
        .iter()
        .map(|source_index| {
            let canonical_path = source_tree_index.source(*source_index).canonical_path();
            source_files
                .get_by_canonical_path(canonical_path)
                .map(|source_record| source_record.id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "module inventory: source row {} is absent from the boundary source file table",
                        source_index.index(),
                    ))
                })
        })
        .collect()
}
/// Prepare one owned, unselected Moth source as an isolated transient semantic unit.
///
/// The source table retains the canonical candidate identities so indexed file-reference
/// resolution uses the same module-root and ownership rules as a canonical module. The requested
/// source and every non-Moth content target it reaches are prepared and retained in this job, while
/// no target becomes another module root. Provider clauses, external package discovery and
/// resolution rows are all job-local; file references use a fresh resource registry whose IDs die
/// with the job.
#[allow(clippy::too_many_arguments)]
fn prepare_check_only_module(
    owner_module_id: ModuleId,
    source_index: SourceRecordIndex,
    candidate_source_indices: &[SourceRecordIndex],
    source_tree_index: &super::source_tree_index::SourceTreeIndex,
    style_directives: &StyleDirectiveRegistry,
    preparation_context: &ModulePreparationContext<'_>,
    source_spans: &mut SourceSpanBuilders<'_>,
    project_path_resolver: &ProjectPathResolver,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_dependency_resolution: DirectoryDependencyResolution<'_>,
    source_module_origins: Arc<SourceModuleOriginTable>,
    stable_origin: StableModuleOriginIdentity,
    fork_source: &StringTableForkSource,
    path_fork_source: &PathInternerForkSource,
    selected_source_texts: &mut SelectedSourceTextMap,
) -> Result<CheckOnlyModuleCompilationJob, PremergeFailure> {
    let candidate_source_ids = compiler_source_ids_for_indices(
        candidate_source_indices,
        source_tree_index,
        preparation_context.source_files,
    )?;
    let source = source_tree_index.source(source_index);
    if !matches!(
        source.classification(),
        SourceClassification::CompilerSemantic(SourceFileKind::Moth)
    ) || !matches!(
        source.ownership(),
        SourceOwnership::Owned(owner) if owner == owner_module_id
    ) {
        return Err(CompilerError::compiler_error(format!(
            "ModuleId {} check-only source row {} is not an owned Moth source",
            owner_module_id.index(),
            source_index.index()
        ))
        .into());
    }

    // Provider-backed discovery is intentionally forked from the canonical builder surface. A
    // check-only source may invoke a provider and create external package/cache/resolution rows,
    // but none of those transient mutations may become visible to a later canonical module or
    // another check-only source.
    let providers = external_imports.providers;
    let mut isolated_external_packages = external_imports.external_packages.clone();
    let mut isolated_external_cache = external_imports.cache.clone();
    let mut isolated_resolution_table = external_imports.resolution_table.clone();
    let mut isolated_external_imports = ExternalImportDiscoveryState {
        external_packages: &mut isolated_external_packages,
        providers,
        cache: &mut isolated_external_cache,
        resolution_table: &mut isolated_resolution_table,
    };

    let source_order = candidate_source_indices
        .iter()
        .enumerate()
        .map(|(order, source_index)| (*source_index, order))
        .collect::<FxHashMap<_, _>>();
    let entry_file_path = source.canonical_path().to_path_buf();
    let fork = fork_source.fork_for_module();
    let (local_string_table, string_table_base_len) = fork.into_parts();
    let path_fork = path_fork_source.fork_for_module();
    let path_base_len = path_fork.base_len();
    let mut syntax = preparation_context.begin_syntax_discovery(
        stable_origin,
        RegisteredModuleSources {
            candidate_source_ids,
            source_module_origins,
        },
        &entry_file_path,
        Some(FileRole::Normal),
        local_string_table,
        path_fork,
        selected_source_texts,
        #[cfg(feature = "timers")]
        None,
    )?;

    let mut provider_bindings = Vec::new();
    let mut source_package_dependencies = Vec::new();
    let mut pending_module_sources = VecDeque::from([source_index]);
    let mut queued_module_sources = BTreeSet::from([source_index]);
    // Resolve file-value paths against a job-local resource registry. The prepared payload retains
    // the settled occurrence table, while physical source IDs and missing watches are discarded
    // with this resolver instead of entering the canonical resource registry.
    let mut transient_resource_inputs = ResourceInputRegistry::new();
    let mut file_reference_resolver =
        FileReferenceResolver::new(source_tree_index, &mut transient_resource_inputs);
    let mut pending_content_sources = BTreeSet::new();
    let mut prepared_content_sources = BTreeSet::new();

    // Same-module clauses form a source closure inside the transient job. Every reached Moth
    // source is prepared into the one module-local header set, while cross-module and
    // source-package clauses remain job-local binding metadata.
    while let Some(current_source_index) = pending_module_sources.pop_front() {
        let current_source = source_tree_index.source(current_source_index);
        if !matches!(
            current_source.classification(),
            SourceClassification::CompilerSemantic(SourceFileKind::Moth)
        ) || !matches!(
            current_source.ownership(),
            SourceOwnership::Owned(owner) if owner == owner_module_id
        ) {
            return Err(graph_inventory_mismatch_error(format!(
                "ModuleId {} reached check-only source row {} outside its owned Moth source set",
                owner_module_id.index(),
                current_source_index.index()
            )));
        }
        let current_order = source_order
            .get(&current_source_index)
            .copied()
            .ok_or_else(|| {
                graph_inventory_mismatch_error(format!(
                    "ModuleId {} reached check-only source ID {} outside its candidate source set",
                    owner_module_id.index(),
                    current_source_index.index()
                ))
            })?;
        let current_file_path = current_source.canonical_path().to_path_buf();
        let input_result = {
            let (syntax_string_table, selected_source_texts, syntax_path_fork) =
                syntax.source_preparation_inputs_and_path_fork_mut();
            prepare_owned_source_input(
                current_source_index,
                source_tree_index,
                preparation_context.source_files,
                source_spans,
                style_directives,
                syntax_string_table,
                syntax_path_fork,
                selected_source_texts,
            )
        };
        let input = input_result.map_err(|error| error.into_failure(syntax.string_table_mut()))?;
        // `prepare_source` already returns the premerge lane, so propagate directly without
        // building an intermediate vessel.
        let prepared_output = syntax.prepare_source(input, source_spans)?;
        for dependency in &prepared_output.file_dependency_clauses {
            let provider = &dependency.dependency;
            let action = {
                let (syntax_string_table, _selected_source_texts, syntax_path_fork) =
                    syntax.source_preparation_inputs_and_path_fork_mut();
                resolve_structural_provider_reference(
                    provider,
                    dependency.binding.clause_kind(),
                    &current_file_path,
                    project_path_resolver,
                    syntax_path_fork,
                    &mut isolated_external_imports,
                    directory_dependency_resolution,
                    syntax_string_table,
                )
            };
            let action = match action {
                Ok(action) => action,
                Err(error) => return Err(error.into_failure(syntax.string_table_mut())),
            };
            if matches!(&action, StructuralProviderAction::Handled) {
                continue;
            }

            let resolved = resolve_directory_dependency_path(
                directory_dependency_resolution,
                provider,
                &current_file_path,
                syntax.string_table_mut(),
            )?;
            match resolved {
                ResolvedDependency::SameModuleSource {
                    source_index: target_source_index,
                    consumer_module_id,
                    ..
                } => {
                    if consumer_module_id != owner_module_id {
                        return Err(graph_inventory_mismatch_error(
                            "Check-only same-module dependency resolved to another module"
                                .to_owned(),
                        ));
                    }
                    add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
                    if queued_module_sources.insert(target_source_index) {
                        pending_module_sources.push_back(target_source_index);
                    }
                }
                ResolvedDependency::CrossModule {
                    provider_module_id,
                    consumer_module_id,
                    ..
                } => {
                    if consumer_module_id != owner_module_id {
                        return Err(graph_inventory_mismatch_error("Check-only cross-module dependency resolved to another consumer module"
                            .to_owned()));
                    }
                    add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
                    provider_bindings.push(CheckOnlyProviderBinding {
                        dependency_shell_id: provider.dependency_shell_id,
                        provider_module_id,
                    });
                }
                ResolvedDependency::SourcePackageSurface {
                    consumer_module_id,
                    dependency_prefix,
                    ..
                } => {
                    if consumer_module_id != owner_module_id {
                        return Err(graph_inventory_mismatch_error("Check-only source-package dependency resolved to another consumer module"
                            .to_owned()));
                    }
                    add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
                    source_package_dependencies.push(CheckOnlySourcePackageDependency {
                        dependency_shell_id: provider.dependency_shell_id,
                        dependency_prefix,
                    });
                }
                ResolvedDependency::BindingPackage => {
                    add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
                }
            }
        }

        let mut discovered_content_sources = Vec::new();
        for file_reference in prepared_output.structural_file_references.iter() {
            let resolved = syntax.resolve_file_reference(
                &mut file_reference_resolver,
                owner_module_id,
                prepared_output.path_syntax.table(),
                file_reference,
                &mut discovered_content_sources,
            )?;
            syntax.record_resolved_file_reference(resolved)?;
        }
        discovered_content_sources.sort_unstable();
        discovered_content_sources.dedup();
        for target_source_index in discovered_content_sources {
            if prepared_content_sources.insert(target_source_index) {
                pending_content_sources.insert(target_source_index);
            }
        }
        syntax.retain_prepared_output(current_order, prepared_output)?;
    }

    // A `.mtf` or `.md` file-value target contributes its synthetic `content` declaration to this
    // check-only source's own prepared header set. It is not a Moth root and therefore never gets a
    // separate graph/check-only job. Process nested content references in SourceId order so a
    // content source can itself depend on another content source without a second traversal.
    while let Some(target_source_index) = pending_content_sources.pop_first() {
        let target = source_tree_index.source(target_source_index);
        let target_kind = match target.classification() {
            SourceClassification::CompilerSemantic(kind) => *kind,
            _ => {
                return Err(graph_inventory_mismatch_error(format!(
                    "ModuleId {} content target source ID {} is not compiler semantic",
                    owner_module_id.index(),
                    target_source_index.index()
                )));
            }
        };
        if target_kind == SourceFileKind::Moth {
            // `.moth` targets are classified as source-kind/no-value references, not content
            // sources. Keep this defensive arm from manufacturing another transient Moth root if
            // a future classifier ever hands one to the content lane.
            continue;
        }
        if !matches!(
            target.ownership(),
            SourceOwnership::Owned(owner) if owner == owner_module_id
        ) {
            return Err(graph_inventory_mismatch_error(format!(
                "ModuleId {} content target source ID {} is not owned by its consumer",
                owner_module_id.index(),
                target_source_index.index()
            )));
        }
        let target_order = source_order.get(&target_source_index).copied().ok_or_else(|| {
            graph_inventory_mismatch_error(format!(
                "ModuleId {} content target source ID {} is absent from its candidate source set",
                owner_module_id.index(),
                target_source_index.index()
            ))
        })?;
        let target_input_result = {
            let (syntax_string_table, selected_source_texts, syntax_path_fork) =
                syntax.source_preparation_inputs_and_path_fork_mut();
            prepare_owned_source_input(
                target_source_index,
                source_tree_index,
                preparation_context.source_files,
                source_spans,
                style_directives,
                syntax_string_table,
                syntax_path_fork,
                selected_source_texts,
            )
        };
        let target_input =
            target_input_result.map_err(|error| error.into_failure(syntax.string_table_mut()))?;
        // Already in the premerge lane; propagate without an intermediate vessel.
        let target_output = syntax.prepare_source(target_input, source_spans)?;
        let mut nested_content_sources = Vec::new();
        for file_reference in target_output.structural_file_references.iter() {
            let resolved = syntax.resolve_file_reference(
                &mut file_reference_resolver,
                owner_module_id,
                target_output.path_syntax.table(),
                file_reference,
                &mut nested_content_sources,
            )?;
            syntax.record_resolved_file_reference(resolved)?;
        }
        nested_content_sources.sort_unstable();
        nested_content_sources.dedup();
        for nested_source_index in nested_content_sources {
            if prepared_content_sources.insert(nested_source_index) {
                pending_content_sources.insert(nested_source_index);
            }
        }
        syntax.retain_prepared_output(target_order, target_output)?;
    }

    // `finish` already returns the premerge lane; propagate directly. The final boundary
    // owns the single vessel conversion.
    let prepared = syntax.finish()?;
    Ok(CheckOnlyModuleCompilationJob {
        owner_module_id,
        string_table_base_len,
        path_base_len,
        provider_bindings,
        source_package_dependencies,
        external_packages: Arc::new(isolated_external_packages),
        external_dependency_resolution_table: isolated_resolution_table,
        prepared,
    })
}

/// Insert resolved dependency edges directly by `ModuleId` into the project module graph.
///
/// WHAT: the namespace already resolved each edge to boundary-local `ModuleId` pairs, so this
///       function inserts provider-before-consumer edges with authored spans without a
///       path-to-ID mapping step. Edges are sorted by (provider, consumer) `ModuleId` pair before
///       insertion so the retained span and insertion order are deterministic and independent
///       of Rayon completion order.
/// WHY: the graph owns edge adjacency and the retained span side table, while the namespace
///      resolves structural references to `ModuleId`s before they reach this insertion boundary.
fn insert_resolved_dependency_edges(
    project_module_graph: &mut ProjectModuleGraph,
    resolved_edges: &[ResolvedDependencyEdge],
) -> Result<(), PremergeFailure> {
    if resolved_edges.is_empty() {
        return Ok(());
    }

    let mut ordered_edges = resolved_edges.to_vec();
    ordered_edges.sort_by(|left, right| {
        left.provider_module_id
            .index()
            .cmp(&right.provider_module_id.index())
            .then_with(|| {
                left.consumer_module_id
                    .index()
                    .cmp(&right.consumer_module_id.index())
            })
    });

    for edge in ordered_edges {
        project_module_graph.add_resolved_dependency_edge(
            edge.provider_module_id,
            edge.consumer_module_id,
            edge.graph_span,
        )?;
    }

    Ok(())
}

/// Build the internal `CompilerError` for a disagreement between the project module graph's
/// normal entry set and the discovered module inventories.
///
/// Reaching this helper means the graph and discovery disagree on which normal entry roots exist,
/// which is a proven invariant violation rather than a user-facing failure.
fn graph_inventory_mismatch_error(reason: String) -> PremergeFailure {
    PremergeFailure::Infrastructure(CompilerError::compiler_error(reason))
}

/// Group module-job drafts by the populated graph's compile waves and attach each stable origin.
///
/// WHAT: iterates the graph's compile waves and groups every discovered job so providers precede
///       consumers. Drafts are keyed by their graph-assigned `ModuleId`, so the grouping matches
///       by identity rather than re-deriving identity from a root path. Each lifted
///       `ModuleCompilationJob` carries the exact `StableModuleOriginIdentity` the graph assigned
///       to that module.
/// WHY: discovery seeds entries in `ModuleId` order and resolves direct dependency edges. The
///      dependency-ordered wave order is known after those edges enter the graph. The graph and
///      discovery must agree exactly on the graph node set: every graph node needs one matching
///      discovered draft and vice versa. Duplicate jobs, missing graph entries and leftover
///      inventories are all internal invariant failures in the premerge lane. A graph cycle is
///      the same kind of internal failure reported by `compile_waves`.
fn order_discovered_modules_by_compile_waves(
    project_module_graph: &ProjectModuleGraph,
    drafts: Vec<ModuleCompilationJobDraft>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    source_package_dependencies: Vec<ResolvedSourcePackageDependency>,
    check_only_specs: Vec<CheckOnlyModuleSpec>,
    source_module_origins: Arc<SourceModuleOriginTable>,
) -> Result<ModuleCompilationSchedule, PremergeFailure> {
    let waves = project_module_graph.compile_waves()?;

    // Index discovered drafts by their graph-assigned `ModuleId`. A duplicate `ModuleId` means
    // two inventories claim the same graph node, which breaks the one-to-one correspondence the
    // wave order depends on; report it as an internal failure instead of silently dropping one.
    let mut draft_by_module_id: FxHashMap<ModuleId, ModuleCompilationJobDraft> =
        FxHashMap::default();
    for draft in drafts {
        let module_id = draft.module_id;
        if draft_by_module_id.insert(module_id, draft).is_some() {
            return Err(graph_inventory_mismatch_error(format!(
                "Module discovery produced duplicate inventories for ModuleId {}; the project module graph expects one discovered job per canonical module",
                module_id.index()
            )));
        }
    }

    // Group every canonical module job by compile wave. Each wave preserves deterministic
    // `ModuleId` order from the graph.
    let mut grouped_waves = Vec::new();
    for wave in &waves {
        let mut wave_modules = Vec::new();
        for module_id in wave {
            let draft = match draft_by_module_id.remove(module_id) {
                Some(draft) => draft,
                None => {
                    return Err(graph_inventory_mismatch_error(format!(
                        "The project module graph lists ModuleId {} that has no matching discovered module job",
                        module_id.index()
                    )));
                }
            };
            #[cfg(test)]
            let stable_origin = project_module_graph
                .node(*module_id)
                .stable_origin()
                .clone();
            wave_modules.push(ModuleCompilationJob {
                module_id: *module_id,
                #[cfg(test)]
                stable_origin,
                string_table_base_len: draft.string_table_base_len,
                path_base_len: draft.path_base_len,
                prepared: draft.prepared,
                #[cfg(feature = "timers")]
                timing_module_key: draft.timing_module_key,
            });
        }
        if !wave_modules.is_empty() {
            grouped_waves.push(wave_modules);
        }
    }

    // Any remaining inventory has no graph node.
    if let Some(leftover) = draft_by_module_id.keys().next() {
        return Err(graph_inventory_mismatch_error(format!(
            "Module discovery returned a job for ModuleId {} that has no project module graph node",
            leftover.index()
        )));
    }
    Ok(ModuleCompilationSchedule {
        waves: grouped_waves,
        provider_bindings,
        source_package_dependencies,
        source_module_origins,
        check_only_jobs: Vec::new(),
        check_only_specs,
    })
}

/// Prepare every graph module through the canonical header-owned Stage 0 path.
///
/// Each owned compiler-semantic source ID enters the module's input lane: ordinary `.moth` sources
/// are tokenized during provider-independent preparation, while templates and Markdown retain raw
/// source until reachable frontend preparation. The retained header dependency shells then drive
/// indexed reachability and provider resolution. The loop keeps module scheduling, reachability and
/// provider resolution serial because provider discovery mutates build-scoped registries;
/// sufficiently large owned-source sets may overlap only candidate reads and ordinary `.moth`
/// tokenization before that serial BFS, while semantic module compilation remains serial.
fn discover_modules_serial_provider_capable(
    seeds: &[ModuleEntrySeed],
    context: ModuleDiscoveryContext<'_, '_>,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    resource_inputs: &mut ResourceInputRegistry,
    include_check_only: bool,
    string_table: &mut StringTable,
    path_interner: &mut PathInternerBuilder,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<ModuleCompilationJobBatch, PremergeFailure> {
    let ModuleDiscoveryContext {
        project_path_resolver,
        style_directives,
        source_spans,
        selected_source_texts,
        directory_dependency_resolution,
        project_module_graph,
        source_module_origins,
    } = context;
    let source_files = source_spans.sources();
    let mut drafts = Vec::with_capacity(seeds.len());
    let mut resolved_edges = Vec::new();
    let mut source_package_dependencies = Vec::new();
    let mut check_only_specs = Vec::new();
    let fork_source = string_table.fork_source();
    let path_fork_source = path_interner.fork_source();
    let preparation_context = ModulePreparationContext {
        source_files,
        style_directives,
        project_path_resolver: Some(project_path_resolver.clone()),
    };
    let source_tree_index = directory_dependency_resolution.source_tree_index();
    for seed in seeds {
        let candidate_source_indices = source_tree_index
            .owned_source_indices(seed.module_id)
            .iter()
            .copied()
            .filter(|source_index| {
                matches!(
                    source_tree_index.source(*source_index).classification(),
                    SourceClassification::CompilerSemantic(_)
                )
            })
            .collect::<Vec<_>>();
        let candidate_source_ids = compiler_source_ids_for_indices(
            &candidate_source_indices,
            source_tree_index,
            source_files,
        )?;
        let source_order = candidate_source_indices
            .iter()
            .enumerate()
            .map(|(order, source_index)| (*source_index, order))
            .collect::<FxHashMap<_, _>>();
        let entry_source_index = source_tree_index
            .source_index_for_canonical_path(&seed.entry_path)
            .ok_or_else(|| {
                graph_inventory_mismatch_error(format!(
                    "ModuleId {} root is absent from the source index",
                    seed.module_id.index()
                ))
            })?;

        let stable_origin = project_module_graph
            .node(seed.module_id)
            .stable_origin()
            .clone();
        #[cfg(feature = "timers")]
        let timing_logical_module_path = stable_origin.logical_module_path().to_owned();
        #[cfg(feature = "timers")]
        let timing_module_key = crate::timing::register_timing_module_for_preparation(
            timing_boundary,
            seed.module_id.index() as u32,
            &timing_logical_module_path,
        );
        #[cfg(feature = "timers")]
        let timing_context = Some(crate::timing::TimingContext::for_module(timing_module_key));

        let fork = fork_source.fork_for_module();
        let (local_string_table, string_table_base_len) = fork.into_parts();
        let path_fork = path_fork_source.fork_for_module();
        let mut syntax = preparation_context.begin_syntax_discovery(
            stable_origin.clone(),
            RegisteredModuleSources {
                candidate_source_ids: candidate_source_ids.clone(),
                source_module_origins: Arc::clone(&source_module_origins),
            },
            &seed.entry_path,
            None,
            local_string_table,
            path_fork,
            selected_source_texts,
            #[cfg(feature = "timers")]
            timing_context,
        )?;

        let mut queued = BTreeSet::new();
        let mut queue = VecDeque::from([entry_source_index]);
        queued.insert(entry_source_index);
        let mut file_reference_resolver =
            FileReferenceResolver::new(source_tree_index, resource_inputs);
        while let Some(source_index) = queue.pop_front() {
            let order = source_order.get(&source_index).copied().ok_or_else(|| {
                graph_inventory_mismatch_error(format!(
                    "ModuleId {} reached source ID {} outside its owned source set",
                    seed.module_id.index(),
                    source_index.index()
                ))
            })?;
            if !matches!(
                source_tree_index.source(source_index).ownership(),
                SourceOwnership::Owned(owner) if owner == seed.module_id
            ) {
                return Err(graph_inventory_mismatch_error(format!(
                    "ModuleId {} reached source ID {} without owning it in SourceTreeIndex",
                    seed.module_id.index(),
                    source_index.index()
                )));
            }
            let source_path = source_tree_index
                .source(source_index)
                .canonical_path()
                .to_path_buf();
            let input_result = crate::timed_stage_attributed!(
                crate::timing::TimingMetric::FrontendPrepare,
                timing_context,
                {
                    let (syntax_string_table, selected_source_texts, syntax_path_fork) =
                        syntax.source_preparation_inputs_and_path_fork_mut();
                    prepare_owned_source_input(
                        source_index,
                        source_tree_index,
                        source_files,
                        source_spans,
                        style_directives,
                        syntax_string_table,
                        syntax_path_fork,
                        selected_source_texts,
                    )
                },
            );
            let input = match input_result {
                Ok(input) => input,
                Err(error) => return Err(error.into_failure(syntax.string_table_mut())),
            };
            // Already in the premerge lane; propagate without an intermediate vessel.
            let prepared_output = syntax.prepare_source(input, source_spans)?;
            for dependency in &prepared_output.file_dependency_clauses {
                let provider = &dependency.dependency;
                let action = {
                    let (syntax_string_table, _selected_source_texts, syntax_path_fork) =
                        syntax.source_preparation_inputs_and_path_fork_mut();
                    resolve_structural_provider_reference(
                        provider,
                        dependency.binding.clause_kind(),
                        &source_path,
                        project_path_resolver,
                        syntax_path_fork,
                        external_imports,
                        directory_dependency_resolution,
                        syntax_string_table,
                    )
                };
                let action = match action {
                    Ok(action) => action,
                    Err(error) => return Err(error.into_failure(syntax.string_table_mut())),
                };
                if matches!(&action, StructuralProviderAction::Handled) {
                    continue;
                }

                let resolved = resolve_directory_dependency_path(
                    directory_dependency_resolution,
                    provider,
                    &source_path,
                    syntax.string_table_mut(),
                )?;
                match resolved {
                    ResolvedDependency::SameModuleSource {
                        source_index: target_source_index,
                        consumer_module_id,
                        ..
                    } => {
                        add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
                        if consumer_module_id != seed.module_id {
                            return Err(graph_inventory_mismatch_error(
                                "Same-module dependency resolved to another module".to_owned(),
                            ));
                        }
                        let inserted = queued.insert(target_source_index);
                        if inserted {
                            queue.push_back(target_source_index);
                        }
                    }
                    ResolvedDependency::CrossModule {
                        provider_module_id,
                        consumer_module_id,
                        ..
                    } => {
                        add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
                        resolved_edges.push(ResolvedDependencyEdge {
                            provider_module_id,
                            consumer_module_id,
                            dependency_shell_id: provider.dependency_shell_id,
                            graph_span: Some(provider.span),
                        });
                    }
                    ResolvedDependency::SourcePackageSurface {
                        consumer_module_id,
                        dependency_prefix,
                        ..
                    } => {
                        add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
                        source_package_dependencies.push(ResolvedSourcePackageDependency {
                            consumer_module_id,
                            dependency_prefix,
                            dependency_shell_id: provider.dependency_shell_id,
                        });
                    }
                    ResolvedDependency::BindingPackage => {
                        add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
                    }
                }
            }

            // File-value paths are graph-active independently of dependency clauses. The focused
            // resolver owns module-root-relative physical validation and records resource inputs;
            // resolved occurrence table.
            let mut discovered_content_sources = Vec::<SourceRecordIndex>::new();
            for file_reference in prepared_output.structural_file_references.iter() {
                // Infrastructure failures propagate typed without an intermediate vessel.
                let resolved = syntax.resolve_file_reference(
                    &mut file_reference_resolver,
                    seed.module_id,
                    prepared_output.path_syntax.table(),
                    file_reference,
                    &mut discovered_content_sources,
                )?;
                syntax.record_resolved_file_reference(resolved)?;
            }
            discovered_content_sources.sort_unstable();
            discovered_content_sources.dedup();
            for target_source_index in discovered_content_sources {
                if queued.insert(target_source_index) {
                    queue.push_back(target_source_index);
                }
            }

            syntax.retain_prepared_output(order, prepared_output)?;
        }
        let check_only_source_indices = if include_check_only {
            classify_check_only_source_indices(
                seed.module_id,
                &candidate_source_indices,
                &queued,
                source_tree_index,
            )
        } else {
            Vec::new()
        };
        // `finish` already returns the premerge lane; propagate directly. The final boundary
        // owns the single vessel conversion.
        let prepared = syntax.finish()?;
        add_frontend_counter(FrontendCounter::ModuleCount, 1);
        add_frontend_counter(
            FrontendCounter::SourceFileCount,
            prepared.semantic.source_file_count,
        );
        add_frontend_counter(
            FrontendCounter::SourceByteCount,
            prepared.semantic.source_byte_count,
        );
        #[cfg(feature = "timers")]
        crate::timing::finalize_timing_module_source_facts(
            timing_module_key,
            prepared.semantic.source_file_count as u64,
            prepared.semantic.source_byte_count as u64,
        );
        for check_only_source_index in check_only_source_indices {
            check_only_specs.push(CheckOnlyModuleSpec {
                owner_module_id: seed.module_id,
                source_index: check_only_source_index,
                candidate_source_indices: candidate_source_indices.clone(),
                stable_origin: stable_origin.clone(),
            });
        }
        let path_base_len = prepared.semantic.path_fork.base_len();
        drafts.push(ModuleCompilationJobDraft {
            module_id: seed.module_id,
            string_table_base_len,
            path_base_len,
            prepared,
            #[cfg(feature = "timers")]
            timing_module_key,
        });
    }

    Ok(ModuleCompilationJobBatch {
        drafts,
        resolved_edges,
        source_package_dependencies,
        check_only_specs,
    })
}
