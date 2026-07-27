//! Directory-project module inventory assembly.
//!
//! WHAT: turns every canonical project module graph node into a `DiscoveredModule`
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

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use std::path::PathBuf;

use super::module_identity::ModuleId;
use super::module_namespace::DirectoryImportResolution;
use super::prepared_source::PreparedSourceInput;
use super::prepared_source_store::PreparedSourceStore;
use super::project_module_graph::ProjectModuleGraph;
use super::project_structure_diagnostics::{config_diagnostic_messages, path_id};
use super::reachable_file_discovery::{
    CollectedReachableInputs, ExternalImportDiscoveryState, ProviderFreeDiscoveryFailed,
    ProviderFreeProjectInventory, ResolvedDependencyEdge, assemble_input_files_from_inventory,
    classify_provider_free_project, collect_reachable_input_files,
    discover_reachable_source_files_provider_free,
};

/// Minimum number of entry modules before the provider-free path uses Rayon.
///
/// WHY: a single module pays the fork/merge overhead without any cross-module work to overlap,
///      so it stays serial. Multi-entry directory builds are the case the parallel path targets.
const PROVIDER_FREE_PARALLEL_MIN_MODULES: usize = 2;

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
/// is lifted to the consumer-facing [`DiscoveredModule`].
struct DiscoveredModuleDraft {
    module_id: ModuleId,
    entry_point: PathBuf,
    input_files: Vec<PreparedSourceInput>,
}

/// Entry point, graph-owned stable origin and all collected source files for one discovered
/// module.
///
/// Carries both graph identities through the build-owned scheduling boundary. `ModuleId` remains
/// the dense project-local job and merge-order key; `StableModuleOriginIdentity` is the portable
/// semantic identity consumed by the frontend and public interface.
pub(crate) struct DiscoveredModule {
    /// Dense project-local identity used only by build-system scheduling and result storage.
    pub(crate) module_id: ModuleId,
    /// The graph-assigned cross-build origin identity for this canonical module.
    pub(crate) stable_origin: StableModuleOriginIdentity,
    pub(crate) entry_point: PathBuf,
    pub(crate) input_files: Vec<PreparedSourceInput>,
}

/// Normal entry modules grouped by the populated graph's compile waves.
///
/// WHAT: owns the wave-preserving data contract between module inventory and directory
///       semantic compilation. Each wave holds every canonical module job in one retained graph
///       dependency wave, preserving the populated graph's dependency ordering and deterministic
///       `ModuleId` order. This temporary discovered-job inventory feeds the provider-store
///       scheduler; completed waves publish immutable interfaces before later consumers bind.
/// WHY: preserving wave boundaries lets the directory compiler execute semantic compilation one
///      dependency wave at a time, with Rayon parallelism only within a ready wave. The graph
///      owns compile-wave order; this contract is the single owner of that order at the inventory
///      boundary so the compiler does not recompute waves or flatten them back into one batch.
pub(crate) struct ModuleEntryCompileWaves {
    waves: Vec<Vec<DiscoveredModule>>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
}

impl ModuleEntryCompileWaves {
    /// Read-only access to the compile waves in deterministic graph order.
    ///
    /// Each wave is non-empty and contains every canonical module assigned to that graph wave.
    #[cfg(test)]
    pub(crate) fn waves(&self) -> &[Vec<DiscoveredModule>] {
        &self.waves
    }

