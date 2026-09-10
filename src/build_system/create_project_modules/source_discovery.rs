//! Stage 0 source traversal, owned-input preparation and shared structural-provider resolution.
//!
//! Given an entry `.moth` file, the synthetic path walks its dependency clauses transitively to
//! build the complete set of source files for one single-file module. Directory projects prepare
//! owned `SourceId`s through the direct-input helper in this module. Both paths assemble
//! `PreparedSourceInput` values for downstream compilation stages.
//! Stage 0 returns move-only typed failures in `SourceDiscoveryError`: a plain `CompilerDiagnostic`
//! for diagnosed input and a typed `CompilerError` for infrastructure failures, plus a finalized
//! form pairing a `PremergeFailure` with its finished `SourceDatabase`. Production discovery never
//! constructs `CompilerMessages`; the parent single-file/test tail owns that single conversion.

use crate::builder_surface::external_import_providers::cache::ExternalImportCacheKey;
use crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache;
use crate::builder_surface::external_import_providers::provider::{
    ExternalImportProvider, ExternalImportProviderContext, ExternalImportRequest,
};
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DependencyClauseKind, InvalidDependencyClauseReason,
    PremergeDiagnosticBatch, PremergeFailure,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::dependency_target::{
    DependencyTargetKind, decode_dependency_target,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, SourcePreparationDelta,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass;
use crate::compiler_frontend::paths::path_normalization::{
    is_relative_dependency_path, join_and_normalize_path,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::path_resolution::ResolvedDependencyFile;
use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::project_globals::{
    is_project_globals_dependency, is_project_globals_namespace,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, SourceDatabase, SourceDatabaseBuilder, SourceId, SourceKind,
    SourceRegistrationIndex, SourceSpan, SourceSpanBuilders,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::{TokenizeFailure, tokenize};
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
use super::prepared_source::{PreparedSourceInput, PreparedSourceKind};
use super::resource_inputs::ResourceInputRegistry;
use super::source_discovery_error::SourceDiscoveryError;
use super::source_loading::{SelectedSourceTextMap, read_source_code, source_read_error};
use super::source_preparation::{
    PreparedDiscoverySource, prepare_discovery_source, prepare_discovery_template_source,
};
use super::source_tree_index::{SourceClassification, SourceRecordIndex, SourceTreeIndex};
#[path = "discovery_finalization.rs"]
mod discovery_finalization;
#[path = "discovery_identity_rebind.rs"]
mod discovery_identity_rebind;
#[path = "discovery_missing_source_load.rs"]
mod discovery_missing_source_load;
#[path = "discovery_owned_source.rs"]
mod discovery_owned_source;
#[path = "discovery_provider_imports.rs"]
mod discovery_provider_imports;
#[path = "discovery_traversal.rs"]
mod discovery_traversal;

pub(super) use discovery_owned_source::prepare_owned_source_input;
pub(super) use discovery_traversal::resolve_structural_provider_reference;
pub(super) use discovery_traversal::{ReachableTraversalOutcome, discover_reachable_source_files};

/// Minimum cache-miss count before Stage 0 uses Rayon for raw source loading.
///
/// The threshold keeps tiny projects and mostly-cached modules on the cheaper serial path while
/// still letting markdown-heavy modules overlap independent filesystem reads.
pub(super) const STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES: usize = 8;

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

/// A reachable source file plus the source kind selected by dependency resolution.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ReachableSourceFile {
    pub(super) path: PathBuf,
    pub(super) kind: SourceFileKind,
}

/// Private source ownership retained across every discovery exit.
struct ReachableSourceInventory {
    reachable: BTreeSet<ReachableSourceFile>,
    queue: VecDeque<ReachableSourceFile>,
    local_source_cache: FxHashMap<PathBuf, PreparedDiscoverySource>,
    traversal_source_files: SourceDatabase,
    resolved_file_references: Vec<SingleFileResolvedReference>,
}

struct DiscoveryWalkContext<'a> {
    canonical_entry_path: &'a Path,
    project_path_resolver: &'a ProjectPathResolver,
    style_directives: &'a StyleDirectiveRegistry,
    source_file_kinds: &'a SourceFileKindRegistry,
}

enum DiscoveryWalkOutcome {
    Complete,
    PreparationFailed,
}

struct TraversalFailure {
    error: SourceDiscoveryError,
}

/// Collected reachable inputs for one entry plus the live final source owner.
///
/// WHAT: discovery owns the provisional traversal table, builds the final canonically ordered
///       table, moves every retained snapshot and original builder into its final owner and
///       returns only final-domain `SourceId` inputs.
/// WHY: no later stage should need to join a prepared input or retained source text by path.
pub(super) struct CollectedReachableInputs {
    pub(super) source_files: SourceDatabaseBuilder,
    pub(super) input_files: Vec<PreparedSourceInput>,
    pub(super) resolved_file_references: Vec<SingleFileResolvedReference>,
}
/// Stage 0 boundary error preserving the finished source owner for the final tail.
///
/// WHAT: carries the typed premerge failure plus the finished source database when
///       discovery finalization already finalized its owner before failing. Plain
///       pre-finalization failures carry `None` because no finished owner exists yet.
/// WHY: the single-file final boundary converts the inner failure once and attaches the
///      finished database as the render source context, instead of dropping snapshots on
///      a diagnosed discovery failure.
#[derive(Debug)]
pub(super) struct CollectReachableInputsError {
    failure: Box<PremergeFailure>,
    source_database: Option<Box<SourceDatabase>>,
}
impl CollectReachableInputsError {
    pub(super) fn into_parts(self) -> (PremergeFailure, Option<SourceDatabase>) {
        let Self {
            failure,
            source_database,
        } = self;
        (*failure, source_database.map(|database| *database))
    }
}
/// One resolved dependency edge ready for direct insertion into the project module graph.
///
/// WHAT: records that an authored structural provider reference resolved through the
///       boundary-aware namespace from a consumer project module to a provider project
///       module, carrying both `ModuleId` values and the exact authored dependency-clause
///       span.
/// WHY: the namespace resolves to boundary-local `ModuleId`s directly, so the graph inserts a
///      provider-before-consumer edge without a path-to-ID mapping step. The authored source
///      span is retained in the graph side table so a later diagnostic owner can attribute the
///      edge to the exact dependency clause without reparsing.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedDependencyEdge {
    pub(super) provider_module_id: ModuleId,
    pub(super) consumer_module_id: ModuleId,
    pub(super) dependency_shell_id: crate::compiler_frontend::symbols::identity::DependencyShellId,
    pub(super) graph_span: Option<SourceSpan>,
}

