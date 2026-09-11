use super::compiled_boundary::{
    CompiledGraphBoundary, CompiledSourcePackage, CompletedSourcePackageRegistry,
};
use super::file_reference_resolution::SingleFileReferenceOutcome;
use super::generated_store::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::ModuleArtifactStore;
use super::module_identity::ModuleId;
use super::prepared_source::{PreparedSourceInput, PreparedSourceKind};
use super::project_module_graph::ProjectModuleGraph;
use super::resource_inputs::ResourceInputRegistry;
use super::source_discovery::{ResolvedDependencyEdge, ResolvedSourcePackageDependency};
use super::*;
use crate::build_system::build::BackendBuilder;
use crate::build_system::create_project_modules::module_namespace::{
    DirectoryDependencyResolution, ModuleNamespaceSet, ResolvedDependency,
};
use crate::build_system::create_project_modules::resolve_project_entry_root;
use crate::build_system::create_project_modules::source_package_discovery::build_source_package_boundary_indexes;
use crate::build_system::project_config::{
    ProjectConfigParseServices, compile_project_config_file, load_project_config,
};
use crate::builder_surface::external_import_providers::provider::{
    ExternalFileExtension, ExternalImportProvider, ExternalImportProviderContext,
    ExternalImportProviderKind, ExternalImportRequest, RequiredRuntimeImport,
    ResolvedExternalImport, RuntimeAssetIdentity,
};
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::{PackageOrigin, SourceFileKind};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DependencyClauseKind, DiagnosticCategory,
    DiagnosticPayload, InvalidAssignmentTargetReason, InvalidCompileTimePathReason,
    InvalidConfigReason, InvalidDependencyClauseReason, InvalidOutputFolderReason, PathKind,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::{ExternalFunctionId, ExternalTypeId};
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::parse_file_headers::FileRole;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::module_compilation::artefact::{
    ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts,
};
use crate::compiler_frontend::module_compilation::{CompiledModuleArtifact, Module};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReferenceOutcome, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceDatabaseBuilder, SourceId, SourceKind,
    SourceRegistrationIndex, SourceSpan,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "create_project_modules_benchmark_tests.rs"]
mod create_project_modules_benchmark_tests;
#[path = "create_project_modules_config_tests.rs"]
mod create_project_modules_config_tests;
#[path = "create_project_modules_discovery_tests.rs"]
mod create_project_modules_discovery_tests;
#[path = "create_project_modules_phase4_tests.rs"]
mod create_project_modules_phase4_tests;
#[path = "create_project_modules_phase5b_tests.rs"]
mod create_project_modules_phase5b_tests;
#[path = "create_project_modules_r5c2a_tests.rs"]
mod create_project_modules_r5c2a_tests;
#[path = "create_project_modules_source_discovery_tests.rs"]
mod create_project_modules_source_discovery_tests;
#[path = "create_project_modules_source_tree_index_tests.rs"]
mod create_project_modules_source_tree_index_tests;
#[path = "create_project_modules_synthetic_tests.rs"]
mod create_project_modules_synthetic_tests;
#[path = "create_project_modules_file_reference_tests.rs"]
mod file_reference_resolution_tests;

/// Serializes tests that reset and read the process-global source-read path counters.
///
/// WHY: source-read counting uses one global atomic and one global tracked-prefix slot. Parallel
/// test execution would otherwise let one test's reset/prefix overwrite another's mid-snapshot, so
/// every test that asserts on per-path read counts holds this lock for its whole window.
///
/// It delegates to the one facade-owned instrumentation lock because two of these tests also open
/// a timing collection session. A private lock would serialize them against each other but not
/// against the collector's other owners, so their session start could find the collector busy.
/// One fence also means there is no lock ordering to get wrong.
#[cfg(test)]
fn lock_source_read_counter_tests() -> std::sync::MutexGuard<'static, ()> {
    crate::timing::lock_instrumentation_tests()
}

/// Names the final component of a discovered module-root directory.
///
/// WHY: `unwrap_or_default` turned a rootless or non-UTF-8 directory into an empty name, so a
/// discovery defect could be compared against an authored name and silently mismatch or, worse,
/// match another empty entry. An asserted path component fails loudly instead.
#[track_caller]
fn root_directory_name(path: &Path) -> &str {
    let name = path
        .file_name()
        .unwrap_or_else(|| panic!("module root {path:?} should have a final component"));

    name.to_str()
        .unwrap_or_else(|| panic!("module root name {name:?} should be valid UTF-8"))
}

fn configured_resolver(config: &Config) -> ProjectPathResolver {
    configured_resolver_with_source_file_kinds(
        config,
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
}

fn configured_resolver_with_source_file_kinds(
    config: &Config,
    source_file_kinds: &crate::builder_surface::SourceFileKindRegistry,
) -> ProjectPathResolver {
    // WHAT: rebuilds the same canonical resolver the real project build uses.
    // WHY: module-discovery tests should exercise the exact path rules used in production.
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let mut index_string_table = StringTable::new();
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root.clone(),
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        source_file_kinds,
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut index_string_table,
    )
    .expect("source tree index should build");

    ProjectPathResolver::new_with_module_roots(
        project_root,
        entry_root,
        PreparedSourcePackageRoots::default(),
        source_file_kinds,
        source_tree_index
            .module_identities()
            .derive_compilation_root_table(),
    )
    .expect("project path resolver should build")
}

fn test_style_directives() -> StyleDirectiveRegistry {
    StyleDirectiveRegistry::built_ins()
}

