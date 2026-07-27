//! BFS traversal over Moth import graphs to find all reachable source files.
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
use crate::compiler_frontend::paths::const_paths::StructuralProviderReference;
use crate::compiler_frontend::paths::path_normalization::join_and_normalize_path;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use super::import_scanning::{ScannedImportSource, scan_imports_with_source};
use super::module_identity::ModuleId;
use super::module_namespace::{DirectoryImportResolution, NamespaceBoundary, ResolvedImport};
use super::prepared_source::PreparedSourceInput;
use super::prepared_source_store::{MothScanOrigin, PreparedSourceStore};
use super::semantic_source_set::SemanticSourceSet;
use super::source_discovery_error::SourceDiscoveryError;
use super::source_loading::{extract_source_code, read_source_code, source_read_error};

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

/// A reachable source file plus the source kind selected by import resolution.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ReachableSourceFile {
    pub(super) path: PathBuf,
    pub(super) kind: SourceFileKind,
}

/// Stage 0 inventory produced by reachable-file discovery.
///
/// WHAT: owns the deterministic input-file list, the retained `ScannedImportSource` for every
///       reachable `.moth` file, and the optional project `SemanticSourceSet` built during
///       per-entry directory traversal. The semantic set is the authority that selects and
///       orders project semantic members; the file list is the reachable-file handle.
///       For single-file synthetic compilation (no project `SourceId` store), `local_source_cache`
///       retains the per-entry `ScannedImportSource` map. For directory projects, prepared source
///       data lives in the shared `PreparedSourceStore` and `local_source_cache` is `None`.
/// WHY: source loading policy belongs to Stage 0. `assemble_input_files_from_inventory` turns
///      this into `PreparedSourceInput` values, so header parsing and later frontend stages
///      receive the state-safe prepared type without knowing whether text came from the
///      prepared-source store, import-scan cache or a later raw file read.
pub(super) struct ReachableSourceInventory {
    pub(super) files: Vec<ReachableSourceFile>,
    local_source_cache: Option<FxHashMap<PathBuf, ScannedImportSource>>,
    pub(super) semantic_source_set: Option<SemanticSourceSet>,
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
    pub(super) provider: StructuralProviderReference,
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
    pub(super) resolved_edges: Vec<ResolvedDependencyEdge>,
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
    pub(super) resolved_edges: Vec<ResolvedDependencyEdge>,
}

/// Retained Stage 0 source proven provider-free during the serial classification pass.
///
/// WHAT: records whether the serial provider-capable owner must replay after classification
///       found a provider-backed import or unsupported package condition.
/// WHY: classification populates the shared `PreparedSourceStore`; the serial fallback reads
///      from the store instead of re-tokenizing every already-scanned `.moth`.
pub(super) struct ProviderFreeProjectInventory {
    /// Whether provider-capable serial replay is required.
    ///
    /// WHAT: set when classification traversed the complete reachable local `.moth` graph and
    ///       encountered a provider-backed import or unsupported package condition that only the
    ///       serial provider-capable owner can resolve.
    /// WHY: the store retains every already-scanned `.moth` so the serial fallback reuses it
    ///      instead of re-tokenizing.
    pub(super) provider_capable_required: bool,
}

