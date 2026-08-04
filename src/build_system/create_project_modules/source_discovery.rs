//! Synthetic single-file source traversal plus shared structural-provider resolution.
//!
//! Given an entry `.moth` file, walks its import declarations transitively to build the complete
//! set of source files that belong to a module. Also assembles `PreparedSourceInput` payloads
//! from those paths for downstream compilation stages.
// Stage 0 deliberately returns full diagnostic/infrastructure payloads in `SourceDiscoveryError`
// so import discovery does not erase source locations or downgrade filesystem failures.

use crate::builder_surface::SourceFileKind;
use crate::builder_surface::external_import_providers::cache::ExternalImportCacheKey;
use crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache;
use crate::builder_surface::external_import_providers::provider::{
    ExternalImportProvider, ExternalImportProviderContext, ExternalImportRequest,
};
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::const_paths::{
    ProviderImportPathView, RetainedProviderReference, ScannedProviderReference,
};
use crate::compiler_frontend::paths::path_normalization::join_and_normalize_path;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use super::module_identity::ModuleId;
use super::module_namespace::DirectoryImportResolution;
use super::prepared_source::PreparedSourceInput;
use super::source_discovery_error::SourceDiscoveryError;
use super::source_loading::{extract_source_code, read_source_code, source_read_error};
use super::source_scanning::{ScannedImportSource, scan_imports_with_source};

/// Record a reachable-discovery stage timing through the central `timers` substrate.
///
/// WHAT: delegates to `timing::record_started_pipeline_timing`, which stores the
///      observation in the active collection scope and emits the stable
///      `MOTH_BENCH timing` line when the output mode permits.
/// WHY:  reachable-file discovery uses dotted `stage0.reachable_discovery.*` metric
///      names. The start token is zero-sized when `timers` is off, so regular builds
///      do not read clocks for instrumentation-only measurements.
fn log_stage_timing(metric: &str, start: crate::timing::PipelineTimingStart) {
    crate::timing::record_started_pipeline_timing(metric, start);
}

/// Minimum cache-miss count before Stage 0 uses Rayon for raw source loading.
///
/// The threshold keeps tiny projects and mostly-cached modules on the cheaper serial path while
/// still letting markdown-heavy modules overlap independent filesystem reads.
pub(super) const STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES: usize = 8;

/// Mutable external-import state shared across Stage 0 reachable-file discovery.
///
/// WHAT: groups provider metadata, the external package registry, and build-scoped provider
/// cache/table state.
/// WHY: Stage 0 needs to mutate provider results while walking imports, but callers should not
/// thread four closely related provider arguments through every discovery function.
pub(crate) struct ExternalImportDiscoveryState<'a> {
    pub(super) external_packages: &'a mut ExternalPackageRegistry,
    pub(super) providers: &'a ExternalImportProviderRegistry,
    pub(super) cache: &'a mut ExternalImportProviderCache,
    pub(super) resolution_table: &'a mut ExternalImportResolutionTable,
}

/// Stage 0 disposition for one retained header-owned provider reference.
pub(super) enum StructuralProviderAction {
    ResolveSource,
    Handled,
}

/// Resolve provider-backed and binding-backed import classes before indexed source resolution.
///
/// Directory module scheduling calls this with provider references retained by header syntax.
/// It never scans tokens or source text.
pub(super) fn resolve_structural_provider_reference(
    provider: &RetainedProviderReference,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_import_resolution: DirectoryImportResolution<'_>,
    string_table: &mut StringTable,
) -> Result<StructuralProviderAction, SourceDiscoveryError> {
    match handle_provider_capable_import(
        &provider.path,
        &provider.path_location,
        canonical_file,
        project_path_resolver,
        external_imports,
        Some(directory_import_resolution),
        string_table,
    )? {
        ImportPolicyAction::QueueLocal => Ok(StructuralProviderAction::ResolveSource),
        ImportPolicyAction::Skip => Ok(StructuralProviderAction::Handled),
    }
}

/// A reachable source file plus the source kind selected by import resolution.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ReachableSourceFile {
    pub(super) path: PathBuf,
    pub(super) kind: SourceFileKind,
}

/// Stage 0 inventory for synthetic single-file compilation.
///
/// WHAT: owns the deterministic source closure and the retained lexical scan for each reachable
///       `.moth` file when no directory-project `SourceId` store exists.
/// WHY: `assemble_input_files_from_inventory` can reuse each scanned source body while producing
///      `PreparedSourceInput` values. Directory projects use `PreparedSourceStore` and canonical
///      module jobs instead of this path.
pub(super) struct ReachableSourceInventory {
    pub(super) files: Vec<ReachableSourceFile>,
    local_source_cache: FxHashMap<PathBuf, ScannedImportSource>,
}

/// One resolved dependency edge ready for direct insertion into the project module graph.
///
/// WHAT: records that an authored structural provider reference resolved through the
///       boundary-aware namespace from a consumer project module to a provider project
///       module, carrying both `ModuleId` values and the exact authored import-clause
///       `SourceLocation`.
/// WHY: the namespace resolves to boundary-local `ModuleId`s directly, so the graph inserts
///      a provider-before-consumer edge without a path-to-ID mapping step. The authored
///      source location is retained in the graph side table so a later diagnostic owner can
///      attribute the edge to the exact import clause without reparsing.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedDependencyEdge {
    pub(super) provider_module_id: ModuleId,
    pub(super) consumer_module_id: ModuleId,
    pub(super) provider: RetainedProviderReference,
    pub(super) graph_location: SourceLocation,
}