fn source_database_for_test(
    source_tree_index: &super::source_tree_index::SourceTreeIndex,
    resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> SourceDatabase {
    let registration_index = source_tree_index.source_registration_index();
    SourceDatabase::from_ordered_registration_index(
        &registration_index,
        resolver.entry_root(),
        Some(resolver),
        string_table,
    )
    .expect("test source database should build from the indexed canonical inventory")
}

const STAGE0_PARALLEL_SOURCE_PREPARE_MIN_FILES: usize = 16;

fn should_parallelize_owned_source_preparation(source_count: usize) -> bool {
    source_count >= STAGE0_PARALLEL_SOURCE_PREPARE_MIN_FILES
}

fn load_missing_source_path_for_test(
    source_path: PathBuf,
    _source_kind: SourceFileKind,
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    match super::source_loading::read_source_code(&source_path) {
        Ok(_) => Ok(()),
        Err(error) => {
            let error = super::source_loading::source_read_error(&source_path, error);
            Err(CompilerMessages::from_error_ref(error, string_table))
        }
    }
}

fn load_missing_source_paths_for_test(
    source_paths: Vec<PathBuf>,
    source_kind: SourceFileKind,
    string_table: &mut StringTable,
) -> Result<(SourceDatabase, Vec<PreparedSourceInput>), CompilerMessages> {
    let canonical_paths = source_paths
        .into_iter()
        .map(|path| {
            fs::canonicalize(&path).map_err(|error| {
                CompilerMessages::from_error_ref(
                    CompilerError::file_error(
                        &path,
                        format!("failed to canonicalize test source path: {error}"),
                    ),
                    string_table,
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    load_missing_source_paths_with_registered_paths_for_test(
        canonical_paths,
        source_kind,
        string_table,
    )
}

/// Load caller-supplied source identities through the selected-source boundary used by discovery.
///
/// Unlike [`load_missing_source_paths_for_test`], this helper does not canonicalize paths first, so
/// a test can register a deterministic identity for a source that is intentionally absent.
fn load_missing_source_paths_with_registered_paths_for_test(
    source_paths: Vec<PathBuf>,
    source_kind: SourceFileKind,
    string_table: &mut StringTable,
) -> Result<(SourceDatabase, Vec<PreparedSourceInput>), CompilerMessages> {
    let entry_path = source_paths.first().ok_or_else(|| {
        CompilerMessages::from_error_ref(
            CompilerError::compiler_error("test source loading requires at least one path"),
            string_table,
        )
    })?;
    let registration_index = SourceRegistrationIndex::from_rows(
        source_paths
            .iter()
            .map(|path| (path.as_path(), SourceKind::Compiler(source_kind))),
    );
    let source_files = SourceDatabase::from_registration_index_sorted_by_logical_path(
        &registration_index,
        entry_path,
        None,
        string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let mut selected_source_texts = super::source_loading::SelectedSourceTextMap::default();
    let mut first_read_error = None;
    for source_path in &source_paths {
        if let Err(error) = selected_source_texts.load(source_path) {
            first_read_error.get_or_insert(error);
        }
    }

    let mut source_owner = SourceDatabaseBuilder::new(source_files);
    let retain_error = selected_source_texts
        .retain_into(source_owner.sources_mut())
        .err();
    let load_error = first_read_error.or(retain_error);
    if let Some(error) = load_error {
        let mut messages = CompilerMessages::from_error_ref(error, string_table);
        let source_files = source_owner
            .finish()
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        messages.set_source_database(Arc::new(source_files));
        return Err(messages);
    }

    let source_files = source_owner
        .finish()
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let mut input_files = Vec::with_capacity(source_paths.len());
    for source_path in &source_paths {
        let source_id = source_files
            .get_by_canonical_path(source_path)
            .map(|record| record.id)
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error("test source has no registered source identity"),
                    string_table,
                )
            })?;
        if source_files.retained_text(source_id).is_none() {
            return Err(CompilerMessages::from_error_ref(
                CompilerError::compiler_error("test source loading left input slot empty"),
                string_table,
            ));
        }
        let source = match source_kind {
            SourceFileKind::MothTemplate => PreparedSourceKind::MothTemplate,
            SourceFileKind::PlainMarkdown => PreparedSourceKind::PlainMarkdown,
            SourceFileKind::Moth => {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "test missing-source helper cannot create an unprepared Moth input",
                    ),
                    string_table,
                ));
            }
        };
        input_files.push(PreparedSourceInput { source_id, source });
    }

    Ok((source_files, input_files))
}

fn prepared_entry_file_path(
    prepared: &crate::compiler_frontend::module_compilation::PreparedModuleInput,
) -> Option<PathBuf> {
    let module_symbols = &prepared.prepared_header_syntax.module_symbols;
    module_symbols
        .file_roles_by_source
        .iter()
        .find_map(|(source_file, role)| {
            if matches!(
                *role,
                FileRole::ActiveModuleRoot | FileRole::ActiveApiOnlyModuleRoot
            ) {
                Some(source_file.to_path_buf(&prepared.string_table))
            } else {
                None
            }
        })
}

fn prepared_entry_canonical_file_path(
    prepared: &crate::compiler_frontend::module_compilation::PreparedModuleInput,
    source_files: &SourceDatabase,
) -> Option<PathBuf> {
    prepared
        .entry_file_path(source_files)
        .ok()
        .map(Path::to_path_buf)
}

fn module_prepared_source_names(
    module: &super::module_inventory::ModuleCompilationJob,
) -> Vec<String> {
    let module_symbols = &module
        .prepared
        .semantic
        .prepared_header_syntax
        .module_symbols;
    let string_table = &module.prepared.semantic.string_table;
    let mut logical_paths = module_symbols
        .module_file_paths
        .iter()
        .map(|source_file| source_file.to_portable_string(string_table))
        .collect::<Vec<_>>();
    logical_paths.sort();

    logical_paths
        .into_iter()
        .map(|logical_path| {
            Path::new(&logical_path)
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

fn module_source_paths(
    module: &super::module_inventory::ModuleCompilationJob,
    source_files: &SourceDatabase,
) -> HashSet<PathBuf> {
    let module_symbols = &module
        .prepared
        .semantic
        .prepared_header_syntax
        .module_symbols;
    module_symbols
        .module_file_paths
        .iter()
        .filter_map(|source_file| {
            module_symbols
                .source_record(source_file, source_files)
                .and_then(|record| {
                    record
                        .canonical_os_path
                        .clone()
                        .map(|canonical| canonical.into_path_buf())
                })
        })
        .collect()
}

fn parse_project_config_for_test(
    config: &mut Config,
    config_path: &std::path::Path,
    style_directives: &StyleDirectiveRegistry,
) -> Result<(), CompilerMessages> {
    let frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    let mut string_table = StringTable::new();
    let mut source_files = SourceDatabase::empty();
    let build_config_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let services = ProjectConfigParseServices {
        style_directives,
        frontend_surface: &frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    compile_project_config_file(
        config,
        config_path,
        &services,
        &mut source_files,
        &mut string_table,
    )
    .map(|_| ())
}

fn parse_project_config_for_test_with_html_keys(
    config: &mut Config,
    config_path: &std::path::Path,
    style_directives: &StyleDirectiveRegistry,
) -> Result<(), CompilerMessages> {
    let frontend_surface =
        crate::projects::html_project::html_project_builder::HtmlProjectBuilder::new()
            .frontend_surface();
    let mut string_table = StringTable::new();
    let mut source_files = SourceDatabase::empty();
    let build_config_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let services = ProjectConfigParseServices {
        style_directives,
        frontend_surface: &frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    compile_project_config_file(
        config,
        config_path,
        &services,
        &mut source_files,
        &mut string_table,
    )
    .map(|_| ())
}

fn parse_project_config_for_test_with_packages(
    config: &mut Config,
    config_path: &std::path::Path,
    style_directives: &StyleDirectiveRegistry,
    frontend_surface: &crate::builder_surface::BuilderSurface,
) -> Result<(), CompilerMessages> {
    let mut string_table = StringTable::new();
    let mut source_files = SourceDatabase::empty();
    let build_config_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let services = ProjectConfigParseServices {
        style_directives,
        frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    compile_project_config_file(
        config,
        config_path,
        &services,
        &mut source_files,
        &mut string_table,
    )
    .map(|_| ())
}

fn discover_modules_for_test(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> Result<ModuleCompilationSchedule, CompilerMessages> {
    discover_modules_for_test_with_resource_inputs(config, resolver, style_directives)
        .map(|(schedule, _resource_inputs, _source_files)| schedule)
}

fn discover_modules_for_test_with_resource_inputs(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> Result<
    (
        ModuleCompilationSchedule,
        ResourceInputRegistry,
        SourceDatabase,
    ),
    CompilerMessages,
> {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .map_err(|failure| failure.into_messages(&string_table))?;
    let source_files = source_database_for_test(&source_tree_index, resolver, &mut string_table);
    let mut source_owner = SourceDatabaseBuilder::new(source_files);
    let mut project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let mut external_packages = ExternalPackageRegistry::new();
    let external_import_providers =
        crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::empty();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_dependency_resolution_table =
        crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
    let source_package_boundary_indexes = build_source_package_boundary_indexes(
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        &external_import_providers,
        &mut string_table,
    )
    .map_err(|failure| failure.into_messages(&string_table))?;
    let module_namespace_set = ModuleNamespaceSet::build(
        &source_tree_index,
        &project_module_graph,
        source_package_boundary_indexes,
        &external_packages,
    );
    let mut external_imports = super::source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: &external_import_providers,
        cache: &mut external_import_cache,
        resolution_table: &mut external_dependency_resolution_table,
    };
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut selected_source_texts = super::source_loading::SelectedSourceTextMap::default();
    let schedule_result = {
        let (source_files, mut source_spans) = source_owner.split();
        discover_all_modules_in_project_with_check_only(
            config,
            resolver,
            source_files,
            &mut source_spans,
            &mut project_module_graph,
            style_directives,
            &mut external_imports,
            DirectoryDependencyResolution::project(&module_namespace_set, &source_tree_index),
            &mut resource_inputs,
            false,
            &mut selected_source_texts,
            &mut string_table,
            #[cfg(feature = "timers")]
            crate::timing::NO_TIMING_BOUNDARY,
        )
    };
    let retain_result = selected_source_texts.retain_into(source_owner.sources_mut());
    source_owner.adopt_pending_span_builders();
    if let Err(error) = retain_result {
        let source_files = source_owner
            .finish()
            .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))?;
        let mut messages = CompilerMessages::from_error_ref(error, &string_table);
        messages.set_source_database(Arc::new(source_files));
        return Err(messages);
    }
    let schedule = match schedule_result {
        Ok(schedule) => schedule,
        Err(failure) => {
            let source_files = source_owner
                .finish()
                .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))?;
            return Err(failure.into_messages_with_source(&string_table, Arc::new(source_files)));
        }
    };
    let source_files = source_owner
        .finish()
        .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))?;
    Ok((schedule, resource_inputs, source_files))
}

fn discover_modules_and_source_files_for_test(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> Result<(ModuleCompilationSchedule, SourceDatabase), CompilerMessages> {
    discover_modules_for_test_with_resource_inputs(config, resolver, style_directives)
        .map(|(schedule, _resource_inputs, source_files)| (schedule, source_files))
}

fn discover_modules_for_test_with_providers(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_import_providers: &ExternalImportProviderRegistry,
) -> Result<ModuleCompilationSchedule, CompilerMessages> {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        external_import_providers,
        &mut string_table,
    )
    .map_err(|failure| failure.into_messages(&string_table))?;
    let source_files = source_database_for_test(&source_tree_index, resolver, &mut string_table);
    let mut source_owner = SourceDatabaseBuilder::new(source_files);
    let mut project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let mut external_packages = ExternalPackageRegistry::new();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_dependency_resolution_table =
        crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
    let source_package_boundary_indexes = build_source_package_boundary_indexes(
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        external_import_providers,
        &mut string_table,
    )
    .map_err(|failure| failure.into_messages(&string_table))?;
    let module_namespace_set = ModuleNamespaceSet::build(
        &source_tree_index,
        &project_module_graph,
        source_package_boundary_indexes,
        &external_packages,
    );
    let mut external_imports = super::source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: external_import_providers,
        cache: &mut external_import_cache,
        resolution_table: &mut external_dependency_resolution_table,
    };
    let mut resource_inputs = ResourceInputRegistry::new();

    let mut selected_source_texts = super::source_loading::SelectedSourceTextMap::default();
    let schedule_result = {
        let (source_files, mut source_spans) = source_owner.split();
        discover_all_modules_in_project_with_check_only(
            config,
            resolver,
            source_files,
            &mut source_spans,
            &mut project_module_graph,
            style_directives,
            &mut external_imports,
            DirectoryDependencyResolution::project(&module_namespace_set, &source_tree_index),
            &mut resource_inputs,
            false,
            &mut selected_source_texts,
            &mut string_table,
            #[cfg(feature = "timers")]
            crate::timing::NO_TIMING_BOUNDARY,
        )
    };
    let retain_result = selected_source_texts.retain_into(source_owner.sources_mut());
    source_owner.adopt_pending_span_builders();
    if let Err(error) = retain_result {
        let source_files = source_owner
            .finish()
            .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))?;
        let mut messages = CompilerMessages::from_error_ref(error, &string_table);
        messages.set_source_database(Arc::new(source_files));
        return Err(messages);
    }
    match schedule_result {
        Ok(schedule) => Ok(schedule),
        Err(failure) => {
            let source_files = source_owner
                .finish()
                .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))?;
            Err(failure.into_messages_with_source(&string_table, Arc::new(source_files)))
        }
    }
}