impl ProviderFreeProjectInventory {
    /// An empty inventory that forces the serial provider-capable path with no retained cache.
    ///
    /// WHY: used for the single-entry case where classification is skipped because the path stays
    ///      serial provider-capable anyway.
    pub(super) fn provider_capable_required() -> Self {
        ProviderFreeProjectInventory {
            provider_capable_required: true,
        }
    }
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
    resolved_edges: &'a mut Vec<ResolvedDependencyEdge>,
    semantic_source_set: Option<&'a mut SemanticSourceSet>,
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
#[allow(clippy::needless_option_as_deref)]
pub(super) fn collect_reachable_input_files(
    entry_path: &Path,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    mut prepared_source_store: Option<&mut PreparedSourceStore>,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    string_table: &mut StringTable,
) -> Result<CollectedReachableInputs, CompilerMessages> {
    let total_start = crate::timing::start_pipeline_timing();

    // 1. Traverse the import graph to find all paths and retained resolved edges.
    let discovery = match discover_reachable_source_files(
        entry_path,
        project_path_resolver,
        style_directives,
        external_imports,
        prepared_source_store.as_deref_mut(),
        directory_import_resolution,
        string_table,
    ) {
        Ok(discovery) => discovery,
        Err(error) => {
            log_stage_timing("stage0.reachable_discovery.total", total_start);
            return Err(error.into_messages(string_table));
        }
    };

    let ReachableDiscoveryResult {
        inventory,
        resolved_edges,
    } = discovery;

    let input_files = assemble_input_files_from_inventory(
        inventory,
        prepared_source_store.as_deref_mut(),
        directory_import_resolution,
        string_table,
    )?;
    log_stage_timing("stage0.reachable_discovery.total", total_start);
    Ok(CollectedReachableInputs {
        input_files,
        resolved_edges,
    })
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
    prepared_source_store: Option<&mut PreparedSourceStore>,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    string_table: &mut StringTable,
) -> Result<Vec<PreparedSourceInput>, CompilerMessages> {
    let ReachableSourceInventory {
        files,
        local_source_cache,
        semantic_source_set,
    } = inventory;

    match semantic_source_set {
        Some(set) => {
            let Some(resolution) = directory_import_resolution else {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Project semantic source set reached input assembly without its project source index",
                    ),
                    string_table,
                ));
            };
            let Some(store) = prepared_source_store else {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Project semantic source set reached input assembly without its prepared source store",
                    ),
                    string_table,
                ));
            };
            assemble_with_semantic_set(
                files,
                set,
                store,
                local_source_cache.unwrap_or_default(),
                resolution.project_source_tree_index(),
                string_table,
            )
        }
        None => {
            let local_cache = local_source_cache.unwrap_or_default();
            assemble_reachable_files(files, local_cache, string_table)
        }
    }
}