/// One authored import from a module to a separately compiled source-package facade.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedSourcePackageImport {
    pub(super) consumer_module_id: ModuleId,
    pub(super) import_prefix: String,
    pub(super) provider: RetainedProviderReference,
}

/// Reachable discovery output pairing the file inventory with direct dependency edges.
///
/// WHAT: the complete retained inventory plus the project-local `ModuleId` edges observed during
///       one traversal. Both the provider-capable serial path and the provider-free
///       worker path return this so the inventory merge has one shape.
/// WHY: dependency edges are collected at the same local-import resolution join as the file
///      inventory, so they share the traversal owner and stay deterministic regardless of which
///      discovery path produced them.
pub(super) struct ReachableDiscoveryResult {
    pub(super) inventory: ReachableSourceInventory,
}

/// Collected reachable inputs for one entry plus the retained dependency edges.
///
/// WHAT: `assemble_input_files_from_inventory` turns the inventory into `PreparedSourceInput`
///       values; direct edges travel alongside so the directory-project graph can record them
///       after discovery.
/// WHY: the single-file flow produces no edges because it has no project module graph, while the
///      directory-project flow retains them for graph insertion.
pub(super) struct CollectedReachableInputs {
    pub(super) input_files: Vec<PreparedSourceInput>,
}

struct MissingSourceFile {
    input_index: usize,
    source_file: ReachableSourceFile,
}

struct LoadedMissingSourceFile {
    input_index: usize,
    source_file: ReachableSourceFile,
    source_code: String,
}

/// Mutable traversal outputs shared by the source-import queue helpers.
struct ReachableQueue<'a> {
    reachable: &'a BTreeSet<ReachableSourceFile>,
    queue: &'a mut VecDeque<ReachableSourceFile>,
}

/// Build a `PreparedSourceInput` from a cache-miss load.
///
/// WHAT: selects the Moth template or PlainMarkdown variant from the resolved source kind. Cache
///       misses are never Moth — every reachable `.moth` is scanned and cached during
///       traversal — so no Moth variant carries a raw load here.
/// WHY: keeps the strict source-kind/token relationship: a loaded file has no retained tokens
///      and cannot become a Moth `PreparedSourceInput`.
fn prepared_input_from_loaded(loaded: LoadedMissingSourceFile) -> PreparedSourceInput {
    let LoadedMissingSourceFile {
        source_file,
        source_code,
        ..
    } = loaded;
    match source_file.kind {
        SourceFileKind::MothTemplate => PreparedSourceInput::MothTemplate {
            source_code,
            source_path: source_file.path,
        },
        SourceFileKind::PlainMarkdown => PreparedSourceInput::PlainMarkdown {
            source_code,
            source_path: source_file.path,
        },
        SourceFileKind::Moth => {
            // Every reachable Moth file is scanned and cached during traversal, so a cache
            // miss can only be Moth template or PlainMarkdown. Reaching this arm is a proven
            // invariant violation rather than a user-facing failure.
            unreachable!("Stage 0 cache-miss load produced a Moth file without retained tokens")
        }
    }
}

struct SourceReadFailure {
    input_index: usize,
    path: PathBuf,
    error: std::io::Error,
}

// -------------------------
//  Public API
// -------------------------

/// Collect all reachable source files for a given entry point and load their content.
pub(super) fn collect_reachable_input_files(
    entry_path: &Path,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    string_table: &mut StringTable,
) -> Result<CollectedReachableInputs, CompilerMessages> {
    let total_start = crate::timing::start_pipeline_timing();

    // 1. Traverse the import graph to find all paths and retained resolved edges.
    let discovery = match discover_reachable_source_files(
        entry_path,
        project_path_resolver,
        style_directives,
        external_imports,
        string_table,
    ) {
        Ok(discovery) => discovery,
        Err(error) => {
            log_stage_timing("stage0.reachable_discovery.total", total_start);
            return Err(error.into_messages(string_table));
        }
    };

    let input_files = assemble_input_files_from_inventory(discovery.inventory, string_table)?;
    log_stage_timing("stage0.reachable_discovery.total", total_start);
    Ok(CollectedReachableInputs { input_files })
}

/// Assemble `PreparedSourceInput` values from a deterministic Stage 0 inventory.
///
/// WHAT: for directory projects, projects from the shared `PreparedSourceStore` (preparing
///       `.mtf`/`.md` slots lazily). For single-file synthetic compilation, uses the retained
///       local source cache and loads remaining Moth template/PlainMarkdown files through the
///       serial/parallel cache-miss path.
/// WHY: inventory assembly is the same whether discovery was provider-capable or provider-free,
///      so it is shared between both paths to keep ordering and loading policy in one place.
pub(super) fn assemble_input_files_from_inventory(
    inventory: ReachableSourceInventory,
    string_table: &mut StringTable,
) -> Result<Vec<PreparedSourceInput>, CompilerMessages> {
    let ReachableSourceInventory {
        files,
        local_source_cache,
    } = inventory;
    assemble_reachable_files(files, local_source_cache, string_table)
}