/// Build the Stage 0 namespace resolution context for one project and run a closure against it.
///
/// WHAT: discovers the indexed Stage 0 namespace inputs and hands their resolver to `body`.
/// WHY: focused tests can assert the tagged resolution result, which integration output hides.
fn with_namespace_resolution(
    config: &Config,
    resolver: &ProjectPathResolver,
    source_packages: &crate::builder_surface::SourcePackageRegistry,
    package_prefix: Option<&str>,
    body: impl FnOnce(&DirectoryDependencyResolution, &mut StringTable),
) {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let external_import_providers =
        crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default();
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        config,
        source_packages,
        resolver.source_file_kinds(),
        &external_import_providers,
        &mut string_table,
    )
    .expect("source tree index should build");
    let project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let external_packages = ExternalPackageRegistry::new();
    let source_package_boundary_indexes = build_source_package_boundary_indexes(
        source_packages,
        resolver.source_file_kinds(),
        &external_import_providers,
        &mut string_table,
    )
    .expect("source package boundary indexes should build");
    let module_namespace_set = ModuleNamespaceSet::build(
        &source_tree_index,
        &project_module_graph,
        source_package_boundary_indexes,
        &external_packages,
    );
    let resolution = if let Some(package_prefix) = package_prefix {
        let package_source_tree_index = module_namespace_set
            .source_package_boundaries()
            .find(|(prefix, _)| *prefix == package_prefix)
            .map(|(_, index)| index)
            .expect("requested source package boundary should be indexed");
        DirectoryDependencyResolution::package(
            &module_namespace_set,
            package_prefix,
            package_source_tree_index,
        )
    } else {
        DirectoryDependencyResolution::project(&module_namespace_set, &source_tree_index)
    };
    body(&resolution, &mut string_table);
}