/// Assemble `PreparedSourceInput` values using the `SemanticSourceSet` as the project semantic
/// membership authority.
///
/// WHAT: project semantic members are selected and ordered by `SourceId` (portable logical
///       identity order) from the set. Remaining reachable files are explicit interface/external
///       inputs for the donor compiler and follow in `BTreeSet` (canonical path) order. The
///       `PreparedSourceStore` is the single source-data owner: prepared `.moth` slots carry
///       retained tokens; `.mtf`/`.md` slots are prepared lazily during assembly.
/// WHY: the set is the authority that determines which reachable files are project semantic
///      members; the store provides retained source text and tokens. Cross-module provider roots,
///      source-package facades and binding-package inputs never enter the semantic set.
fn assemble_with_semantic_set(
    files: Vec<ReachableSourceFile>,
    set: SemanticSourceSet,
    store: &mut PreparedSourceStore,
    mut non_project_source_cache: FxHashMap<PathBuf, ScannedImportSource>,
    source_tree_index: &super::source_tree_index::SourceTreeIndex,
    string_table: &mut StringTable,
) -> Result<Vec<PreparedSourceInput>, CompilerMessages> {
    let member_paths: FxHashSet<&Path> = set
        .members()
        .iter()
        .map(|source_id| source_tree_index.source(*source_id).canonical_path())
        .collect();

    // Verify every semantic member was reached by the BFS. An unreached member is an impossible
    // project source membership mismatch surfaced as an internal build-system error.
    let reachable_paths: FxHashSet<&Path> = files.iter().map(|file| file.path.as_path()).collect();
    for source_id in set.members() {
        let record = source_tree_index.source(*source_id);
        if !reachable_paths.contains(record.canonical_path()) {
            let error = CompilerError::compiler_error(format!(
                "Project semantic source set member {} (SourceId {}) was not reached by the reachable traversal; every project semantic member must be a reachable file",
                record.canonical_path().display(),
                source_id.index(),
            ));
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    }

    // Project sources owned by another module are structural provider references, not semantic
    // inputs. Their completed interfaces are supplied by the graph scheduler to header binding.
    // Non-project sources remain explicit inputs until their separate package graph is cut over.
    let external_input_files: Vec<&ReachableSourceFile> = files
        .iter()
        .filter(|source_file| {
            !member_paths.contains(source_file.path.as_path())
                && source_tree_index
                    .source_id_for_canonical_path(&source_file.path)
                    .is_none()
        })
        .collect();

    let mut input_slots: Vec<Option<PreparedSourceInput>> = (0..set.members().len()
        + external_input_files.len())
        .map(|_| None)
        .collect();
    let mut missing_sources = Vec::new();

    // Semantic members in deterministic `SourceId` order.
    for (input_index, source_id) in set.members().iter().enumerate() {
        let prepared = store
            .project_project_prepared_input(*source_id, source_tree_index, string_table)
            .map_err(|error| error.into_messages(string_table))?;
        input_slots[input_index] = Some(prepared);
    }

    // External inputs remain in canonical-path (BTreeSet) order. Cross-module project sources
    // are deliberately absent: their provider interfaces replace donor syntax at binding.
    // Package sources stay in the traversal-local cache until their own boundary store lands.
    for (input_index, source_file) in (set.members().len()..).zip(external_input_files) {
        fill_input_slot(
            &mut non_project_source_cache,
            &source_file.path,
            source_file.kind,
            input_index,
            &mut input_slots,
            &mut missing_sources,
        );
    }

    load_and_join_input_slots(input_slots, missing_sources, string_table)
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
    /// Skip this external edge and record that provider-capable serial replay is required.
    ///
    /// WHAT: classification returns this for provider-backed imports and unsupported package
    ///       conditions that only the serial provider-capable owner can resolve. The traversal
    ///       keeps scanning every reachable local `.moth` file and retains its complete cache, only
    ///       declining to follow the external edge.
    SkipProviderCapableRequired,
}

/// Stage 0 import policy that customizes the shared reachable-file traversal.
///
/// WHAT: the three discovery paths (provider-capable serial, provider-free classification,
///       provider-free worker) differ only in how they react to external package imports. This
///       enum keeps those differences explicit while letting the shared BFS own queue handling,
///       canonicalization, source preparation and local queuing.
///
/// Source preparation is owned by the [`TraversalSourceStorage`] parameter, not the policy. The
/// policy only decides import actions; the storage decides where scanned source data is retained.
enum ImportPolicy<'a, 'b> {
    /// Full provider-capable path. Mutates provider cache and resolution tables.
    Capable {
        external_imports: &'a mut ExternalImportDiscoveryState<'b>,
    },
    /// Conservative pre-scan that proves a directory build has no provider-backed imports.
    FreeClassification(&'a ExternalPackageRegistry),
    /// Provider-free worker path that reuses retained lexical data proved safe by classification.
    FreeWorker {
        external_packages: &'a ExternalPackageRegistry,
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
            ImportPolicy::FreeClassification(external_packages) => {
                handle_provider_free_classification_import(
                    import_path,
                    canonical_file,
                    external_packages,
                    directory_import_resolution,
                    string_table,
                )
            }
            ImportPolicy::FreeWorker { external_packages } => handle_provider_free_worker_import(
                import_path,
                canonical_file,
                external_packages,
                directory_import_resolution,
                string_table,
            ),
        }
    }
}

/// Source preparation strategy for one reachable-file traversal.
///
/// WHAT: abstracts where scanned `.moth` source data is retained so the shared BFS does not
///       duplicate traversal logic. Directory projects use the shared `PreparedSourceStore`;
///       single-file synthetic compilation uses a local path-keyed cache.
enum TraversalSourceStorage<'a> {
    /// Directory project: prepare sources lazily in the shared store (read-write).
    Store { store: &'a mut PreparedSourceStore },
    /// Provider-free worker: read from the already-populated store (read-only).
    StoreReadOnly { store: &'a PreparedSourceStore },
    /// Single-file synthetic compilation: local path-keyed cache.
    Local,
}

/// Result of scanning one `.moth` file during traversal.
struct ScannedMothSource {
    imports: Vec<StructuralProviderReference>,
    fresh_read: bool,
    source_byte_count: usize,
}

fn scan_and_cache_local_moth_source(
    canonical_file: &Path,
    style_directives: &StyleDirectiveRegistry,
    local_source_cache: &mut Option<FxHashMap<PathBuf, ScannedImportSource>>,
    string_table: &mut StringTable,
) -> Result<ScannedMothSource, SourceDiscoveryError> {
    if let Some(scanned) = local_source_cache
        .as_ref()
        .and_then(|cache| cache.get(canonical_file))
    {
        return Ok(ScannedMothSource {
            imports: scanned.imports.clone(),
            fresh_read: false,
            source_byte_count: 0,
        });
    }

    let scanned = scan_imports_with_source(canonical_file, style_directives, string_table)?;
    let imports = scanned.imports.clone();
    let source_byte_count = scanned.source_code.len();
    local_source_cache
        .as_mut()
        .expect("local source cache exists for mutable traversal storage")
        .insert(canonical_file.to_path_buf(), scanned);

    Ok(ScannedMothSource {
        imports,
        fresh_read: true,
        source_byte_count,
    })
}

/// Scan one `.moth` file for imports using the traversal's source storage strategy.
///
/// WHAT: for directory projects, prepares or reuses the source in the shared `PreparedSourceStore`.
///       For single-file synthetic compilation, scans and caches in the local path-keyed cache.
///       Workers read from the already-populated store without preparing.
fn scan_moth_source(
    canonical_file: &Path,
    style_directives: &StyleDirectiveRegistry,
    source_storage: &mut TraversalSourceStorage<'_>,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    local_source_cache: &mut Option<FxHashMap<PathBuf, ScannedImportSource>>,
    string_table: &mut StringTable,
) -> Result<ScannedMothSource, SourceDiscoveryError> {
    match source_storage {
        TraversalSourceStorage::Store { store } => {
            let resolution = directory_import_resolution.ok_or_else(|| {
                CompilerError::compiler_error(
                    "Store source storage requires directory import resolution",
                )
            })?;
            let source_tree_index = resolution.project_source_tree_index();
            let Some(source_id) = source_tree_index.source_id_for_canonical_path(canonical_file)
            else {
                return scan_and_cache_local_moth_source(
                    canonical_file,
                    style_directives,
                    local_source_cache,
                    string_table,
                );
            };
            let (imports, origin) = store.prepare_or_get_project_moth_imports(
                source_id,
                canonical_file,
                style_directives,
                string_table,
            )?;
            let (fresh_read, source_byte_count) = match origin {
                MothScanOrigin::FreshRead { source_byte_count } => (true, source_byte_count),
                MothScanOrigin::ReusedFromStore => (false, 0),
            };
            Ok(ScannedMothSource {
                imports,
                fresh_read,
                source_byte_count,
            })
        }
        TraversalSourceStorage::StoreReadOnly { store } => {
            let resolution = directory_import_resolution.ok_or_else(|| {
                CompilerError::compiler_error(
                    "StoreReadOnly source storage requires directory import resolution",
                )
            })?;
            let source_id = resolution
                .project_source_tree_index()
                .source_id_for_canonical_path(canonical_file)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Provider-free worker reached a source outside the project prepared-source boundary",
                    )
                })?;
            let imports = store.get_project_moth_imports(source_id)?;
            Ok(ScannedMothSource {
                imports,
                fresh_read: false,
                source_byte_count: 0,
            })
        }
        TraversalSourceStorage::Local => scan_and_cache_local_moth_source(
            canonical_file,
            style_directives,
            local_source_cache,
            string_table,
        ),
    }
}

