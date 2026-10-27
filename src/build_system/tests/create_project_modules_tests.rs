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
use crate::build_system::create_project_modules::source_tree_index::SourceTreeIndex;
use crate::build_system::project_config::{
    ProjectConfigFailure, ProjectConfigParseServices, compile_project_config_file,
    load_project_config,
};
use crate::builder_surface::external_import_providers::provider::{
    ExternalFileExtension, ExternalImportProvider, ExternalImportProviderContext,
    ExternalImportProviderKind, ExternalImportRequest, RequiredRuntimeImport,
    ResolvedExternalImport, RuntimeAssetIdentity,
};
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::{PackageOrigin, SourceFileKind};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureFailure, InfrastructureFailureKind,
};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terse};
use crate::compiler_frontend::compiler_messages::{
    DependencyClauseKind, DiagnosticBag, InvalidDependencyClauseReason, PathKind, PreparedFailure,
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
    LocalSpan, SourceCapacityError, SourceDatabase, SourceDatabaseBuilder, SourceDatabaseError,
    SourceId, SourceKind, SourceRegistrationIndex, SourceSpan,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
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
#[path = "create_project_modules_module_graph_tests.rs"]
mod create_project_modules_module_graph_tests;
#[path = "create_project_modules_package_registry_tests.rs"]
mod create_project_modules_package_registry_tests;
#[path = "create_project_modules_source_discovery_tests.rs"]
mod create_project_modules_source_discovery_tests;
#[path = "create_project_modules_source_tree_index_tests.rs"]
mod create_project_modules_source_tree_index_tests;
#[path = "create_project_modules_structure_collision_tests.rs"]
mod create_project_modules_structure_collision_tests;
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
    let _path_fork = PathInternerFork::empty();
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

fn prepared_failure_result<T>(
    failure: PreparedFailure,
    source_database: Option<Arc<SourceDatabase>>,
    root_string_table: &StringTable,
) -> CompilationResult<T> {
    let mut accumulator = BoundaryAccumulator::with_root_string_table(root_string_table.clone());
    match failure {
        PreparedFailure::Diagnosed(batch) => {
            accumulator.append_prepared(batch, source_database.map(|database| (database, None)));
        }
        PreparedFailure::Infrastructure(errors) => {
            accumulator.append_infrastructure(errors);
        }
    }
    match accumulator.finish(InstallMode::Unfrozen) {
        Ok(reports) => Ok(CompilationOutcome::Diagnosed(reports)),
        Err(errors) => Err(errors),
    }
}

fn project_config_failure_result(
    failure: ProjectConfigFailure,
    source_builder: Option<SourceDatabaseBuilder>,
    root_string_table: &StringTable,
) -> CompilationResult<()> {
    match failure {
        ProjectConfigFailure::Prepared(failure) => {
            let source_database = source_builder.map(|mut builder| {
                builder
                    .assemble_finalized_tables()
                    .expect("test config source table should assemble");
                Arc::new(
                    builder
                        .finish()
                        .expect("test config source builder should satisfy lifecycle invariants"),
                )
            });
            prepared_failure_result(failure, source_database, root_string_table)
        }
        ProjectConfigFailure::Diagnosed(reports) => Ok(CompilationOutcome::Diagnosed(reports)),
        ProjectConfigFailure::Infrastructure(errors) => Err(errors),
    }
}

type TestSourceLoadFailure = (PreparedFailure, Option<Box<SourceDatabase>>);

fn load_missing_source_path_for_test(
    source_path: PathBuf,
    _source_kind: SourceFileKind,
) -> Result<(), InfrastructureFailure> {
    match super::source_loading::read_source_code(&source_path) {
        Ok(_) => Ok(()),
        Err(error) => Err(super::source_loading::source_read_error(
            &source_path,
            error,
        )),
    }
}
fn finalize_loaded_source_spans_for_test(
    source_owner: &mut SourceDatabaseBuilder,
) -> Result<(), InfrastructureFailure> {
    let source_ids = source_owner
        .sources()
        .iter()
        .filter_map(|source| {
            source_owner
                .sources()
                .retained_text(source.id)
                .map(|_| source.id)
        })
        .collect::<Vec<_>>();
    for source_id in source_ids {
        if !source_owner.is_source_finalized(source_id)? {
            source_owner.finish_source(source_id)?;
        }
    }
    source_owner.assemble_finalized_tables()
}

fn load_missing_source_paths_for_test(
    source_paths: Vec<PathBuf>,
    source_kind: SourceFileKind,
    string_table: &mut StringTable,
) -> Result<(SourceDatabase, Vec<PreparedSourceInput>), TestSourceLoadFailure> {
    let canonical_paths = source_paths
        .into_iter()
        .map(|path| {
            fs::canonicalize(&path).map_err(|error| {
                (
                    InfrastructureFailure::filesystem(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, &path, format!("failed to canonicalize test source path: {error}"))
                    .into(),
                    None,
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
) -> Result<(SourceDatabase, Vec<PreparedSourceInput>), TestSourceLoadFailure> {
    let entry_path = source_paths.first().ok_or_else(|| {
        (
            InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                "test source loading requires at least one path",
            )
            .into(),
            None,
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
    .map_err(|error| match error {
        SourceDatabaseError::Capacity(capacity) => (
            PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
                DiagnosticBag::from(token_lexical::source_span_capacity(
                    0,
                    u32::MAX,
                    capacity.resource(),
                    None,
                )),
                string_table.clone(),
                None,
                None,
            )),
            None,
        ),
        SourceDatabaseError::Infrastructure(error) => (error.into(), None),
    })?;

    let mut selected_source_texts = super::source_loading::SelectedSourceTextMap::default();
    let mut first_read_error = None;
    for source_path in &source_paths {
        if let Err(error) = selected_source_texts.load(source_path) {
            first_read_error.get_or_insert((source_path.clone(), error));
        }
    }

    let mut source_owner = SourceDatabaseBuilder::new(source_files);
    let mut path_fork = source_owner.sources().fork_path_interner();
    let retain_error = selected_source_texts
        .retain_into(source_owner.sources_mut(), &mut path_fork)
        .err();
    finalize_loaded_source_spans_for_test(&mut source_owner)
        .map_err(|error| (error.into(), None))?;
    let load_error = match first_read_error {
        Some((path, error)) => {
            let source_id = source_owner
                .sources()
                .get_by_canonical_path(&path)
                .expect("selected source should stay registered")
                .id;
            Some(
                super::source_loading::registered_source_read_error(
                    error,
                    source_id,
                    source_owner.sources(),
                    &mut path_fork,
                )
                .into_failure(string_table, Arc::new(path_fork.shared_table())),
            )
        }
        None => retain_error
            .map(|error| error.into_failure(string_table, Arc::new(path_fork.shared_table()))),
    };
    if let Some(error) = load_error {
        let source_files = source_owner
            .finish()
            .map_err(|error| (PreparedFailure::from(error), None))?;
        return Err((error, Some(Box::new(source_files))));
    }

    let source_files = source_owner
        .finish()
        .map_err(|error| (error.into(), None))?;
    let mut input_files = Vec::with_capacity(source_paths.len());
    for source_path in &source_paths {
        let Some(source_id) = source_files
            .get_by_canonical_path(source_path)
            .map(|record| record.id)
        else {
            return Err((
                InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, "test source has no registered source identity")
                    .into(),
                Some(Box::new(source_files)),
            ));
        };
        if source_files.retained_text(source_id).is_none() {
            return Err((
                InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, "test source loading left input slot empty").into(),
                Some(Box::new(source_files)),
            ));
        }
        let source = match source_kind {
            SourceFileKind::MothTemplate | SourceFileKind::PlainMarkdown => {
                PreparedSourceKind::Deferred
            }
            SourceFileKind::Moth => {
                return Err((
                    InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, "test missing-source helper cannot create an unprepared Moth input")
                    .into(),
                    Some(Box::new(source_files)),
                ));
            }
        };
        input_files.push(PreparedSourceInput { source_id, source });
    }

    Ok((source_files, input_files))
}

const STAGE0_PARALLEL_SOURCE_PREPARE_MIN_FILES: usize = 16;

fn should_parallelize_owned_source_preparation(source_count: usize) -> bool {
    source_count >= STAGE0_PARALLEL_SOURCE_PREPARE_MIN_FILES
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
                Some(prepared.path_fork.render_native(
                    *source_file,
                    &prepared.string_table,
                    &mut Vec::new(),
                ))
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
    let _string_table = &module.prepared.semantic.string_table;
    let mut logical_paths = module_symbols
        .module_file_paths
        .iter()
        .map(|source_file| {
            module.prepared.semantic.path_fork.render_portable(
                *source_file,
                &module.prepared.semantic.string_table,
                &mut Vec::new(),
            )
        })
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

fn finish_config_source_for_test(source_files: Option<SourceDatabaseBuilder>) {
    let mut source_owner =
        source_files.expect("successful config should retain its source builder");
    source_owner
        .assemble_finalized_tables()
        .expect("config span table should assemble");
    source_owner
        .finish()
        .expect("test config source builder should satisfy lifecycle invariants");
}

fn parse_project_config_for_test(
    config: &mut Config,
    config_path: &std::path::Path,
    style_directives: &StyleDirectiveRegistry,
) -> CompilationResult<()> {
    let frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    let mut string_table = StringTable::new();
    let mut source_files = Some(SourceDatabaseBuilder::new(SourceDatabase::empty()));
    let build_config_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let services = ProjectConfigParseServices {
        style_directives,
        frontend_surface: &frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    let result = compile_project_config_file(
        config,
        config_path,
        &services,
        &mut source_files,
        &mut string_table,
    );
    match result {
        Ok(_) => {
            finish_config_source_for_test(source_files.take());
            Ok(CompilationOutcome::Success {
                value: (),
                warnings: None,
            })
        }
        Err(failure) => project_config_failure_result(failure, source_files.take(), &string_table),
    }
}

fn parse_project_config_for_test_with_html_keys(
    config: &mut Config,
    config_path: &std::path::Path,
    style_directives: &StyleDirectiveRegistry,
) -> CompilationResult<()> {
    let frontend_surface =
        crate::projects::html_project::html_project_builder::HtmlProjectBuilder::new()
            .frontend_surface();
    let mut string_table = StringTable::new();
    let mut source_files = Some(SourceDatabaseBuilder::new(SourceDatabase::empty()));
    let build_config_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let services = ProjectConfigParseServices {
        style_directives,
        frontend_surface: &frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    let result = compile_project_config_file(
        config,
        config_path,
        &services,
        &mut source_files,
        &mut string_table,
    );
    match result {
        Ok(_) => {
            finish_config_source_for_test(source_files.take());
            Ok(CompilationOutcome::Success {
                value: (),
                warnings: None,
            })
        }
        Err(failure) => project_config_failure_result(failure, source_files.take(), &string_table),
    }
}

fn parse_project_config_for_test_with_packages(
    config: &mut Config,
    config_path: &std::path::Path,
    style_directives: &StyleDirectiveRegistry,
    frontend_surface: &crate::builder_surface::BuilderSurface,
) -> CompilationResult<()> {
    let mut string_table = StringTable::new();
    let mut source_files = Some(SourceDatabaseBuilder::new(SourceDatabase::empty()));
    let build_config_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let services = ProjectConfigParseServices {
        style_directives,
        frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    let result = compile_project_config_file(
        config,
        config_path,
        &services,
        &mut source_files,
        &mut string_table,
    );
    match result {
        Ok(_) => {
            finish_config_source_for_test(source_files.take());
            Ok(CompilationOutcome::Success {
                value: (),
                warnings: None,
            })
        }
        Err(failure) => project_config_failure_result(failure, source_files.take(), &string_table),
    }
}

fn discover_modules_for_test(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> CompilationResult<ModuleCompilationSchedule> {
    match discover_modules_for_test_with_resource_inputs(config, resolver, style_directives) {
        Ok(CompilationOutcome::Success { value, warnings }) => Ok(CompilationOutcome::Success {
            value: value.0,
            warnings,
        }),
        Ok(CompilationOutcome::Diagnosed(reports)) => Ok(CompilationOutcome::Diagnosed(reports)),
        Err(errors) => Err(errors),
    }
}

fn discover_modules_for_test_with_resource_inputs(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> CompilationResult<(
    ModuleCompilationSchedule,
    ResourceInputRegistry,
    SourceDatabase,
)> {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let source_tree_index = match super::source_tree_index::SourceTreeIndex::discover(
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
    ) {
        Ok(index) => index,
        Err(failure) => return prepared_failure_result(failure, None, &string_table),
    };
    let source_files = source_database_for_test(&source_tree_index, resolver, &mut string_table);
    let path_fork = source_files.fork_path_interner();
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
    let source_package_boundary_indexes = match build_source_package_boundary_indexes(
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        &external_import_providers,
        &mut string_table,
    ) {
        Ok(indexes) => indexes,
        Err(failure) => return prepared_failure_result(failure, None, &string_table),
    };
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
        let path_interner = source_owner.sources_mut().fork_path_builder();
        let (source_files, mut source_spans) = source_owner.split();
        discover_all_modules_in_project_with_check_only(
            config,
            resolver,
            source_files,
            &mut source_spans,
            &mut project_module_graph,
            style_directives,
            &mut external_imports,
            DirectoryDependencyResolution::project(
                &module_namespace_set,
                &source_tree_index,
                &path_fork,
            ),
            &mut resource_inputs,
            false,
            &mut selected_source_texts,
            &mut string_table,
            &path_interner,
            #[cfg(feature = "timers")]
            crate::timing::NO_TIMING_BOUNDARY,
        )
    };
    let mut path_fork = source_owner.sources().fork_path_interner();
    let retain_result =
        selected_source_texts.retain_into(source_owner.sources_mut(), &mut path_fork);
    if retain_result.is_ok() {
        source_owner.adopt_pending_span_builders();
    }
    if let Err(error) = retain_result {
        finalize_loaded_source_spans_for_test(&mut source_owner)
            .expect("source span builders should finalize");
        let failure = error.into_failure(&mut string_table, Arc::new(path_fork.shared_table()));
        if let Err(finish_error) = source_owner.finish() {
            return Err(finish_error.into());
        }
        return prepared_failure_result(failure, None, &string_table);
    }
    finalize_loaded_source_spans_for_test(&mut source_owner)
        .expect("source span builders should finalize");
    let schedule = match schedule_result {
        Ok(schedule) => schedule,
        Err(failure) => {
            let source_files = source_owner.finish()?;
            return prepared_failure_result(failure, Some(Arc::new(source_files)), &string_table);
        }
    };
    let source_files = source_owner.finish()?;
    Ok(CompilationOutcome::Success {
        value: (schedule, resource_inputs, source_files),
        warnings: None,
    })
}

fn discover_modules_and_source_files_for_test(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> CompilationResult<(ModuleCompilationSchedule, SourceDatabase)> {
    match discover_modules_for_test_with_resource_inputs(config, resolver, style_directives) {
        Ok(CompilationOutcome::Success { value, warnings }) => Ok(CompilationOutcome::Success {
            value: (value.0, value.2),
            warnings,
        }),
        Ok(CompilationOutcome::Diagnosed(reports)) => Ok(CompilationOutcome::Diagnosed(reports)),
        Err(errors) => Err(errors),
    }
}

fn discover_modules_for_test_with_providers(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_import_providers: &ExternalImportProviderRegistry,
) -> CompilationResult<ModuleCompilationSchedule> {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let source_tree_index = match super::source_tree_index::SourceTreeIndex::discover(
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
    ) {
        Ok(index) => index,
        Err(failure) => return prepared_failure_result(failure, None, &string_table),
    };
    let source_files = source_database_for_test(&source_tree_index, resolver, &mut string_table);
    let path_fork = source_files.fork_path_interner();
    let mut source_owner = SourceDatabaseBuilder::new(source_files);
    let mut project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let mut external_packages = ExternalPackageRegistry::new();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_dependency_resolution_table =
        crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
    let source_package_boundary_indexes = match build_source_package_boundary_indexes(
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        external_import_providers,
        &mut string_table,
    ) {
        Ok(indexes) => indexes,
        Err(failure) => return prepared_failure_result(failure, None, &string_table),
    };
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
        let path_interner = source_owner.sources_mut().fork_path_builder();
        let (source_files, mut source_spans) = source_owner.split();
        discover_all_modules_in_project_with_check_only(
            config,
            resolver,
            source_files,
            &mut source_spans,
            &mut project_module_graph,
            style_directives,
            &mut external_imports,
            DirectoryDependencyResolution::project(
                &module_namespace_set,
                &source_tree_index,
                &path_fork,
            ),
            &mut resource_inputs,
            false,
            &mut selected_source_texts,
            &mut string_table,
            &path_interner,
            #[cfg(feature = "timers")]
            crate::timing::NO_TIMING_BOUNDARY,
        )
    };
    let mut path_fork = source_owner.sources().fork_path_interner();
    let retain_result =
        selected_source_texts.retain_into(source_owner.sources_mut(), &mut path_fork);
    if retain_result.is_ok() {
        source_owner.adopt_pending_span_builders();
    }
    if let Err(error) = retain_result {
        finalize_loaded_source_spans_for_test(&mut source_owner)
            .expect("source span builders should finalize");
        let failure = error.into_failure(&mut string_table, Arc::new(path_fork.shared_table()));
        if let Err(finish_error) = source_owner.finish() {
            return Err(finish_error.into());
        }
        return prepared_failure_result(failure, None, &string_table);
    }
    finalize_loaded_source_spans_for_test(&mut source_owner)
        .expect("source span builders should finalize");
    match schedule_result {
        Ok(schedule) => {
            if let Err(error) = source_owner.finish() {
                return Err(error.into());
            }
            Ok(CompilationOutcome::Success {
                value: schedule,
                warnings: None,
            })
        }
        Err(failure) => {
            let source_files = source_owner.finish()?;
            prepared_failure_result(failure, Some(Arc::new(source_files)), &string_table)
        }
    }
}
fn with_namespace_resolution(
    config: &Config,
    resolver: &ProjectPathResolver,
    source_packages: &crate::builder_surface::SourcePackageRegistry,
    package_prefix: Option<&str>,
    body: impl FnOnce(
        &ModuleNamespaceSet,
        &SourceTreeIndex,
        Option<&str>,
        &mut StringTable,
        &[(String, PathId)],
        &mut PathInternerFork,
    ),
) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
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
    let provider_paths = ["child", "helper", "project"]
        .into_iter()
        .map(|segment| {
            (
                segment.to_owned(),
                path_fork
                    .try_intern_child(PathId::ROOT, string_table.intern(segment))
                    .expect("test path fits"),
            )
        })
        .collect::<Vec<_>>();
    body(
        &module_namespace_set,
        &source_tree_index,
        package_prefix,
        &mut string_table,
        &provider_paths,
        &mut path_fork,
    );
}

/// Build a retained provider-root dependency for one shell.
fn provider_root(
    path_segments: &[&str],
    provider_paths: &[(String, PathId)],
) -> RetainedDependencyPath {
    let key = path_segments
        .first()
        .copied()
        .expect("provider path should have one segment");
    let path = provider_paths
        .iter()
        .find(|(segment, _)| segment == key)
        .map(|(_, path)| *path)
        .expect("provider path should be pre-interned");
    RetainedDependencyPath {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        path,
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        provider_target: None,
        local_source_id: None,
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
    let mut path_fork = PathInternerFork::empty();
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
        &super::module_namespace::ModuleNamespaceSet::default(),
        resolver,
        style_directives,
        &mut external_imports,
        &source_file_kinds,
        &mut resource_inputs,
        &mut path_fork,
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
            let logical_path = identity.logical_path;
            let file_id = identity.id;
            for header in prepared
                .semantic
                .prepared_header_syntax
                .headers
                .iter()
                .filter(|header| {
                    module_symbols
                        .source_paths_by_source_id
                        .get(&header.tokens.source())
                        .copied()
                        == Some(logical_path)
                })
            {
                assert_eq!(header.tokens.source(), file_id);
                let syntax = &prepared.semantic.prepared_header_syntax;
                let source_owner = syntax
                    .source_token_owners
                    .get(&header.tokens.source())
                    .expect("header should have its source token owner");
                assert_eq!(source_owner.tokens_ref().source(), file_id);
                assert_eq!(
                    source_owner
                        .tokens_ref()
                        .path_syntax_arc()
                        .expect("header source should retain its path table")
                        .owner_source(),
                    Some(file_id),
                    "header path rows must use the final source identity"
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
    };
    let stable_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local("synthetic-rebound-identity"),
        Path::new(""),
        ModuleRootRole::Normal,
    )
    .expect("synthetic origin should construct");
    #[cfg(feature = "timers")]
    let (prepared, _) = preparation_context
        .prepare_module(
            stable_origin,
            input_files,
            &mut span_view,
            &entry_file_path,
            local_string_table,
            source_files.fork_path_interner(),
            source_byte_count,
            None,
        )
        .expect("synthetic outputs should prepare against the retained source table");
    #[cfg(not(feature = "timers"))]
    let (prepared, _) = preparation_context
        .prepare_module(
            stable_origin,
            input_files,
            &mut span_view,
            &entry_file_path,
            local_string_table,
            source_files.fork_path_interner(),
            source_byte_count,
        )
        .expect("synthetic outputs should prepare against the retained source table");
    synthetic_prepared_identity_snapshot(&prepared, source_files)
}

/// Discover modules and return the populated project module graph plus the shared source database
/// and string table so focused invariant tests can inspect inserted edges and retained
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
    let path_fork = source_files.fork_path_interner();
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
        let path_interner = source_owner.sources_mut().fork_path_builder();
        let (source_files, mut source_spans) = source_owner.split();
        discover_all_modules_in_project_with_check_only(
            config,
            resolver,
            source_files,
            &mut source_spans,
            &mut project_module_graph,
            style_directives,
            &mut external_imports,
            DirectoryDependencyResolution::project(
                &module_namespace_set,
                &source_tree_index,
                &path_fork,
            ),
            &mut resource_inputs,
            false,
            &mut selected_source_texts,
            &mut string_table,
            &path_interner,
            #[cfg(feature = "timers")]
            crate::timing::NO_TIMING_BOUNDARY,
        )
    }
    .expect("module discovery should pass for focused graph-edge tests");
    let mut path_fork = source_owner.sources().fork_path_interner();
    let retain_result =
        selected_source_texts.retain_into(source_owner.sources_mut(), &mut path_fork);
    source_owner.adopt_pending_span_builders();
    retain_result.expect("selected source snapshots should retain");
    finalize_loaded_source_spans_for_test(&mut source_owner)
        .expect("discovery span builders should finalize");
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

pub(super) fn diagnosed_reports<T>(
    result: &CompilationResult<T>,
) -> &crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet {
    match result {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        Ok(CompilationOutcome::Success { .. }) => {
            panic!("expected a diagnosed compilation result")
        }
        Err(errors) => panic!("expected diagnostics, got infrastructure errors: {errors:?}"),
    }
}

pub(super) fn successful_compilation_value<T>(result: CompilationResult<T>) -> T {
    match result {
        Ok(CompilationOutcome::Success { value, .. }) => value,
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            panic!("expected compilation success, got diagnostics: {reports:?}")
        }
        Err(errors) => {
            panic!("expected compilation success, got infrastructure errors: {errors:?}")
        }
    }
}

fn first_invalid_config<T>(result: &CompilationResult<T>) -> InvalidConfigView {
    let diagnostic = first_error_diagnostic(result);
    assert_eq!(diagnostic.descriptor().code, "MOTH-CONFIG-0001");
    diagnostic
        .family_view::<InvalidConfigView>()
        .expect("expected typed invalid-config facts")
}

fn discover_modules_for_test_messages(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> CompilationResult<ModuleCompilationSchedule> {
    discover_modules_for_test(config, resolver, style_directives)
}

fn first_error_diagnostic<T>(result: &CompilationResult<T>) -> DiagnosticView<'_> {
    diagnosed_reports(result)
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .map(|(_, diagnostic)| diagnostic)
        .expect("expected at least one typed error diagnostic")
}

fn assert_primary_span_text<T>(
    result: &CompilationResult<T>,
    source_path: &Path,
    expected_text: &str,
) {
    let (report, diagnostic) = diagnosed_reports(result)
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("expected a typed error diagnostic with a source owner");
    let site = diagnostic.primary_site();
    let span = match site {
        SiteRef::Absent => panic!("diagnostic should retain its exact primary span"),
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => span,
    };
    let context = DiagnosticRenderContext::for_report(report);
    let position = context
        .site_position(site)
        .expect("diagnostic should retain its exact source owner");
    let source_path = fs::canonicalize(source_path).expect("diagnostic source path should resolve");
    assert_eq!(position.source, span.source());
    assert_eq!(position.host_path, Some(source_path.as_path()));
    assert_eq!(position.start.line, position.end.line);
    let excerpt = position
        .line
        .chars()
        .skip(position.start.column as usize)
        .take((position.end.column - position.start.column) as usize)
        .collect::<String>();
    assert_eq!(excerpt, expected_text);
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
    ) -> Result<Option<ResolvedExternalImport>, PreparedFailure> {
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
    ) -> Result<Option<ResolvedExternalImport>, PreparedFailure> {
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
    ) -> Result<Option<ResolvedExternalImport>, PreparedFailure> {
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
    ) -> Result<Option<ResolvedExternalImport>, PreparedFailure> {
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
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

pub(super) fn compiled_package(prefix: &str) -> CompiledSourcePackage {
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
            numeric_proofs: NumericProofs::default(),
            path_table: Arc::new(PathInternerFork::empty().snapshot_table()),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: Vec::new(),
            functions: HirModuleLinkFacts::default(),
        },
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::new(),
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
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
        |namespace_set,
         source_tree_index,
         _package_prefix,
         string_table,
         provider_paths,
         path_fork| {
            let resolution =
                DirectoryDependencyResolution::project(namespace_set, source_tree_index, path_fork);
            let provider = provider_root(&["child"], provider_paths);
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@helper add\n#[:entry]\n").expect("should write entry");
    fs::write(
        package_root.join("@mod.moth"),
        "export:\n    add |a I32, b I32| -> I32:\n        return a + b\n    ;\n;\n",
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
        |namespace_set,
         source_tree_index,
         _package_prefix,
         string_table,
         provider_paths,
         path_fork| {
            let resolution =
                DirectoryDependencyResolution::project(namespace_set, source_tree_index, path_fork);
            let provider = provider_root(&["helper"], provider_paths);
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

#[test]
fn authored_oversized_snapshot_is_named_by_its_logical_path() {
    // WHAT: an authored source past the u32 snapshot bound reaches the user as a path-level
    //       source-span capacity diagnostic, never as a path-less capacity error.
    // WHY: the rejection belongs to one user file, so the message must name the file to shorten.
    let source_path = PathBuf::from("/project/huge.moth");
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        std::iter::once(&source_path),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identities should build");
    let source_id = database
        .get_by_canonical_path(&source_path)
        .expect("the authored source should be registered")
        .id;
    let mut path_fork = database.fork_path_interner();
    let failure = super::source_loading::retained_source_error(
        SourceDatabaseError::Capacity(SourceCapacityError::SourceSnapshotTooLarge),
        source_id,
        &database,
        &mut path_fork,
    )
    .into_failure(&mut string_table, Arc::new(path_fork.shared_table()));
    let PreparedFailure::Diagnosed(batch) = failure else {
        panic!("an oversized authored source must produce a diagnosed failure");
    };
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    let reports = accumulator
        .finish(InstallMode::Unfrozen)
        .expect("the diagnosed source should freeze");
    assert_eq!(reports.counts().errors, 1);
    let (report, diagnostic) = reports
        .iter()
        .next()
        .expect("the oversized source should diagnose once");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0036");
    let rendered = terse::format_terse_diagnostic_view(
        diagnostic,
        DiagnosticRenderContext::for_report(report),
    );
    assert!(
        rendered.contains("Source 'huge.moth' is at least 4294967295 bytes"),
        "the oversize diagnostic must name the logical path: {rendered}"
    );
}