/// Build a retained provider-root dependency for one shell.
fn provider_root(path_segments: &[&str], string_table: &mut StringTable) -> RetainedDependencyPath {
    let mut path = crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    for segment in path_segments {
        path.push_str(segment, string_table);
    }
    RetainedDependencyPath {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        path,
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        dependency_shell_id: crate::compiler_frontend::symbols::identity::DependencyShellId::new(
            crate::compiler_frontend::source::SourceId::from_index(0),
            0,
        ),
    }
}

/// Collect production Stage 0 inputs with their original live source owner.
fn collect_synthetic_inputs_for_test(
    entry_file_path: &Path,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> (SourceDatabaseBuilder, Vec<PreparedSourceInput>, StringTable) {
    let mut string_table = StringTable::new();
    let mut external_packages = ExternalPackageRegistry::new();
    let external_import_providers =
        crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::empty();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_dependency_resolution_table =
        crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
    let mut external_imports = super::source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: &external_import_providers,
        cache: &mut external_import_cache,
        resolution_table: &mut external_dependency_resolution_table,
    };
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();

    let collected = super::source_discovery::collect_reachable_input_files(
        entry_file_path,
        resolver,
        style_directives,
        &mut external_imports,
        &source_file_kinds,
        &mut resource_inputs,
        &mut string_table,
    )
    .expect("synthetic source discovery should succeed");

    (collected.source_files, collected.input_files, string_table)
}