/// Shared BFS over import declarations with a policy-controlled import handler.
///
/// WHAT: follows each Moth file's declared imports, resolves them to canonical typed source
///       files, and returns the full ordered set of files reachable from the entry points.
/// WHY: queue seeding, canonicalization, visited-set handling, root queueing, Markdown skipping,
///      import scanning, source-cache insertion, and local import queueing are identical across all
///      Stage 0 discovery paths. Keeping them in one place prevents the provider-capable and
///      provider-free paths from drifting.
/// Outcome of a shared reachable-file traversal.
///
/// WHAT: the complete retained inventory plus whether a provider-backed or unsupported package
///       import required the serial provider-capable owner.
/// WHY: classification never discards its cache when replay is required. The serial fallback
///      consumes the retained cache for every already-scanned `.moth`, so the flag stays separate
///      from the inventory itself.
struct ReachableTraversalOutcome {
    inventory: ReachableSourceInventory,
    provider_capable_required: bool,
    resolved_edges: Vec<ResolvedDependencyEdge>,
}

#[allow(clippy::too_many_arguments)]
fn traverse_reachable_source_files(
    entry_paths: &[PathBuf],
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    policy: &mut ImportPolicy<'_, '_>,
    source_storage: &mut TraversalSourceStorage<'_>,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    build_semantic_source_set: bool,
    string_table: &mut StringTable,
) -> Result<ReachableTraversalOutcome, SourceDiscoveryError> {
    let mut reachable = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut local_source_cache = match source_storage {
        TraversalSourceStorage::StoreReadOnly { .. } => None,
        TraversalSourceStorage::Store { .. } | TraversalSourceStorage::Local => {
            Some(FxHashMap::default())
        }
    };
    let mut imports_scanned: usize = 0;
    let mut provider_capable_required = false;
    let mut resolved_edges: Vec<ResolvedDependencyEdge> = Vec::new();

    // Build the project semantic source set for one normal entry module when the traversal is a
    // per-entry directory discovery. Classification passes multiple entries and does not build
    // a set; single-file discovery has no directory index. The set is seeded from the entry root
    // file's `SourceId` and grows as same-owner imports resolve through the project namespace.
    let mut semantic_source_set = if build_semantic_source_set {
        let resolution = directory_import_resolution.ok_or_else(|| {
            CompilerError::compiler_error(
                "Project semantic source-set traversal requires directory import resolution",
            )
        })?;
        if entry_paths.len() != 1 {
            return Err(CompilerError::compiler_error(
                "Project semantic source-set traversal requires exactly one entry seed",
            )
            .into());
        }
        let entry_path = &entry_paths[0];

        {
            let (source_id, module_id) = resolution
                .source_id_and_module_for_path(entry_path)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Project semantic source set could not resolve entry root file {} to an owned \
                         project source; the entry root must be an indexed owned source",
                        entry_path.display(),
                    ))
                })?;
            Some(
                SemanticSourceSet::from_entry_source(
                    source_id,
                    module_id,
                    resolution.project_source_tree_index(),
                )
                .map_err(SourceDiscoveryError::from)?,
            )
        }
    } else {
        None
    };

    // Seed with entry points in deterministic order.
    for entry_path in entry_paths {
        queue.push_back(ReachableSourceFile {
            path: entry_path.clone(),
            kind: SourceFileKind::Moth,
        });
    }

    // Seed every source-backed package root file so its authored public surface is available.
    // WHY: imports may directly resolve to a target file after Stage 0 path scanning, but the
    // root file still needs to be compiled so its public declarations can be checked later.
    for (_, root_path) in project_path_resolver.source_package_public_surface_files() {
        queue.push_back(ReachableSourceFile {
            path: root_path.clone(),
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
        let scanned = match scan_moth_source(
            &canonical_file,
            style_directives,
            source_storage,
            directory_import_resolution,
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
                directory_import_resolution,
                string_table,
            )?;

            match action {
                ImportPolicyAction::Skip => continue,
                ImportPolicyAction::QueueLocal => {
                    let import_resolve_start = crate::timing::start_pipeline_timing();
                    let mut reachable_queue = ReachableQueue {
                        reachable: &reachable,
                        queue: &mut queue,
                        resolved_edges: &mut resolved_edges,
                        semantic_source_set: semantic_source_set.as_mut(),
                    };
                    let result = resolve_and_queue_local_import(
                        provider,
                        &canonical_file,
                        project_path_resolver,
                        directory_import_resolution,
                        string_table,
                        &mut reachable_queue,
                    );
                    log_stage_timing(
                        "stage0.reachable_discovery.import_resolve",
                        import_resolve_start,
                    );
                    result?;
                }
                ImportPolicyAction::SkipProviderCapableRequired => {
                    provider_capable_required = true;
                    continue;
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

    // Finalize the semantic source set: sort by `SourceId`, deduplicate and verify every member
    // is owned by the entry module. The set is consumed by `PreparedSourceInput` assembly as the
    // project semantic membership authority.
    let semantic_source_set = if let Some(set) = semantic_source_set {
        let resolution = directory_import_resolution.ok_or_else(|| {
            CompilerError::compiler_error(
                "Project semantic source set lost directory import resolution before finalization",
            )
        })?;
        Some(
            set.finish(resolution.project_source_tree_index())
                .map_err(SourceDiscoveryError::from)?,
        )
    } else {
        None
    };

    Ok(ReachableTraversalOutcome {
        inventory: ReachableSourceInventory {
            files: reachable.into_iter().collect(),
            local_source_cache,
            semantic_source_set,
        },
        provider_capable_required,
        resolved_edges,
    })
}

/// BFS over import declarations starting from `entry_point`, preserving source kind.
///
/// WHAT: follows each Moth file's declared imports, resolves them to canonical typed source
/// files, and returns the full ordered set of files reachable from the entry point.
/// WHY: source kind belongs to Stage 0 input discovery. Builder-supported content assets can be
///      loaded and carried forward without being treated as Moth module roots.
#[allow(clippy::needless_option_as_deref)]
pub(super) fn discover_reachable_source_files(
    entry_point: &Path,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    mut prepared_source_store: Option<&mut PreparedSourceStore>,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    string_table: &mut StringTable,
) -> Result<ReachableDiscoveryResult, SourceDiscoveryError> {
    let mut policy = ImportPolicy::Capable { external_imports };

    let mut source_storage = match (
        prepared_source_store.as_deref_mut(),
        directory_import_resolution,
    ) {
        (Some(store), Some(_)) => TraversalSourceStorage::Store { store },
        _ => TraversalSourceStorage::Local,
    };

    let outcome = traverse_reachable_source_files(
        &[entry_point.to_path_buf()],
        project_path_resolver,
        style_directives,
        &mut policy,
        &mut source_storage,
        directory_import_resolution,
        directory_import_resolution.is_some(),
        string_table,
    )?;

    // Provider-capable traversal never marks replay required, so the flag is always false here.
    debug_assert!(
        !outcome.provider_capable_required,
        "provider-capable traversal must not mark provider-capable replay required"
    );
    Ok(ReachableDiscoveryResult {
        inventory: outcome.inventory,
        resolved_edges: outcome.resolved_edges,
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
/// WHY: this logic is identical between the provider-capable and provider-free discovery paths;
///      extracting it prevents the two traversal policies from drifting. A graph edge is retained
///      only when indexed resolution crosses project module roots.
fn resolve_and_queue_local_import(
    provider: &StructuralProviderReference,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    string_table: &mut StringTable,
    reachable_queue: &mut ReachableQueue<'_>,
) -> Result<(), SourceDiscoveryError> {
    // Directory projects resolve compiler-semantic imports through the boundary-aware namespace
    // using indexed facts, replacing filesystem candidate probing and public-surface fallback.
    // Single-file synthetic compilation retains its filesystem-backed resolver path because it
    // has no directory source index or project module graph.
    if let Some(directory_import_resolution) = directory_import_resolution {
        return resolve_and_queue_via_namespace(
            provider,
            canonical_file,
            directory_import_resolution,
            string_table,
            reachable_queue,
        );
    }

    resolve_and_queue_via_filesystem(
        provider,
        canonical_file,
        project_path_resolver,
        string_table,
        reachable_queue,
    )
}

/// Resolve a compiler-semantic import through the boundary-aware namespace and enqueue files.
///
/// WHAT: uses indexed facts to resolve the import to a tagged `ResolvedImport`, queues the
///       canonical source file or module root file, and retains a `ResolvedDependencyEdge` for
///       project-local cross-module imports. No filesystem candidate probing or
///       public-surface fallback runs here.
fn resolve_and_queue_via_namespace(
    provider: &StructuralProviderReference,
    canonical_file: &Path,
    directory_import_resolution: DirectoryImportResolution<'_>,
    string_table: &mut StringTable,
    reachable_queue: &mut ReachableQueue<'_>,
) -> Result<(), SourceDiscoveryError> {
    let resolved = directory_import_resolution
        .resolve_import(provider, canonical_file, string_table)
        .map_err(SourceDiscoveryError::from)?;

    match resolved {
        ResolvedImport::SameModuleSource {
            source_id,
            canonical_path,
            source_kind,
            consumer_module_id,
            boundary,
        } => {
            if boundary == NamespaceBoundary::Project
                && let Some(set) = reachable_queue.semantic_source_set.as_deref_mut()
                && consumer_module_id == set.entry_module_id()
            {
                set.add_same_owner_source(source_id);
            }
            let source_file = resolved_source_file(&canonical_path, source_kind);
            if !reachable_queue.reachable.contains(&source_file) {
                reachable_queue.queue.push_back(source_file);
            }
        }
        ResolvedImport::CrossModuleProject {
            provider_module_id,
            consumer_module_id,
            root_file,
        } => {
            reachable_queue.resolved_edges.push(ResolvedDependencyEdge {
                provider_module_id,
                consumer_module_id,
                provider: provider.clone(),
            });
            let root_source = ReachableSourceFile {
                path: root_file,
                kind: SourceFileKind::Moth,
            };
            if !reachable_queue.reachable.contains(&root_source) {
                reachable_queue.queue.push_back(root_source);
            }
        }
        ResolvedImport::RootFile { root_file } => {
            let root_source = ReachableSourceFile {
                path: root_file,
                kind: SourceFileKind::Moth,
            };
            if !reachable_queue.reachable.contains(&root_source) {
                reachable_queue.queue.push_back(root_source);
            }
        }
        ResolvedImport::BindingPackage => {}
    }

    Ok(())
}

/// Resolve a compiler-semantic import through the filesystem-backed resolver for single-file
/// synthetic compilation.
///
/// Single-file compilation has no directory source index or project module graph, so it retains
/// the original resolver path. No dependency edges are collected because there is no project
/// module graph to populate.
fn resolve_and_queue_via_filesystem(
    provider: &StructuralProviderReference,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
    reachable_queue: &mut ReachableQueue<'_>,
) -> Result<(), SourceDiscoveryError> {
    let resolved = project_path_resolver
        .resolve_import_to_source_file_with_public_surface_fallback(
            &provider.path,
            canonical_file,
            string_table,
        )
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

// -------------------------
//  Provider-free discovery
// -------------------------

/// Conservative pre-scan that proves a directory build has no reachable provider-backed imports.
///
/// WHAT: walks the same import graph as discovery, tokenizing and caching every reachable `.moth`
///       file. It always completes the full local traversal and retains the complete scan cache.
///       When a provider-backed import or unsupported package condition requires the serial
///       provider-capable owner, it records `provider_capable_required` and skips that external
///       edge rather than aborting and discarding the cache.
/// WHY: the provider-capable discovery path mutates `ExternalImportDiscoveryState`; proving the
///      project is provider-free before forking lets multi-entry module discovery run in Rayon
///      workers that never touch provider registry deltas. When replay is required, the serial
///      fallback consumes the retained cache so the lexer never runs twice for the same source.
pub(super) fn classify_provider_free_project(
    entry_points: &[PathBuf],
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_packages: &ExternalPackageRegistry,
    prepared_source_store: &mut PreparedSourceStore,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    string_table: &mut StringTable,
) -> Result<ProviderFreeProjectInventory, SourceDiscoveryError> {
    let total_start = crate::timing::start_pipeline_timing();
    let mut policy = ImportPolicy::FreeClassification(external_packages);

    let mut source_storage = TraversalSourceStorage::Store {
        store: prepared_source_store,
    };

    let outcome = match traverse_reachable_source_files(
        entry_points,
        project_path_resolver,
        style_directives,
        &mut policy,
        &mut source_storage,
        directory_import_resolution,
        false,
        string_table,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            log_stage_timing("stage0.reachable_discovery.total", total_start);
            return Err(error);
        }
    };

    log_stage_timing("stage0.reachable_discovery.total", total_start);
    Ok(ProviderFreeProjectInventory {
        provider_capable_required: outcome.provider_capable_required,
    })
}

fn handle_provider_free_classification_import(
    import_path: &InternedPath,
    _canonical_file: &Path,
    external_packages: &ExternalPackageRegistry,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    string_table: &mut StringTable,
) -> Result<ImportPolicyAction, SourceDiscoveryError> {
    if external_packages.is_virtual_package_import(import_path, string_table) {
        if directory_import_resolution.is_some_and(|resolution| {
            resolution.has_binding_package_import(import_path, string_table)
        }) {
            return Ok(ImportPolicyAction::QueueLocal);
        }
        return Ok(ImportPolicyAction::Skip);
    }

    if external_packages
        .unsupported_known_package_import(import_path, string_table)
        .is_some()
    {
        // The serial provider-capable path reports this diagnostic with full context. Skip the
        // external edge and mark replay required without discarding the retained cache.
        return Ok(ImportPolicyAction::SkipProviderCapableRequired);
    }

    if provider_backed_import_prefix(import_path, string_table).is_some() {
        // Registered provider extensions need the serial provider-capable path so the provider is
        // called and the resolution table is populated. Unsupported non-Moth extensions also
        // fall back so the existing diagnostic shape is preserved. Skip the external edge and mark
        // replay required without discarding the retained cache.
        return Ok(ImportPolicyAction::SkipProviderCapableRequired);
    }

    Ok(ImportPolicyAction::QueueLocal)
}

/// Marker error returned from Rayon workers when provider-free discovery fails.
///
/// WHAT: workers use worker-local `StringTable` values, so their diagnostics are not interpretable
///       on the main thread. Instead of exposing interned IDs cross-thread, the worker reports
///       failure and the caller falls back to the serial provider-capable path.
#[derive(Debug)]
pub(super) struct ProviderFreeDiscoveryFailed;

/// Provider-free BFS over one module's import graph.
///
/// WHAT: shares the same traversal mechanics as provider-capable discovery but skips all
///       provider-backed import handling and reuses source text proved safe by classification.
/// WHY: this function is safe to call inside Rayon workers because it only needs an immutable
///      `ExternalPackageRegistry` and a worker-local `StringTable`.
pub(super) fn discover_reachable_source_files_provider_free(
    entry_point: &Path,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_packages: &ExternalPackageRegistry,
    prepared_source_store: &PreparedSourceStore,
    directory_import_resolution: DirectoryImportResolution<'_>,
    string_table: &mut StringTable,
) -> Result<ReachableDiscoveryResult, SourceDiscoveryError> {
    let total_start = crate::timing::start_pipeline_timing();

    let mut policy = ImportPolicy::FreeWorker { external_packages };
    let mut source_storage = TraversalSourceStorage::StoreReadOnly {
        store: prepared_source_store,
    };
    let outcome = match traverse_reachable_source_files(
        &[entry_point.to_path_buf()],
        project_path_resolver,
        style_directives,
        &mut policy,
        &mut source_storage,
        Some(directory_import_resolution),
        true,
        string_table,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            log_stage_timing("stage0.reachable_discovery.total", total_start);
            return Err(error);
        }
    };

    // Workers only run when classification proved the project is provider-free, so the worker
    // policy never marks replay required.
    debug_assert!(
        !outcome.provider_capable_required,
        "provider-free worker traversal must not mark provider-capable replay required"
    );

    log_stage_timing("stage0.reachable_discovery.total", total_start);
    Ok(ReachableDiscoveryResult {
        inventory: outcome.inventory,
        resolved_edges: outcome.resolved_edges,
    })
}

fn handle_provider_free_worker_import(
    import_path: &InternedPath,
    canonical_file: &Path,
    external_packages: &ExternalPackageRegistry,
    directory_import_resolution: Option<DirectoryImportResolution<'_>>,
    string_table: &mut StringTable,
) -> Result<ImportPolicyAction, SourceDiscoveryError> {
    if external_packages.is_virtual_package_import(import_path, string_table) {
        if directory_import_resolution.is_some_and(|resolution| {
            resolution.has_binding_package_import(import_path, string_table)
        }) {
            return Ok(ImportPolicyAction::QueueLocal);
        }
        return Ok(ImportPolicyAction::Skip);
    }

    // Defensive check: classification should already have rejected unsupported packages.
    if let Some(package_path) =
        external_packages.unsupported_known_package_import(import_path, string_table)
    {
        return Err(SourceDiscoveryError::from(
            unsupported_builder_package_error(canonical_file, package_path, string_table),
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

/// Resolves a provider import prefix to a canonical filesystem path without appending `.moth` or
/// using public-surface fallback.
///
/// WHAT: reuses the normal base/boundary/case rules from `ProjectPathResolver` but skips the
/// `.moth` extension logic and public-surface fallback used for Moth source imports.
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

/// Test-only entry point that discovers reachable files and returns the `SemanticSourceSet`
/// before it is consumed by `PreparedSourceInput` assembly.
#[cfg(test)]
pub(super) fn discover_semantic_source_set_for_test(
    entry_path: &Path,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_import_resolution: DirectoryImportResolution<'_>,
    string_table: &mut StringTable,
) -> Result<Option<SemanticSourceSet>, CompilerMessages> {
    let discovery = discover_reachable_source_files(
        entry_path,
        project_path_resolver,
        style_directives,
        external_imports,
        None,
        Some(directory_import_resolution),
        string_table,
    )
    .map_err(|error| error.into_messages(string_table))?;

    Ok(discovery.inventory.semantic_source_set)
}
