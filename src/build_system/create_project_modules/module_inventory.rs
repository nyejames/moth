//! Directory-project module inventory assembly.
//!
//! WHAT: turns every canonical project module graph node into a `ModuleCompilationJob`
//! records carrying their graph-assigned stable module origin and all transitively reachable
//! input files.
//! WHY: module inventory is the Stage 0 bridge between the structural graph and parallel frontend
//! compilation. The graph-owned `StableModuleOriginIdentity` travels with each module so semantic
//! compilation receives a canonical identity instead of reconstructing one from an entry path.
//! Entry root paths and deterministic compile-wave grouping come from the graph's compile waves so
//! entry classification has one owner; the directory compiler consumes one wave at a time,
//! permitting parallelism only within a ready wave. Root setup and source-backed package
//! validation live in sibling modules.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;

use rustc_hash::FxHashMap;

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;

use super::frontend_orchestration::ModulePreparationContext;
use super::module_identity::ModuleId;
use super::module_namespace::{DirectoryImportResolution, ResolvedImport};
use super::prepared_module::PreparedModule;
use super::prepared_source_store::PreparedSourceStore;
use super::project_module_graph::ProjectModuleGraph;
use super::project_structure_diagnostics::{config_diagnostic_messages, path_id};
use super::source_discovery::{
    ExternalImportDiscoveryState, ResolvedDependencyEdge, ResolvedSourcePackageImport,
    StructuralProviderAction, resolve_structural_provider_reference,
};
use super::source_tree_index::SourceClassification;

/// One normal entry module seed carrying its graph-assigned `ModuleId` and canonical root file.
///
/// WHAT: discovery seeds entry modules in deterministic `ModuleId` order. The `ModuleId` travels
///       through serial and parallel discovery so the deterministic compile-wave reorder can
///       match by identity rather than re-deriving identity from a root path, and so the
///       graph-owned `StableModuleOriginIdentity` is preserved for each discovered module.
/// WHY: the graph owns the canonical origin identity; discovery must not reconstruct it. Carrying
///      the dense `ModuleId` keeps the graph as the single identity owner through reorder.
struct ModuleEntrySeed {
    module_id: ModuleId,
    entry_path: PathBuf,
}

/// Discovery-internal inventory carrying the graph-assigned `ModuleId` through serial and parallel
/// discovery so the compile-wave reorder can match by identity.
///
/// The graph-owned `StableModuleOriginIdentity` is attached once, after reorder, when each draft
/// is lifted to the consumer-facing [`ModuleCompilationJob`].
struct ModuleCompilationJobDraft {
    module_id: ModuleId,
    entry_point: PathBuf,
    string_table_base_len: usize,
    prepared: PreparedModule,
    #[cfg(test)]
    input_files: Vec<super::prepared_source::PreparedSourceInput>,
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
    pub(crate) entry_point: PathBuf,
    pub(crate) string_table_base_len: usize,
    pub(crate) prepared: PreparedModule,
    #[cfg(test)]
    pub(crate) input_files: Vec<super::prepared_source::PreparedSourceInput>,
}

struct ModuleCompilationJobBatch {
    drafts: Vec<ModuleCompilationJobDraft>,
    resolved_edges: Vec<ResolvedDependencyEdge>,
    source_package_imports: Vec<ResolvedSourcePackageImport>,
}

/// Immutable Stage 0 owners shared while the serial discovery pass prepares graph modules.
struct ModuleDiscoveryContext<'a> {
    project_path_resolver: &'a ProjectPathResolver,
    style_directives: &'a StyleDirectiveRegistry,
    directory_import_resolution: DirectoryImportResolution<'a>,
    project_module_graph: &'a ProjectModuleGraph,
    source_origin_lookup: &'a FxHashMap<PathBuf, StableModuleOriginIdentity>,
}