#[derive(Debug, PartialEq, Eq)]
struct SyntheticPreparedIdentitySnapshot {
    logical_path: String,
    file_id: SourceId,
    shell_ids: Vec<DependencyShellId>,
    selected_source_names: Vec<String>,
}

/// Capture final identities and clause-owned selection facts after module preparation, independent
/// of traversal order.
fn synthetic_prepared_identity_snapshot(
    prepared: &super::prepared_module::PreparedModule,
    source_files: &SourceDatabase,
) -> Vec<SyntheticPreparedIdentitySnapshot> {
    let module_symbols = &prepared.semantic.prepared_header_syntax.module_symbols;
    let path_table = source_files.paths();
    let mut path_scratch = Vec::new();
    let mut snapshot = source_files
        .iter()
        .map(|identity| {
            let logical_path = source_files.legacy_logical_path(identity.id);
            let file_id = identity.id;
            for header in prepared
                .semantic
                .prepared_header_syntax
                .headers
                .iter()
                .filter(|header| header.source_file == logical_path)
            {
                assert_eq!(header.tokens.file_id, file_id);
                assert_eq!(
                    header.tokens.canonical_os_path.as_deref(),
                    identity.canonical_os_path.as_deref()
                );
                assert!(
                    header
                        .tokens
                        .path_syntax
                        .paths()
                        .iter()
                        .all(|path| path.span.source() == file_id),
                    "header path spans must use the final source identity"
                );
            }
            let clauses = module_symbols
                .file_dependency_clauses_by_source
                .get(&logical_path)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let shell_ids = clauses
                .iter()
                .map(|clause| {
                    assert_eq!(
                        clause.dependency.span.source(),
                        file_id,
                        "rebased dependency span must belong to its owning prepared file"
                    );
                    assert_eq!(
                        clause.dependency.dependency_shell_id.source, file_id,
                        "rebased shell source must belong to its owning prepared file"
                    );
                    clause.dependency.dependency_shell_id
                })
                .collect::<Vec<_>>();
            let selections = module_symbols
                .dependency_selections_by_source
                .get(&logical_path)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let selected_source_names = selections
                .iter()
                .map(|selection| {
                    assert_eq!(
                        selection.source_span.source(),
                        file_id,
                        "dependency selection span must belong to its owning prepared file"
                    );
                    if let Some(alias) = &selection.local_alias {
                        assert_eq!(
                            alias.span.source(),
                            file_id,
                            "dependency alias span must belong to its owning prepared file"
                        );
                    }
                    prepared
                        .semantic
                        .string_table
                        .resolve(selection.source_name)
                        .to_owned()
                })
                .collect::<Vec<_>>();

            SyntheticPreparedIdentitySnapshot {
                logical_path: path_table.render_portable(
                    identity.logical_path,
                    &prepared.semantic.string_table,
                    &mut path_scratch,
                ),
                file_id,
                shell_ids,
                selected_source_names,
            }
        })
        .collect::<Vec<_>>();
    snapshot.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    snapshot
}

fn synthetic_identity_fixture(dependency_order: &[&str]) -> Vec<SyntheticPreparedIdentitySnapshot> {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let dependency_clauses = dependency_order
        .iter()
        .map(|name| format!("@{name} greet\n"))
        .collect::<String>();
    fs::write(root.join("main.moth"), dependency_clauses).expect("should write synthetic entry");
    for name in ["alpha", "beta"] {
        fs::write(
            root.join(format!("{name}.moth")),
            "greet||:\n    io.line([: [\"hello\"]])\n;\n",
        )
        .expect("should write synthetic provider");
    }

    let entry_file_path =
        fs::canonicalize(root.join("main.moth")).expect("entry should canonicalize");
    let config = Config::new(root.clone());
    let resolver = configured_resolver(&config);
    let style_directives = test_style_directives();
    let (mut span_owners, input_files, string_table) =
        collect_synthetic_inputs_for_test(&entry_file_path, &resolver, &style_directives);
    let (source_files, mut span_view) = span_owners.split();
    let local_string_table = string_table.fork_source().fork_for_module().into_parts().0;

    let source_byte_count = input_files
        .iter()
        .map(|input| {
            source_files
                .retained_text(input.source_id())
                .expect("synthetic snapshot should be retained")
                .len()
        })
        .sum();
    let preparation_context = super::module_preparation::ModulePreparationContext {
        source_files,
        style_directives: &style_directives,
        project_path_resolver: Some(resolver),
    };
    let stable_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local("synthetic-rebound-identity"),
        Path::new(""),
        ModuleRootRole::Normal,
    )
    .expect("synthetic origin should construct");
    #[cfg(feature = "timers")]
    let prepared = preparation_context
        .prepare_module(
            stable_origin,
            input_files,
            &mut span_view,
            &entry_file_path,
            local_string_table,
            source_byte_count,
            None,
        )
        .expect("synthetic outputs should prepare against the retained source table");
    #[cfg(not(feature = "timers"))]
    let prepared = preparation_context
        .prepare_module(
            stable_origin,
            input_files,
            &mut span_view,
            &entry_file_path,
            local_string_table,
            source_byte_count,
        )
        .expect("synthetic outputs should prepare against the retained source table");
    synthetic_prepared_identity_snapshot(&prepared, source_files)
}