/// Assemble `PreparedSourceInput` values without a semantic set (single-file synthetic path).
fn assemble_reachable_files(
    files: Vec<ReachableSourceFile>,
    mut source_cache: FxHashMap<PathBuf, ScannedImportSource>,
    string_table: &mut StringTable,
) -> Result<Vec<PreparedSourceInput>, CompilerMessages> {
    let input_file_count = files.len();
    let mut input_slots: Vec<Option<PreparedSourceInput>> =
        (0..input_file_count).map(|_| None).collect();
    let mut missing_sources = Vec::new();

    for (input_index, source_file) in files.into_iter().enumerate() {
        fill_input_slot(
            &mut source_cache,
            &source_file.path,
            source_file.kind,
            input_index,
            &mut input_slots,
            &mut missing_sources,
        );
    }

    load_and_join_input_slots(input_slots, missing_sources, string_table)
}

/// Fill one input slot from the retained cache or queue it for disk loading.
fn fill_input_slot(
    source_cache: &mut FxHashMap<PathBuf, ScannedImportSource>,
    canonical_path: &Path,
    source_kind: SourceFileKind,
    input_index: usize,
    input_slots: &mut [Option<PreparedSourceInput>],
    missing_sources: &mut Vec<MissingSourceFile>,
) {
    if let Some(scanned_source) = source_cache.remove(canonical_path) {
        add_frontend_counter(FrontendCounter::Stage0SourceCacheHitCount, 1);

        input_slots[input_index] = Some(PreparedSourceInput::Moth {
            source_code: scanned_source.source_code,
            source_path: canonical_path.to_path_buf(),
            tokens: Box::new(scanned_source.tokens),
        });
    } else {
        add_frontend_counter(FrontendCounter::Stage0SourceCacheMissCount, 1);

        missing_sources.push(MissingSourceFile {
            input_index,
            source_file: ReachableSourceFile {
                path: canonical_path.to_path_buf(),
                kind: source_kind,
            },
        });
    }
}

/// Load missing sources from disk and join all slots into the final ordered `Vec`.
fn load_and_join_input_slots(
    input_slots: Vec<Option<PreparedSourceInput>>,
    missing_sources: Vec<MissingSourceFile>,
    string_table: &mut StringTable,
) -> Result<Vec<PreparedSourceInput>, CompilerMessages> {
    let input_file_count = input_slots.len();
    let mut input_slots = input_slots;

    let source_load_start = crate::timing::start_pipeline_timing();
    let loaded_missing_sources = match load_missing_sources(missing_sources, string_table) {
        Ok(loaded_missing_sources) => loaded_missing_sources,
        Err(messages) => {
            log_stage_timing("stage0.reachable_discovery.source_load", source_load_start);
            return Err(messages);
        }
    };
    log_stage_timing("stage0.reachable_discovery.source_load", source_load_start);
    for loaded in loaded_missing_sources {
        add_frontend_counter(
            FrontendCounter::Stage0SourceBytesLoaded,
            loaded.source_code.len(),
        );

        let input_index = loaded.input_index;
        input_slots[input_index] = Some(prepared_input_from_loaded(loaded));
    }

    let mut input_files = Vec::with_capacity(input_file_count);
    for slot in input_slots {
        let Some(input_file) = slot else {
            let error = CompilerError::compiler_error(
                "Stage 0 source inventory slot was empty after successful loading",
            );
            return Err(CompilerMessages::from_error_ref(error, string_table));
        };

        input_files.push(input_file);
    }

    Ok(input_files)
}

// -------------------------
//  Reachable Discovery
// -------------------------

/// Action a traversal policy wants the shared BFS to take for one import path.
enum ImportPolicyAction {
    /// Do not follow this import.
    Skip,
    /// Resolve and queue the import as a normal local Moth import.
    QueueLocal,
}

/// Stage 0 import policy that customizes the shared reachable-file traversal.
///
/// WHAT: the provider-capable path owns external imports while the shared BFS owns queue
///       handling, canonicalization, source preparation and local queuing.
///
/// Source preparation is owned by the [`TraversalSourceStorage`] parameter, not the policy. The
/// policy only decides import actions; the storage decides where scanned source data is retained.
enum ImportPolicy<'a, 'b> {
    /// Full provider-capable path. Mutates provider cache and resolution tables.
    Capable {
        external_imports: &'a mut ExternalImportDiscoveryState<'b>,
    },
}

impl<'a, 'b> ImportPolicy<'a, 'b> {
    /// Decide how to handle one import path.
    fn handle_import(
        &mut self,
        import_path: &InternedPath,
        import_location: &SourceLocation,
        canonical_file: &Path,
        project_path_resolver: &ProjectPathResolver,
        directory_import_resolution: Option<DirectoryImportResolution<'_>>,
        string_table: &mut StringTable,
    ) -> Result<ImportPolicyAction, SourceDiscoveryError> {
        match self {
            ImportPolicy::Capable {
                external_imports: state,
            } => handle_provider_capable_import(
                import_path,
                import_location,
                canonical_file,
                project_path_resolver,
                state,
                directory_import_resolution,
                string_table,
            ),
        }
    }
}

/// Result of scanning one `.moth` file during traversal.
struct ScannedMothSource {
    imports: Vec<ScannedProviderReference>,
    fresh_read: bool,
    source_byte_count: usize,
}