/// Normal entry modules grouped by the populated graph's compile waves.
///
/// WHAT: owns the wave-preserving data contract between module inventory and directory
///       semantic compilation. Each wave holds every canonical module job in one retained graph
///       dependency wave, preserving the populated graph's dependency ordering and deterministic
///       `ModuleId` order. This schedule feeds the provider-store
///       scheduler; completed waves publish immutable interfaces before later consumers bind.
/// WHY: preserving wave boundaries lets the directory compiler execute semantic compilation one
///      dependency wave at a time, with Rayon parallelism only within a ready wave. The graph
///      owns compile-wave order; this contract is the single owner of that order at the inventory
///      boundary so the compiler does not recompute waves or flatten them back into one batch.
pub(crate) struct ModuleCompilationSchedule {
    waves: Vec<Vec<ModuleCompilationJob>>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    source_package_imports: Vec<ResolvedSourcePackageImport>,
}

impl ModuleCompilationSchedule {
    /// Read-only access to the compile waves in deterministic graph order.
    ///
    /// Each wave is non-empty and contains every canonical module assigned to that graph wave.
    #[cfg(test)]
    pub(crate) fn waves(&self) -> &[Vec<ModuleCompilationJob>] {
        &self.waves
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<Vec<ModuleCompilationJob>>,
        Vec<ResolvedDependencyEdge>,
        Vec<ResolvedSourcePackageImport>,
    ) {
        (
            self.waves,
            self.provider_bindings,
            self.source_package_imports,
        )
    }
}