/// Discover modules and return the populated project module graph plus the shared source database
/// and string table so focused Phase 5b invariant tests can inspect inserted edges and retained
/// source locations.
fn discover_modules_and_graph_for_test(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> (
    ModuleCompilationSchedule,
    super::project_module_graph::ProjectModuleGraph,
    super::source_tree_index::SourceTreeIndex,
    SourceDatabase,
    StringTable,
) {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("source tree index should build");
    let source_files = source_database_for_test(&source_tree_index, resolver, &mut string_table);
    let mut source_owner = SourceDatabaseBuilder::new(source_files);
    let mut project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let mut external_packages = ExternalPackageRegistry::new();
    let external_import_providers =
        crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::empty();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_dependency_resolution_table =
        crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
    let source_package_boundary_indexes = build_source_package_boundary_indexes(
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        &external_import_providers,
        &mut string_table,
    )
    .expect("source package boundary indexes should build");
    let module_namespace_set = ModuleNamespaceSet::build(
        &source_tree_index,
        &project_module_graph,
        source_package_boundary_indexes,
        &external_packages,
    );
    let mut external_imports = super::source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: &external_import_providers,
        cache: &mut external_import_cache,
        resolution_table: &mut external_dependency_resolution_table,
    };
    let mut resource_inputs = ResourceInputRegistry::new();

    let mut selected_source_texts = super::source_loading::SelectedSourceTextMap::default();
    let modules = {
        let (source_files, mut source_spans) = source_owner.split();
        discover_all_modules_in_project_with_check_only(
            config,
            resolver,
            source_files,
            &mut source_spans,
            &mut project_module_graph,
            style_directives,
            &mut external_imports,
            DirectoryDependencyResolution::project(&module_namespace_set, &source_tree_index),
            &mut resource_inputs,
            false,
            &mut selected_source_texts,
            &mut string_table,
            #[cfg(feature = "timers")]
            crate::timing::NO_TIMING_BOUNDARY,
        )
    }
    .expect("module discovery should pass for focused graph-edge tests");
    let retain_result = selected_source_texts.retain_into(source_owner.sources_mut());
    source_owner.adopt_pending_span_builders();
    retain_result.expect("selected source snapshots should retain");
    let source_files = source_owner
        .finish()
        .expect("discovery source tables should finalize");

    (
        modules,
        project_module_graph,
        source_tree_index,
        source_files,
        string_table,
    )
}

fn assert_has_config_error(messages: &CompilerMessages) {
    assert!(
        messages
            .error_diagnostics()
            .any(|diagnostic| diagnostic.kind.category() == DiagnosticCategory::Config),
        "expected config-classified diagnostic"
    );
}

fn first_invalid_config_reason(messages: &CompilerMessages) -> &InvalidConfigReason {
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("expected one diagnostic");

    let DiagnosticPayload::InvalidConfig { reason, .. } = &diagnostic.payload else {
        panic!(
            "expected invalid config diagnostic, got {:?}",
            diagnostic.payload
        );
    };

    reason
}

fn discover_modules_for_test_messages(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> Result<ModuleCompilationSchedule, CompilerMessages> {
    discover_modules_for_test(config, resolver, style_directives)
}

fn first_error_diagnostic(messages: &CompilerMessages) -> &CompilerDiagnostic {
    messages
        .error_diagnostics()
        .next()
        .expect("expected at least one typed error diagnostic")
}

fn assert_primary_span_text(messages: &CompilerMessages, source_path: &Path, expected_text: &str) {
    let diagnostic = first_error_diagnostic(messages);
    let source_files = messages
        .source_database_for_diagnostic(0)
        .expect("diagnostic should retain its source database");
    let source_path = fs::canonicalize(source_path).expect("diagnostic source path should resolve");
    let source_id = source_files
        .get_by_canonical_path(&source_path)
        .expect("diagnostic source should remain in the source database")
        .id;
    let span = diagnostic
        .primary_span
        .expect("diagnostic should retain its exact primary span");
    assert_eq!(span.source(), source_id);
    let range = span.byte_range(source_files);
    let source_text = source_files
        .retained_text(source_id)
        .expect("diagnostic source snapshot should remain available");
    assert_eq!(
        &source_text[range.start() as usize..range.end() as usize],
        expected_text
    );
}

#[derive(Debug)]
struct CountingExternalImportProvider {
    calls: Arc<AtomicUsize>,
    extensions: Vec<ExternalFileExtension>,
}

/// Provider probe that records the exact filesystem target selected by synthetic Stage 0.
#[derive(Debug)]
struct RecordingExternalImportProvider {
    canonical_paths: Arc<std::sync::Mutex<Vec<PathBuf>>>,
    extensions: Vec<ExternalFileExtension>,
}

impl RecordingExternalImportProvider {
    fn new(canonical_paths: Arc<std::sync::Mutex<Vec<PathBuf>>>) -> Self {
        Self {
            canonical_paths,
            extensions: vec![ExternalFileExtension::from("js")],
        }
    }
}

impl ExternalImportProvider for RecordingExternalImportProvider {
    fn kind(&self) -> ExternalImportProviderKind {
        ExternalImportProviderKind::new("recording-js")
    }

    fn supported_extensions(&self) -> &[ExternalFileExtension] {
        &self.extensions
    }

    fn resolve_external_import(
        &self,
        request: ExternalImportRequest,
        _context: &mut ExternalImportProviderContext,
    ) -> Result<Option<ResolvedExternalImport>, CompilerMessages> {
        self.canonical_paths
            .lock()
            .expect("recorded provider paths lock poisoned")
            .push(request.canonical_source_path);
        Ok(None)
    }
}

/// Provider fixture that gives each canonical source a distinct package identity.
#[derive(Debug)]
struct CanonicalPathPackageProvider {
    extensions: Vec<ExternalFileExtension>,
}

impl CanonicalPathPackageProvider {
    fn new() -> Self {
        Self {
            extensions: vec![ExternalFileExtension::from("js")],
        }
    }
}

impl ExternalImportProvider for CanonicalPathPackageProvider {
    fn kind(&self) -> ExternalImportProviderKind {
        ExternalImportProviderKind::new("canonical-path-package-js")
    }

    fn supported_extensions(&self) -> &[ExternalFileExtension] {
        &self.extensions
    }

    fn resolve_external_import(
        &self,
        request: ExternalImportRequest,
        context: &mut ExternalImportProviderContext,
    ) -> Result<Option<ResolvedExternalImport>, CompilerMessages> {
        let package_path = format!(
            "@test/{}",
            request
                .canonical_source_path
                .to_string_lossy()
                .replace(['/', '\\', '.'], "_")
        );
        let package_id = context
            .package_registry
            .register_package(&package_path, PackageOrigin::Builder)
            .expect("canonical provider package should register");
        Ok(Some(ResolvedExternalImport {
            package_id,
            exported_types: vec![ExternalTypeId(package_id.0)],
            exported_free_functions: vec![ExternalFunctionId::Synthetic(package_id.0)],
            runtime_asset: None,
            diagnostics: vec![],
            required_runtime_imports: vec![],
        }))
    }
}

impl CountingExternalImportProvider {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self {
            calls,
            extensions: vec![ExternalFileExtension::from("js")],
        }
    }
}