fn scan_and_cache_local_moth_source(
    canonical_file: &Path,
    style_directives: &StyleDirectiveRegistry,
    local_source_cache: &mut FxHashMap<PathBuf, ScannedImportSource>,
    string_table: &mut StringTable,
) -> Result<ScannedMothSource, SourceDiscoveryError> {
    if let Some(scanned) = local_source_cache.get(canonical_file) {
        return Ok(ScannedMothSource {
            imports: scanned.imports.clone(),
            fresh_read: false,
            source_byte_count: 0,
        });
    }

    let scanned = scan_imports_with_source(canonical_file, style_directives, string_table)?;
    let imports = scanned.imports.clone();
    let source_byte_count = scanned.source_code.len();
    local_source_cache.insert(canonical_file.to_path_buf(), scanned);

    Ok(ScannedMothSource {
        imports,
        fresh_read: true,
        source_byte_count,
    })
}

/// BFS over the synthetic single-file compilation's import declarations.
///
/// WHAT: follows each Moth file's declared imports, resolves them to canonical typed source
///       files, and returns the full ordered set of files reachable from the entry points.
/// WHY: directory projects use indexed header-owned discovery; this filesystem traversal exists
///      only for a file invoked directly as one synthetic module.
/// Outcome of the synthetic single-file traversal.
struct ReachableTraversalOutcome {
    inventory: ReachableSourceInventory,
}

#[allow(clippy::too_many_arguments)]
fn traverse_reachable_source_files(
    entry_paths: &[PathBuf],
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    policy: &mut ImportPolicy<'_, '_>,
    string_table: &mut StringTable,
) -> Result<ReachableTraversalOutcome, SourceDiscoveryError> {
    let mut reachable = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut local_source_cache = FxHashMap::default();
    let mut imports_scanned: usize = 0;

    // Seed with entry points in deterministic order.
    for entry_path in entry_paths {
        queue.push_back(ReachableSourceFile {
            path: entry_path.clone(),
            kind: SourceFileKind::Moth,
        });
    }

    while let Some(next_file) = queue.pop_front() {
        let canonical_file = fs::canonicalize(&next_file.path).map_err(|error| {
            CompilerError::file_error(
                &next_file.path,
                format!("Failed to canonicalize module file path: {error}"),
                string_table,
            )
        })?;
        let reachable_file = ReachableSourceFile {
            path: canonical_file.clone(),
            kind: next_file.kind,
        };

        if !reachable.insert(reachable_file.clone()) {
            continue;
        }

        match next_file.kind {
            SourceFileKind::MothTemplate => {
                // Moth template is a Moth template body with a small compile-time scope, so the
                // same-directory root may supply visible constants. Plain Markdown is raw
                // content and has no Moth scope; the root still re-exports it normally
                // because the root file itself is scanned as ordinary Moth source.
                queue_same_directory_root_for_moth_template(
                    &canonical_file,
                    project_path_resolver,
                    &reachable,
                    &mut queue,
                );
                continue;
            }
            SourceFileKind::PlainMarkdown => {
                // Markdown files are importless content assets. They are carried forward for
                // header-stage preparation but are never scanned for imports.
                continue;
            }
            SourceFileKind::Moth => {}
        }

        let import_scan_start = crate::timing::start_pipeline_timing();
        let scanned = match scan_and_cache_local_moth_source(
            &canonical_file,
            style_directives,
            &mut local_source_cache,
            string_table,
        ) {
            Ok(scanned) => scanned,
            Err(error) => {
                log_stage_timing("stage0.reachable_discovery.import_scan", import_scan_start);
                return Err(error);
            }
        };
        log_stage_timing("stage0.reachable_discovery.import_scan", import_scan_start);

        if scanned.fresh_read {
            add_frontend_counter(
                FrontendCounter::Stage0SourceBytesLoaded,
                scanned.source_byte_count,
            );
        }

        let import_references = scanned.imports.clone();
        imports_scanned += import_references.len();

        for provider in &import_references {
            // Stage 0 resolves reachability through the provider path today; the structural
            // reference retains `path_location` for the graph boundary alongside that path.
            let import_path = &provider.path;
            let action = policy.handle_import(
                import_path,
                &provider.path_location,
                &canonical_file,
                project_path_resolver,
                None,
                string_table,
            )?;

            match action {
                ImportPolicyAction::Skip => continue,
                ImportPolicyAction::QueueLocal => {
                    let import_resolve_start = crate::timing::start_pipeline_timing();
                    let mut reachable_queue = ReachableQueue {
                        reachable: &reachable,
                        queue: &mut queue,
                    };
                    let result = resolve_and_queue_local_import(
                        provider.path_view(),
                        &canonical_file,
                        project_path_resolver,
                        string_table,
                        &mut reachable_queue,
                    );
                    log_stage_timing(
                        "stage0.reachable_discovery.import_resolve",
                        import_resolve_start,
                    );
                    result?;
                }
            }
        }
    }

    // Record concise counters for the completed traversal. Counters are only
    // recorded when `benchmark_counters` is active, and reach stdout only when
    // `MOTH_COUNTERS` requests it (summary/full).
    crate::timing::record_counter(
        "stage0.reachable_discovery.reachable_files",
        reachable.len() as f64,
    );
    crate::timing::record_counter(
        "stage0.reachable_discovery.imports_scanned",
        imports_scanned as f64,
    );

    Ok(ReachableTraversalOutcome {
        inventory: ReachableSourceInventory {
            files: reachable.into_iter().collect(),
            local_source_cache,
        },
    })
}

