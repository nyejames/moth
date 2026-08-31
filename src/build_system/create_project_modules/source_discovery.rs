//! Stage 0 source traversal, owned-input preparation and shared structural-provider resolution.
//!
//! Given an entry `.moth` file, the synthetic path walks its dependency clauses transitively to
//! build the complete set of source files for one single-file module. Directory projects prepare
//! owned `SourceId`s through the direct-input helper in this module. Both paths assemble
//! `PreparedSourceInput` values for downstream compilation stages.
// Stage 0 deliberately returns full diagnostic/infrastructure payloads in `SourceDiscoveryError`
// so dependency discovery does not erase source locations or downgrade filesystem failures.

use crate::builder_surface::external_import_providers::cache::ExternalImportCacheKey;
use crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache;
use crate::builder_surface::external_import_providers::provider::{
    ExternalImportProvider, ExternalImportProviderContext, ExternalImportRequest,
};
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::dependency_target::{
    DependencyTargetKind, decode_dependency_target,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass;
use crate::compiler_frontend::paths::path_normalization::{
    is_relative_dependency_path, join_and_normalize_path,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::path_resolution::ResolvedDependencyFile;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::SourceFileTable;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::{
    StringIdRemap, StringTable, StringTableForkSource,
};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use crate::counter_observation;

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use super::file_reference_resolution::{
    SingleFileReferenceOutcome, SingleFileReferenceResolver, SingleFileResolvedReference,
};
use super::module_identity::ModuleId;
use super::module_namespace::DirectoryDependencyResolution;
use super::prepared_source::PreparedSourceInput;
use super::resource_inputs::ResourceInputRegistry;
use super::source_discovery_error::SourceDiscoveryError;
use super::source_loading::{extract_source_code, read_source_code, source_read_error};
use super::source_preparation::{
    PreparedDiscoverySource, prepare_discovery_source, prepare_discovery_template_source,
};
use super::source_tree_index::{SourceClassification, SourceId, SourceTreeIndex};

/// Minimum cache-miss count before Stage 0 uses Rayon for raw source loading.
///
/// The threshold keeps tiny projects and mostly-cached modules on the cheaper serial path while
/// still letting markdown-heavy modules overlap independent filesystem reads.
pub(super) const STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES: usize = 8;

/// Minimum owned-source batch size before Stage 0 overlaps independent reads and tokenization.
///
/// Tiny directory modules stay on the cheaper iterator path; the directory boundary still keeps
/// provider resolution serial while larger owned-source batches use Rayon for provider-free work.
const STAGE0_PARALLEL_SOURCE_PREPARE_MIN_FILES: usize = 16;

pub(super) fn should_parallelize_owned_source_preparation(source_count: usize) -> bool {
    source_count >= STAGE0_PARALLEL_SOURCE_PREPARE_MIN_FILES
}

/// Mutable external-import state shared across Stage 0 reachable-file discovery.
///
/// WHAT: groups provider metadata, the external package registry, and build-scoped provider
/// cache/table state.
/// WHY: Stage 0 needs to mutate provider results while walking dependencies, but callers should not
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

/// Resolve provider-backed and binding-backed dependency classes before indexed source resolution.
///
/// Directory module scheduling calls this with provider references retained by header syntax.
/// It never scans tokens or source text.
pub(super) fn resolve_structural_provider_reference(
    provider: &RetainedDependencyPath,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_dependency_resolution: DirectoryDependencyResolution<'_>,
    string_table: &mut StringTable,
) -> Result<StructuralProviderAction, SourceDiscoveryError> {
    match handle_provider_capable_dependency(
        ProviderCapableDependencyInput {
            dependency_path: &provider.path,
            dependency_location: &provider.location,
            target: &provider.target,
            canonical_file,
            project_path_resolver,
            directory_dependency_resolution: Some(directory_dependency_resolution),
            string_table,
        },
        external_imports,
    )? {
        DependencyPolicyAction::QueueLocal => Ok(StructuralProviderAction::ResolveSource),
        DependencyPolicyAction::Skip => Ok(StructuralProviderAction::Handled),
    }
}

/// A reachable source file plus the source kind selected by dependency resolution.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ReachableSourceFile {
    pub(super) path: PathBuf,
    pub(super) kind: SourceFileKind,
}

/// Stage 0 inventory for synthetic single-file compilation.
///
/// WHAT: owns the deterministic source closure and the complete retained file output for each
///       reachable tokenized source when no directory-project `SourceTreeIndex` ownership
///       inventory is used.
/// WHY: synthetic discovery prepares each Moth file before resolving its dependencies, then moves that
///      complete output into the module input lane after final identities are known. Directory
///      projects prepare their owned `SourceId`s directly in the module-inventory queue instead.
pub(super) struct ReachableSourceInventory {
    pub(super) files: Vec<ReachableSourceFile>,
    local_source_cache: FxHashMap<PathBuf, PreparedDiscoverySource>,
    pub(super) resolved_file_references: Vec<SingleFileResolvedReference>,
}

/// One resolved dependency edge ready for direct insertion into the project module graph.
///
/// WHAT: records that an authored structural provider reference resolved through the
///       boundary-aware namespace from a consumer project module to a provider project
///       module, carrying both `ModuleId` values and the exact authored dependency-clause
///       `SourceLocation`.
/// WHY: the namespace resolves to boundary-local `ModuleId`s directly, so the graph inserts
///      a provider-before-consumer edge without a path-to-ID mapping step. The authored
///      source location is retained in the graph side table so a later diagnostic owner can
///      attribute the edge to the exact dependency clause without reparsing.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedDependencyEdge {
    pub(super) provider_module_id: ModuleId,
    pub(super) consumer_module_id: ModuleId,
    pub(super) dependency_shell_id: crate::compiler_frontend::symbols::identity::DependencyShellId,
    pub(super) graph_location: SourceLocation,
}

