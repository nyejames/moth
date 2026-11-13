//! Stage 0 source traversal, owned-input preparation and shared structural-provider resolution.
//!
//! Given an entry `.moth` file, the synthetic path walks its dependency clauses transitively to
//! build the consumer-owned source set and retained package-provider edges. Package source stays
//! in its independent Stage 0 boundary. Directory projects prepare
//! owned `SourceId`s through the direct-input helper in this module. Both paths assemble
//! `PreparedSourceInput` values for downstream compilation stages.
//! Stage 0 returns move-only typed failures in `SourceDiscoveryError`: a `DiagnosticDraft`, a
//! producer-captured `PreparedFailure` (including provider imports), a finalized form pairing a
//! `PreparedFailure` with its `SourceDatabase` and an `InfrastructureFailure`. The
//! `source_discovery_error` module owns the reporting rules.

use crate::builder_surface::external_import_providers::cache::ExternalImportCacheKey;
use crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache;
use crate::builder_surface::external_import_providers::provider::{
    ExternalImportProvider, ExternalImportProviderContext, ExternalImportRequest,
};
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::{InvalidDependencyClauseReason, PreparedFailure};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::dependency_target::{
    CheckedExternalProviderTarget, DependencyTargetKind,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, SourcePreparationDelta,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::path_resolution::ResolvedDependencyFile;
use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::project_globals::{
    is_project_globals_dependency, is_project_globals_namespace,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, SourceDatabase, SourceDatabaseBuilder, SourceId, SourceKind,
    SourceRegistrationIndex, SourceSpanBuilders,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
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
use super::module_namespace::{ModuleNamespaceSet, ResolvedDependency};
use super::prepared_source::{PreparedSourceInput, PreparedSourceKind};
use super::resource_inputs::ResourceInputRegistry;
use super::source_discovery_error::SourceDiscoveryError;
use super::source_loading::{SelectedSourceTextMap, read_source_code};
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
#[path = "provider_sources.rs"]
mod provider_sources;

pub(super) use discovery_owned_source::prepare_owned_source_input;
pub(super) use discovery_traversal::resolve_structural_provider_reference;
pub(super) use discovery_traversal::{ReachableTraversalOutcome, discover_reachable_source_files};
#[cfg(test)]
pub(super) use provider_sources::PROVIDER_SOURCE_ID_BASE;
pub(crate) use provider_sources::{BoundaryProviderSources, ProviderOwnedSourceStaging};

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
    source_package_dependencies: Vec<ResolvedSourcePackageDependency>,
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
    pub(super) source_package_dependencies: Vec<ResolvedSourcePackageDependency>,
}
/// Stage 0 boundary error for the single-file tail: the failure plus the finished source
/// database when finalization had already produced one. See `FinalizedDiscoveryFailure`.
#[derive(Debug)]
pub(super) struct CollectReachableInputsError {
    failure: PreparedFailure,
    source_database: Option<Box<SourceDatabase>>,
}
impl CollectReachableInputsError {
    pub(super) fn into_parts(self) -> (PreparedFailure, Option<SourceDatabase>) {
        let Self {
            failure,
            source_database,
        } = self;
        (failure, source_database.map(|database| *database))
    }
}
/// One resolved dependency edge ready for direct insertion into the project module graph.
///
/// WHAT: records that an authored structural provider reference resolved through the
///       boundary-aware namespace from a consumer project module to a provider project module,
///       carrying both `ModuleId` values.
/// WHY: the namespace resolves to boundary-local `ModuleId`s directly, so the graph inserts a
///      provider-before-consumer edge without a path-to-ID mapping step.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedDependencyEdge {
    pub(super) provider_module_id: ModuleId,
    pub(super) provider_surface: crate::compiler_frontend::semantic_identity::ModuleRootRole,
    pub(super) consumer_module_id: ModuleId,
    pub(super) dependency_shell_id: crate::compiler_frontend::symbols::identity::DependencyShellId,
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
/// [`PreparedFailure`] once and attaches the finished source database when present.
#[allow(
    clippy::too_many_arguments,
    reason = "reachable-input collection keeps the entry path, resolver, directives, and mutable external-import/kind/resource/path/string state as separate borrows"
)]
pub(super) fn collect_reachable_input_files(
    entry_path: &Path,
    namespace_set: &ModuleNamespaceSet,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    source_file_kinds: &SourceFileKindRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<CollectedReachableInputs, CollectReachableInputsError> {
    let mut provider_sources = ProviderOwnedSourceStaging::new();
    let discovery = match discover_reachable_source_files(
        entry_path,
        namespace_set,
        project_path_resolver,
        style_directives,
        external_imports,
        &mut provider_sources,
        source_file_kinds,
        resource_inputs,
        path_fork,
        string_table,
    ) {
        Ok(discovery) => discovery,
        Err(SourceDiscoveryError::Finalized(boxed)) => {
            let (failure, source_database) = boxed.into_parts();
            return Err(CollectReachableInputsError {
                failure,
                source_database: Some(Box::new(source_database)),
            });
        }
        Err(other) => {
            let path_table = std::sync::Arc::new(path_fork.shared_table());
            return Err(CollectReachableInputsError {
                failure: other.into_failure(string_table, path_table),
                source_database: None,
            });
        }
    };

    let ReachableTraversalOutcome {
        source_files,
        input_files,
        resolved_file_references,
        source_package_dependencies,
    } = discovery;
    Ok(CollectedReachableInputs {
        source_files,
        input_files,
        resolved_file_references,
        source_package_dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder_surface::{BuilderSurface, SourceFileKind};
    use crate::compiler_frontend::paths::module_roots::ModuleRootTable;

    /// Build the synthetic Stage 0 boundary services for one entry file without running the
    /// semantic pipeline.
    #[allow(
        clippy::too_many_arguments,
        reason = "the synthetic traversal fixture keeps the surface, resolver, namespace set and mutable external-import state separate for the caller"
    )]
    fn synthetic_traversal_fixture(
        entry: &Path,
        surface: &mut BuilderSurface,
        string_table: &mut StringTable,
    ) -> (
        ProjectPathResolver,
        ModuleNamespaceSet,
        PathInternerFork,
        ResourceInputRegistry,
        ExternalImportProviderCache,
        ExternalImportResolutionTable,
        ExternalPackageRegistry,
    ) {
        let entry = fs::canonicalize(entry).expect("entry should canonicalize");
        let source_root = entry
            .parent()
            .expect("entry has a containing directory")
            .to_path_buf();
        let source_package_indexes =
            super::super::source_package_discovery::build_source_package_boundary_indexes(
                &surface.source_packages,
                &surface.source_file_kinds,
                &surface.external_import_providers,
                string_table,
            )
            .expect("source package indexes should build");
        let prepared_roots = source_package_indexes.prepared_source_package_roots();
        let resolver = ProjectPathResolver::new_with_module_roots(
            source_root.clone(),
            source_root,
            prepared_roots,
            &surface.source_file_kinds,
            ModuleRootTable::empty(),
        );
        let namespace_set = ModuleNamespaceSet::for_single_file(
            source_package_indexes,
            &surface.binding_packages,
            &resolver,
        )
        .expect("synthetic namespace set should build");
        (
            resolver,
            namespace_set,
            PathInternerFork::empty(),
            ResourceInputRegistry::new(),
            ExternalImportProviderCache::new(),
            ExternalImportResolutionTable::new(),
            ExternalPackageRegistry::new(),
        )
    }

    #[test]
    fn synthetic_traversal_follows_qualified_occurrence_to_same_module_source() {
        let temp = tempfile::tempdir().expect("should create temporary source root");
        let directory = temp.path();
        let entry = directory.join("entry.moth");
        fs::write(
            &entry,
            "message = @helper answer
",
        )
        .expect("should write entry");
        fs::write(
            directory.join("helper.moth"),
            "answer #= 42
",
        )
        .expect("should write helper");

        let mut surface = BuilderSurface::with_mandatory_core();
        let mut string_table = StringTable::new();
        let (
            resolver,
            namespace_set,
            mut path_fork,
            mut resource_inputs,
            mut cache,
            mut resolution_table,
            mut packages,
        ) = synthetic_traversal_fixture(&entry, &mut surface, &mut string_table);
        let mut external_imports = ExternalImportDiscoveryState {
            external_packages: &mut packages,
            providers: &surface.external_import_providers,
            cache: &mut cache,
            resolution_table: &mut resolution_table,
        };
        let mut provider_sources = ProviderOwnedSourceStaging::new();
        let canonical_entry = fs::canonicalize(&entry).expect("entry should canonicalize");
        let outcome = discover_reachable_source_files(
            &canonical_entry,
            &namespace_set,
            &resolver,
            &StyleDirectiveRegistry::built_ins(),
            &mut external_imports,
            &mut provider_sources,
            &surface.source_file_kinds,
            &mut resource_inputs,
            &mut path_fork,
            &mut string_table,
        )
        .expect("synthetic traversal should follow the qualified occurrence");

        assert_eq!(
            outcome.input_files.len(),
            2,
            "the same-module helper is reachable only through the qualified occurrence"
        );
    }

    #[test]
    fn synthetic_traversal_follows_moth_template_occurrence_to_helper_source() {
        let temp = tempfile::tempdir().expect("should create temporary source root");
        let directory = temp.path();
        let entry = directory.join("entry.moth");
        fs::write(&entry, "title #= @intro.mtf\n").expect("should write entry");
        fs::write(directory.join("intro.mtf"), "# [@settings site_name]\n")
            .expect("should write template");
        fs::write(directory.join("settings.moth"), "site_name #= \"Moth\"\n")
            .expect("should write helper");

        let mut surface = BuilderSurface::with_mandatory_core();
        let mut string_table = StringTable::new();
        let (
            resolver,
            namespace_set,
            mut path_fork,
            mut resource_inputs,
            mut cache,
            mut resolution_table,
            mut packages,
        ) = synthetic_traversal_fixture(&entry, &mut surface, &mut string_table);
        let mut external_imports = ExternalImportDiscoveryState {
            external_packages: &mut packages,
            providers: &surface.external_import_providers,
            cache: &mut cache,
            resolution_table: &mut resolution_table,
        };
        let mut provider_sources = ProviderOwnedSourceStaging::new();
        let canonical_entry = fs::canonicalize(&entry).expect("entry should canonicalize");
        let outcome = discover_reachable_source_files(
            &canonical_entry,
            &namespace_set,
            &resolver,
            &StyleDirectiveRegistry::built_ins(),
            &mut external_imports,
            &mut provider_sources,
            &surface.source_file_kinds,
            &mut resource_inputs,
            &mut path_fork,
            &mut string_table,
        )
        .expect("synthetic traversal should follow the template-head occurrence");

        assert_eq!(
            outcome.input_files.len(),
            3,
            "the template content source and the helper it selects must both be reachable"
        );
    }

    #[test]
    fn synthetic_traversal_follows_moth_occurrence_to_template_content_source() {
        let temp = tempfile::tempdir().expect("should create temporary source root");
        let directory = temp.path();
        let entry = directory.join("entry.moth");
        fs::write(&entry, "message = @intro content\n").expect("should write entry");
        fs::write(directory.join("intro.mtf"), "hello\n").expect("should write template");

        let mut surface = BuilderSurface::with_mandatory_core();
        let mut string_table = StringTable::new();
        let (
            resolver,
            namespace_set,
            mut path_fork,
            mut resource_inputs,
            mut cache,
            mut resolution_table,
            mut packages,
        ) = synthetic_traversal_fixture(&entry, &mut surface, &mut string_table);
        let mut external_imports = ExternalImportDiscoveryState {
            external_packages: &mut packages,
            providers: &surface.external_import_providers,
            cache: &mut cache,
            resolution_table: &mut resolution_table,
        };
        let mut provider_sources = ProviderOwnedSourceStaging::new();
        let canonical_entry = fs::canonicalize(&entry).expect("entry should canonicalize");
        let outcome = discover_reachable_source_files(
            &canonical_entry,
            &namespace_set,
            &resolver,
            &StyleDirectiveRegistry::built_ins(),
            &mut external_imports,
            &mut provider_sources,
            &surface.source_file_kinds,
            &mut resource_inputs,
            &mut path_fork,
            &mut string_table,
        )
        .expect("synthetic traversal should follow the .moth occurrence to the template source");

        assert_eq!(
            outcome.input_files.len(),
            2,
            "the `.mtf` content source is reachable only through the `.moth` occurrence"
        );
    }

    #[test]
    fn deferred_source_infrastructure_failure_dominates_diagnostic_in_both_orders() {
        let temp = tempfile::tempdir().expect("should create temporary source root");
        let invalid_utf8_path = temp.path().join("invalid.md");
        fs::write(&invalid_utf8_path, [0xff]).expect("should write invalid UTF-8 source");
        let directory_path = temp.path().join("directory.md");
        fs::create_dir(&directory_path).expect("should create directory where file is expected");
        let invalid_utf8_path =
            fs::canonicalize(invalid_utf8_path).expect("invalid UTF-8 source should canonicalize");
        let directory_path =
            fs::canonicalize(directory_path).expect("directory source should canonicalize");

        for ordered_paths in [
            [invalid_utf8_path.clone(), directory_path.clone()],
            [directory_path.clone(), invalid_utf8_path.clone()],
        ] {
            let files = ordered_paths
                .into_iter()
                .map(|path| ReachableSourceFile {
                    path,
                    kind: SourceFileKind::PlainMarkdown,
                })
                .collect::<Vec<_>>();
            let registration_index = SourceRegistrationIndex::from_rows(
                files
                    .iter()
                    .map(|file| (file.path.as_path(), SourceKind::Compiler(file.kind))),
            );
            let mut string_table = StringTable::new();
            let source_files = SourceDatabase::from_registration_index_sorted_by_logical_path(
                &registration_index,
                &files[0].path,
                None,
                &mut string_table,
            )
            .expect("deferred sources should register");
            let mut source_builder = SourceDatabaseBuilder::new(source_files);
            let missing_sources = files
                .iter()
                .enumerate()
                .map(|(input_index, source_file)| MissingSourceFile {
                    input_index,
                    source_file: source_file.clone(),
                })
                .collect();
            let load_results = discovery_missing_source_load::load_missing_sources(missing_sources);
            let mut path_fork = source_builder.sources().fork_path_interner();

            let error = discovery_finalization::finalize_missing_source_loads(
                &files,
                load_results,
                &mut source_builder,
                &mut string_table,
                &mut path_fork,
            )
            .expect_err("the deferred directory read should fail");

            assert!(
                matches!(
                    &error,
                    SourceDiscoveryError::Premerge(PreparedFailure::Infrastructure(_))
                ),
                "infrastructure failure should dominate regardless of load order: {error:?}"
            );
        }
    }
}