/// BFS over import declarations starting from `entry_point`, preserving source kind.
///
/// WHAT: follows each Moth file's declared imports, resolves them to canonical typed source
/// files, and returns the full ordered set of files reachable from the entry point.
/// WHY: source kind belongs to Stage 0 input discovery. Builder-supported content assets can be
///      loaded and carried forward without being treated as Moth module roots.
pub(super) fn discover_reachable_source_files(
    entry_point: &Path,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    string_table: &mut StringTable,
) -> Result<ReachableDiscoveryResult, SourceDiscoveryError> {
    let mut policy = ImportPolicy::Capable { external_imports };

    let outcome = traverse_reachable_source_files(
        &[entry_point.to_path_buf()],
        project_path_resolver,
        style_directives,
        &mut policy,
        string_table,
    )?;

    Ok(ReachableDiscoveryResult {
        inventory: outcome.inventory,
    })
}

/// Build the `TraversalSourceStorage` for one traversal from the optional store and resolution.
///
/// Directory projects pass `Some(store)` and `Some(resolution)` so the store owns source
/// preparation. Single-file synthetic compilation passes `None` for both, using the local cache.
/// Resolve a compiler-semantic Moth import and enqueue its indexed or synthetic-file target.
///
/// WHAT: handles cross-module root queuing, implementation-file discovery and direct dependency
///       edge retention for an import that is not provider-backed or a virtual package import.
/// WHY: one owner keeps indexed resolution, same-module queuing and graph-edge retention aligned.
///      A graph edge is retained only when indexed resolution crosses project module roots.
fn resolve_and_queue_local_import(
    provider: ProviderImportPathView<'_>,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
    reachable_queue: &mut ReachableQueue<'_>,
) -> Result<(), SourceDiscoveryError> {
    resolve_and_queue_via_filesystem(
        provider,
        canonical_file,
        project_path_resolver,
        string_table,
        reachable_queue,
    )
}

/// Resolve a compiler-semantic import through the filesystem-backed resolver for single-file
/// synthetic compilation.
///
/// Single-file compilation has no directory source index or project module graph, so it retains
/// the original resolver path. No dependency edges are collected because there is no project
/// module graph to populate.
fn resolve_and_queue_via_filesystem(
    provider: ProviderImportPathView<'_>,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
    reachable_queue: &mut ReachableQueue<'_>,
) -> Result<(), SourceDiscoveryError> {
    let resolved = project_path_resolver
        .resolve_import_to_source_file(provider.path, canonical_file, string_table)
        .map_err(SourceDiscoveryError::from)?;

    let resolved_source_file = resolved_source_file(&resolved.path, resolved.kind);
    if !reachable_queue.reachable.contains(&resolved_source_file) {
        reachable_queue.queue.push_back(resolved_source_file);
    }

    Ok(())
}

fn handle_provider_capable_import(
    import_path: &InternedPath,
    import_location: &SourceLocation,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    string_table: &mut StringTable,
) -> Result<ImportPolicyAction, SourceDiscoveryError> {
    // Skip virtual package imports — AST resolution handles those.
    if external_imports
        .external_packages
        .is_virtual_package_import(import_path, string_table)
    {
        if directory_import_resolution.is_some_and(|resolution| {
            resolution.has_binding_package_import(import_path, string_table)
        }) {
            return Ok(ImportPolicyAction::QueueLocal);
        }
        return Ok(ImportPolicyAction::Skip);
    }

    // Check for unsupported builder-specific core packages.
    if let Some(package_path) = external_imports
        .external_packages
        .unsupported_known_package_import(import_path, string_table)
    {
        return Err(SourceDiscoveryError::from(
            unsupported_builder_package_error(canonical_file, package_path, string_table),
        ));
    }

    // Detect provider-backed import prefixes (e.g. `./drawing.js` from
    //    `@./drawing.js/draw` or `@./drawing.js`).
    //    If a provider supports the extension, resolve the prefix, call the provider,
    //    and register the result. Do not add external files to the Moth input list.
    if let Some((prefix_path, prefix_str, extension)) =
        provider_backed_import_prefix(import_path, string_table)
    {
        if let Some(provider) = external_imports.providers.find_by_extension(&extension) {
            let provider_imports_start = crate::timing::start_pipeline_timing();
            let result = resolve_provider_backed_import(
                ProviderBackedImportRequest {
                    importer_canonical_path: canonical_file,
                    import_path,
                    import_location,
                    prefix_path: &prefix_path,
                    raw_prefix: &prefix_str,
                    provider,
                    project_path_resolver,
                    directory_import_resolution,
                },
                external_imports,
                string_table,
            );
            log_stage_timing(
                "stage0.reachable_discovery.provider_imports",
                provider_imports_start,
            );
            result?;
            crate::timing::record_counter("stage0.reachable_discovery.provider_imports", 1.0);
            return Ok(ImportPolicyAction::Skip);
        }

        // No provider registered for this extension — report unsupported extension.
        return Err(SourceDiscoveryError::from(
            unsupported_external_extension_error(
                canonical_file,
                import_path,
                &extension,
                string_table,
            ),
        ));
    }

    Ok(ImportPolicyAction::QueueLocal)
}