    pub(crate) fn into_parts(self) -> (Vec<Vec<DiscoveredModule>>, Vec<ResolvedDependencyEdge>) {
        (self.waves, self.provider_bindings)
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
pub(crate) fn discover_all_modules_in_project(
    config: &Config,
    project_path_resolver: &ProjectPathResolver,
    project_module_graph: &mut ProjectModuleGraph,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_import_resolution: DirectoryImportResolution<'_>,
    string_table: &mut StringTable,
) -> Result<ModuleEntryCompileWaves, CompilerMessages> {
    let seeds = module_seeds_in_module_id_order(project_module_graph);

    if project_module_graph.entry_modules().is_empty() {
        return Err(config_diagnostic_messages(
            config,
            "entry_root",
            InvalidConfigReason::NoRootModuleEntries {
                entry_root: path_id(project_path_resolver.entry_root(), string_table),
            },
            string_table,
        ));
    }

    // The shared project-boundary prepared-source store. Every reachable `.moth` source is
    // read, tokenized and stored at most once across all entry traversals. The store replaces
    // the per-entry path-keyed `ScannedImportSource` caches.
    let mut prepared_source_store = PreparedSourceStore::new(
        directory_import_resolution
            .project_source_tree_index()
            .source_count(),
    );

    // Conservative gate: only take the provider-free parallel path when the entire reachable
    // import graph contains no provider-backed imports and no unsupported non-Moth
    // extensions. This keeps provider cache/resolution table mutations on the serial path.
    // WHY: classification populates the shared `PreparedSourceStore`. It records
    //      `provider_capable_required` and skips the external edge when a provider-backed or
    //      unsupported package import needs the serial owner, but it never aborts and discards
    //      the store. The serial fallback reuses the already-prepared store so the lexer never
    //      runs twice for the same source. Skip classification for the common single-entry case
    //      because that path stays serial provider-capable anyway.
    let provider_free_inventory = if seeds.len() >= PROVIDER_FREE_PARALLEL_MIN_MODULES {
        let entry_paths: Vec<PathBuf> = seeds.iter().map(|seed| seed.entry_path.clone()).collect();
        classify_provider_free_project(
            &entry_paths,
            project_path_resolver,
            style_directives,
            &*external_imports.external_packages,
            &mut prepared_source_store,
            Some(directory_import_resolution),
            string_table,
        )
        .map_err(|error| error.into_messages(string_table))?
    } else {
        ProviderFreeProjectInventory::provider_capable_required()
    };

    let (drafts, resolved_edges) = if !provider_free_inventory.provider_capable_required {
        match discover_modules_provider_free_parallel(
            &seeds,
            project_path_resolver,
            style_directives,
            &*external_imports.external_packages,
            &mut prepared_source_store,
            directory_import_resolution,
            string_table,
        ) {
            Ok(outcome) => outcome,
            Err(ProviderFreeDiscoveryFailed) => {
                // Worker-local diagnostics cannot be rendered on the parent string table. Retry on
                // the serial provider-capable path so the existing Stage 0 diagnostic owner reports
                // the real filesystem/import failure with stable path identity. The store retains
                // every already-scanned `.moth` so the lexer never runs twice for the same source.
                discover_modules_serial_provider_capable(
                    &seeds,
                    project_path_resolver,
                    style_directives,
                    external_imports,
                    &mut prepared_source_store,
                    directory_import_resolution,
                    string_table,
                )?
            }
        }
    } else {
        discover_modules_serial_provider_capable(
            &seeds,
            project_path_resolver,
            style_directives,
            external_imports,
            &mut prepared_source_store,
            directory_import_resolution,
            string_table,
        )?
    };

    // Insert the resolved dependency edges directly by ModuleId before the graph completes.
    // Edges are idempotent, so duplicate observations across entry closures collapse without
    // changing the graph.
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
                edge.provider.path_location,
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

/// Group discovered module drafts by the populated graph's compile waves and lift each draft to
/// a `DiscoveredModule` carrying its graph-owned stable origin.
///
/// WHAT: iterates the graph's compile waves and groups every discovered job so providers precede
///       consumers. Drafts are keyed by their
///       graph-assigned `ModuleId`, so the grouping matches by identity rather than re-deriving
///       identity from a root path. Each lifted `DiscoveredModule` carries the exact
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
    drafts: Vec<DiscoveredModuleDraft>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    string_table: &mut StringTable,
) -> Result<ModuleEntryCompileWaves, CompilerMessages> {
    let waves = project_module_graph
        .compile_waves()
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // Index discovered drafts by their graph-assigned `ModuleId`. A duplicate `ModuleId` means
    // two inventories claim the same graph node, which breaks the one-to-one correspondence the
    // wave order depends on; report it as an internal failure instead of silently dropping one.
    let mut draft_by_module_id: FxHashMap<ModuleId, DiscoveredModuleDraft> = FxHashMap::default();
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
            let stable_origin = project_module_graph
                .node(*module_id)
                .stable_origin()
                .clone();
            wave_modules.push(DiscoveredModule {
                module_id: *module_id,
                stable_origin,
                entry_point: draft.entry_point,
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

    Ok(ModuleEntryCompileWaves {
        waves: grouped_waves,
        provider_bindings,
    })
}

/// Serial provider-capable fallback.
///
/// WHAT: the original Stage 0 module loop. It mutates `ExternalImportDiscoveryState` and the
///       shared `StringTable`, so it is kept serial and is used whenever the project is not
///       proven provider-free. It also retains direct dependency edges observed during each
///       entry's traversal so the graph can record provider-before-consumer order.
fn discover_modules_serial_provider_capable(
    seeds: &[ModuleEntrySeed],
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    prepared_source_store: &mut PreparedSourceStore,
    directory_import_resolution: DirectoryImportResolution<'_>,
    string_table: &mut StringTable,
) -> Result<(Vec<DiscoveredModuleDraft>, Vec<ResolvedDependencyEdge>), CompilerMessages> {
    let mut drafts = Vec::with_capacity(seeds.len());
    let mut resolved_edges = Vec::new();

    for seed in seeds {
        let CollectedReachableInputs {
            input_files,
            resolved_edges: entry_edges,
        } = collect_reachable_input_files(
            &seed.entry_path,
            project_path_resolver,
            style_directives,
            external_imports,
            Some(prepared_source_store),
            Some(directory_import_resolution),
            string_table,
        )?;

        drafts.push(DiscoveredModuleDraft {
            module_id: seed.module_id,
            entry_point: seed.entry_path.clone(),
            input_files,
        });
        resolved_edges.extend(entry_edges);
    }

    Ok((drafts, resolved_edges))
}

/// Parallel provider-free module discovery.
///
/// WHAT: discovers each module's reachable files in a separate Rayon worker using a worker-local
///       `StringTable`; the shared `StringTable` is only used again when assembling
///       `PreparedSourceInput` values on the main thread. Workers also return direct dependency
///       edges whose boundary-local IDs and parent-valid source locations need no remapping.
/// WHY: provider-free BFS is embarrassingly parallel across entry points and does not need the
///      mutable provider state that makes provider-capable discovery serial.
fn discover_modules_provider_free_parallel(
    seeds: &[ModuleEntrySeed],
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_packages: &crate::compiler_frontend::external_packages::ExternalPackageRegistry,
    prepared_source_store: &mut PreparedSourceStore,
    directory_import_resolution: DirectoryImportResolution<'_>,
    string_table: &mut StringTable,
) -> Result<(Vec<DiscoveredModuleDraft>, Vec<ResolvedDependencyEdge>), ProviderFreeDiscoveryFailed>
{
    // Phase 1: Run provider-free BFS for each entry seed in parallel. Each worker forks a local
    // `StringTable` from the parent so classification's retained tokens (parent-valid StringIds)
    // stay interpretable without re-tokenizing. Workers read from the shared `PreparedSourceStore`
    // (populated by classification) without mutating it.
    let store_ref: &PreparedSourceStore = &*prepared_source_store;
    let fork_source = string_table.fork_source();
    let mut indexed_outcomes: Vec<(
        usize,
        super::reachable_file_discovery::ReachableSourceInventory,
        Vec<ResolvedDependencyEdge>,
    )> = seeds
        .par_iter()
        .enumerate()
        .map(|(index, seed)| {
            let mut local_string_table = fork_source.fork_for_module().into_parts().0;
            let discovery = discover_reachable_source_files_provider_free(
                &seed.entry_path,
                project_path_resolver,
                style_directives,
                external_packages,
                store_ref,
                directory_import_resolution,
                &mut local_string_table,
            )
            .map_err(|_| ProviderFreeDiscoveryFailed)?;

            Ok((index, discovery.inventory, discovery.resolved_edges))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Deterministic ordering: seeds are created in graph-assigned ModuleId order, so restoring
    // their original indexes restores ModuleId order regardless of worker completion order.
    indexed_outcomes.sort_by_key(|(index, _, _)| *index);

    // Phase 2: Assemble `PreparedSourceInput` values serially on the main thread from the shared
    // store. The immutable worker borrows have ended, so the store can be mutably reborrowed for
    // lazy `.mtf`/`.md` preparation during assembly.
    let mut drafts = Vec::with_capacity(seeds.len());
    let mut resolved_edges = Vec::new();
    for (index, inventory, entry_edges) in indexed_outcomes {
        let input_files = assemble_input_files_from_inventory(
            inventory,
            Some(prepared_source_store),
            Some(directory_import_resolution),
            string_table,
        )
        .map_err(|_| ProviderFreeDiscoveryFailed)?;
        let seed = &seeds[index];
        drafts.push(DiscoveredModuleDraft {
            module_id: seed.module_id,
            entry_point: seed.entry_path.clone(),
            input_files,
        });
        resolved_edges.extend(entry_edges);
    }

    Ok((drafts, resolved_edges))
}