impl ExternalImportProvider for CountingExternalImportProvider {
    fn kind(&self) -> ExternalImportProviderKind {
        ExternalImportProviderKind::new("counting-js")
    }

    fn supported_extensions(&self) -> &[ExternalFileExtension] {
        &self.extensions
    }

    fn resolve_external_import(
        &self,
        _request: ExternalImportRequest,
        _context: &mut ExternalImportProviderContext,
    ) -> Result<Option<ResolvedExternalImport>, CompilerMessages> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }
}

/// A counting provider that returns a resolved import so the build cache stores a result and
/// repeated reaches of the same physical source reuse it.
#[derive(Debug)]
struct ResolvingCountingProvider {
    calls: Arc<AtomicUsize>,
    extensions: Vec<ExternalFileExtension>,
}

impl ResolvingCountingProvider {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self {
            calls,
            extensions: vec![ExternalFileExtension::from("js")],
        }
    }
}

/// Build one JS runtime asset fixture whose origin carries a stable provider owner.
fn fixture_js_runtime_asset(canonical_source_path: PathBuf) -> RuntimeAssetIdentity {
    let owner = StableResourceOwnerId::Provider(StableProviderResourceOwnerId::new(
        "html-js",
        StablePackageIdentity::binding(PackageOrigin::Builder, "@test/fixtures"),
    ));

    RuntimeAssetIdentity {
        origin: StableResourceOriginId::new(
            owner,
            PortableResourcePath::from_portable_spelling("_moth/js/fixture.js".to_owned())
                .expect("fixture asset logical path should be valid"),
        ),
        canonical_source_path,
        asset_kind: "js".to_owned(),
        source_span: None,
    }
}

impl ExternalImportProvider for ResolvingCountingProvider {
    fn kind(&self) -> ExternalImportProviderKind {
        ExternalImportProviderKind::new("resolving-js")
    }

    fn supported_extensions(&self) -> &[ExternalFileExtension] {
        &self.extensions
    }

    fn resolve_external_import(
        &self,
        request: ExternalImportRequest,
        context: &mut ExternalImportProviderContext,
    ) -> Result<Option<ResolvedExternalImport>, CompilerMessages> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let package_id = context
            .package_registry
            .register_package("@test/resolving", PackageOrigin::Builder)
            .expect("test package registration should succeed");
        Ok(Some(ResolvedExternalImport {
            package_id,
            exported_types: vec![ExternalTypeId(package_id.0)],
            exported_free_functions: vec![ExternalFunctionId::Synthetic(package_id.0)],
            runtime_asset: Some(fixture_js_runtime_asset(request.canonical_source_path)),
            diagnostics: vec![],
            required_runtime_imports: vec![RequiredRuntimeImport {
                module_name: "@moth/runtime".to_owned(),
                imported_names: vec!["mothOk".to_owned()],
            }],
        }))
    }
}

/// Write a two-module project where `module_a` depends on its direct child `module_b`, plus the
/// config, and return the parsed config and resolver.
fn write_cross_module_project(
    root: &std::path::Path,
) -> (
    Config,
    ProjectPathResolver,
    StyleDirectiveRegistry,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let src = root.join("src");
    let module_a = src.join("module_a");
    let module_b = module_a.join("module_b");
    fs::create_dir_all(&module_a).expect("should create module_a");
    fs::create_dir_all(&module_b).expect("should create module_b");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(module_a.join("@pageA.moth"), "@module_b\n#[:pageA]\n").expect("should write pageA");
    fs::write(module_b.join("@api.moth"), "export:\n    b #= 1\n;\n")
        .expect("should write module_b root");
    fs::write(module_b.join("impl.moth"), "impl #= 1\n").expect("should write module_b impl");

    let mut config = Config::new(root.to_path_buf());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let canonical_module_a = fs::canonicalize(&module_a).expect("module_a should canonicalize");
    let canonical_module_b = fs::canonicalize(&module_b).expect("module_b should canonicalize");
    (
        config,
        resolver,
        style_directives,
        canonical_module_a,
        canonical_module_b,
    )
}