fn load_missing_sources(
    missing_sources: Vec<MissingSourceFile>,
    string_table: &mut StringTable,
) -> Result<Vec<LoadedMissingSourceFile>, CompilerMessages> {
    if missing_sources.is_empty() {
        return Ok(Vec::new());
    }

    if missing_sources.len() < STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES {
        add_frontend_counter(
            FrontendCounter::Stage0SerialSourceLoadCount,
            missing_sources.len(),
        );
        return load_missing_sources_serial(missing_sources, string_table);
    }

    add_frontend_counter(
        FrontendCounter::Stage0ParallelSourceLoadCount,
        missing_sources.len(),
    );
    load_missing_sources_parallel(missing_sources, string_table)
}

fn load_missing_sources_serial(
    missing_sources: Vec<MissingSourceFile>,
    string_table: &mut StringTable,
) -> Result<Vec<LoadedMissingSourceFile>, CompilerMessages> {
    let mut loaded_sources = Vec::with_capacity(missing_sources.len());

    for missing in missing_sources {
        let source_code = match extract_source_code(&missing.source_file.path, string_table) {
            Ok(source_code) => source_code,
            Err(error) => return Err(SourceDiscoveryError::from(error).into_messages(string_table)),
        };

        loaded_sources.push(LoadedMissingSourceFile {
            input_index: missing.input_index,
            source_file: missing.source_file,
            source_code,
        });
    }

    Ok(loaded_sources)
}

fn load_missing_sources_parallel(
    missing_sources: Vec<MissingSourceFile>,
    string_table: &mut StringTable,
) -> Result<Vec<LoadedMissingSourceFile>, CompilerMessages> {
    let mut loaded_sources = missing_sources
        .into_par_iter()
        .map(
            |missing| match read_source_code(&missing.source_file.path) {
                Ok(source_code) => Ok(LoadedMissingSourceFile {
                    input_index: missing.input_index,
                    source_file: missing.source_file,
                    source_code,
                }),
                Err(error) => Err(SourceReadFailure {
                    input_index: missing.input_index,
                    path: missing.source_file.path,
                    error,
                }),
            },
        )
        .collect::<Vec<_>>();

    loaded_sources.sort_by_key(|result| match result {
        Ok(loaded) => loaded.input_index,
        Err(failure) => failure.input_index,
    });

    let mut ordered_loaded_sources = Vec::with_capacity(loaded_sources.len());
    for loaded in loaded_sources {
        match loaded {
            Ok(loaded) => ordered_loaded_sources.push(loaded),
            Err(failure) => {
                let error = source_read_error(&failure.path, failure.error, string_table);
                return Err(SourceDiscoveryError::from(error).into_messages(string_table));
            }
        }
    }

    Ok(ordered_loaded_sources)
}

#[cfg(test)]
pub(super) fn load_missing_source_path_for_test(
    source_path: PathBuf,
    source_kind: SourceFileKind,
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    let missing_sources = vec![MissingSourceFile {
        input_index: 0,
        source_file: ReachableSourceFile {
            path: source_path,
            kind: source_kind,
        },
    }];

    load_missing_sources(missing_sources, string_table).map(|_| ())
}

#[cfg(test)]
pub(super) fn load_missing_source_paths_for_test(
    source_paths: Vec<PathBuf>,
    source_kind: SourceFileKind,
    string_table: &mut StringTable,
) -> Result<Vec<PreparedSourceInput>, CompilerMessages> {
    let missing_sources = source_paths
        .into_iter()
        .enumerate()
        .map(|(input_index, source_path)| MissingSourceFile {
            input_index,
            source_file: ReachableSourceFile {
                path: source_path,
                kind: source_kind,
            },
        })
        .collect();

    load_missing_sources(missing_sources, string_table).map(|loaded_sources| {
        loaded_sources
            .into_iter()
            .map(prepared_input_from_loaded)
            .collect()
    })
}

fn resolved_source_file(path: &Path, kind: SourceFileKind) -> ReachableSourceFile {
    ReachableSourceFile {
        path: path.to_path_buf(),
        kind,
    }
}

fn queue_same_directory_root_for_moth_template(
    moth_template_path: &Path,
    project_path_resolver: &ProjectPathResolver,
    reachable: &BTreeSet<ReachableSourceFile>,
    queue: &mut VecDeque<ReachableSourceFile>,
) {
    let Some(directory) = moth_template_path.parent() else {
        return;
    };

    let Some(root_path) = project_path_resolver.module_root_file_for_directory(directory) else {
        return;
    };

    let root_source_file = ReachableSourceFile {
        path: root_path,
        kind: SourceFileKind::Moth,
    };
    if !reachable.contains(&root_source_file) {
        queue.push_back(root_source_file);
    }
}

// -------------------------
//  Provider-backed import resolution
// -------------------------