/// Discovers one compilation job for every canonical module in the directory project.
///
/// Root files are seeded from every graph node in deterministic `ModuleId`
/// order. Reachable-file discovery retains direct `ModuleId` edges for cross-module imports;
/// after discovery completes the edges enter the graph as provider-before-consumer order, and the
/// returned modules are ordered by the populated
/// graph's compile waves. The
/// directory compiler consumes these waves sequentially, permitting Rayon parallelism only within
/// a ready wave. Only normal roots remain entry candidates, but support and facade roots now own
/// API-only semantic jobs. A defensive
/// graph cycle, a missing project-local root or a graph/inventory disagreement surfaces through
/// the existing `CompilerMessages`/string-table boundary without panicking.
#[allow(clippy::too_many_arguments)]
pub(crate) fn discover_all_modules_in_project(
    config: &Config,
    project_path_resolver: &ProjectPathResolver,
    project_module_graph: &mut ProjectModuleGraph,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_import_resolution: DirectoryImportResolution<'_>,
    string_table: &mut StringTable,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<ModuleCompilationSchedule, CompilerMessages> {
    discover_all_modules_in_boundary(
        config,
        project_path_resolver,
        project_module_graph,
        style_directives,
        external_imports,
        directory_import_resolution,
        true,
        string_table,
        #[cfg(feature = "timers")]
        timing_boundary,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn discover_all_modules_in_package(
    config: &Config,
    project_path_resolver: &ProjectPathResolver,
    package_module_graph: &mut ProjectModuleGraph,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_import_resolution: DirectoryImportResolution<'_>,
    string_table: &mut StringTable,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<ModuleCompilationSchedule, CompilerMessages> {
    discover_all_modules_in_boundary(
        config,
        project_path_resolver,
        package_module_graph,
        style_directives,
        external_imports,
        directory_import_resolution,
        false,
        string_table,
        #[cfg(feature = "timers")]
        timing_boundary,
    )
}

#[allow(clippy::too_many_arguments)]
fn discover_all_modules_in_boundary(
    config: &Config,
    project_path_resolver: &ProjectPathResolver,
    project_module_graph: &mut ProjectModuleGraph,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_import_resolution: DirectoryImportResolution<'_>,
    require_normal_entry: bool,
    string_table: &mut StringTable,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<ModuleCompilationSchedule, CompilerMessages> {
    let seeds = module_seeds_in_module_id_order(project_module_graph);
    let source_origin_lookup = project_module_graph
        .build_source_origin_lookup(directory_import_resolution.source_tree_index())
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    if require_normal_entry && project_module_graph.entry_modules().is_empty() {
        return Err(config_diagnostic_messages(
            config,
            "entry_root",
            InvalidConfigReason::NoRootModuleEntries {
                entry_root: path_id(project_path_resolver.entry_root(), string_table),
            },
            string_table,
        ));
    }

    // The shared project-boundary prepared-source store. Every selected `.moth` source is read,
    // tokenized and prepared at most once across the canonical module inventory.
    let mut prepared_source_store = PreparedSourceStore::new(
        directory_import_resolution
            .source_tree_index()
            .source_count(),
    );

    let ModuleCompilationJobBatch {
        drafts,
        resolved_edges,
        source_package_imports,
    } = discover_modules_serial_provider_capable(
        &seeds,
        ModuleDiscoveryContext {
            project_path_resolver,
            style_directives,
            directory_import_resolution,
            project_module_graph,
            source_origin_lookup: &source_origin_lookup,
        },
        external_imports,
        &mut prepared_source_store,
        string_table,
        #[cfg(feature = "timers")]
        timing_boundary,
    )?;

    // Insert the resolved dependency edges directly by ModuleId before the graph completes.
    // Edges are idempotent, so duplicate retained import shells collapse without changing the
    // graph.
    insert_resolved_dependency_edges(project_module_graph, &resolved_edges, string_table)?;

    // Freeze the graph's adjacency into sorted `Vec<ModuleId>` storage before compile waves are
    // computed. The no-edge production graph also completes here so scheduling always reads one
    // frozen adjacency. Mutation or scheduling in an invalid phase is an internal `CompilerError`.
    project_module_graph
        .complete()
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // Order the discovered modules by the completed graph's compile waves so providers precede
    // consumers in the returned inventory waves. Discovery seeded entries in `ModuleId` order;
    // this groups the result into dependency-ordered compile waves without re-running discovery.
    // Every canonical module appears in exactly one wave, and the directory compiler consumes one
    // wave at a time with parallelism only within a ready wave.
    order_discovered_modules_by_compile_waves(
        project_module_graph,
        drafts,
        resolved_edges,
        source_package_imports,
        string_table,
    )
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

/// Insert resolved dependency edges directly by `ModuleId` into the project module graph.
///
/// WHAT: the namespace already resolved each edge to boundary-local `ModuleId` pairs, so this
///       function inserts provider-before-consumer edges with authored locations without a
///       path-to-ID mapping step. Edges are sorted by (provider, consumer) `ModuleId` pair before
///       insertion so the retained location and insertion order are deterministic and independent
///       of Rayon completion order.
/// WHY: the graph owns edge adjacency and the retained location side table, while the namespace
///      resolves structural references to `ModuleId`s before they reach this insertion boundary.
fn insert_resolved_dependency_edges(
    project_module_graph: &mut ProjectModuleGraph,
    resolved_edges: &[ResolvedDependencyEdge],
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
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
        project_module_graph
            .add_resolved_dependency_edge(
                edge.provider_module_id,
                edge.consumer_module_id,
                edge.graph_location,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    }

    Ok(())
}

/// Build the internal `CompilerError` for a disagreement between the project module graph's
/// normal entry set and the discovered module inventories.
///
/// Reaching this helper means the graph and discovery disagree on which normal entry roots exist,
/// which is a proven invariant violation rather than a user-facing failure.
fn graph_inventory_mismatch_error(
    reason: String,
    string_table: &mut StringTable,
) -> CompilerMessages {
    CompilerMessages::from_error_ref(CompilerError::compiler_error(reason), string_table)
}

/// Group module-job drafts by the populated graph's compile waves and attach each stable origin.
///
/// WHAT: iterates the graph's compile waves and groups every discovered job so providers precede
///       consumers. Drafts are keyed by their
///       graph-assigned `ModuleId`, so the grouping matches by identity rather than re-deriving
///       identity from a root path. Each lifted `ModuleCompilationJob` carries the exact
///       `StableModuleOriginIdentity` the graph assigned to that module.
/// WHY: discovery seeds entries in `ModuleId` order and resolves direct dependency edges. The
///      dependency-ordered wave order is known after those edges enter the graph. The graph and
///      discovery must agree exactly on the graph node set: every
///      graph node needs one matching discovered draft and vice versa. Duplicate jobs,
///      missing graph entries and leftover inventories are all internal invariant failures
///      surfaced through the `CompilerMessages`/string-table boundary. A graph cycle is the same
///      kind of internal failure reported by `compile_waves`.
fn order_discovered_modules_by_compile_waves(
    project_module_graph: &ProjectModuleGraph,
    drafts: Vec<ModuleCompilationJobDraft>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    source_package_imports: Vec<ResolvedSourcePackageImport>,
    string_table: &mut StringTable,
) -> Result<ModuleCompilationSchedule, CompilerMessages> {
    let waves = project_module_graph
        .compile_waves()
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // Index discovered drafts by their graph-assigned `ModuleId`. A duplicate `ModuleId` means
    // two inventories claim the same graph node, which breaks the one-to-one correspondence the
    // wave order depends on; report it as an internal failure instead of silently dropping one.
    let mut draft_by_module_id: FxHashMap<ModuleId, ModuleCompilationJobDraft> =
        FxHashMap::default();
    for draft in drafts {
        let module_id = draft.module_id;
        if draft_by_module_id.insert(module_id, draft).is_some() {
            return Err(graph_inventory_mismatch_error(
                format!(
                    "Module discovery produced duplicate inventories for ModuleId {}; the project module graph expects one discovered job per canonical module",
                    module_id.index()
                ),
                string_table,
            ));
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
                    return Err(graph_inventory_mismatch_error(
                        format!(
                            "The project module graph lists ModuleId {} that has no matching discovered module job",
                            module_id.index()
                        ),
                        string_table,
                    ));
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
                entry_point: draft.entry_point,
                string_table_base_len: draft.string_table_base_len,
                prepared: draft.prepared,
                #[cfg(test)]
                input_files: draft.input_files,
            });
        }
        if !wave_modules.is_empty() {
            grouped_waves.push(wave_modules);
        }
    }

    // Any remaining inventory has no graph node.
    if let Some(leftover) = draft_by_module_id.keys().next() {
        return Err(graph_inventory_mismatch_error(
            format!(
                "Module discovery returned a job for ModuleId {} that has no project module graph node",
                leftover.index()
            ),
            string_table,
        ));
    }

    Ok(ModuleCompilationSchedule {
        waves: grouped_waves,
        provider_bindings,
        source_package_imports,
    })
}

/// Prepare every graph module through the canonical header-owned Stage 0 path.
///
/// Each source is read and tokenized through the boundary store, then its retained header import
/// shells drive indexed reachability and provider resolution. The loop stays serial because
/// provider discovery mutates build-scoped registries; semantic compilation remains wave-parallel.
fn discover_modules_serial_provider_capable(
    seeds: &[ModuleEntrySeed],
    context: ModuleDiscoveryContext<'_>,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    prepared_source_store: &mut PreparedSourceStore,
    string_table: &mut StringTable,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<ModuleCompilationJobBatch, CompilerMessages> {
    let ModuleDiscoveryContext {
        project_path_resolver,
        style_directives,
        directory_import_resolution,
        project_module_graph,
        source_origin_lookup,
    } = context;
    let mut drafts = Vec::with_capacity(seeds.len());
    let mut resolved_edges = Vec::new();
    let mut source_package_imports = Vec::new();

    for seed in seeds {
        let module_edge_start = resolved_edges.len();
        let source_tree_index = directory_import_resolution.source_tree_index();
        let candidate_source_ids = source_tree_index
            .owned_source_ids(seed.module_id)
            .iter()
            .copied()
            .filter(|source_id| {
                matches!(
                    source_tree_index.source(*source_id).classification(),
                    SourceClassification::CompilerSemantic(_)
                )
            })
            .collect::<Vec<_>>();
        let source_order = candidate_source_ids
            .iter()
            .enumerate()
            .map(|(order, source_id)| (*source_id, order))
            .collect::<FxHashMap<_, _>>();
        let entry_source_id = source_tree_index
            .source_id_for_canonical_path(&seed.entry_path)
            .ok_or_else(|| {
                graph_inventory_mismatch_error(
                    format!(
                        "ModuleId {} root is absent from the source index",
                        seed.module_id.index()
                    ),
                    string_table,
                )
            })?;

        let fork = string_table.fork_for_module();
        let (local_string_table, string_table_base_len) = fork.into_parts();
        let preparation_context = ModulePreparationContext {
            style_directives,
            project_path_resolver: Some(project_path_resolver.clone()),
        };
        let stable_origin = project_module_graph
            .node(seed.module_id)
            .stable_origin()
            .clone();
        #[cfg(feature = "timers")]
        let timing_context =
            crate::timing::TimingModuleContext::for_module(crate::timing::TimingModuleKey {
                boundary: timing_boundary,
                module_index: seed.module_id.index() as u32,
            });
        let mut syntax = preparation_context.begin_syntax_discovery(
            stable_origin,
            source_origin_lookup,
            candidate_source_ids
                .iter()
                .map(|source_id| source_tree_index.source(*source_id).canonical_path()),
            &seed.entry_path,
            local_string_table,
            #[cfg(feature = "timers")]
            timing_context,
        )?;

        let mut queued = BTreeSet::new();
        let mut queue = VecDeque::from([entry_source_id]);
        #[cfg(test)]
        let mut input_files = Vec::new();
        queued.insert(entry_source_id);
        while let Some(source_id) = queue.pop_front() {
            let input = match prepared_source_store.prepare_or_get_project_input(
                source_id,
                source_tree_index,
                style_directives,
                syntax.string_table_mut(),
            ) {
                Ok(input) => input,
                Err(error) => return Err(error.into_messages(syntax.string_table_mut())),
            };
            let order = source_order.get(&source_id).copied().ok_or_else(|| {
                graph_inventory_mismatch_error(
                    format!(
                        "ModuleId {} reached source ID {} outside its owned source set",
                        seed.module_id.index(),
                        source_id.index()
                    ),
                    syntax.string_table_mut(),
                )
            })?;
            let providers = syntax.prepare_source(order, &input)?;
            #[cfg(test)]
            input_files.push(input.clone());
            for provider in providers {
                let action = match resolve_structural_provider_reference(
                    &provider,
                    input.source_path(),
                    project_path_resolver,
                    external_imports,
                    directory_import_resolution,
                    syntax.string_table_mut(),
                ) {
                    Ok(action) => action,
                    Err(error) => return Err(error.into_messages(syntax.string_table_mut())),
                };
                if matches!(action, StructuralProviderAction::Handled) {
                    continue;
                }

                let resolved = directory_import_resolution
                    .resolve_import(
                        provider.path_view(),
                        input.source_path(),
                        syntax.string_table_mut(),
                    )
                    .map_err(|diagnostic| {
                        CompilerMessages::from_diagnostics(
                            vec![diagnostic],
                            syntax.string_table_mut().clone(),
                        )
                    })?;
                match resolved {
                    ResolvedImport::SameModuleSource {
                        source_id: target_source_id,
                        consumer_module_id,
                        ..
                    } => {
                        if consumer_module_id != seed.module_id {
                            return Err(graph_inventory_mismatch_error(
                                "Same-module import resolved to another module".to_owned(),
                                syntax.string_table_mut(),
                            ));
                        }
                        if queued.insert(target_source_id) {
                            queue.push_back(target_source_id);
                        }
                    }
                    ResolvedImport::CrossModule {
                        provider_module_id,
                        consumer_module_id,
                        ..
                    } => resolved_edges.push(ResolvedDependencyEdge {
                        provider_module_id,
                        consumer_module_id,
                        graph_location: provider.path_location.clone(),
                        provider: provider.clone(),
                    }),
                    ResolvedImport::SourcePackageSurface {
                        consumer_module_id,
                        import_prefix,
                        ..
                    } => source_package_imports.push(ResolvedSourcePackageImport {
                        consumer_module_id,
                        import_prefix,
                        provider: provider.clone(),
                    }),
                    ResolvedImport::BindingPackage => {}
                }
            }
        }
        let prepared = syntax.finish()?;
        let graph_location_remap =
            string_table.merge_delta_from(&prepared.string_table, string_table_base_len);
        for edge in &mut resolved_edges[module_edge_start..] {
            edge.graph_location.remap_string_ids(&graph_location_remap);
        }
        #[cfg(test)]
        input_files.sort_by_key(|input| {
            source_order
                .get(
                    &source_tree_index
                        .source_id_for_canonical_path(input.source_path())
                        .expect("a prepared test input must retain its indexed canonical path"),
                )
                .copied()
                .expect("a prepared test input must belong to the module source order")
        });

        drafts.push(ModuleCompilationJobDraft {
            module_id: seed.module_id,
            entry_point: seed.entry_path.clone(),
            string_table_base_len,
            prepared,
            #[cfg(test)]
            input_files,
        });
    }

    Ok(ModuleCompilationJobBatch {
        drafts,
        resolved_edges,
        source_package_imports,
    })
}