/// One authored dependency from a module to a separately compiled source-package facade.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedSourcePackageDependency {
    pub(super) consumer_module_id: ModuleId,
    pub(super) dependency_prefix: String,
    pub(super) dependency_shell_id: crate::compiler_frontend::symbols::identity::DependencyShellId,
}

/// Reachable discovery output pairing the file inventory with direct dependency edges.
///
/// WHAT: the complete retained inventory plus the project-local `ModuleId` edges observed during
///       one traversal. Both the provider-capable serial path and the provider-free
///       worker path return this so the inventory merge has one shape.
/// WHY: dependency edges are collected at the same local-dependency resolution join as the file
///      inventory, so they share the traversal owner and stay deterministic regardless of which
///      discovery path produced them.
pub(super) struct ReachableDiscoveryResult {
    pub(super) inventory: ReachableSourceInventory,
}

/// Collected reachable inputs for one entry plus the retained dependency edges.
///
/// WHAT: inventory assembly turns the inventory into `PreparedSourceInput`
///       values; direct edges travel alongside so the directory-project graph can record them
///       after discovery.
/// WHY: the single-file flow produces no edges because it has no project module graph, while the
///      directory-project flow retains them for graph insertion.
pub(super) struct CollectedReachableInputs {
    pub(super) input_files: Vec<PreparedSourceInput>,
    pub(super) resolved_file_references: Vec<SingleFileResolvedReference>,
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

/// Mutable traversal outputs shared by the source-dependency queue helpers.
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
    source_file_kinds: &SourceFileKindRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
) -> Result<CollectedReachableInputs, CompilerMessages> {
    // 1. Traverse the dependency graph to find all paths and retained resolved edges.
    let discovery = match discover_reachable_source_files(
        entry_path,
        project_path_resolver,
        style_directives,
        external_imports,
        source_file_kinds,
        resource_inputs,
        string_table,
    ) {
        Ok(discovery) => discovery,
        Err(error) => {
            return Err(error.into_messages(string_table));
        }
    };

    let ReachableSourceInventory {
        files,
        local_source_cache,
        resolved_file_references,
    } = discovery.inventory;
    let input_files = assemble_reachable_files(files, local_source_cache, string_table)?;
    Ok(CollectedReachableInputs {
        input_files,
        resolved_file_references,
    })
}