/// Scans the components of an import path and returns the first file prefix whose final component
/// has an explicit non-`.moth` extension.
///
/// WHAT: for grouped syntax such as `import @./drawing.js { draw }` the tokenized path is
/// `@./drawing.js/draw`; this helper extracts the prefix `./drawing.js` and the extension `js`.
/// For a bare namespace import such as `import @./helper.js` the path is `@./helper.js`; the
/// prefix is `./helper.js`.
/// WHY: provider resolution must happen for the file prefix, while any remaining components are
/// symbol names to be resolved inside the provider-created package.
fn provider_backed_import_prefix(
    import_path: &InternedPath,
    string_table: &StringTable,
) -> Option<(InternedPath, String, String)> {
    let components = import_path.as_components();
    if components.is_empty() {
        return None;
    }

    // Walk components to find the provider-owned file segment. Any later path components are
    // grouped-import symbol names, not filesystem path segments.
    for (index, component) in components.iter().enumerate() {
        let segment = string_table.resolve(*component);
        let path = Path::new(segment);
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };

        if SourceFileKind::from_extension(extension).is_some() {
            continue;
        }

        let prefix_components = components[..=index].to_vec();
        let prefix_path = InternedPath::from_components(prefix_components);
        let prefix_str = prefix_path.to_portable_string(string_table);
        return Some((prefix_path, prefix_str, extension.to_owned()));
    }

    None
}

struct ProviderBackedImportRequest<'a> {
    importer_canonical_path: &'a Path,
    import_path: &'a InternedPath,
    import_location: &'a SourceLocation,
    prefix_path: &'a InternedPath,
    raw_prefix: &'a str,
    provider: &'a std::sync::Arc<dyn ExternalImportProvider>,
    project_path_resolver: &'a ProjectPathResolver,
    directory_import_resolution: Option<DirectoryImportResolution<'a>>,
}

/// Resolves a provider-backed import prefix to a canonical filesystem path, checks the build cache,
/// calls the provider if needed, and records the result in the resolution table and package registry.
fn resolve_provider_backed_import(
    request: ProviderBackedImportRequest<'_>,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    string_table: &mut StringTable,
) -> Result<(), SourceDiscoveryError> {
    // Directory projects resolve provider-owned targets through the same boundary-aware
    // namespace as compiler-semantic imports. Single-file synthetic compilation retains its
    // separate filesystem-backed resolver.
    let canonical_source_path = match request.directory_import_resolution {
        Some(resolution) => resolution
            .resolve_provider_target(
                request.prefix_path,
                request.importer_canonical_path,
                request.import_location,
                string_table,
            )
            .map_err(SourceDiscoveryError::from)?,
        None => resolve_provider_target_via_filesystem(&request, string_table)?,
    };

    invoke_provider_and_record_resolution(
        canonical_source_path,
        &request,
        external_imports,
        string_table,
    )
}

/// Resolve a single-file provider import target through the filesystem.
///
/// WHAT: the single-file synthetic-module mode retains its filesystem-backed provider
/// resolution, canonicalizing the normalized candidate and checking the module boundary from
/// the resolver's root tables. This is deliberately separate from the directory index path.
fn resolve_provider_target_via_filesystem(
    request: &ProviderBackedImportRequest<'_>,
    string_table: &mut StringTable,
) -> Result<PathBuf, SourceDiscoveryError> {
    let canonical_source_path = resolve_provider_prefix_to_canonical_path(
        request.prefix_path,
        request.importer_canonical_path,
        request.project_path_resolver,
        string_table,
    )?;

    // Enforce module/package boundaries for provider-backed imports.
    check_provider_import_module_boundary(
        request.importer_canonical_path,
        &canonical_source_path,
        request.import_path,
        request.project_path_resolver,
        string_table,
    )?;

    Ok(canonical_source_path)
}

/// Check the build cache, call the provider when needed, and record the result in the
/// resolution table and package registry.
///
/// WHAT: shared between the directory index path and the single-file filesystem path. The
/// `canonical_source_path` is the resolved target's IO handle from either path.
fn invoke_provider_and_record_resolution(
    canonical_source_path: PathBuf,
    request: &ProviderBackedImportRequest<'_>,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    string_table: &mut StringTable,
) -> Result<(), SourceDiscoveryError> {
    let cache_key = ExternalImportCacheKey {
        canonical_source_path: canonical_source_path.clone(),
        provider_kind: request.provider.kind(),
    };

    // Use cached result when available.
    if let Some(cached) = external_imports.cache.get(&cache_key) {
        let source_file_logical = source_file_logical_path(
            request.importer_canonical_path,
            request.project_path_resolver,
            string_table,
        )?;
        let import_prefix_logical = source_file_logical_path(
            &canonical_source_path,
            request.project_path_resolver,
            string_table,
        )?;
        insert_external_import_resolution(
            external_imports.resolution_table,
            source_file_logical,
            request.raw_prefix,
            import_prefix_logical,
            cached.clone(),
        );
        return Ok(());
    }

    let provider_request = ExternalImportRequest {
        import_path: request.import_path.to_portable_string(string_table),
        canonical_source_path: canonical_source_path.clone(),
        source_location:
            crate::compiler_frontend::compiler_messages::source_location::SourceLocation::from_path(
                request.importer_canonical_path,
                string_table,
            ),
    };

    let result = {
        let mut context = ExternalImportProviderContext {
            package_registry: external_imports.external_packages,
            cache: external_imports.cache,
            string_table,
        };

        request
            .provider
            .resolve_external_import(provider_request, &mut context)
            .map_err(SourceDiscoveryError::from)?
    };

    if let Some(resolved) = result {
        external_imports.cache.insert(cache_key, resolved.clone());

        let source_file_logical = source_file_logical_path(
            request.importer_canonical_path,
            request.project_path_resolver,
            string_table,
        )?;
        let import_prefix_logical = source_file_logical_path(
            &canonical_source_path,
            request.project_path_resolver,
            string_table,
        )?;
        insert_external_import_resolution(
            external_imports.resolution_table,
            source_file_logical,
            request.raw_prefix,
            import_prefix_logical,
            resolved,
        );
    }

    Ok(())
}