fn compiled_package(prefix: &str) -> CompiledSourcePackage {
    use crate::compiler_frontend::semantic_identity::{
        ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
    };

    let package_identity = StablePackageIdentity::source_package(
        crate::builder_surface::PackageOrigin::ProjectLocal,
        prefix,
    );
    let origin = StableModuleOriginIdentity::from_portable_path(
        package_identity.clone(),
        format!("{prefix}/@mod.moth"),
        ModuleRootRole::Normal,
    );
    let root_path = PathBuf::from(format!("{prefix}/@mod.moth"));
    let graph = ProjectModuleGraph::from_normal_roots(vec![(
        origin.clone(),
        PathBuf::from(prefix),
        root_path,
    )]);
    let root_module_id = graph
        .entry_modules()
        .first()
        .copied()
        .expect("one entry module");

    let mut modules = ModuleArtifactStore::new(1);
    modules
        .publish_success(
            root_module_id,
            CompiledModuleArtifact {
                module: empty_module(),
                interface: PublicSemanticInterface {
                    module_origin: origin,
                    export_bindings: Vec::new(),
                    export_diagnostic_provenance: Vec::new(),
                    binding_exports: Vec::new(),
                    declarations: Vec::new(),
                    reusable_evidence: Vec::new(),
                    concrete_call_summaries: Vec::new(),
                },
            },
        )
        .expect("test package root should publish");

    CompiledSourcePackage {
        package_identity,
        root_module_id,
        boundary: CompiledGraphBoundary {
            structure: graph,
            modules,
            generated: BoundaryGeneratedFunctionStore::default(),
            diagnosed: Vec::new(),
            blocked: Vec::new(),
        },
    }
}

fn empty_module() -> Module {
    Module {
        executable: ModuleExecutable {
            hir: HirModule::new(),
            resource_table: ModuleResourceTable::new(),
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: Vec::new(),
            functions: HirModuleLinkFacts::default(),
        },
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::new(),
            warnings: Vec::new(),
            const_top_level_fragments: Vec::new(),
            root_activity:
                crate::compiler_frontend::module_compilation::ModuleRootActivity::default(),
            doc_fragments: Vec::new(),
            materialisation_context: None,
        },
    }
}

fn dependency_prefixes(dependencies: &[&[&str]]) -> Vec<Vec<String>> {
    dependencies
        .iter()
        .map(|row| row.iter().map(|prefix| (*prefix).to_owned()).collect())
        .collect()
}

#[test]
fn direct_selection_resolves_cross_module_child_facade() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("child")).expect("should create child module dir");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@child greet\n#[:entry]\n").expect("should write entry");
    fs::write(
        src.join("child/@mod.moth"),
        "export:\n    greet || -> String:\n        return \"hi\"\n    ;\n;\n",
    )
    .expect("should write child module root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    let declaring_source =
        fs::canonicalize(src.join("@page.moth")).expect("declaring_source should canonicalize");

    with_namespace_resolution(
        &config,
        &resolver,
        &crate::builder_surface::SourcePackageRegistry::default(),
        None,
        |resolution, string_table| {
            let provider = provider_root(&["child"], string_table);
            let resolved = resolution
                .resolve_dependency(&provider, &declaring_source, string_table)
                .expect("a direct-selection child-module facade should resolve");
            match resolved {
                ResolvedDependency::CrossModule { root_file, .. } => {
                    assert!(
                        root_file.ends_with("child/@mod.moth"),
                        "expected the child module facade root file, got {:?}",
                        root_file
                    );
                }
                other => panic!("expected CrossModule, got {:?}", other),
            }
        },
    );
}

#[test]
fn direct_selection_resolves_source_package_facade() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let package_root = root.join("builder/helper");
    fs::create_dir_all(&src).expect("should create src dir");
    fs::create_dir_all(&package_root).expect("should create helper package dir");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@helper add\n#[:entry]\n").expect("should write entry");
    fs::write(
        package_root.join("@mod.moth"),
        "export:\n    add |a Int, b Int| -> Int:\n        return a + b\n    ;\n;\n",
    )
    .expect("should write helper package root");

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::default();
    source_packages.register_filesystem_root("helper", package_root, PackageOrigin::Builder);

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    let declaring_source =
        fs::canonicalize(src.join("@page.moth")).expect("declaring_source should canonicalize");

    with_namespace_resolution(
        &config,
        &resolver,
        &source_packages,
        None,
        |resolution, string_table| {
            let provider = provider_root(&["helper"], string_table);
            let resolved = resolution
                .resolve_dependency(&provider, &declaring_source, string_table)
                .expect("a direct-selection source-package facade should resolve");
            match resolved {
                ResolvedDependency::SourcePackageSurface { root_file, .. } => {
                    assert!(
                        root_file.ends_with("helper/@mod.moth"),
                        "expected the helper package facade root file, got {:?}",
                        root_file
                    );
                }
                other => panic!("expected SourcePackageSurface, got {:?}", other),
            }
        },
    );
}