/// Prepare one owned compiler-semantic `SourceId` directly into the module's input lane.
///
/// WHAT: reads and tokenizes the selected source exactly once, producing the owned input consumed
///       by this module's header preparation queue. The caller's queued set is the ownership
///       proof: a canonical source ID is handed to this function at most once for its owning
///       module.
/// WHY: `SourceTreeIndex` owns source identity and ownership; retaining a second project-wide
///      payload store would duplicate complete source strings and token buffers without a second
///      compiler consumer. The returned input is moved into the module job after its header
///      references have been resolved.
pub(super) fn prepare_owned_source_input(
    source_id: SourceId,
    source_tree_index: &SourceTreeIndex,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
) -> Result<PreparedSourceInput, SourceDiscoveryError> {
    let record = source_tree_index.source(source_id);
    let SourceClassification::CompilerSemantic(source_kind) = record.classification() else {
        return Err(SourceDiscoveryError::from(CompilerError::compiler_error(
            format!(
                "Project source ID {} is not compiler semantic",
                source_id.index(),
            ),
        )));
    };
    let source_code = extract_source_code(record.canonical_path(), string_table)?;
    let source_byte_len = source_code.len();
    let tokens = if *source_kind == SourceFileKind::Moth {
        let interned_path = InternedPath::try_from_filesystem_path(
            record.canonical_path(),
            string_table,
        )
        .map_err(|NonUtf8PathComponent { path }| {
            SourceDiscoveryError::from(CompilerError::file_error(
                &path,
                format!(
                    "Source file path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                ),
                string_table,
            ))
        })?;
        Some(Box::new(
            tokenize(
                &source_code,
                &interned_path,
                TokenizerEntryMode::SourceFile,
                style_directives,
                string_table,
                None,
            )
            .map_err(SourceDiscoveryError::Diagnostic)?,
        ))
    } else {
        None
    };

    Ok(match *source_kind {
        SourceFileKind::Moth => {
            let Some(tokens) = tokens else {
                return Err(SourceDiscoveryError::from(CompilerError::compiler_error(
                    "Moth source preparation completed without a token stream",
                )));
            };
            PreparedSourceInput::Moth {
                source_byte_len,
                source_path: record.canonical_path().to_path_buf(),
                tokens,
            }
        }
        SourceFileKind::MothTemplate => PreparedSourceInput::MothTemplate {
            source_code,
            source_path: record.canonical_path().to_path_buf(),
        },
        SourceFileKind::PlainMarkdown => PreparedSourceInput::PlainMarkdown {
            source_code,
            source_path: record.canonical_path().to_path_buf(),
        },
    })
}

/// One provider-independent source input prepared against a batch-local string-table fork.
///
/// WHAT: retains either a tokenized source input or its source-local failure until the serial
///       reachability walk decides whether the source is semantically reachable.
/// WHY: batching all owned candidates can speculatively read/tokenize unreachable files, but it
///      must not surface their diagnostics or merge their strings into the module unless the
///      existing header-owned BFS reaches that source.
pub(super) struct PreparedOwnedSource {
    string_table: StringTable,
    base_len: usize,
    input: Result<PreparedSourceInput, SourceDiscoveryError>,
}

/// Prepare a module's provider-independent owned-source candidates as one deterministic batch.
///
/// Reads and tokenization touch no provider registry, resolution table or external cache, so a
/// sufficiently large candidate batch can use Rayon. The returned map is consumed by the serial
/// BFS in deterministic reachability order; selected inputs merge their local string delta and
/// remap retained tokens immediately before header preparation. Retained outputs are later sorted
/// by canonical source order before the module handoff. Unreachable candidates are dropped
/// without merging, preserving the existing semantic source set and diagnostic behaviour.
pub(super) fn prepare_owned_source_inputs(
    source_ids: &[SourceId],
    source_tree_index: &SourceTreeIndex,
    style_directives: &StyleDirectiveRegistry,
    fork_source: &StringTableForkSource,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> FxHashMap<SourceId, PreparedOwnedSource> {
    let prepare_source = |&source_id: &SourceId| {
        let fork = fork_source.fork_for_module();
        let (mut string_table, base_len) = fork.into_parts();
        let input = crate::timed_stage_attributed!(
            crate::timing::TimingMetric::FrontendPrepare,
            timing_context,
            prepare_owned_source_input(
                source_id,
                source_tree_index,
                style_directives,
                &mut string_table,
            ),
        );

        (
            source_id,
            PreparedOwnedSource {
                string_table,
                base_len,
                input,
            },
        )
    };

    let prepared_sources = source_ids
        .par_iter()
        .map(prepare_source)
        .collect::<Vec<_>>();

    prepared_sources.into_iter().collect()
}

/// Merge one selected batched input into the module string table before header preparation.
pub(super) fn merge_prepared_owned_source(
    source_id: SourceId,
    prepared_sources: &mut FxHashMap<SourceId, PreparedOwnedSource>,
    string_table: &mut StringTable,
) -> Result<PreparedSourceInput, SourceDiscoveryError> {
    let Some(prepared) = prepared_sources.remove(&source_id) else {
        return Err(SourceDiscoveryError::from(CompilerError::compiler_error(
            format!(
                "Prepared source ID {} is absent from the owned-source batch",
                source_id.index(),
            ),
        )));
    };
    let remap = string_table.merge_delta_from(&prepared.string_table, prepared.base_len);

    match prepared.input {
        Ok(mut input) => {
            remap_prepared_source_input(&mut input, &remap)?;
            Ok(input)
        }
        Err(error) => Err(remap_source_discovery_error(error, &remap, string_table)),
    }
}

fn remap_prepared_source_input(
    input: &mut PreparedSourceInput,
    remap: &StringIdRemap,
) -> Result<(), CompilerError> {
    if let PreparedSourceInput::Moth { tokens, .. } = input {
        tokens.remap_preparing_string_ids(remap)?;
    }

    Ok(())
}

fn remap_source_discovery_error(
    error: SourceDiscoveryError,
    remap: &StringIdRemap,
    string_table: &StringTable,
) -> SourceDiscoveryError {
    match error {
        SourceDiscoveryError::Diagnostic(mut diagnostic) => {
            diagnostic.remap_string_ids(remap);
            SourceDiscoveryError::Diagnostic(diagnostic)
        }
        SourceDiscoveryError::Messages(mut messages) => {
            messages.remap_string_ids(remap);
            messages.string_table = string_table.clone();
            SourceDiscoveryError::Messages(messages)
        }
        SourceDiscoveryError::Infrastructure(mut error) => {
            error.remap_string_ids(remap);
            SourceDiscoveryError::Infrastructure(error)
        }
    }
}

/// Assemble `PreparedSourceInput` values without a semantic set (single-file synthetic path).
fn assemble_reachable_files(
    files: Vec<ReachableSourceFile>,
    mut source_cache: FxHashMap<PathBuf, PreparedDiscoverySource>,
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
    source_cache: &mut FxHashMap<PathBuf, PreparedDiscoverySource>,
    canonical_path: &Path,
    source_kind: SourceFileKind,
    input_index: usize,
    input_slots: &mut [Option<PreparedSourceInput>],
    missing_sources: &mut Vec<MissingSourceFile>,
) {
    if let Some(scanned_source) = source_cache.remove(canonical_path) {
        add_frontend_counter(FrontendCounter::Stage0SourceCacheHitCount, 1);
        let PreparedDiscoverySource {
            prepared_output,
            source_byte_len,
            source_kind,
        } = scanned_source;

        input_slots[input_index] = Some(match source_kind {
            SourceFileKind::Moth => PreparedSourceInput::MothPrepared {
                source_byte_len,
                source_path: canonical_path.to_path_buf(),
                output: Box::new(prepared_output),
            },
            SourceFileKind::MothTemplate => PreparedSourceInput::MothTemplatePrepared {
                source_byte_len,
                source_path: canonical_path.to_path_buf(),
                output: Box::new(prepared_output),
            },
            SourceFileKind::PlainMarkdown => {
                unreachable!("plain Markdown cannot enter the prepared source cache")
            }
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

    let loaded_missing_sources = match load_missing_sources(missing_sources, string_table) {
        Ok(loaded_missing_sources) => loaded_missing_sources,
        Err(messages) => {
            return Err(messages);
        }
    };
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

/// Action a traversal policy wants the shared BFS to take for one dependency path.
enum DependencyPolicyAction {
    /// Do not follow this dependency.
    Skip,
    /// Resolve and queue the dependency as a normal local Moth dependency.
    QueueLocal,
}

/// Stage 0 dependency policy that customizes the shared reachable-file traversal.
///
/// WHAT: the provider-capable path owns external-provider resolution while the shared BFS owns queue
///       handling, canonicalization, source preparation and local queuing.
///
/// The policy only decides dependency actions for the synthetic single-file traversal. Directory
/// projects use indexed discovery and prepare each owned `SourceId` directly in the module queue;
/// the synthetic traversal alone retains its isolated local scan cache until input assembly.
enum DependencyPolicy<'a, 'b> {
    /// Full provider-capable path. Mutates provider cache and resolution tables.
    Capable {
        external_imports: &'a mut ExternalImportDiscoveryState<'b>,
    },
}

struct ProviderCapableDependencyInput<'a> {
    dependency_path: &'a InternedPath,
    dependency_location: &'a SourceLocation,
    target: &'a DependencyTargetKind,
    canonical_file: &'a Path,
    project_path_resolver: &'a ProjectPathResolver,
    directory_dependency_resolution: Option<DirectoryDependencyResolution<'a>>,
    string_table: &'a mut StringTable,
}

impl<'a, 'b> DependencyPolicy<'a, 'b> {
    /// Decide how to handle one dependency path.
    fn handle_dependency(
        &mut self,
        input: ProviderCapableDependencyInput<'_>,
    ) -> Result<DependencyPolicyAction, SourceDiscoveryError> {
        match self {
            DependencyPolicy::Capable {
                external_imports: state,
            } => handle_provider_capable_dependency(input, state),
        }
    }
}

/// Read/preparation accounting for one `.moth` file during traversal.
///
/// The complete `PreparedDiscoverySource` remains in `local_source_cache`; this result deliberately
/// carries no detached clause vector because a retained clause range is meaningful only with its
/// owning file selection table.
struct ScannedMothSource {
    fresh_read: bool,
    source_byte_count: usize,
}

fn scan_and_cache_local_moth_source(
    canonical_file: &Path,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &ProjectPathResolver,
    entry_file_path: &Path,
    source_files: &mut SourceFileTable,
    local_source_cache: &mut FxHashMap<PathBuf, PreparedDiscoverySource>,
    string_table: &mut StringTable,
) -> Result<ScannedMothSource, SourceDiscoveryError> {
    if local_source_cache.contains_key(canonical_file) {
        return Ok(ScannedMothSource {
            fresh_read: false,
            source_byte_count: 0,
        });
    }

    let scanned = prepare_discovery_source(
        canonical_file,
        style_directives,
        &Some(project_path_resolver.clone()),
        entry_file_path,
        source_files,
        string_table,
    )?;
    let source_byte_count = scanned.source_byte_len;
    local_source_cache.insert(canonical_file.to_path_buf(), scanned);

    Ok(ScannedMothSource {
        fresh_read: true,
        source_byte_count,
    })
}

fn scan_and_cache_local_moth_template_source(
    canonical_file: &Path,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &ProjectPathResolver,
    entry_file_path: &Path,
    source_files: &mut SourceFileTable,
    local_source_cache: &mut FxHashMap<PathBuf, PreparedDiscoverySource>,
    string_table: &mut StringTable,
) -> Result<ScannedMothSource, SourceDiscoveryError> {
    if local_source_cache.contains_key(canonical_file) {
        return Ok(ScannedMothSource {
            fresh_read: false,
            source_byte_count: 0,
        });
    }

    let scanned = prepare_discovery_template_source(
        canonical_file,
        style_directives,
        &Some(project_path_resolver.clone()),
        entry_file_path,
        source_files,
        string_table,
    )?;
    let source_byte_count = scanned.source_byte_len;
    local_source_cache.insert(canonical_file.to_path_buf(), scanned);

    Ok(ScannedMothSource {
        fresh_read: true,
        source_byte_count,
    })
}

/// BFS over the synthetic single-file compilation's dependency clauses.
///
/// WHAT: follows each Moth file's declared dependencies, resolves them to canonical typed source
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
    policy: &mut DependencyPolicy<'_, '_>,
    source_file_kinds: &SourceFileKindRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
) -> Result<ReachableTraversalOutcome, SourceDiscoveryError> {
    let mut reachable = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut local_source_cache = FxHashMap::default();
    // Traversal-local source identities: header preparation stamps retained shells from real
    // FileIds, but the full inventory is unknown during the BFS. `prepare_module` later
    // rebuilds the deterministic sorted table and rebinds every token to it.
    let mut traversal_source_files = SourceFileTable::empty();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let mut dependency_clauses_scanned: usize = 0;

    // The entry identity table keys on canonical paths; canonicalize the entry once so
    // header preparation recognises the active root by FileId instead of path text.
    let canonical_entry_path = fs::canonicalize(&entry_paths[0]).map_err(|error| {
        CompilerError::file_error(
            &entry_paths[0],
            format!("Failed to canonicalize entry file path: {error}"),
            string_table,
        )
    })?;
    let root_directory = canonical_entry_path.parent().ok_or_else(|| {
        SourceDiscoveryError::from(CompilerError::compiler_error(
            "canonical synthetic entry path has no containing module root",
        ))
    })?;
    let mut file_reference_resolver = SingleFileReferenceResolver::new(
        root_directory.to_path_buf(),
        source_file_kinds,
        resource_inputs,
    );
    let mut resolved_file_references = Vec::new();

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

        if next_file.kind == SourceFileKind::MothTemplate {
            // Moth template is a Moth template body with a small compile-time scope, so the
            // same-directory root may supply visible constants. Its retained output is scanned
            // here as well, allowing content references to reach a fixed point without a second
            // parse during module preparation.
            queue_same_directory_root_for_moth_template(
                &canonical_file,
                project_path_resolver,
                &reachable,
                &mut queue,
            );
        } else if next_file.kind == SourceFileKind::PlainMarkdown {
            // Markdown files are importless content assets. They are carried forward for
            // header-stage preparation but are never scanned for dependencies.
            continue;
        }

        let scan_result = match next_file.kind {
            SourceFileKind::Moth => scan_and_cache_local_moth_source(
                &canonical_file,
                style_directives,
                project_path_resolver,
                &canonical_entry_path,
                &mut traversal_source_files,
                &mut local_source_cache,
                string_table,
            ),
            SourceFileKind::MothTemplate => scan_and_cache_local_moth_template_source(
                &canonical_file,
                style_directives,
                project_path_resolver,
                &canonical_entry_path,
                &mut traversal_source_files,
                &mut local_source_cache,
                string_table,
            ),
            SourceFileKind::PlainMarkdown => unreachable!(),
        };
        let scanned = match scan_result {
            Ok(scanned) => scanned,
            Err(error) => {
                return Err(error);
            }
        };

        if scanned.fresh_read {
            add_frontend_counter(
                FrontendCounter::Stage0SourceBytesLoaded,
                scanned.source_byte_count,
            );
        }

        let dependency_clauses = &local_source_cache
            .get(&canonical_file)
            .expect("fresh or cached Moth source must remain in the complete source cache")
            .prepared_output
            .file_dependency_clauses;
        #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
        {
            dependency_clauses_scanned += dependency_clauses.len();
        }

        for clause in dependency_clauses {
            // Stage 0 resolves each retained clause root once. Direct selections remain binding
            // facts inside the completed provider surface and never become independent source
            // paths during discovery.
            let provider = &clause.dependency;
            let dependency_path = &provider.path;
            let action = policy.handle_dependency(ProviderCapableDependencyInput {
                dependency_path,
                dependency_location: &provider.location,
                target: &provider.target,
                canonical_file: &canonical_file,
                project_path_resolver,
                directory_dependency_resolution: None,
                string_table,
            })?;

            match action {
                DependencyPolicyAction::Skip => continue,
                DependencyPolicyAction::QueueLocal => {
                    let mut reachable_queue = ReachableQueue {
                        reachable: &reachable,
                        queue: &mut queue,
                    };
                    let result = resolve_and_queue_local_dependency(
                        provider,
                        &canonical_file,
                        project_path_resolver,
                        string_table,
                        &mut reachable_queue,
                    );
                    result?;
                }
            }
        }

        // Structural file references are already classified by preparation. Resolve every
        // occurrence through the same physical resolver used by directory modules, then queue
        // supported content targets so discovery reaches the complete content closure.
        let prepared_path_syntax = &local_source_cache
            .get(&canonical_file)
            .expect("fresh or cached Moth source must remain in the complete source cache")
            .prepared_output
            .path_syntax;
        let structural_file_references = local_source_cache
            .get(&canonical_file)
            .expect("fresh or cached Moth source must remain in the complete source cache")
            .prepared_output
            .structural_file_references
            .references()
            .to_vec();
        for reference in structural_file_references {
            let resolved = file_reference_resolver
                .resolve(
                    &canonical_file,
                    prepared_path_syntax.table(),
                    &reference,
                    string_table,
                )
                .map_err(SourceDiscoveryError::from)?;
            if let SingleFileReferenceOutcome::Source { canonical } = &resolved.outcome
                && resolved.class == PreparedFileReferenceClass::ContentSource
            {
                let extension = canonical
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default();
                let Some(kind) = source_file_kinds.kind_for_extension(extension) else {
                    return Err(SourceDiscoveryError::from(CompilerError::compiler_error(
                        "resolved supported content target has no registered source kind",
                    )));
                };
                let content_file = ReachableSourceFile {
                    path: canonical.clone(),
                    kind,
                };
                if !reachable.contains(&content_file) {
                    queue.push_back(content_file);
                }
            }
            resolved_file_references.push(resolved);
        }
    }

    // Record concise counters for the completed traversal. Counters are only
    // recorded when `benchmark_counters` is active, and reach stdout only when
    // `MOTH_COUNTERS` requests it (summary/full).
    counter_observation!(
        "stage0.reachable_discovery.reachable_files",
        reachable.len() as f64,
    );
    counter_observation!(
        "stage0.reachable_discovery.dependency_clauses_scanned",
        dependency_clauses_scanned as f64,
    );

    Ok(ReachableTraversalOutcome {
        inventory: ReachableSourceInventory {
            files: reachable.into_iter().collect(),
            local_source_cache,
            resolved_file_references,
        },
    })
}

/// BFS over dependency clauses starting from `entry_point`, preserving source kind.
///
/// WHAT: follows each Moth file's declared dependencies, resolves them to canonical typed source
/// files, and returns the full ordered set of files reachable from the entry point.
/// WHY: source kind belongs to Stage 0 input discovery. Builder-supported content assets can be
///      loaded and carried forward without being treated as Moth module roots.
pub(super) fn discover_reachable_source_files(
    entry_point: &Path,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    source_file_kinds: &SourceFileKindRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
) -> Result<ReachableDiscoveryResult, SourceDiscoveryError> {
    let mut policy = DependencyPolicy::Capable { external_imports };

    let outcome = traverse_reachable_source_files(
        &[entry_point.to_path_buf()],
        project_path_resolver,
        style_directives,
        &mut policy,
        source_file_kinds,
        resource_inputs,
        string_table,
    )?;

    Ok(ReachableDiscoveryResult {
        inventory: outcome.inventory,
    })
}

/// Resolve a compiler-semantic Moth dependency and enqueue its indexed or synthetic-file target.
///
/// WHAT: handles cross-module root queuing, implementation-file discovery and direct dependency
///       edge retention for a dependency that is not provider-backed or a virtual package dependency.
/// WHY: one owner keeps indexed resolution, same-module queuing and graph-edge retention aligned.
///      A graph edge is retained only when indexed resolution crosses project module roots.
fn resolve_and_queue_local_dependency(
    provider: &RetainedDependencyPath,
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

/// Resolve a compiler-semantic dependency through the filesystem-backed resolver for single-file
/// synthetic compilation.
///
/// Single-file compilation has no directory source index or project module graph, so ordinary bare
/// source clauses use the prepared owning-module-root table while relative and registered-package
/// paths use the normal filesystem resolver. No dependency edges are collected because there is
/// no project module graph to populate.
fn resolve_and_queue_via_filesystem(
    provider: &RetainedDependencyPath,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
    reachable_queue: &mut ReachableQueue<'_>,
) -> Result<(), SourceDiscoveryError> {
    let resolved = if is_relative_dependency_path(&provider.path, string_table)
        || project_path_resolver
            .source_package_root_for_dependency(&provider.path, string_table)
            .is_some()
    {
        project_path_resolver
            .resolve_dependency_to_source_file(&provider.path, canonical_file, string_table)
            .map_err(SourceDiscoveryError::from)?
    } else {
        match resolve_module_root_bare_dependency(
            &provider.path,
            canonical_file,
            project_path_resolver,
            string_table,
        )? {
            Some(resolved) => resolved,
            None => project_path_resolver
                .resolve_dependency_to_source_file(&provider.path, canonical_file, string_table)
                .map_err(SourceDiscoveryError::from)?,
        }
    };

    // Extensionless source clauses bind to a resolved source file in the synthetic traversal.
    add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
    let resolved_source_file = resolved_source_file(&resolved.path, resolved.kind);
    if !reachable_queue.reachable.contains(&resolved_source_file) {
        reachable_queue.queue.push_back(resolved_source_file);
    }

    Ok(())
}

/// Resolve a bare dependency from the retained module-root topology before entry-root lookup.
///
/// WHAT: maps a module path to its prepared root facade, or resolves a bare dependency from the
///      declaring file's owning module root.
/// WHY: synthetic discovery has no indexed namespace, but it must still implement the same
///      module-root-relative source contract as directory discovery without allowing an entry-root
///      namesake to shadow the declaring module's source.
fn resolve_module_root_bare_dependency(
    provider: &InternedPath,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<Option<ResolvedDependencyFile>, SourceDiscoveryError> {
    let Some(first_component) = provider.as_components().first() else {
        return Ok(None);
    };
    let first_segment = string_table.resolve(*first_component);
    if matches!(first_segment, "." | "..") {
        return Ok(None);
    }

    let module_root = project_path_resolver.module_root_for_file(canonical_file);
    let Some(module_root) = module_root else {
        return Ok(None);
    };

    let root_candidate = join_and_normalize_path(&module_root, provider, string_table);
    if let Some(root_file) = project_path_resolver.module_root_file_for_directory(&root_candidate) {
        return Ok(Some(ResolvedDependencyFile {
            path: root_file,
            kind: SourceFileKind::Moth,
        }));
    }

    if module_root == project_path_resolver.entry_root() {
        return Ok(None);
    }

    let module_prefix = module_root
        .strip_prefix(project_path_resolver.entry_root())
        .ok()
        .ok_or_else(|| {
            SourceDiscoveryError::from(CompilerError::compiler_error(format!(
                "Owning module root '{}' is outside entry root '{}'",
                module_root.display(),
                project_path_resolver.entry_root().display()
            )))
        })?;
    let module_prefix = InternedPath::try_from_filesystem_path(module_prefix, string_table)
        .map_err(|NonUtf8PathComponent { path }| {
            SourceDiscoveryError::from(CompilerError::file_error(
                &path,
                format!(
                    "Owning module path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                ),
                string_table,
            ))
        })?;
    let mut module_local_components = module_prefix.as_components().to_vec();
    module_local_components.extend_from_slice(provider.as_components());
    let module_local_provider = InternedPath::from_components(module_local_components);

    project_path_resolver
        .resolve_dependency_to_source_file(&module_local_provider, canonical_file, string_table)
        .map(Some)
        .map_err(SourceDiscoveryError::from)
}

fn handle_provider_capable_dependency(
    input: ProviderCapableDependencyInput<'_>,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
) -> Result<DependencyPolicyAction, SourceDiscoveryError> {
    let ProviderCapableDependencyInput {
        dependency_path,
        dependency_location,
        target,
        canonical_file,
        project_path_resolver,
        directory_dependency_resolution,
        string_table,
    } = input;
    // Skip virtual package dependencies — AST resolution handles those.
    if external_imports
        .external_packages
        .is_virtual_package_dependency(dependency_path, string_table)
    {
        if directory_dependency_resolution.is_some_and(|resolution| {
            resolution.has_binding_package_dependency(dependency_path, string_table)
        }) {
            return Ok(DependencyPolicyAction::QueueLocal);
        }
        // Extensionless binding-package clauses bind through the external package registry.
        add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
        return Ok(DependencyPolicyAction::Skip);
    }

    // Check for unsupported builder-specific core packages.
    if let Some(package_path) = external_imports
        .external_packages
        .unsupported_known_package_dependency(dependency_path, string_table)
    {
        return Err(SourceDiscoveryError::from(
            unsupported_builder_package_error(canonical_file, package_path, string_table),
        ));
    }

    // Consume the retained provider classification. Header syntax already identified the
    // first explicit non-source extension, so Stage 0 must not rescan path components.
    if let Some(decoded) = decode_dependency_target(dependency_path, target, string_table)
        .map_err(SourceDiscoveryError::from)?
    {
        let prefix_path = decoded.prefix_path();
        let prefix_str = prefix_path.to_portable_string(string_table);
        let extension = decoded.extension_spelling().to_owned();
        if let Some(provider) = external_imports.providers.find_by_extension(&extension) {
            let result = resolve_provider_backed_import(
                ProviderBackedImportRequest {
                    consumer_canonical_path: canonical_file,
                    import_path: dependency_path,
                    import_location: dependency_location,
                    prefix_path: &prefix_path,
                    raw_prefix: &prefix_str,
                    provider,
                    project_path_resolver,
                    directory_dependency_resolution,
                },
                external_imports,
                string_table,
            );
            result?;
            counter_observation!("stage0.reachable_discovery.provider_imports", 1.0);
            // Explicit-extension registered-provider clauses bind through the provider registry.
            add_frontend_counter(FrontendCounter::ResolvedProviderClauseCount, 1);
            return Ok(DependencyPolicyAction::Skip);
        }

        // No provider registered for this extension — report unsupported extension.
        return Err(SourceDiscoveryError::from(
            unsupported_external_extension_error(
                canonical_file,
                dependency_path,
                &extension,
                string_table,
            ),
        ));
    }

    Ok(DependencyPolicyAction::QueueLocal)
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

struct ProviderBackedImportRequest<'a> {
    consumer_canonical_path: &'a Path,
    import_path: &'a InternedPath,
    import_location: &'a SourceLocation,
    prefix_path: &'a InternedPath,
    raw_prefix: &'a str,
    provider: &'a std::sync::Arc<dyn ExternalImportProvider>,
    project_path_resolver: &'a ProjectPathResolver,
    directory_dependency_resolution: Option<DirectoryDependencyResolution<'a>>,
}

/// Resolves a provider-backed import prefix to a canonical filesystem path, checks the build cache,
/// calls the provider if needed, and records the result in the resolution table and package registry.
fn resolve_provider_backed_import(
    request: ProviderBackedImportRequest<'_>,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    string_table: &mut StringTable,
) -> Result<(), SourceDiscoveryError> {
    // Directory projects resolve provider-owned targets through the same boundary-aware
    // namespace as compiler-semantic dependencies. Single-file synthetic compilation retains its
    // separate filesystem-backed resolver.
    let canonical_source_path = match request.directory_dependency_resolution {
        Some(resolution) => resolution
            .resolve_provider_target(
                request.prefix_path,
                request.consumer_canonical_path,
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
        request.consumer_canonical_path,
        request.project_path_resolver,
        string_table,
    )?;

    // Enforce module/package boundaries for provider-backed imports.
    check_provider_dependency_module_boundary(
        request.consumer_canonical_path,
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
            request.consumer_canonical_path,
            request.project_path_resolver,
            string_table,
        )?;
        external_imports.resolution_table.insert(
            source_file_logical,
            request.raw_prefix,
            cached.clone(),
        );
        return Ok(());
    }

    let provider_request = ExternalImportRequest {
        import_path: request.import_path.to_portable_string(string_table),
        canonical_source_path: canonical_source_path.clone(),
        source_location:
            crate::compiler_frontend::compiler_messages::source_location::SourceLocation::from_path(
                request.consumer_canonical_path,
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
            request.consumer_canonical_path,
            request.project_path_resolver,
            string_table,
        )?;
        external_imports
            .resolution_table
            .insert(source_file_logical, request.raw_prefix, resolved);
    }

    Ok(())
}

/// Resolves a provider import prefix to a canonical filesystem path without selecting a compiler
/// source extension candidate.
///
/// WHAT: reuses the normal base/boundary/case rules from `ProjectPathResolver` but skips the
/// extension candidate selection used by isolated compiler-source resolution.
fn resolve_provider_prefix_to_canonical_path(
    prefix_path: &InternedPath,
    declaring_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<PathBuf, SourceDiscoveryError> {
    let (base_kind, filesystem_base) = if let Some(package_root) =
        project_path_resolver.source_package_root_for_dependency(prefix_path, string_table)
    {
        (
            crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase::SourcePackageRoot,
            package_root,
        )
    } else {
        // Dependency paths are module-root-relative. Synthetic traversal has no indexed module
        // namespace, so it derives the same owning boundary from the resolver's retained module
        // roots and uses the entry root only for the default module.
        let module_root = project_path_resolver
            .module_root_for_file(declaring_file)
            .unwrap_or_else(|| project_path_resolver.entry_root().to_path_buf());
        (
            crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase::EntryRoot,
            module_root,
        )
    };

    let normalized = join_and_normalize_path(&filesystem_base, prefix_path, string_table);

    let canonical = fs::canonicalize(&normalized)
        .map_err(|error| {
            CompilerError::file_error(
                declaring_file,
                format!(
                    "Failed to canonicalize external import prefix '{}': {error}",
                    normalized.display()
                ),
                string_table,
            )
        })
        .map_err(SourceDiscoveryError::from)?;

    crate::compiler_frontend::paths::dependency_resolution::validate_dependency_boundary(
        &canonical,
        &base_kind,
        &filesystem_base,
        prefix_path,
        declaring_file,
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

/// Enforce that a provider-backed dependency does not cross a module or source-backed package boundary.
///
/// WHAT: .js files are private implementation details of the module or package that owns them.
///       Cross-module or cross-package .js dependencies bypass the public surface and are rejected.
/// WHY: provider-backed dependencies must obey the same visibility boundaries as .moth source dependencies.
fn check_provider_dependency_module_boundary(
    declaring_file: &Path,
    target_file: &Path,
    dependency_path: &InternedPath,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<(), SourceDiscoveryError> {
    let consumer_container = provider_dependency_container(project_path_resolver, declaring_file);
    let target_container = provider_dependency_container(project_path_resolver, target_file);

    if consumer_container != target_container {
        let location = SourceLocation::from_path(declaring_file, string_table);
        return Err(SourceDiscoveryError::from(
            CompilerDiagnostic::cross_module_import_not_exported(dependency_path.clone(), location),
        ));
    }

    Ok(())
}

/// Determine the boundary "container" of a file for provider import checks.
///
/// WHAT: returns the module root, source-backed package root, or entry root that contains the file.
/// WHY: two files in the same container may freely import each other's .js files.
fn provider_dependency_container(
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
    consumer_file: &Path,
    package_path: &str,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let package_path_id = string_table.intern(package_path);
    let location =
        crate::compiler_frontend::compiler_messages::source_location::SourceLocation::from_path(
            consumer_file,
            string_table,
        );
    CompilerDiagnostic::unsupported_builder_package(package_path_id, location)
}

fn unsupported_external_extension_error(
    consumer_file: &Path,
    import_path: &InternedPath,
    extension: &str,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let extension_id = string_table.intern(extension);
    let location =
        crate::compiler_frontend::compiler_messages::source_location::SourceLocation::from_path(
            consumer_file,
            string_table,
        );
    CompilerDiagnostic::unsupported_external_extension(import_path.clone(), extension_id, location)
}