fn insert_external_import_resolution(
    external_import_resolution_table: &mut ExternalImportResolutionTable,
    source_file_logical: String,
    raw_import_prefix: &str,
    logical_import_prefix: String,
    resolved: crate::builder_surface::external_import_providers::provider::ResolvedExternalImport,
) {
    external_import_resolution_table.insert(
        source_file_logical.clone(),
        logical_import_prefix.clone(),
        resolved.clone(),
    );

    if raw_import_prefix != logical_import_prefix {
        external_import_resolution_table.insert(source_file_logical, raw_import_prefix, resolved);
    }
}

/// Resolves a provider import prefix to a canonical filesystem path without selecting a compiler
/// source extension candidate.
///
/// WHAT: reuses the normal base/boundary/case rules from `ProjectPathResolver` but skips the
/// extension candidate selection used by isolated compiler-source resolution.
fn resolve_provider_prefix_to_canonical_path(
    prefix_path: &InternedPath,
    importer_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<PathBuf, SourceDiscoveryError> {
    let (base_kind, filesystem_base) = project_path_resolver
        .resolve_path_base_for_provider(prefix_path, importer_file, string_table)
        .map_err(SourceDiscoveryError::from)?;

    let normalized = join_and_normalize_path(&filesystem_base, prefix_path, string_table);

    let canonical = fs::canonicalize(&normalized)
        .map_err(|error| {
            CompilerError::file_error(
                importer_file,
                format!(
                    "Failed to canonicalize external import prefix '{}': {error}",
                    normalized.display()
                ),
                string_table,
            )
        })
        .map_err(SourceDiscoveryError::from)?;

    crate::compiler_frontend::paths::import_resolution::validate_import_boundary(
        &canonical,
        &base_kind,
        &filesystem_base,
        prefix_path,
        importer_file,
        string_table,
    )
    .map_err(SourceDiscoveryError::from)?;

    Ok(canonical)
}

/// Derives the portable logical path for a canonical source file.
fn source_file_logical_path(
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<String, SourceDiscoveryError> {
    let logical = project_path_resolver
        .logical_path_for_canonical_file(canonical_file, string_table)
        .map_err(SourceDiscoveryError::from)?;
    let logical_text = logical.to_str().ok_or_else(|| {
        SourceDiscoveryError::from(CompilerError::file_error(
            &logical,
            format!(
                "Source file logical path {logical:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
            ),
            string_table,
        ))
    })?;
    Ok(logical_text.replace('\\', "/"))
}

// -------------------------
//  Provider import boundary check
// -------------------------

/// Enforce that a provider-backed import does not cross a module or source-backed package boundary.
///
/// WHAT: .js files are private implementation details of the module or package that owns them.
///       Cross-module or cross-package .js imports bypass the public surface and are rejected.
/// WHY: provider-backed imports must obey the same visibility boundaries as .moth source imports.
fn check_provider_import_module_boundary(
    importer_file: &Path,
    target_file: &Path,
    import_path: &InternedPath,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<(), SourceDiscoveryError> {
    let importer_container = provider_import_container(project_path_resolver, importer_file);
    let target_container = provider_import_container(project_path_resolver, target_file);

    if importer_container != target_container {
        let location = SourceLocation::from_path(importer_file, string_table);
        return Err(SourceDiscoveryError::from(
            CompilerDiagnostic::cross_module_import_not_exported(import_path.clone(), location),
        ));
    }

    Ok(())
}

/// Determine the boundary "container" of a file for provider import checks.
///
/// WHAT: returns the module root, source-backed package root, or entry root that contains the file.
/// WHY: two files in the same container may freely import each other's .js files.
fn provider_import_container(
    project_path_resolver: &ProjectPathResolver,
    file: &Path,
) -> Option<PathBuf> {
    // Module roots are the most specific boundaries.
    if let Some(root) = project_path_resolver.module_root_for_file(file) {
        return Some(root);
    }

    // Source-backed packages are the next boundary. Use the resolver's nearest-root policy so nested
    // packages do not inherit provider access from an outer registered root.
    if let Some((_, root)) = project_path_resolver.source_package_for_file(file) {
        return Some(root.to_path_buf());
    }

    // Everything under the entry root belongs to the default module.
    if file.starts_with(project_path_resolver.entry_root()) {
        return Some(project_path_resolver.entry_root().to_path_buf());
    }

    None
}

// -------------------------
//  Diagnostic Helpers
// -------------------------

fn unsupported_builder_package_error(
    importer: &Path,
    package_path: &str,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let package_path_id = string_table.intern(package_path);
    let location =
        crate::compiler_frontend::compiler_messages::source_location::SourceLocation::from_path(
            importer,
            string_table,
        );
    CompilerDiagnostic::unsupported_builder_package(package_path_id, location)
}

fn unsupported_external_extension_error(
    importer: &Path,
    import_path: &InternedPath,
    extension: &str,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let extension_id = string_table.intern(extension);
    let location =
        crate::compiler_frontend::compiler_messages::source_location::SourceLocation::from_path(
            importer,
            string_table,
        );
    CompilerDiagnostic::unsupported_external_extension(import_path.clone(), extension_id, location)
}