/// One authored dependency from a module to a separately compiled source-package facade.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedSourcePackageDependency {
    pub(super) consumer_module_id: ModuleId,
    pub(super) dependency_prefix: String,
    pub(super) dependency_shell_id: crate::compiler_frontend::symbols::identity::DependencyShellId,
}

/// Mutable traversal outputs shared by the source-dependency queue helpers.
struct ReachableQueue<'a> {
    reachable: &'a BTreeSet<ReachableSourceFile>,
    queue: &'a mut VecDeque<ReachableSourceFile>,
}
struct MissingSourceFile {
    input_index: usize,
    source_file: ReachableSourceFile,
}

struct LoadedMissingSourceFile {
    input_index: usize,
    source_code: String,
}

struct SourceReadFailure {
    input_index: usize,
    path: PathBuf,
    error: std::io::Error,
}

enum MissingSourceLoadResult {
    Loaded(LoadedMissingSourceFile),
    Failed(SourceReadFailure),
}

fn missing_source_load_input_index(result: &MissingSourceLoadResult) -> usize {
    match result {
        MissingSourceLoadResult::Loaded(loaded) => loaded.input_index,
        MissingSourceLoadResult::Failed(failure) => failure.input_index,
    }
}

// -------------------------
//  Public API
// -------------------------

/// Collect all reachable source files for a given entry point and load their content.
///
/// Failures travel as [`CollectReachableInputsError`]; the final boundary converts the inner
/// [`PremergeFailure`] once and attaches the finished source database when present.
pub(super) fn collect_reachable_input_files(
    entry_path: &Path,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    source_file_kinds: &SourceFileKindRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
) -> Result<CollectedReachableInputs, CollectReachableInputsError> {
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
        Err(SourceDiscoveryError::Finalized(boxed)) => {
            let (failure, source_database) = boxed.into_parts();
            return Err(CollectReachableInputsError {
                failure: Box::new(failure),
                source_database: Some(Box::new(source_database)),
            });
        }
        Err(other) => {
            return Err(CollectReachableInputsError {
                failure: Box::new(other.into_failure(string_table)),
                source_database: None,
            });
        }
    };

    let ReachableTraversalOutcome {
        source_files,
        input_files,
        resolved_file_references,
    } = discovery;
    Ok(CollectedReachableInputs {
        source_files,
        input_files,
        resolved_file_references,
    })
}
