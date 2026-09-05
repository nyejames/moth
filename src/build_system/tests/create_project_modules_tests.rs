use super::compiled_boundary::{
    CompiledGraphBoundary, CompiledSourcePackage, CompletedSourcePackageRegistry,
};
use super::file_reference_resolution::SingleFileReferenceOutcome;
use super::generated_store::BoundaryGeneratedFunctionStore;
use super::module_artifact_store::ModuleArtifactStore;
use super::module_identity::ModuleId;
use super::prepared_source::PreparedSourceInput;
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
use crate::compiler_frontend::compiler_errors::{CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
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
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{SourceDatabase, SourceId};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        PreparedSourcePackageRoots::empty(),
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
    let mut source_files = SourceDatabase::from_ordered_registration_index(
        &registration_index,
        resolver.entry_root(),
        Some(resolver),
        string_table,
    )
    .expect("test source database should build from the indexed canonical inventory");
    super::source_loading::load_registered_source_texts(
        &mut source_files,
        &registration_index,
        string_table,
    )
    .expect("test source database should preload indexed source text");
    source_files
}

fn prepared_entry_file_path(
    prepared: &crate::compiler_frontend::module_compilation::PreparedModuleInput,
) -> Option<&Path> {
    let module_symbols = &prepared.prepared_header_syntax.module_symbols;
    module_symbols
        .file_roles_by_source
        .iter()
        .find_map(|(source_file, role)| {
            if matches!(
                *role,
                FileRole::ActiveModuleRoot | FileRole::ActiveApiOnlyModuleRoot
            ) {
                module_symbols
                    .canonical_os_path_by_source
                    .get(source_file)
                    .map(PathBuf::as_path)
            } else {
                None
            }
        })
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

fn module_source_paths(module: &super::module_inventory::ModuleCompilationJob) -> HashSet<PathBuf> {
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
                .canonical_os_path_by_source
                .get(source_file)
                .cloned()
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
        Some(&mut source_files),
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
        Some(&mut source_files),
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
        Some(&mut source_files),
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
        .map(|(schedule, _resource_inputs)| schedule)
}

fn discover_modules_for_test_with_resource_inputs(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> Result<(ModuleCompilationSchedule, ResourceInputRegistry), CompilerMessages> {
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
    )?;
    let source_files = source_database_for_test(&source_tree_index, resolver, &mut string_table);
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
    )?;
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
    let schedule = discover_all_modules_in_project(
        config,
        resolver,
        &source_files,
        &mut project_module_graph,
        style_directives,
        &mut external_imports,
        DirectoryDependencyResolution::project(&module_namespace_set, &source_tree_index),
        &mut resource_inputs,
        &mut string_table,
        #[cfg(feature = "timers")]
        crate::timing::NO_TIMING_BOUNDARY,
    )?;
    Ok((schedule, resource_inputs))
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
    )?;
    let source_files = source_database_for_test(&source_tree_index, resolver, &mut string_table);
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
    )?;
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

    discover_all_modules_in_project(
        config,
        resolver,
        &source_files,
        &mut project_module_graph,
        style_directives,
        &mut external_imports,
        DirectoryDependencyResolution::project(&module_namespace_set, &source_tree_index),
        &mut resource_inputs,
        &mut string_table,
        #[cfg(feature = "timers")]
        crate::timing::NO_TIMING_BOUNDARY,
    )
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
        path,
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: SourceLocation::default(),
        dependency_shell_id: crate::compiler_frontend::symbols::identity::DependencyShellId::new(
            crate::compiler_frontend::source::SourceId::from_index(0),
            0,
        ),
    }
}

/// Collect synthetic inputs through the production Stage 0 path while retaining the merged table
/// needed to inspect the rebased syntax identities.
fn collect_synthetic_inputs_for_test(
    entry_file_path: &Path,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> (Vec<PreparedSourceInput>, StringTable) {
    let mut string_table = StringTable::new();
    let mut external_packages = ExternalPackageRegistry::new();
    let external_import_providers = crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::empty();
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

    (collected.input_files, string_table)
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
    let mut snapshot = source_files
        .iter()
        .map(|identity| {
            let logical_path = &identity.logical_path;
            let file_id = identity.id;
            for header in prepared
                .semantic
                .prepared_header_syntax
                .headers
                .iter()
                .filter(|header| header.source_file == *logical_path)
            {
                assert_eq!(header.tokens.file_id, Some(file_id));
                assert_eq!(
                    header.tokens.canonical_os_path.as_deref(),
                    identity.canonical_os_path.as_deref()
                );
                assert!(
                    header
                        .tokens
                        .tokens
                        .iter()
                        .all(|token| token.location.scope == *logical_path),
                    "header token locations must use the final logical source scope"
                );
                assert!(
                    header
                        .tokens
                        .path_syntax
                        .paths()
                        .iter()
                        .all(|path| path.location.scope == *logical_path),
                    "header path locations must use the final logical source scope"
                );
            }
            let clauses = module_symbols
                .file_dependency_clauses_by_source
                .get(logical_path)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let shell_ids = clauses
                .iter()
                .map(|clause| {
                    assert_eq!(clause.location.scope, *logical_path);
                    assert_eq!(clause.dependency.location.scope, *logical_path);
                    assert_eq!(
                        clause.dependency.dependency_shell_id.source, file_id,
                        "rebased shell source must belong to its owning prepared file"
                    );
                    clause.dependency.dependency_shell_id
                })
                .collect::<Vec<_>>();
            let selections = module_symbols
                .dependency_selections_by_source
                .get(logical_path)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let selected_source_names = selections
                .iter()
                .map(|selection| {
                    assert_eq!(selection.source_location.scope, *logical_path);
                    if let Some(alias) = &selection.local_alias {
                        assert_eq!(alias.location.scope, *logical_path);
                    }
                    prepared
                        .semantic
                        .string_table
                        .resolve(selection.source_name)
                        .to_owned()
                })
                .collect::<Vec<_>>();

            SyntheticPreparedIdentitySnapshot {
                logical_path: logical_path.to_portable_string(&prepared.semantic.string_table),
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
    let (input_files, mut string_table) =
        collect_synthetic_inputs_for_test(&entry_file_path, &resolver, &style_directives);
    let source_files = Arc::new(
        SourceDatabase::build(
            input_files.iter().map(PreparedSourceInput::source_path),
            &entry_file_path,
            Some(&resolver),
            &mut string_table,
        )
        .expect("synthetic source database should build"),
    );
    let source_byte_count = input_files
        .iter()
        .map(PreparedSourceInput::source_byte_len)
        .sum();
    let local_string_table = string_table.fork_source().fork_for_module().into_parts().0;
    let preparation_context = super::module_preparation::ModulePreparationContext {
        source_files: &source_files,
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
            &entry_file_path,
            local_string_table,
            source_byte_count,
        )
        .expect("synthetic outputs should prepare against the retained source table");
    synthetic_prepared_identity_snapshot(&prepared, &source_files)
}

#[test]
fn synthetic_rebinding_makes_file_and_shell_identities_discovery_order_independent() {
    let forward = synthetic_identity_fixture(&["alpha", "beta"]);
    let reversed = synthetic_identity_fixture(&["beta", "alpha"]);

    assert_eq!(forward, reversed);
    assert_eq!(
        forward
            .iter()
            .map(|file| file.logical_path.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha.moth", "beta.moth", "main.moth"],
        "final source identities must use deterministic logical paths"
    );
    assert_eq!(
        forward.iter().map(|file| file.file_id).collect::<Vec<_>>(),
        vec![
            SourceId::from_index(1),
            SourceId::from_index(2),
            SourceId::from_index(3),
        ],
        "final SourceIds must come from the sorted complete closure"
    );
    assert_eq!(
        forward[2].shell_ids,
        vec![
            DependencyShellId::new(SourceId::from_index(3), 0),
            DependencyShellId::new(SourceId::from_index(3), 1)
        ]
    );
    assert_eq!(forward[2].selected_source_names, vec!["greet", "greet"]);
}

#[test]
fn synthetic_preparation_reuses_complete_outputs_for_one_final_header_pass() {
    let _test_guard = lock_source_read_counter_tests();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("main.moth"), "@helper greet\n").expect("should write entry");
    fs::write(
        root.join("helper.moth"),
        "greet||:\n    io.line([: [\"hello\"]])\n;\n",
    )
    .expect("should write helper");

    let entry_file_path =
        fs::canonicalize(root.join("main.moth")).expect("entry should canonicalize");
    let helper_file_path =
        fs::canonicalize(root.join("helper.moth")).expect("helper should canonicalize");
    let canonical_root = fs::canonicalize(&root).expect("fixture root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);
    crate::compiler_frontend::reset_file_frontend_prepare_count_for_test(&canonical_root);
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let _counter_capture =
        crate::compiler_frontend::instrumentation::capture_frontend_counters_for_test();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    crate::compiler_frontend::instrumentation::reset_frontend_counters();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counter_guard =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");

    let config = Config::new(root.clone());
    let resolver = configured_resolver(&config);
    let style_directives = test_style_directives();
    let (input_files, mut string_table) =
        collect_synthetic_inputs_for_test(&entry_file_path, &resolver, &style_directives);
    let source_files = Arc::new(
        SourceDatabase::build(
            input_files.iter().map(PreparedSourceInput::source_path),
            &entry_file_path,
            Some(&resolver),
            &mut string_table,
        )
        .expect("synthetic source database should build"),
    );

    assert!(
        input_files
            .iter()
            .all(|input| matches!(input, PreparedSourceInput::MothPrepared { .. })),
        "every synthetic Moth source must carry its complete first preparation"
    );
    assert_eq!(
        super::source_loading::source_read_count_for_path_for_test(&entry_file_path),
        1,
        "the entry source must be read once"
    );
    assert_eq!(
        super::source_loading::source_read_count_for_path_for_test(&helper_file_path),
        1,
        "the imported source must be read once"
    );
    assert_eq!(
        crate::compiler_frontend::file_frontend_prepare_count_for_path_for_test(&entry_file_path),
        1,
        "the entry source must receive one first preparation"
    );
    assert_eq!(
        crate::compiler_frontend::file_frontend_prepare_count_for_path_for_test(&helper_file_path),
        1,
        "the imported source must receive one first preparation"
    );

    let source_byte_count = input_files
        .iter()
        .map(PreparedSourceInput::source_byte_len)
        .sum();
    let local_string_table = string_table.fork_source().fork_for_module().into_parts().0;
    let preparation_context = super::module_preparation::ModulePreparationContext {
        source_files: &source_files,
        style_directives: &style_directives,
        project_path_resolver: Some(resolver),
    };
    let stable_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local("synthetic-exactly-once"),
        Path::new(""),
        ModuleRootRole::Normal,
    )
    .expect("synthetic origin should construct");

    #[cfg(feature = "timers")]
    let prepared = preparation_context
        .prepare_module(
            stable_origin,
            input_files,
            &entry_file_path,
            local_string_table,
            source_byte_count,
            None,
        )
        .expect("retained synthetic outputs should prepare once");
    #[cfg(not(feature = "timers"))]
    let prepared = preparation_context
        .prepare_module(
            stable_origin,
            input_files,
            &entry_file_path,
            local_string_table,
            source_byte_count,
        )
        .expect("retained synthetic outputs should prepare once");

    assert_eq!(prepared.semantic.source_file_count, 2);
    assert_eq!(
        prepared
            .semantic
            .prepared_header_syntax
            .module_symbols
            .module_file_paths
            .len(),
        2,
        "one final header aggregation must retain both prepared source identities"
    );
    assert_eq!(
        super::source_loading::source_read_count_for_path_for_test(&entry_file_path),
        1,
        "final aggregation must not reread the entry source"
    );
    assert_eq!(
        super::source_loading::source_read_count_for_path_for_test(&helper_file_path),
        1,
        "final aggregation must not reread the imported source"
    );
    assert_eq!(
        crate::compiler_frontend::file_frontend_prepare_count_for_path_for_test(&entry_file_path),
        1,
        "final aggregation must not prepare the entry source again"
    );
    assert_eq!(
        crate::compiler_frontend::file_frontend_prepare_count_for_path_for_test(&helper_file_path),
        1,
        "final aggregation must not prepare the imported source again"
    );

    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    crate::compiler_frontend::instrumentation::log_frontend_counters();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let observations = counter_guard.finish();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert_eq!(counter_value("file_preparation_pass_count"), 2.0);
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert_eq!(counter_value("prepared_file_count"), 2.0);
}

#[test]
fn synthetic_diagnosed_preparation_is_not_consumed_again() {
    let _test_guard = lock_source_read_counter_tests();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("main.moth"), "@helper\n").expect("should write entry");
    fs::write(root.join("helper.moth"), "@core/math sin,\n")
        .expect("should write malformed helper");

    let config = Config::new(root.clone());
    let resolver = configured_resolver(&config);
    let style_directives = test_style_directives();
    let entry_file_path =
        fs::canonicalize(root.join("main.moth")).expect("entry should canonicalize");
    let helper_file_path =
        fs::canonicalize(root.join("helper.moth")).expect("helper should canonicalize");
    let canonical_root = fs::canonicalize(&root).expect("fixture root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);
    crate::compiler_frontend::reset_file_frontend_prepare_count_for_test(&canonical_root);
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let _counter_capture =
        crate::compiler_frontend::instrumentation::capture_frontend_counters_for_test();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    crate::compiler_frontend::instrumentation::reset_frontend_counters();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counter_guard =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let mut string_table = StringTable::new();
    let mut external_packages = ExternalPackageRegistry::new();
    let external_import_providers = crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::empty();
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

    let messages = match super::source_discovery::collect_reachable_input_files(
        &root.join("main.moth"),
        &resolver,
        &style_directives,
        &mut external_imports,
        &source_file_kinds,
        &mut resource_inputs,
        &mut string_table,
    ) {
        Ok(_) => panic!("malformed synthetic preparation should diagnose"),
        Err(messages) => messages,
    };
    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(
        diagnostics.len(),
        1,
        "a diagnosed synthetic preparation must not be consumed by a second preparation pass"
    );
    assert!(matches!(
        diagnostics[0].payload,
        DiagnosticPayload::InvalidDependencyClause { .. }
    ));
    assert_eq!(
        super::source_loading::source_read_count_for_path_for_test(&entry_file_path),
        1,
        "a diagnosed entry source must be read once"
    );
    assert_eq!(
        super::source_loading::source_read_count_for_path_for_test(&helper_file_path),
        1,
        "a diagnosed imported source must be read once"
    );
    assert_eq!(
        crate::compiler_frontend::file_frontend_prepare_count_for_path_for_test(&entry_file_path),
        1,
        "a diagnosed entry source must be prepared once"
    );
    assert_eq!(
        crate::compiler_frontend::file_frontend_prepare_count_for_path_for_test(&helper_file_path),
        1,
        "a diagnosed imported source must be prepared once"
    );

    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    crate::compiler_frontend::instrumentation::log_frontend_counters();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let observations = counter_guard.finish();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert_eq!(counter_value("file_preparation_pass_count"), 2.0);
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert_eq!(
        counter_value("prepared_file_count"),
        0.0,
        "diagnosed preparation attempts must not be counted as successful retained outputs"
    );
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

/// Discover modules and return the populated project module graph plus the shared string table
/// so focused Phase 5b invariant tests can inspect inserted edges and retained source locations.
fn discover_modules_and_graph_for_test(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> (
    ModuleCompilationSchedule,
    super::project_module_graph::ProjectModuleGraph,
    super::source_tree_index::SourceTreeIndex,
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

    let modules = discover_all_modules_in_project(
        config,
        resolver,
        &source_files,
        &mut project_module_graph,
        style_directives,
        &mut external_imports,
        DirectoryDependencyResolution::project(&module_namespace_set, &source_tree_index),
        &mut resource_inputs,
        &mut string_table,
        #[cfg(feature = "timers")]
        crate::timing::NO_TIMING_BOUNDARY,
    )
    .expect("module discovery should pass for focused graph-edge tests");

    (
        modules,
        project_module_graph,
        source_tree_index,
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

#[test]
fn source_tree_index_collects_one_scan_and_applies_skip_policy() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let entry_root = root.clone();
    let nested = entry_root.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module directory");

    for directory_name in [
        ".git",
        "target",
        "node_modules",
        "release",
        "dev",
        "dist",
        "build",
        ".cache",
        "generated",
        "scratch",
    ] {
        let directory = entry_root.join(directory_name);
        fs::create_dir_all(&directory).expect("should create skipped directory");
        fs::write(directory.join("@skipped.moth"), "").expect("should write skipped root");
    }

    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");
    fs::write(entry_root.join("ordinary.moth"), "").expect("should write ordinary source");
    fs::write(nested.join("@nested.moth"), "").expect("should write nested root");

    let mut config = Config::new(root.clone());
    config.html_section.dev_output = Some(String::from("scratch"));
    config.html_section.release_output = Some(String::from("generated"));
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();
    let validated_output_settings =
        crate::build_system::project_config::validate_directory_output_settings(
            &config,
            &mut string_table,
        )
        .expect("configured output folders should validate");

    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root.clone(),
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: Some(&validated_output_settings),
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("source tree index should build");

    assert_eq!(index.entry_root(), canonical_entry_root);
    let graph = super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&index);
    let entry_root_files: Vec<PathBuf> = graph
        .entry_modules()
        .iter()
        .map(|module_id| graph.node(*module_id).root_file().to_path_buf())
        .collect();
    assert_eq!(entry_root_files.len(), 2);
    assert!(entry_root_files[0].ends_with("@home.moth"));
    assert_eq!(index.stats().dirs_visited, 2);
    assert_eq!(index.stats().dirs_skipped, 10);
    assert_eq!(index.stats().files_seen, 3);
    assert_eq!(index.stats().normal_root_files_seen, 2);
    assert_eq!(index.stats().module_roots_found, 2);

    let root_directories = index
        .module_roots()
        .root_directories()
        .map(|path| root_directory_name(path.as_path()))
        .collect::<Vec<_>>();
    assert_eq!(
        root_directories[0],
        root_directory_name(&canonical_entry_root)
    );
    assert_eq!(root_directories[1], "nested");
}

#[test]
fn source_tree_index_ignores_collision_in_fixed_skipped_directory() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let entry_root = root.clone();

    // Fixed-skipped directory with collision-shaped contents. Configured output directories
    // remain outside the source-entry tree and do not receive a compatibility exception.
    let target_dir = entry_root.join("target");
    fs::create_dir_all(target_dir.join("helper")).expect("should create target/helper");
    fs::write(target_dir.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");

    // Real module root that should be discovered.
    let nested = entry_root.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module");
    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");
    fs::write(nested.join("@nested.moth"), "").expect("should write nested root");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();

    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("fixed-skipped collision-shaped inputs must not trigger collision diagnostics");

    let graph = super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&index);
    assert_eq!(graph.entry_modules().len(), 2);
    assert_eq!(index.stats().dirs_skipped, 1);
}

#[test]
fn source_tree_index_ignores_package_prefix_collision_in_skipped_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let entry_root = root.join("src");
    fs::create_dir_all(&entry_root).expect("should create entry root");

    // Fixed-skipped directory whose name matches a source-backed package prefix.
    // Under the skip policy this folder is not dependency-bindable, so no prefix collision.
    fs::create_dir_all(entry_root.join("target")).expect("should create target folder");
    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::default();
    source_packages.register_filesystem_root(
        "target",
        fs::canonicalize(entry_root.join("target")).unwrap(),
        PackageOrigin::Builder,
    );

    let mut string_table = StringTable::new();
    super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: None,
        },
        &config,
        &source_packages,
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("skipped folder matching a package prefix must not trigger prefix collision");
}

#[test]
fn source_tree_index_detects_collision_in_non_skipped_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let entry_root = root.join("src");
    fs::create_dir_all(entry_root.join("helper")).expect("should create helper folder");
    fs::write(entry_root.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");
    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();

    let messages = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect_err("non-skipped bst/folder collision should be rejected");

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourceFileFolderCollision { .. }
    ));
}

#[test]
fn bounded_module_roots_for_single_file_indexes_nested_roots_with_ignored_directories() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let module_dir = root.join("module");
    let nested = module_dir.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module");

    // Ignored directory with collision-shaped contents.
    let target_dir = module_dir.join("target");
    fs::create_dir_all(target_dir.join("helper")).expect("should create target/helper");
    fs::write(target_dir.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");

    fs::write(module_dir.join("@home.moth"), "").expect("should write entry root");
    fs::write(nested.join("@nested.moth"), "").expect("should write nested root");

    let config = Config::new(root.clone());
    let entry_file = fs::canonicalize(module_dir.join("@home.moth")).unwrap();
    let mut string_table = StringTable::new();

    let module_roots =
        super::source_tree_index::SourceTreeIndex::bounded_module_roots_for_single_file(
            &entry_file,
            &config,
            &crate::builder_surface::SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect("single-file normal module root should index its tree without collision errors");

    let root_directories = module_roots
        .root_directories()
        .map(|path| root_directory_name(path.as_path()))
        .collect::<Vec<_>>();
    assert_eq!(root_directories.len(), 2);
    assert!(root_directories.contains(&"module"));
    assert!(root_directories.contains(&"nested"));
}

#[test]
fn bounded_module_roots_for_single_file_rejects_dependency_name_collisions() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let module_dir = root.join("module");
    fs::create_dir_all(module_dir.join("helper")).expect("should create helper directory");
    fs::write(module_dir.join("helper.moth"), "helper #= 1\n")
        .expect("should write colliding source file");
    fs::write(module_dir.join("@home.moth"), "").expect("should write entry root");

    let config = Config::new(root.clone());
    let entry_file = fs::canonicalize(module_dir.join("@home.moth")).unwrap();
    let mut string_table = StringTable::new();

    let messages = super::source_tree_index::SourceTreeIndex::bounded_module_roots_for_single_file(
        &entry_file,
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect_err("single-file normal module roots should reject real dependency-name collisions");

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourceFileFolderCollision { .. }
    ));
}

#[test]
fn source_tree_index_rejects_duplicate_normal_module_root_files() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let entry_root = root.join("src");
    fs::create_dir_all(&entry_root).expect("should create entry root");
    fs::write(entry_root.join("@home.moth"), "").expect("should write page root");
    fs::write(entry_root.join("@layout.moth"), "").expect("should write layout root");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();
    let messages = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect_err("a module directory may contain only one normal module root");

    let reason = first_invalid_config_reason(&messages);
    let InvalidConfigReason::MultipleModuleRootFiles {
        directory,
        candidates,
    } = reason
    else {
        panic!("expected duplicate module root diagnostic, got {reason:?}");
    };
    assert_eq!(
        *directory,
        string_table.intern(&fs::canonicalize(&entry_root).unwrap().display().to_string())
    );
    assert_eq!(candidates.len(), 2);
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
        authored_import_location: SourceLocation::default(),
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

#[test]
fn parses_config_constant_declarations() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n    version = \"1.2.3\",\n    template_const_loop_iteration_limit = 10001,\n|\nhtml #= |\n    page_url_style = \"trailing_slash\",\n    redirect_index_html = true,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test_with_html_keys(&mut config, &config_path, &style_directives)
        .expect("config should parse");

    assert_eq!(config.entry_root, PathBuf::from("src"));
    assert_eq!(config.project_name, "docs");
    assert_eq!(config.version, Some("1.2.3".to_owned()));
    assert_eq!(
        config.html_section.page_url_style.as_deref(),
        Some("trailing_slash")
    );
    assert_eq!(config.html_section.redirect_index_html, Some(true));
    assert_eq!(
        config.html_section.dev_output.as_deref(),
        Some("dev"),
        "the html section's schema defaults should own the output roots"
    );
    assert_eq!(
        config.html_section.release_output.as_deref(),
        Some("release")
    );
    assert_eq!(config.template_const_loop_iteration_limit, 10001);
}

#[test]
fn persists_direct_project_config_resolution_records_in_live_config() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let config_path = root.join(settings::CONFIG_FILE_NAME);
    fs::write(
        &config_path,
        "project #= |\n    name = \"docs\",\n    author #Config of String?,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root);
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("optional direct config should parse");

    assert_eq!(config.author, None);
    assert_eq!(config.config_resolution_records.len(), 1);
    assert_ne!(config.config_resolution_records[0].fingerprint.0, 0);
}

#[test]
fn loads_canonical_config_file_from_project_root() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::create_dir_all(root.join("src/alpha")).expect("should create first module directory");
    fs::create_dir_all(root.join("src/beta")).expect("should create second module directory");
    fs::write(
        root.join("src/alpha/@alpha.moth"),
        "@drawing.js as drawing\nvalue = 1\n",
    )
    .expect("should write first module root");
    fs::write(
        root.join("src/alpha/drawing.js"),
        "export function draw() {}\n",
    )
    .expect("should write first module provider");
    fs::write(root.join("src/beta/@beta.moth"), "").expect("should write second module root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let mut frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    let provider_calls = Arc::new(AtomicUsize::new(0));
    frontend_surface
        .external_import_providers
        .register(Arc::new(ResolvingCountingProvider::new(Arc::clone(
            &provider_calls,
        ))));
    let build_config_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let services = ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    let mut string_table = StringTable::new();
    let mut source_files = SourceDatabase::empty();

    let validated_output_settings = load_project_config(
        &mut config,
        &services,
        &mut string_table,
        Some(&mut source_files),
    )
    .expect("canonical config should load");

    assert_eq!(config.config_file_path(), root.join("config.moth"));
    assert_eq!(config.entry_root, PathBuf::from("src"));

    let mut project_source_files = Some(Arc::new(source_files));
    let frontend = compile_project_frontend_with_inputs(
        &mut config,
        crate::build_system::BuildProfile::Dev,
        validated_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
        &mut project_source_files,
        &build_config_inputs,
        FrontendCompilationMode::Canonical,
    )
    .expect("directory frontend should extend the config source database");

    assert_eq!(
        provider_calls.load(Ordering::SeqCst),
        1,
        "the provider-backed alpha source should resolve its physical provider once",
    );
    let alpha = frontend
        .project
        .successful_artefacts_in_module_id_order()
        .find(|artefact| {
            artefact
                .module
                .metadata
                .entry_point
                .ends_with(Path::new("alpha/@alpha.moth"))
        })
        .expect("expected alpha module artefact");
    let alpha_provider_package_paths = alpha
        .module
        .link_facts
        .external_import_candidates
        .iter()
        .filter_map(|candidate| {
            alpha
                .module
                .link_facts
                .external_package_registry
                .get_package_by_id(candidate.package_id)
                .map(|package| package.path.clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        alpha_provider_package_paths,
        vec!["@test/resolving".to_owned()],
        "alpha should retain the provider candidate resolved for its own source",
    );

    let project_source_files = project_source_files
        .as_ref()
        .expect("directory frontend should retain the project source database");
    let canonical_config = fs::canonicalize(root.join(settings::CONFIG_FILE_NAME))
        .expect("config should canonicalize");
    let canonical_alpha =
        fs::canonicalize(root.join("src/alpha/@alpha.moth")).expect("alpha should canonicalize");
    let canonical_beta =
        fs::canonicalize(root.join("src/beta/@beta.moth")).expect("beta should canonicalize");
    let canonical_alpha_provider =
        fs::canonicalize(root.join("src/alpha/drawing.js")).expect("provider should canonicalize");
    assert_eq!(
        project_source_files
            .get_by_canonical_path(&canonical_config)
            .expect("config should have a source identity")
            .id,
        SourceId::from_index(1)
    );
    assert_eq!(
        project_source_files
            .get_by_canonical_path(&canonical_alpha)
            .expect("alpha should have a source identity")
            .id,
        SourceId::from_index(2)
    );
    assert_eq!(
        project_source_files
            .get_by_canonical_path(&canonical_alpha_provider)
            .expect("provider should have a source identity")
            .id,
        SourceId::from_index(3),
    );
    assert_eq!(
        project_source_files
            .get_by_canonical_path(&canonical_beta)
            .expect("beta should have a source identity")
            .id,
        SourceId::from_index(4)
    );
}

#[test]
fn applies_grouped_project_record_to_config_fields() {
    // The grouped `project #= |...|` record validates against the project schema root and
    // applies its compiler-owned fields; open metadata is accepted and dropped.
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n    version = \"1.2.3\",\n    custom_channel = \"alpha\",\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("a grouped project record should parse and apply");

    assert_eq!(config.project_name, "docs");
    assert_eq!(config.entry_root, PathBuf::from("src"));
    assert_eq!(config.version, Some("1.2.3".to_owned()));
}

#[test]
fn grouped_project_record_accepts_declare_first_helper_records() {
    // Record-valued helpers must fold into the grouped project record without becoming
    // UnknownKey on the closed file-root schema.
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "project_metadata #= |\n    channel = \"alpha\",\n|\nproject #= |\n    name = \"docs\",\n    metadata = project_metadata,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("a declare-first helper record should fold into the grouped project record");

    assert_eq!(config.project_name, "docs");
}

#[test]
fn directory_projects_require_config_moth() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create a directory project entry root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    let build_config_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let services = ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    let mut string_table = StringTable::new();
    let mut source_files = SourceDatabase::empty();

    let messages = load_project_config(
        &mut config,
        &services,
        &mut string_table,
        Some(&mut source_files),
    )
    .expect_err("a directory project without config.moth must fail");
    assert!(
        messages.diagnostics().any(|diagnostic| matches!(
            diagnostic.payload,
            crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::MissingConfigFile,
                ..
            }
        )),
        "expected MissingConfigFile, got {messages:?}"
    );
}

#[test]
fn rejects_direct_canonical_config_dependency_paths() {
    let mut string_table = StringTable::new();

    for dependency_path in ["config", "config.moth"] {
        let path = crate::compiler_frontend::symbols::interned_path::InternedPath::from_single_str(
            dependency_path,
            &mut string_table,
        );

        assert!(
            crate::compiler_frontend::source_packages::root_file::dependency_path_references_config_file(
                &path,
                &string_table,
            ),
            "direct config import should be treated as a special file: {dependency_path}"
        );
    }

    let mut nested_source_path =
        crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    nested_source_path.push_str("config", &mut string_table);
    nested_source_path.push_str("init_config", &mut string_table);

    assert!(
        !crate::compiler_frontend::source_packages::root_file::dependency_path_references_config_file(
            &nested_source_path,
            &string_table,
        ),
        "a folder named config must remain a valid source path prefix"
    );

    let mut ordinary_config_path =
        crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    ordinary_config_path.push_str("config", &mut string_table);
    ordinary_config_path.push_str("project", &mut string_table);

    assert!(
        !crate::compiler_frontend::source_packages::root_file::dependency_path_references_config_file(
            &ordinary_config_path,
            &string_table,
        ),
        "a nested path with a non-config final component remains a valid source path"
    );
}

#[test]
fn rejects_unknown_config_key() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "custom_key #= \"custom_value\"\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::MissingProjectRecord,
                ..
            }
        ),
        "a helper-only config.moth still requires a project record, got: {:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_section_output_inside_or_equal_to_entry_root() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    // `entry_root` covers `src`, so an html `release_output` of `src/out` is inside the
    // entry root. The builder surface is required because the section is the only html
    // authoring path.
    fs::write(
        &config_path,
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    release_output = \"src/out\",\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages =
        parse_project_config_for_test_with_html_keys(&mut config, &config_path, &style_directives)
            .expect_err("output inside entry root should fail");

    let diagnostic = first_error_diagnostic(&messages);
    let DiagnosticPayload::InvalidConfig {
        reason:
            InvalidConfigReason::InvalidOutputFolder {
                reason: InvalidOutputFolderReason::InsideOrEqualToEntryRoot,
                ..
            },
        ..
    } = &diagnostic.payload
    else {
        panic!(
            "expected InsideOrEqualToEntryRoot diagnostic, got: {:?}",
            diagnostic.payload
        );
    };

    // The output roots resolve from the grouped section now, so the diagnostic underlines the
    // section's value location on its authored line. Locations are 0-indexed.
    assert_eq!(
        diagnostic
            .primary_location
            .scope
            .to_path_buf(&messages.string_table),
        config_path.as_path(),
        "the diagnostic should point at the authored config file"
    );
    assert_eq!(
        diagnostic.primary_location.start_pos.line_number, 4,
        "the html section is authored after the grouped project record"
    );
}

#[test]
fn rejects_config_plain_and_mutable_bindings() {
    // Both `=` and `~=` produce the same `PlainBindingUnsupported` reason. The canonical
    // `config_plain_project_rejected` and `config_mutable_key_rejected` cases cover the
    // user-visible rejection; this unit retains the typed reason for both binding modes.
    for (operator, label) in [("=", "plain"), ("~=", "mutable")] {
        let _temp1 = tempfile::tempdir().expect("should create temp dir");
        let root = _temp1.path().to_path_buf();
        fs::create_dir_all(&root).expect("should create root dir");
        let config_path = root.join(settings::CONFIG_FILE_NAME);

        fs::write(&config_path, format!("entry_root {operator} \"src\"\n"))
            .expect("should write config");

        let mut config = Config::new(root.clone());
        let style_directives = test_style_directives();
        let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
            .expect_err("config should reject binding");

        let diagnostic = first_error_diagnostic(&messages);
        assert!(
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::PlainBindingUnsupported,
                    ..
                }
            ),
            "unexpected diagnostic payload for {label} binding: {:?}",
            diagnostic.payload
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

#[test]
fn parses_config_explicit_hash_binding_mode() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "version #String = \"1.0\"\nproject #= |\n    name = \"docs\",\n    entry_root = \"src\",\n    version = version,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("config should parse");

    assert_eq!(config.entry_root, PathBuf::from("src"));
    assert_eq!(config.project_name, "docs");
    assert_eq!(config.version, Some("1.0".to_owned()));
}

#[test]
fn rejects_config_function_declarations() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "helper ||:\n    entry_root = \"src\"\n;\n")
        .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::FunctionUnsupported,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_config_named_support_type_declarations() {
    // User-authored `config.moth` is a flat surface of anonymous const records and
    // compiler-owned constants: named structs, choices and type aliases are rejected by the
    // compiler-owned config dialect validator, and record-shaped helpers must be declared as
    // anonymous const records instead.
    let cases = [
        (
            "struct",
            "SupportType = |\n    value String,\n|\nproject #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
        ),
        (
            "choice",
            "Mode ::\n    Ready,\n;\nproject #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
        ),
        (
            "alias",
            "EntryRoot as String\nentry_root #EntryRoot = \"src\"\n",
        ),
    ];

    for (case_name, source) in cases {
        let _temp2 = tempfile::tempdir().expect("should create temp dir");
        let root = _temp2.path().to_path_buf();
        fs::create_dir_all(&root).expect("should create root dir");
        let config_path = root.join(settings::CONFIG_FILE_NAME);

        fs::write(&config_path, source).expect("should write config");

        let mut config = Config::new(root.clone());
        let style_directives = test_style_directives();
        let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
            .expect_err("named support types should be rejected");

        let diagnostic = first_error_diagnostic(&messages);
        assert!(
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::NamedTypeUnsupported,
                    ..
                }
            ),
            "unexpected diagnostic payload for {case_name}: {:?}",
            diagnostic.payload
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

#[test]
fn rejects_config_standalone_template() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "[: hello]\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::StandaloneTemplateUnsupported,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_config_const_page_fragment() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "#[: hello]\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::StandaloneTemplateUnsupported,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_project_local_config_dependency_even_when_module_root_exists() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::create_dir_all(root.join("settings")).expect("should create settings module");
    fs::write(root.join("settings/@mod.moth"), "value #= \"src\"\n")
        .expect("should write settings root");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "@settings value\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidDependencyClause {
                reason: InvalidDependencyClauseReason::DependencyClauseNotAllowed,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_builder_config_dependency_without_discovering_the_package_root() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let package_root = root.join("builder/defaults");
    fs::create_dir_all(&package_root).expect("should create Builder package folder");
    fs::write(package_root.join("@first.moth"), "value #= 1\n")
        .expect("should write first invalid package root");
    fs::write(package_root.join("@second.moth"), "value #= 2\n")
        .expect("should write second invalid package root");

    let config_path = root.join(settings::CONFIG_FILE_NAME);
    fs::write(&config_path, "@defaults value\n").expect("should write config dependency");

    let mut frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "defaults",
        package_root,
        PackageOrigin::Builder,
    );

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test_with_packages(
        &mut config,
        &config_path,
        &style_directives,
        &frontend_surface,
    )
    .expect_err("config dependency should fail before source-package discovery");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidDependencyClause {
                reason: InvalidDependencyClauseReason::DependencyClauseNotAllowed,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn project_local_package_folder_does_not_register_source_metadata() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let package_root = root.join("packages/widgets");
    fs::create_dir_all(&package_root).expect("should create project-local package");
    fs::create_dir_all(root.join("src")).expect("should create entry root");
    fs::write(root.join("src/@page.moth"), "x ~= 1\n").expect("should write entry");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");

    let mut string_table = StringTable::new();
    let resolver = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect("ordinary project-local folders must not affect canonical Stage 0");

    assert!(
        resolver.source_package_roots().is_empty(),
        "ordinary project-local folders must not register source packages"
    );
}

#[test]
fn ordinary_package_folder_does_not_collide_with_entry_root() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/helper")).expect("should create src/helper");
    fs::create_dir_all(root.join("lib/helper")).expect("should create lib/helper");
    fs::write(root.join("src/@page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/helper/@mod.moth"), "foo #= 1\n").expect("should write root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let result = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    );

    let resolver = result.expect("ordinary package folders are not canonical package roots");
    assert!(resolver.source_package_roots().is_empty());
}

#[test]
fn accepts_config_const_record_field_projection() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "Defaults #= |\n    entry_root = \"src\",\n|\n\nproject #= |\n    name = \"docs\",\n    entry_root = Defaults.entry_root,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("config with const-record field projection should succeed");

    assert_eq!(
        config.entry_root,
        PathBuf::from("src"),
        "entry_root should resolve through const-record field projection"
    );
}

#[test]
fn malformed_dependency_path_keeps_precise_location_during_module_discovery() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@core//math sin\n#[:ok]\n")
        .expect("should write malformed entry");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    let messages = match discover_modules_for_test_messages(&config, &resolver, &style_directives) {
        Ok(_) => panic!("malformed dependency path should fail discovery"),
        Err(messages) => messages,
    };

    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = diagnostics[0];
    assert_eq!(
        diagnostic
            .primary_location
            .scope
            .to_path_buf(&messages.string_table),
        src.join("@page.moth")
            .canonicalize()
            .expect("entry path should canonicalize")
    );
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 0);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 1);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidPath {
                path_kind: PathKind::EmptyComponent
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn config_dependency_parse_failure_keeps_precise_location_in_compiler_messages() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);
    fs::write(&config_path, "@core/math sin\n").expect("should write invalid config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = diagnostics[0];
    assert_eq!(
        diagnostic
            .primary_location
            .scope
            .to_path_buf(&messages.string_table),
        config_path
    );
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 0);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 1);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidDependencyClause {
                reason: InvalidDependencyClauseReason::DependencyClauseNotAllowed,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn discover_modules_uses_reachable_files_only() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("libs")).expect("should create libs folder");
    fs::create_dir_all(src.join("styles")).expect("should create styles folder");
    fs::create_dir_all(src.join("docs")).expect("should create docs folder");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::create_dir_all(src.join("errors")).expect("should create errors folder");
    fs::write(src.join("@page.moth"), "@libs/html basic\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("errors/@404.moth"), "#[:404]\n").expect("should write 404");
    fs::write(src.join("libs/html.moth"), "basic #= [:basic]\n").expect("should write lib");
    fs::write(src.join("styles/docs.moth"), "navbar #= [:nav]\n").expect("should write style");
    fs::write(src.join("docs/outdated.moth"), "this is invalid syntax")
        .expect("should write outdated file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config parse");
    let resolver = configured_resolver(&config);
    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(modules.len(), 2);

    let page_module = modules
        .iter()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                == Some(OsStr::new("@page.moth"))
        })
        .expect("should include #page module");
    assert_eq!(
        page_module.prepared.semantic.source_file_count, 2,
        "frontend preparation should retain only the reachable entry and provider sources"
    );
    let page_paths: HashSet<_> = module_prepared_source_names(page_module)
        .into_iter()
        .collect();

    assert!(page_paths.contains("@page.moth"));
    assert!(page_paths.contains("html.moth"));
    assert!(!page_paths.contains("outdated.moth"));
}

#[test]
fn discover_modules_resolves_relative_child_dependencies() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("components")).expect("should create components folder");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@components/widget\nio.line([: [\"page\"]])\n",
    )
    .expect("should write page");
    fs::write(
        src.join("components/widget.moth"),
        "@components/common\nwidget #= common\n",
    )
    .expect("should write widget file");
    fs::write(src.join("components/common.moth"), "common #= \"common\"\n")
        .expect("should write common");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(modules.len(), 1, "expected exactly one entry module");

    let discovered: HashSet<_> = module_prepared_source_names(modules[0])
        .into_iter()
        .collect();

    assert!(discovered.contains("@page.moth"));
    assert!(discovered.contains("widget.moth"));
    assert!(discovered.contains("common.moth"));
}

#[test]
fn dependency_clause_keeps_one_cross_module_edge_for_multiple_selections() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("child")).expect("should create child module dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@child greet, farewell\nio.line([: [\"page\"]])\n",
    )
    .expect("should write page");
    fs::write(
        src.join("child/@mod.moth"),
        "export:\n    greet || -> String:\n        return \"hi\"\n    ;\n    farewell || -> String:\n        return \"bye\"\n    ;\n;\n",
    )
    .expect("should write child module root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let (waves, provider_bindings, _, _) = modules.into_parts();
    let modules: Vec<_> = waves.iter().flatten().collect();
    assert_eq!(
        modules.len(),
        2,
        "entry and child module must both be discovered"
    );

    let shells: Vec<_> = provider_bindings
        .iter()
        .map(|edge| edge.dependency_shell_id)
        .collect();
    assert_eq!(
        shells,
        vec![
            crate::compiler_frontend::symbols::identity::DependencyShellId::new(
                crate::compiler_frontend::source::SourceId::from_index(1),
                0
            )
        ],
        "one authored clause must publish one provider graph edge"
    );
}

#[test]
fn module_root_relative_dependency_resolves_from_the_entry_root() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let lib = root.join("lib");
    fs::create_dir_all(src.join("helpers")).expect("should create source helpers");
    fs::create_dir_all(lib.join("helpers")).expect("should create root-folder helpers");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@helpers/theme\nio.line([: [\"page\"]])\n",
    )
    .expect("should write page");
    fs::write(
        src.join("helpers/theme.moth"),
        "source_theme #= \"source\"\n",
    )
    .expect("should write source");
    fs::write(
        lib.join("helpers/theme.moth"),
        "package_theme #= \"package\"\n",
    )
    .expect("should write root-folder helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(modules.len(), 1, "expected exactly one entry module");

    let source_theme = fs::canonicalize(src.join("helpers/theme.moth")).expect("canonical source");
    let package_theme =
        fs::canonicalize(lib.join("helpers/theme.moth")).expect("canonical package file");
    let discovered_paths = module_source_paths(modules[0]);

    assert!(
        discovered_paths.contains(&source_theme),
        "module-root-relative dependencies should resolve from the entry root"
    );
    assert!(
        !discovered_paths.contains(&package_theme),
        "module-root-relative resolution must not pull in an unrelated same-stem package file"
    );
}

#[test]
fn synthetic_module_root_resolution_prefers_owning_nested_module() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let child = src.join("child");
    fs::create_dir_all(&child).expect("should create nested module directory");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "io.line([: [\"page\"]])\n")
        .expect("should write entry module root");
    fs::write(src.join("helpers.moth"), "").expect("should write entry-root namesake");
    fs::write(child.join("@mod.moth"), "").expect("should write nested module root");
    fs::write(child.join("renderer.moth"), "@helpers\n")
        .expect("should write nested declaring_source");
    fs::write(child.join("helpers.moth"), "").expect("should write nested module dependency");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config parse");
    let resolver = configured_resolver(&config);

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

    let collected = super::source_discovery::collect_reachable_input_files(
        &child.join("renderer.moth"),
        &resolver,
        &style_directives,
        &mut external_imports,
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut ResourceInputRegistry::new(),
        &mut string_table,
    )
    .expect("synthetic nested traversal should succeed");

    let discovered_paths: HashSet<_> = collected
        .input_files
        .iter()
        .map(|input| input.source_path().to_path_buf())
        .collect();
    let entry_namesake =
        fs::canonicalize(src.join("helpers.moth")).expect("canonical entry namesake");
    let nested_dependency =
        fs::canonicalize(child.join("helpers.moth")).expect("canonical nested dependency");

    assert!(
        discovered_paths.contains(&nested_dependency),
        "synthetic bare dependencies must resolve from their owning nested module root"
    );
    assert!(
        !discovered_paths.contains(&entry_namesake),
        "an entry-root namesake must not shadow an owning nested module dependency"
    );
}

#[test]
fn discover_all_modules_finds_normal_roots_across_multiple_directories() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("nested")).expect("should create nested folder");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "io.line([: [\"page\"]])\n")
        .expect("should write entry normal root");
    fs::create_dir_all(src.join("layout")).expect("should create layout folder");
    fs::write(
        src.join("layout/@layout.moth"),
        "io.line([: [\"layout\"]])\n",
    )
    .expect("should write layout normal root");
    fs::write(src.join("nested/@lib.moth"), "io.line([: [\"lib\"]])\n")
        .expect("should write nested normal root");
    fs::write(src.join("nested/file.moth"), "io.line([: [\"regular\"]])\n")
        .expect("should write regular");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(modules.len(), 3, "expected one module per root directory");

    let entry_names = modules
        .iter()
        .map(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect::<HashSet<_>>();

    assert!(entry_names.contains("@page.moth"));
    assert!(entry_names.contains("@layout.moth"));
    assert!(entry_names.contains("@lib.moth"));
}

#[test]
fn directory_stage0_resolves_resource_from_consuming_module_root() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("assets")).expect("should create assets directory");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "unused #= @assets/logo.svg\n#[:ok]\n",
    )
    .expect("should write entry");
    fs::write(src.join("assets/logo.svg"), "resource bytes").expect("should write resource");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("resource reference should resolve during Stage 0");
    let module = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("entry identity should be retained")
                .file_name()
                .is_some_and(|name| name == "@page.moth")
        })
        .expect("entry module should be discovered");

    let references = module.prepared.semantic.resolved_file_references.iter();
    let references = references.collect::<Vec<_>>();
    assert_eq!(references.len(), 1);
    assert!(matches!(
        &references[0].outcome,
        ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
            owner_relative_path,
            ..
        }) if owner_relative_path.as_str() == "assets/logo.svg"
    ));
}

#[test]
fn directory_stage0_retains_missing_resource_diagnostic_without_aborting_discovery() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source directory");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "unused #= @assets/missing.svg\n#[:ok]\n",
    )
    .expect("should write entry");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("missing user-authored resource should remain a retained outcome");
    let module = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("entry identity should be retained")
                .file_name()
                .is_some_and(|name| name == "@page.moth")
        })
        .expect("entry module should be discovered");
    let references = module
        .prepared
        .semantic
        .resolved_file_references
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(references.len(), 1);
    assert!(matches!(
        references[0].outcome,
        ResolvedFileReferenceOutcome::Diagnostic(_)
    ));
}

#[test]
fn directory_stage0_identifies_moth_value_without_preparing_it() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source directory");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "helpers = @missing.moth\n#[:ok]\n")
        .expect("should write entry");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect(".moth value should be identified during Stage 0");
    let module = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("entry identity should be retained")
                .file_name()
                .is_some_and(|name| name == "@page.moth")
        })
        .expect("entry module should be discovered");
    assert_eq!(module_prepared_source_names(module), vec!["@page.moth"]);
    let references = module
        .prepared
        .semantic
        .resolved_file_references
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(references.len(), 1);
    assert!(matches!(
        references[0].outcome,
        ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::IdentifiedSourceKind)
    ));
}

#[test]
fn directory_stage0_classifies_not_a_directory_as_typed_path_failure() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source directory");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "value #= @not_a_directory/value.mtf\n#[:ok]\n",
    )
    .expect("should write entry");
    fs::write(src.join("not_a_directory"), "regular file").expect("should write blocker file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("NotADirectory should remain a typed path outcome");
    let module = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("entry identity should be retained")
                .file_name()
                .is_some_and(|name| name == "@page.moth")
        })
        .expect("entry module should be discovered");
    let references = module
        .prepared
        .semantic
        .resolved_file_references
        .iter()
        .collect::<Vec<_>>();
    assert!(matches!(
        &references[0].outcome,
        ResolvedFileReferenceOutcome::Diagnostic(diagnostic)
            if matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidCompileTimePath {
                    reason: InvalidCompileTimePathReason::TargetNotRegular,
                    ..
                }
            )
    ));
}

#[test]
fn directory_stage0_rejects_missing_targets_under_child_module_roots_without_watch() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("child")).expect("should create child module directory");
    fs::create_dir_all(src.join("support")).expect("should create support module directory");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "child_missing #= @child/missing.svg\nchild_existing #= @child/existing.svg\nsupport_missing #= @support/missing.svg\nsupport_existing #= @support/existing.svg\n#[:ok]\n",
    )
    .expect("should write entry");
    fs::write(src.join("child/existing.svg"), "child resource")
        .expect("should write child resource");
    fs::write(src.join("support/existing.svg"), "support resource")
        .expect("should write support resource");
    fs::write(src.join("child/@child.moth"), "#[:ok]\n").expect("should write child root");
    fs::write(
        src.join("support/+support.moth"),
        "export:\n    render || -> String:\n        return \"support\"\n    ;\n;\n",
    )
    .expect("should write support root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    let (modules, resource_inputs) =
        discover_modules_for_test_with_resource_inputs(&config, &resolver, &style_directives)
            .expect("child-boundary failures should remain retained outcomes");
    let module = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("entry identity should be retained")
                .file_name()
                .is_some_and(|name| name == "@page.moth")
        })
        .expect("entry module should be discovered");
    let reasons = module
        .prepared
        .semantic
        .resolved_file_references
        .iter()
        .filter_map(|reference| match &reference.outcome {
            ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => match &diagnostic.payload {
                DiagnosticPayload::InvalidCompileTimePath { reason, .. } => Some(*reason),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(reasons.len(), 4);
    assert!(
        reasons
            .iter()
            .all(|reason| *reason == InvalidCompileTimePathReason::EscapesModuleBoundary)
    );
    assert!(resource_inputs.records().is_empty());
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn directory_stage0_rejects_missing_symlink_ancestors_without_watch() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let _outside_root = tempfile::tempdir().expect("should create outside temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let child = src.join("child");
    let support = src.join("support");
    fs::create_dir_all(&child).expect("should create child module directory");
    fs::create_dir_all(&support).expect("should create support module directory");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "external #= @external-alias/missing.svg\nchild #= @child-alias/missing.svg\nsupport #= @support-alias/missing.svg\ndangling_external #= @dangling-external/missing.svg\ndangling_child #= @dangling-child/missing.svg\ndangling_support #= @dangling-support/missing.svg\n#[:ok]\n",
    )
    .expect("should write entry");
    fs::write(child.join("@child.moth"), "#[:ok]\n").expect("should write child root");
    fs::write(
        support.join("+support.moth"),
        "export:\n    render || -> String:\n        return \"support\"\n    ;\n;\n",
    )
    .expect("should write support root");
    symlink(_outside_root.path(), src.join("external-alias"))
        .expect("should create external alias");
    symlink(&child, src.join("child-alias")).expect("should create child alias");
    symlink(&support, src.join("support-alias")).expect("should create support alias");
    symlink(
        _outside_root.path().join("missing-target"),
        src.join("dangling-external"),
    )
    .expect("should create dangling external alias");
    symlink(child.join("missing-target"), src.join("dangling-child"))
        .expect("should create dangling child alias");
    symlink(support.join("missing-target"), src.join("dangling-support"))
        .expect("should create dangling support alias");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    let (modules, resource_inputs) =
        discover_modules_for_test_with_resource_inputs(&config, &resolver, &style_directives)
            .expect("symlink-ancestor failures should remain retained outcomes");
    let module = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("entry identity should be retained")
                .file_name()
                .is_some_and(|name| name == "@page.moth")
        })
        .expect("entry module should be discovered");
    let reasons = module
        .prepared
        .semantic
        .resolved_file_references
        .iter()
        .filter_map(|reference| match &reference.outcome {
            ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => match &diagnostic.payload {
                DiagnosticPayload::InvalidCompileTimePath { reason, .. } => Some(*reason),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(reasons.len(), 6);
    assert!(
        reasons.iter().all(
            |reason| *reason == InvalidCompileTimePathReason::EscapesSymlink
                || *reason == InvalidCompileTimePathReason::EscapesModuleBoundary
        ),
        "unexpected symlink-ancestor reasons: {reasons:?}"
    );
    assert!(resource_inputs.records().is_empty());
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[test]
fn multi_module_retained_path_diagnostic_keeps_its_module_string_table() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let nested = src.join("nested");
    fs::create_dir_all(nested.join("assets")).expect("should create nested module directory");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@a.moth"), "#[:ok]\n").expect("should write first root");
    fs::write(
        nested.join("@b.moth"),
        "asset #= @Assets/logo.svg\n#[:ok]\n",
    )
    .expect("should write second root");
    fs::write(nested.join("assets/logo.svg"), "resource").expect("should write resource");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should retain the path diagnostic");
    let module = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("entry identity should be retained")
                .file_name()
                .is_some_and(|name| name == "@b.moth")
        })
        .expect("nested module should be discovered");
    let reference = module
        .prepared
        .semantic
        .resolved_file_references
        .iter()
        .next()
        .expect("nested module should retain one file reference");
    let diagnostic = match &reference.outcome {
        ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => diagnostic,
        outcome => panic!("expected a retained diagnostic, got {outcome:?}"),
    };
    let DiagnosticPayload::InvalidCompileTimePath {
        reason: InvalidCompileTimePathReason::CaseMismatch { provided, expected },
        ..
    } = &diagnostic.payload
    else {
        panic!("expected a retained case-mismatch diagnostic");
    };
    assert_eq!(
        module.prepared.semantic.string_table.resolve(*provided),
        "Assets"
    );
    assert_eq!(
        module.prepared.semantic.string_table.resolve(*expected),
        "assets"
    );
    let rendered = crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_with_context(
        diagnostic,
        crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext::new(
            &module.prepared.semantic.string_table,
        ),
    );
    assert!(rendered.contains("Assets") && rendered.contains("assets"));
}

#[test]
fn synthetic_stage0_resolves_content_and_resource_references() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let docs = root.join("docs");
    let assets = root.join("assets");
    fs::create_dir_all(&docs).expect("should create docs directory");
    fs::create_dir_all(&assets).expect("should create assets directory");
    let entry = root.join("main.moth");
    fs::write(
        &entry,
        "template #= @docs/intro.mtf\nmarkdown #= @docs/notes.md\nlogo #= @assets/logo.svg\nsource #= @helper.moth\n",
    )
    .expect("should write synthetic entry");
    fs::write(
        docs.join("intro.mtf"),
        "template body [@docs/second.mtf] [@assets/logo.svg]\n",
    )
    .expect("should write template");
    fs::write(docs.join("second.mtf"), "second template body\n")
        .expect("should write transitive template");
    fs::write(docs.join("notes.md"), "# notes\n").expect("should write markdown");
    fs::write(assets.join("logo.svg"), "resource bytes").expect("should write resource");
    fs::write(root.join("helper.moth"), "value #= 1\n").expect("should write source target");

    let config = Config::new(root.clone());
    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
    source_file_kinds.register("md", SourceFileKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);
    let style_directives = test_style_directives();
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
    let mut resource_inputs = ResourceInputRegistry::new();

    let collected = super::source_discovery::collect_reachable_input_files(
        &entry,
        &resolver,
        &style_directives,
        &mut external_imports,
        &source_file_kinds,
        &mut resource_inputs,
        &mut string_table,
    )
    .expect("synthetic structural references should resolve");

    let input_paths = collected
        .input_files
        .iter()
        .map(|input| {
            input
                .source_path()
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<HashSet<_>>();
    assert!(input_paths.contains("main.moth"));
    assert!(input_paths.contains("intro.mtf"));
    assert!(input_paths.contains("second.mtf"));
    assert!(input_paths.contains("notes.md"));
    assert!(!input_paths.contains("helper.moth"));

    let references = collected.resolved_file_references;
    assert_eq!(references.len(), 6);
    assert!(references.iter().any(|reference| {
        reference.class == PreparedFileReferenceClass::ContentSource
            && matches!(reference.outcome, SingleFileReferenceOutcome::Source { .. })
    }));
    assert!(references.iter().any(|reference| {
        reference.class == PreparedFileReferenceClass::ResourceFile
            && matches!(
                reference.outcome,
                SingleFileReferenceOutcome::Resource { .. }
            )
    }));
    assert!(references.iter().any(|reference| {
        reference.class == PreparedFileReferenceClass::SourceKindNoFileValue
            && matches!(
                reference.outcome,
                SingleFileReferenceOutcome::IdentifiedSourceKind
            )
    }));
    assert_eq!(resource_inputs.records().len(), 1);
    resource_inputs
        .validate()
        .expect("resource registry is coherent");
}

#[test]
fn ordinary_synthetic_stage0_rejects_child_and_support_boundaries() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = fs::canonicalize(_tmp_root.path()).expect("test root should canonicalize");
    let child = root.join("child");
    let support = root.join("support");
    fs::create_dir_all(&child).expect("should create child directory");
    fs::create_dir_all(&support).expect("should create support directory");
    let entry = root.join("main.moth");
    fs::write(
        &entry,
        "child_content #= @child/existing.mtf\nsupport_resource #= @support/existing.svg\nchild_missing #= @child/missing.mtf\nsupport_missing #= @support/missing.svg\n",
    )
    .expect("should write synthetic entry");
    fs::write(child.join("@child.moth"), "#[:ok]\n").expect("should write child root");
    fs::write(child.join("existing.mtf"), "template body\n").expect("should write child content");
    fs::write(
        support.join("+support.moth"),
        "export:\n    render || -> String:\n        return \"support\"\n    ;\n;\n",
    )
    .expect("should write support root");
    fs::write(support.join("existing.svg"), "resource").expect("should write support resource");

    let source_file_kinds = {
        let mut kinds = crate::builder_surface::SourceFileKindRegistry::new();
        kinds.register("mtf", SourceFileKind::MothTemplate);
        kinds
    };
    let resolver = ProjectPathResolver::new_with_module_roots(
        root.clone(),
        root,
        PreparedSourcePackageRoots::empty(),
        &source_file_kinds,
        crate::compiler_frontend::paths::module_roots::ModuleRootTable::empty(),
    )
    .expect("synthetic resolver should build without prepared module roots");

    let style_directives = test_style_directives();
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
    let mut resource_inputs = ResourceInputRegistry::new();
    let collected = super::source_discovery::collect_reachable_input_files(
        &entry,
        &resolver,
        &style_directives,
        &mut external_imports,
        &source_file_kinds,
        &mut resource_inputs,
        &mut string_table,
    )
    .expect("ordinary synthetic Stage 0 should retain boundary diagnostics");

    assert_eq!(collected.input_files.len(), 1);
    assert_eq!(collected.resolved_file_references.len(), 4);
    assert!(collected.resolved_file_references.iter().all(|reference| {
        matches!(
            &reference.outcome,
            SingleFileReferenceOutcome::Diagnostic(diagnostic)
                if matches!(
                    &diagnostic.payload,
                    DiagnosticPayload::InvalidCompileTimePath {
                        reason: InvalidCompileTimePathReason::EscapesModuleBoundary,
                        ..
                    }
                )
        )
    }));
    assert!(resource_inputs.records().is_empty());
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[test]
fn compile_single_file_frontend_retains_ordinary_boundary_registry() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = fs::canonicalize(_tmp_root.path()).expect("test root should canonicalize");
    let child = root.join("child");
    let support = root.join("support");
    fs::create_dir_all(&child).expect("should create child directory");
    fs::create_dir_all(&support).expect("should create support directory");
    let entry = root.join("main.moth");
    fs::write(
        &entry,
        "child_content #= @child/existing.mtf\nsupport_resource #= @support/existing.svg\nchild_missing #= @child/missing.mtf\nsupport_missing #= @support/missing.svg\n#[:ok]\n",
    )
    .expect("should write synthetic entry");
    fs::write(child.join("@child.moth"), "#[:ok]\n").expect("should write child root");
    fs::write(child.join("existing.mtf"), "template body\n").expect("should write child content");
    fs::write(
        support.join("+support.moth"),
        "export:\n    render || -> String:\n        return \"support\"\n    ;\n;\n",
    )
    .expect("should write support root");
    fs::write(support.join("existing.svg"), "resource").expect("should write support resource");

    let config = Config::new(entry.clone());
    let mut builder_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    builder_surface
        .source_file_kinds
        .register("mtf", SourceFileKind::MothTemplate);
    let mut string_table = StringTable::new();
    let frontend = super::compilation::compile_single_file_frontend(
        &config,
        crate::compiler_frontend::FrontendBuildProfile::Dev,
        &test_style_directives(),
        &mut builder_surface,
        entry.extension().expect("entry should have an extension"),
        &mut string_table,
    )
    .expect("ordinary synthetic frontend should retain user diagnostics");

    // AST now consumes file-value paths through Stage 0's resolved table, so the
    // boundary-rejected value occurrences in the entry surface as this module's retained user
    // diagnostics and the module is published as diagnosed. Stage 0 outcome rows are inspected by
    // the preceding production discovery test; this invocation proves the complete frontend
    // handoff preserves the physical registry even on the diagnosed path.
    assert_eq!(
        frontend
            .project
            .successful_artefacts_in_module_id_order()
            .count(),
        0
    );
    assert_eq!(frontend.project.diagnosed.len(), 1);
    assert!(frontend.has_diagnosed_or_blocked());
    assert!(frontend.resource_inputs.records().is_empty());
    assert!(
        frontend
            .resource_inputs
            .missing_watch_interests()
            .is_empty()
    );
}

#[test]
fn directory_stage0_reaches_content_reference_fixed_point() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("docs")).expect("should create docs directory");
    fs::create_dir_all(src.join("assets")).expect("should create assets directory");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "unused #= @docs/one.mtf\n#[:ok]\n")
        .expect("should write entry");
    fs::write(
        src.join("docs/one.mtf"),
        "[@docs/two.mtf]\n[@assets/logo.svg]\n[@assets/logo.svg]\n",
    )
    .expect("should write first content source");
    fs::write(src.join("docs/two.mtf"), "second content\n")
        .expect("should write second content source");
    fs::write(src.join("assets/logo.svg"), "resource bytes").expect("should write resource");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("content references should reach a fixed point");
    let module = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("entry identity should be retained")
                .file_name()
                .is_some_and(|name| name == "@page.moth")
        })
        .expect("entry module should be discovered");

    let prepared_sources = module_prepared_source_names(module);
    assert!(prepared_sources.contains(&"@page.moth".to_owned()));
    assert!(prepared_sources.contains(&"one.mtf".to_owned()));
    assert!(
        prepared_sources.contains(&"two.mtf".to_owned()),
        "prepared sources: {prepared_sources:?}"
    );

    let references = module
        .prepared
        .semantic
        .resolved_file_references
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(
        references.len(),
        4,
        "each physical authored occurrence is retained"
    );
    assert!(
        references
            .iter()
            .all(|reference| matches!(reference.outcome, ResolvedFileReferenceOutcome::Target(_)))
    );
    let resource_ids = references
        .iter()
        .filter_map(|reference| match &reference.outcome {
            ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
                source,
                ..
            }) => Some(*source),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(resource_ids.len(), 2);
    assert_eq!(resource_ids[0], resource_ids[1]);
}

#[test]
fn accepts_folded_template_initializer_for_compile_time_config_binding() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "project #= |\n    name = [:docs],\n|\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("folded template initializer should be accepted");

    assert_eq!(
        config.project_name, "docs",
        "folded template should become config string value"
    );
}

#[test]
fn accepts_config_local_reference_to_earlier_private_const() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "version #= \"0.2.0\"\nproject #= |\n    name = \"docs\",\n    version = version,\n    author = version,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("config with private const reference should succeed");

    assert_eq!(
        config.version,
        Some("0.2.0".to_owned()),
        "version should be set"
    );
    assert_eq!(
        config.author,
        Some("0.2.0".to_owned()),
        "author should resolve through private const reference"
    );
}

#[test]
fn rejects_config_unresolved_local_reference() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "helper #= missing_value\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(&diagnostic.payload, DiagnosticPayload::UnknownName { .. }),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_config_non_compile_time_constant_value() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "project #= Error(\"bad\").message\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::CompileTimeEvaluationError {
                reason: CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_duplicate_plain_config_bindings_before_config_validation() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "entry_root = \"src\"\nentry_root = \"other\"\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    // The frontend catches duplicate start-body declarations as assignments to immutable variables
    // before config validation runs.
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidAssignmentTarget {
                reason: InvalidAssignmentTargetReason::ImmutableBinding,
                ..
            }
        ),
        "expected immutable-assignment diagnostic for duplicate private keys, got: {:?}",
        diagnostic.payload
    );
}

#[test]
fn accepts_config_scalar_private_helper() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "helper #= \"src\"\nproject #= |\n    name = \"docs\",\n    entry_root = helper,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("scalar helpers should fold into the project record");

    assert_eq!(config.entry_root, PathBuf::from("src"));
}

#[test]
fn rejects_config_runtime_call_in_value() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "project #= io.line([: [\"hello\"]])\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::CompileTimeEvaluationError {
                reason: CompileTimeEvaluationErrorReason::ExternalFunctionCallInConstantContext,
                ..
            }
        ),
        "expected external-function-call-in-constant-context diagnostic for runtime call, got: {:?}",
        diagnostic.payload
    );
}

// ── Config value shape enforcement tests ──────────────────────────────────────

#[test]
fn accepts_valid_bool_config_keys() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "project #= |\n    name = \"docs\",\n|\nhtml #= |\n    redirect_index_html = false,\n    html_inject_core_css = true,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test_with_html_keys(&mut config, &config_path, &style_directives)
        .expect("valid boolean config values should parse");

    assert_eq!(config.html_section.redirect_index_html, Some(false));
    assert_eq!(config.html_section.html_inject_core_css, Some(true));
}

#[test]
fn rejects_core_string_key_with_bool_value() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "project #= |\n    name = \"docs\",\n    entry_root = true,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let reason = first_invalid_config_reason(&messages);
    let InvalidConfigReason::InvalidConfigValueShape { expected } = reason else {
        panic!("expected invalid config value shape, got {reason:?}");
    };
    assert_eq!(
        messages.string_table.resolve(*expected),
        "a string value",
        "string-key shape mismatch must report the expected string shape"
    );
}

#[test]
fn rejects_backend_bool_key_with_string_value() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "html #= |\n    redirect_index_html = \"false\",\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages =
        parse_project_config_for_test_with_html_keys(&mut config, &config_path, &style_directives)
            .expect_err("config should fail");

    let reason = first_invalid_config_reason(&messages);
    let InvalidConfigReason::InvalidConfigValueShape { expected } = reason else {
        panic!("expected invalid config value shape, got {reason:?}");
    };
    assert_eq!(
        messages.string_table.resolve(*expected),
        "a boolean value",
        "backend bool-key shape mismatch must report the expected bool shape"
    );
}

#[test]
fn accepts_config_local_reference_after_shape_enforcement() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "version #= \"0.2.0\"\nproject #= |\n    name = \"docs\",\n    entry_root = version,\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("config with local const reference should succeed");

    assert_eq!(
        config.entry_root,
        PathBuf::from("0.2.0"),
        "entry_root should be set through const reference"
    );
}

#[test]
fn detects_duplicate_top_level_config_constants() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "project #= |\n    name = \"docs\",\n|\nproject #= |\n    name = \"other\",\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::DuplicateKey,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

// ── Canonical config identity tests ──────────────────────────────────────────

#[test]
fn authored_config_keeps_non_canonical_spelling_in_duplicate_diagnostic() {
    // The caller-provided config path spelling is preserved as the authored source-location
    // identity even when it is non-canonical. The resolver directory comes only from the
    // canonical config parent, while diagnostics keep the authored spelling.
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::create_dir_all(root.join("sub")).expect("should create sub dir");
    let config_path = root.join("config.moth");
    fs::write(
        &config_path,
        "project #= |\n    name = \"docs\",\n|\nproject #= |\n    name = \"other\",\n|\n",
    )
    .expect("should write config");

    // Spell the config path with a `..` detour so it is not equal to its canonical form.
    let non_canonical_config_path = root.join("sub").join("..").join("config.moth");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages =
        parse_project_config_for_test(&mut config, &non_canonical_config_path, &style_directives)
            .expect_err("duplicate authored config key should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::DuplicateKey,
                ..
            }
        ),
        "expected authored duplicate key diagnostic, got: {:?}",
        diagnostic.payload
    );

    // The diagnostic location scope must keep the non-canonical authored spelling, proving the
    // same interned identity used for tokenization and classification is preserved for rendering.
    let rendered_scope = diagnostic
        .primary_location
        .scope
        .to_portable_string(&messages.string_table);
    assert!(
        rendered_scope.contains("sub") && rendered_scope.contains(".."),
        "expected non-canonical authored spelling in diagnostic scope, got: {rendered_scope}"
    );
}

#[test]
fn authored_config_resolver_uses_canonical_parent_for_noncanonical_spelling() {
    // A non-canonical config path that detours through a sibling directory must still derive
    // the resolver directory from the canonical config parent and apply the config value.
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::create_dir_all(root.join("sub")).expect("should create sub dir");
    let config_path = root.join("config.moth");
    fs::write(
        &config_path,
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let non_canonical_config_path = root.join("sub").join("..").join("config.moth");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &non_canonical_config_path, &style_directives)
        .expect("non-canonical authored spelling should resolve and apply config");

    assert_eq!(config.entry_root, PathBuf::from("src"));
}

#[test]
fn project_local_lib_directory_is_ignored_as_source_package_root() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::create_dir_all(root.join("lib/helper")).expect("should create lib/helper");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/@page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/helper/@mod.moth"), "foo #= 1\n").expect("should write root");
    fs::write(root.join("lib/helper/utils.moth"), "bar #= 2\n").expect("should write lib file");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();
    let resolver = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect("resolver should build");

    assert!(
        resolver.source_package_roots().is_empty(),
        "the legacy lib folder must not become a source-backed package root"
    );

    // Dependency path `@helper/utils` must not resolve through the legacy lib folder.
    let mut path = crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    path.push_str("helper", &mut string_table);
    path.push_str("utils", &mut string_table);

    let declaring_source = root.join("src/@page.moth");
    assert!(
        resolver
            .resolve_dependency_to_source_file(&path, &declaring_source, &mut string_table)
            .is_err(),
        "legacy lib folders must not resolve as source packages"
    );
}

#[test]
fn builder_package_prefix_is_independent_of_ordinary_lib_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::create_dir_all(root.join("lib/html")).expect("should create lib/html");
    fs::create_dir_all(root.join("builder/html")).expect("should create builder/html");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/@page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/html/@mod.moth"), "foo #= 1\n").expect("should write root");
    fs::write(root.join("builder/html/@mod.moth"), "foo #= 1\n")
        .expect("should write builder package root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut builder_frontend_surface = crate::builder_surface::SourcePackageRegistry::new();
    builder_frontend_surface.register_filesystem_root(
        "html",
        root.join("builder/html"),
        PackageOrigin::Builder,
    );

    let result = super::project_roots::build_project_path_resolver(
        &config,
        &builder_frontend_surface,
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    );

    let resolver = result.expect("builder package should remain independently registered");
    assert_eq!(
        resolver.source_package_roots().get("html"),
        Some(&fs::canonicalize(root.join("builder/html")).unwrap())
    );
}

#[test]
fn entry_root_requires_at_least_one_root_entry_file() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let resolver = configured_resolver(&config);
    let Err(messages) = discover_modules_for_test(&config, &resolver, &style_directives) else {
        panic!("entry root without @*.moth entries should fail");
    };

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::NoRootModuleEntries { .. }
    ));
}

// ── Phase 4 project-structure collision tests ─────────────────────────────────

#[test]
fn rejects_reserved_project_globals_module_folder_at_entry_root() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/project")).expect("should create project module folder");
    fs::write(root.join("src/@page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let messages = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect_err("entry-root project folder must not claim @project");

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::ProjectGlobalsNameReserved
    ));
}

#[test]
fn rejects_reserved_project_globals_module_root_file() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/@project.moth"), "x ~= 1\n")
        .expect("should write reserved module root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let messages = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect_err("@project.moth must not create a real project module");

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::ProjectGlobalsNameReserved
    ));
}

#[test]
fn rejects_reserved_project_globals_facade_root_file() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/@page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("+project.moth"), "export:\n;\n")
        .expect("should write reserved facade root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let messages = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect_err("+project.moth must not claim the reserved project root");

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::ProjectGlobalsNameReserved
    ));
}

#[test]
fn rejects_nested_reserved_project_globals_dependency() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let resolver = configured_resolver(&config);
    for (source, expected_kind) in [
        (
            "@project/details\nx ~= 1\n",
            DependencyClauseKind::Namespace,
        ),
        (
            "@project/details value\nx ~= 1\n",
            DependencyClauseKind::DirectSelection,
        ),
        (
            "@project/details as project_details\nx ~= 1\n",
            DependencyClauseKind::NamespaceAlias,
        ),
    ] {
        fs::write(root.join("src/@page.moth"), source).expect("should write entry");
        let Err(messages) = discover_modules_for_test(&config, &resolver, &style_directives) else {
            panic!("nested @project dependency must be rejected");
        };
        let diagnostic = first_error_diagnostic(&messages);
        assert!(
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidDependencyClause {
                    clause_kind,
                    reason: InvalidDependencyClauseReason::ProjectGlobalsPathReserved,
                } if *clause_kind == expected_kind
            ),
            "got {:?}, expected {:?}",
            diagnostic.payload,
            expected_kind
        );
    }
}

#[test]
fn source_package_rejects_exact_reserved_project_globals_dependency() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let src = root.join("src");
    let package_root = root.join("builder/helper");
    fs::create_dir_all(&src).expect("should create src");
    fs::create_dir_all(&package_root).expect("should create helper package");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:entry]\n").expect("should write project entry");
    fs::write(package_root.join("@mod.moth"), "value ~= 1\n")
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
    let declaring_source = fs::canonicalize(root.join("builder/helper/@mod.moth"))
        .expect("package source should canonicalize");

    with_namespace_resolution(
        &config,
        &resolver,
        &source_packages,
        Some("helper"),
        |resolution, string_table| {
            let provider = provider_root(&["project"], string_table);
            let mut external_packages = ExternalPackageRegistry::new();
            let providers = ExternalImportProviderRegistry::empty();
            let mut cache =
                crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new();
            let mut resolution_table =
                crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
            let mut external_imports = super::source_discovery::ExternalImportDiscoveryState {
                external_packages: &mut external_packages,
                providers: &providers,
                cache: &mut cache,
                resolution_table: &mut resolution_table,
            };

            let Err(error) = super::source_discovery::resolve_structural_provider_reference(
                &provider,
                DependencyClauseKind::Namespace,
                &declaring_source,
                &resolver,
                &mut external_imports,
                *resolution,
                string_table,
            ) else {
                panic!("source-package @project dependency must be rejected");
            };
            let messages = error.into_messages(string_table);
            let diagnostic = first_error_diagnostic(&messages);
            assert!(matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDependencyClause {
                    clause_kind: DependencyClauseKind::Namespace,
                    reason: InvalidDependencyClauseReason::ProjectGlobalsPathReserved,
                }
            ));
        },
    );
}

#[test]
fn rejects_moth_file_and_folder_collision_in_same_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/UI")).expect("should create src/UI");
    fs::write(root.join("src/UI/@page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("src/ui.moth"), "y ~= 2\n").expect("should write colliding file");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let result = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    );

    assert!(
        result.is_err(),
        "ui.moth + UI/ collision should be rejected"
    );
    let messages = result.expect_err("checked above");
    assert_has_config_error(&messages);
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourceFileFolderCollision { .. }
    ));
}

#[test]
fn rejects_template_file_and_folder_collision_in_same_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/ui")).expect("should create src/ui");
    fs::write(root.join("src/ui/@page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("src/ui.mtf"), "template\n").expect("should write colliding file");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let messages = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect_err("ui.mtf + ui/ collision should be rejected");

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourceFileFolderCollision { .. }
    ));
}

#[test]
fn allows_same_stem_in_different_directories() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/components")).expect("should create src/components");
    fs::create_dir_all(root.join("src/pages")).expect("should create src/pages");
    fs::write(root.join("src/components/card.moth"), "x ~= 1\n").expect("should write card");
    fs::write(root.join("src/pages/card.moth"), "y ~= 2\n").expect("should write another card");
    fs::write(root.join("src/@page.moth"), "z ~= 3\n").expect("should write entry");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect("same stem in different directories should be allowed");
}

#[test]
fn rejects_collision_with_empty_folder() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/helper")).expect("should create src/helper");
    fs::write(root.join("src/helper.moth"), "x ~= 1\n").expect("should write colliding file");
    fs::write(root.join("src/@page.moth"), "y ~= 2\n").expect("should write entry");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let result = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    );

    assert!(
        result.is_err(),
        "collision with an empty folder should be rejected"
    );
    let messages = result.expect_err("checked above");
    assert_has_config_error(&messages);
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourceFileFolderCollision { .. }
    ));
}

#[test]
fn js_file_with_same_stem_as_folder_does_not_trigger_collision() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/helper")).expect("should create src/helper");
    fs::write(root.join("src/helper.js"), "// js\n").expect("should write js file");
    fs::write(root.join("src/@page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect(".js file with same stem as folder should not trigger collision");
}

#[test]
fn unsupported_js_import_without_provider_reports_moth_import_0021() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    // Entry file imports a .js file explicitly.
    fs::write(src.join("@page.moth"), "@drawing.js as drawing\n#[:ok]\n")
        .expect("should write entry");

    // The .js file actually exists on disk.
    fs::write(src.join("drawing.js"), "export function draw() {}\n").expect("should write js file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("unsupported .js import should fail discovery"),
        Err(messages) => messages,
    };

    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0021",
        "expected unsupported external extension diagnostic, got {:?}",
        diagnostic
    );
    if let DiagnosticPayload::UnsupportedExternalExtension { path, extension } = &diagnostic.payload
    {
        let path_text = path.to_portable_string(&messages.string_table);
        assert_eq!(path_text, "drawing.js", "unexpected path in diagnostic");
        assert_eq!(
            messages.string_table.resolve(*extension),
            "js",
            "unexpected extension in diagnostic"
        );
    } else {
        panic!(
            "expected UnsupportedExternalExtension payload, got {:?}",
            diagnostic.payload
        );
    }
}

#[test]
fn explicit_moth_extension_still_reports_moth_import_0020() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "@helper.moth\n#[:ok]\n").expect("should write entry");

    fs::write(
        src.join("helper.moth"),
        "greet || -> String:\n    return \"hi\"\n;\n",
    )
    .expect("should write helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("explicit .moth extension should fail discovery"),
        Err(messages) => messages,
    };

    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0020",
        "expected explicit .moth extension diagnostic, got {:?}",
        diagnostic
    );
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::ExplicitMothExtension { .. }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn unsupported_moth_template_dependency_without_builder_support_reports_moth_import_0025() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "@intro\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.mtf"), "hello\n").expect("should write moth template file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("unsupported .mtf dependency should fail discovery"),
        Err(messages) => messages,
    };

    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0025",
        "expected unsupported source file kind diagnostic, got {:?}",
        diagnostic
    );
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::UnsupportedSourceFileKind { .. }
    ));
}

#[test]
fn direct_moth_template_extension_dependency_reports_moth_import_0024() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "@intro.mtf\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.mtf"), "hello\n").expect("should write moth template file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("direct .mtf dependency should fail discovery"),
        Err(messages) => messages,
    };

    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0024",
        "expected explicit source extension diagnostic, got {:?}",
        diagnostic
    );
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::ExplicitSourceExtension { .. }
    ));
}

#[test]
fn moth_template_files_are_reachable_without_dependency_scanning() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "@intro\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.mtf"), "@missing\n").expect("should write moth template file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect(".mtf body text must not be scanned for dependencies");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    let input_paths: HashSet<_> = module_prepared_source_names(modules[0])
        .into_iter()
        .collect();
    assert!(input_paths.contains("@page.moth"));
    assert!(input_paths.contains("intro.mtf"));
    assert!(modules[0].prepared.contains_moth_template);
}

#[test]
fn reachable_moth_template_queues_same_directory_root_file() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let docs = src.join("docs");
    fs::create_dir_all(&docs).expect("should create docs dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "@docs\n#[:ok]\n").expect("should write entry");
    fs::write(docs.join("intro.mtf"), "hello\n").expect("should write moth template file");
    fs::write(docs.join("@docs.moth"), "@intro\ntitle #= \"Docs\"\n").expect("should write root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("reachable .mtf should discover same-directory normal module root");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    // The entry module depends on a Moth template file from the `docs` module root, so `docs` is its
    // provider and precedes it in the returned inventory order. Find the entry module by its
    // root file rather than assuming index 0.
    let entry_module = modules
        .iter()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                .is_some_and(|name| name == "@page.moth")
        })
        .expect("entry module should be discovered");
    let input_paths: HashSet<_> = module_prepared_source_names(entry_module)
        .into_iter()
        .collect();
    assert!(input_paths.contains("@page.moth"));
    assert!(!input_paths.contains("intro.mtf"));
    assert!(!input_paths.contains("@docs.moth"));

    let docs_module = modules
        .iter()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                .is_some_and(|name| name == "@docs.moth")
        })
        .expect("docs provider module should be discovered");

    let docs_input_names: HashSet<_> = module_prepared_source_names(docs_module)
        .into_iter()
        .collect();
    assert!(docs_input_names.contains("intro.mtf"));
    assert!(docs_input_names.contains("@docs.moth"));
    assert!(docs_module.prepared.contains_moth_template);
}

#[test]
fn unreferenced_moth_template_file_under_entry_root_is_ignored() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.mtf"), "hello\n").expect("should write moth template file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("unreferenced .mtf file should not affect discovery");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(module_prepared_source_names(modules[0]), vec!["@page.moth"]);
    assert!(!modules[0].prepared.contains_moth_template);
}

#[test]
fn extensionless_moth_dependency_and_virtual_package_dependency_still_work() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    // Normal extensionless dependencies still resolve as Moth source files, while virtual package
    // dependencies continue to stay out of Stage 0 filesystem traversal.
    fs::write(src.join("@page.moth"), "@helper\n@core/io line\n#[:ok]\n")
        .expect("should write entry");

    fs::write(
        src.join("helper.moth"),
        "greet || -> String:\n    return \"hi\"\n;\n",
    )
    .expect("should write helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(modules.len(), 1);

    let discovered: HashSet<_> = module_prepared_source_names(modules[0])
        .into_iter()
        .collect();

    assert!(discovered.contains("@page.moth"));
    assert!(discovered.contains("helper.moth"));
}

#[test]
fn indexed_module_inventory_includes_referenced_markdown_without_scanning_its_body() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "@intro\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "@missing\n").expect("should write markdown file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    source_file_kinds.register("md", crate::builder_surface::SourceFileKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect(".md body text must not be scanned for dependencies");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    let input_paths: HashSet<_> = module_prepared_source_names(modules[0])
        .into_iter()
        .collect();
    assert!(input_paths.contains("@page.moth"));
    assert!(input_paths.contains("intro.md"));

    assert!(!modules[0].prepared.contains_moth_template);
}

#[test]
fn indexed_module_inventory_excludes_unrelated_module_root_from_markdown_owner() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("other")).expect("should create other module dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "@intro\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "hello\n").expect("should write markdown file");
    fs::write(src.join("other/@other.moth"), "export:\n    x #= 1\n;\n")
        .expect("should write other module root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    source_file_kinds.register("md", crate::builder_surface::SourceFileKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("reachable .md should not queue an unrelated module root");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    let input_paths: HashSet<_> = module_prepared_source_names(modules[0])
        .into_iter()
        .collect();
    assert!(input_paths.contains("@page.moth"));
    assert!(input_paths.contains("intro.md"));
    assert!(!input_paths.contains("@other.moth"));

    assert!(!modules[0].prepared.contains_moth_template);
}

#[test]
fn indexed_module_inventory_ignores_unreferenced_markdown_file() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "hello\n").expect("should write markdown file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    source_file_kinds.register("md", crate::builder_surface::SourceFileKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("unreferenced .md file should not affect discovery");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(module_prepared_source_names(modules[0]), vec!["@page.moth"]);
    assert!(!modules[0].prepared.contains_moth_template);
}

#[test]
fn indexed_module_inventory_rejects_direct_markdown_extension_dependency() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "@intro.md\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "hello\n").expect("should write markdown file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("md", crate::builder_surface::SourceFileKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("direct .md dependency should fail discovery"),
        Err(messages) => messages,
    };

    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0024",
        "expected explicit source extension diagnostic, got {:?}",
        diagnostic
    );
    if let DiagnosticPayload::ExplicitSourceExtension { path, extension } = &diagnostic.payload {
        assert_eq!(
            path.to_portable_string(&messages.string_table),
            "intro.md",
            "unexpected dependency path in explicit source extension diagnostic"
        );
        assert_eq!(
            messages.string_table.resolve(*extension),
            "md",
            "unexpected extension in explicit source extension diagnostic"
        );
    } else {
        panic!(
            "expected ExplicitSourceExtension payload, got {:?}",
            diagnostic.payload
        );
    }
}

#[test]
fn indexed_module_inventory_rejects_unsupported_markdown_dependency() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("@page.moth"), "@intro\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "hello\n").expect("should write markdown file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("unsupported .md dependency should fail discovery"),
        Err(messages) => messages,
    };

    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0025",
        "expected unsupported source file kind diagnostic, got {:?}",
        diagnostic
    );
    if let DiagnosticPayload::UnsupportedSourceFileKind { path, extension } = &diagnostic.payload {
        assert_eq!(
            path.to_portable_string(&messages.string_table),
            "intro",
            "unexpected dependency path in unsupported source file kind diagnostic"
        );
        assert_eq!(
            messages.string_table.resolve(*extension),
            "md",
            "unexpected extension in unsupported source file kind diagnostic"
        );
    } else {
        panic!(
            "expected UnsupportedSourceFileKind payload, got {:?}",
            diagnostic.payload
        );
    }
}

#[test]
fn stage0_reuses_scanned_moth_source_when_assembling_input_files() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@helper\n#[:entry]\n").expect("should write entry");
    fs::write(src.join("helper.moth"), "message #= \"helper\"\n").expect("should write helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let _counter_guard = lock_source_read_counter_tests();
    let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);
    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(modules.len(), 1);
    for source in [src.join("@page.moth"), src.join("helper.moth")] {
        let canonical = fs::canonicalize(source).expect("source should canonicalize");
        assert_eq!(
            super::source_loading::source_read_count_for_path_for_test(&canonical),
            1,
            "each selected Moth source should be read exactly once"
        );
    }
    assert_eq!(
        module_prepared_source_names(modules[0]),
        vec!["@page.moth", "helper.moth"]
    );
}

#[test]
fn stage0_parallel_owned_batch_is_speculative_and_deterministic() {
    assert!(!super::source_discovery::should_parallelize_owned_source_preparation(15));
    assert!(super::source_discovery::should_parallelize_owned_source_preparation(16));

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@reachable\n@leaf\n#[:entry]\n")
        .expect("should write entry");
    fs::write(
        src.join("reachable.moth"),
        "@leaf\nreachable_value #= \"reachable_unique\"\n",
    )
    .expect("should write reachable source");
    fs::write(src.join("leaf.moth"), "leaf_value #= \"leaf_unique\"\n")
        .expect("should write leaf source");

    let unreachable_names = (0..13)
        .map(|index| format!("unreachable_{index:02}.moth"))
        .collect::<Vec<_>>();
    for (index, name) in unreachable_names.iter().enumerate() {
        fs::write(
            src.join(name),
            format!("unreachable_unique_{index:02} = \"unterminated\n"),
        )
        .expect("should write unreachable malformed source");
    }

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let _counter_guard = lock_source_read_counter_tests();
    let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);
    crate::compiler_frontend::reset_file_frontend_prepare_count_for_test(&canonical_root);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("unreachable malformed sources must not enter the module diagnostics");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(modules.len(), 1);
    let module = modules[0];

    assert_eq!(
        module.prepared.semantic.source_file_count, 3,
        "only the entry and its two reachable dependencies should be retained"
    );
    assert_eq!(
        module_prepared_source_names(module),
        vec!["@page.moth", "leaf.moth", "reachable.moth"],
        "retained source identities must stay in canonical source order"
    );

    let all_source_paths = std::iter::once(src.join("@page.moth"))
        .chain(std::iter::once(src.join("reachable.moth")))
        .chain(std::iter::once(src.join("leaf.moth")))
        .chain(unreachable_names.iter().map(|name| src.join(name)))
        .collect::<Vec<_>>();
    for source_path in &all_source_paths {
        let canonical_path = fs::canonicalize(source_path).expect("source should canonicalize");
        assert_eq!(
            super::source_loading::source_read_count_for_path_for_test(&canonical_path),
            1,
            "each owned candidate should be read exactly once, including speculative sources"
        );
    }

    let selected_source_paths = [
        src.join("@page.moth"),
        src.join("reachable.moth"),
        src.join("leaf.moth"),
    ];
    for source_path in selected_source_paths {
        let canonical_path = fs::canonicalize(source_path).expect("source should canonicalize");
        assert_eq!(
            crate::compiler_frontend::file_frontend_prepare_count_for_path_for_test(
                &canonical_path
            ),
            1,
            "each reachable source should receive one header preparation"
        );
    }
    for name in &unreachable_names {
        let canonical_path =
            fs::canonicalize(src.join(name)).expect("unreachable source should canonicalize");
        assert_eq!(
            crate::compiler_frontend::file_frontend_prepare_count_for_path_for_test(
                &canonical_path
            ),
            0,
            "unreachable speculative tokenizer failures must not enter header preparation"
        );
    }

    assert!(
        module
            .prepared
            .semantic
            .string_table
            .iter()
            .all(|(_, text)| !text.starts_with("unreachable_unique_")),
        "unreachable speculative token strings must not enter the retained module table"
    );

    let reachable_path =
        fs::canonicalize(src.join("reachable.moth")).expect("reachable source should canonicalize");
    let reachable_header = module
        .prepared
        .semantic
        .prepared_header_syntax
        .headers
        .iter()
        .find(|header| header.tokens.canonical_os_path.as_deref() == Some(reachable_path.as_path()))
        .expect("reachable source should retain one header stream");
    assert!(
        reachable_header
            .tokens
            .path_syntax
            .paths()
            .iter()
            .any(|path| path
                .root
                .to_portable_string(&module.prepared.semantic.string_table)
                == "leaf"),
        "reachable dependency path should survive the non-identity string remap"
    );
}

#[test]
fn stage0_loads_asset_sources_and_preserves_deterministic_input_order() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@intro\n@notes\n#[:entry]\n").expect("should write entry");
    fs::write(src.join("intro.mtf"), "moth template body\n").expect("should write moth template");
    fs::write(src.join("notes.md"), "# Markdown body\n").expect("should write markdown");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    source_file_kinds.register("md", crate::builder_surface::SourceFileKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("asset source discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(
        module_prepared_source_names(modules[0]),
        vec!["@page.moth", "intro.mtf", "notes.md"]
    );
    assert!(modules[0].prepared.contains_moth_template);
}

#[test]
fn stage0_parallel_missing_source_loading_preserves_input_order() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let source_paths = (0..super::source_discovery::STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES)
        .map(|index| {
            let path = root.join(format!("asset_{index}.md"));
            fs::write(&path, format!("# Asset {index}\n")).expect("should write markdown asset");
            path
        })
        .collect::<Vec<_>>();
    let mut string_table = StringTable::new();

    let input_files = super::source_discovery::load_missing_source_paths_for_test(
        source_paths,
        crate::builder_surface::SourceFileKind::PlainMarkdown,
        &mut string_table,
    )
    .expect("parallel missing source loading should pass");

    let loaded_names = input_files
        .iter()
        .map(|input| {
            input
                .source_path()
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<Vec<_>>();
    let expected_names = (0..super::source_discovery::STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES)
        .map(|index| format!("asset_{index}.md"))
        .collect::<Vec<_>>();

    assert_eq!(loaded_names, expected_names);
    for (index, input_file) in input_files.iter().enumerate() {
        match input_file {
            PreparedSourceInput::PlainMarkdown { source_code, .. } => {
                let expected_source = format!("# Asset {index}\n");
                assert_eq!(source_code.as_deref(), Some(expected_source.as_str()));
            }
            _ => panic!("missing-source loading should produce PlainMarkdown inputs"),
        }
    }
}

#[test]
fn stage0_missing_source_load_preserves_file_error_shape() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let missing_source = root.join("missing.md");
    let mut string_table = StringTable::new();

    let messages = super::source_discovery::load_missing_source_path_for_test(
        missing_source.clone(),
        crate::builder_surface::SourceFileKind::PlainMarkdown,
        &mut string_table,
    )
    .expect_err("missing source read should fail");

    let (_error_type, message, location) = messages
        .first_infrastructure_error_for_tests()
        .expect("expected infrastructure file error");
    assert!(
        message.contains("Error reading file when adding new moth files to parse"),
        "unexpected infrastructure message: {message}"
    );
    assert!(
        location
            .scope
            .to_portable_string(&messages.string_table)
            .contains("missing.md"),
        "missing source path should be preserved in the diagnostic location"
    );
}

#[test]
fn provider_backed_imports_are_resolved_without_becoming_source_inputs() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@drawing.js as drawing\n#[:entry]\n",
    )
    .expect("should write entry");
    fs::write(src.join("drawing.js"), "export function draw() {}\n").expect("should write js");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(CountingExternalImportProvider::new(Arc::clone(
        &calls,
    ))));

    let modules =
        discover_modules_for_test_with_providers(&config, &resolver, &style_directives, &providers)
            .expect("provider-backed import should resolve during discovery");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(module_prepared_source_names(modules[0]), vec!["@page.moth"]);
}

#[test]
fn synthetic_nested_module_provider_resolves_from_owning_module_root() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let feature = src.join("feature");
    fs::create_dir_all(&feature).expect("should create nested module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:entry]\n").expect("should write entry root");
    fs::write(
        feature.join("@page.moth"),
        "@drawing.js as drawing\n#[:feature]\n",
    )
    .expect("should write nested entry");
    fs::write(src.join("drawing.js"), "entry provider\n")
        .expect("should write conflicting entry provider");
    fs::write(feature.join("drawing.js"), "feature provider\n")
        .expect("should write nested provider");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    let nested_entry =
        fs::canonicalize(feature.join("@page.moth")).expect("nested entry should canonicalize");
    let nested_provider =
        fs::canonicalize(feature.join("drawing.js")).expect("nested provider should canonicalize");

    let recorded_paths = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(RecordingExternalImportProvider::new(Arc::clone(
        &recorded_paths,
    ))));
    let mut external_packages = ExternalPackageRegistry::new();
    let mut cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut resolution_table = crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
    let mut external_imports = super::source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: &providers,
        cache: &mut cache,
        resolution_table: &mut resolution_table,
    };
    let mut string_table = StringTable::new();

    super::source_discovery::collect_reachable_input_files(
        &nested_entry,
        &resolver,
        &style_directives,
        &mut external_imports,
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut ResourceInputRegistry::new(),
        &mut string_table,
    )
    .expect("synthetic nested provider should resolve");

    assert_eq!(
        *recorded_paths
            .lock()
            .expect("recorded provider paths lock poisoned"),
        vec![nested_provider],
        "prefix-free providers must resolve from the consuming file's owning module root"
    );
}

#[test]
fn synthetic_nested_provider_keys_do_not_collide_with_entry_relative_spellings() {
    for clauses in [
        "@feature/drawing.js as nested\n@drawing.js as local\n",
        "@drawing.js as local\n@feature/drawing.js as nested\n",
    ] {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        let feature = src.join("feature");
        fs::create_dir_all(feature.join("feature")).expect("should create nested provider folder");
        fs::write(
            root.join(settings::CONFIG_FILE_NAME),
            "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
        )
        .expect("should write config");
        fs::write(src.join("@page.moth"), "#[:entry]\n").expect("should write entry root");
        fs::write(
            feature.join("@page.moth"),
            format!("{clauses}#[:feature]\n"),
        )
        .expect("should write nested entry");
        fs::write(feature.join("drawing.js"), "local provider\n")
            .expect("should write local provider");
        fs::write(feature.join("feature/drawing.js"), "nested provider\n")
            .expect("should write nested provider");

        let mut config = Config::new(root.clone());
        let style_directives = test_style_directives();
        parse_project_config_for_test(
            &mut config,
            &root.join(settings::CONFIG_FILE_NAME),
            &style_directives,
        )
        .expect("config should parse");
        let resolver = configured_resolver(&config);
        let nested_entry =
            fs::canonicalize(feature.join("@page.moth")).expect("nested entry should canonicalize");

        let mut providers = ExternalImportProviderRegistry::empty();
        providers.register(Arc::new(CanonicalPathPackageProvider::new()));
        let mut external_packages = ExternalPackageRegistry::new();
        let mut cache = crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new();
        let mut resolution_table = crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
        let mut external_imports = super::source_discovery::ExternalImportDiscoveryState {
            external_packages: &mut external_packages,
            providers: &providers,
            cache: &mut cache,
            resolution_table: &mut resolution_table,
        };
        let mut string_table = StringTable::new();

        super::source_discovery::collect_reachable_input_files(
            &nested_entry,
            &resolver,
            &style_directives,
            &mut external_imports,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &mut ResourceInputRegistry::new(),
            &mut string_table,
        )
        .expect("both nested provider clauses should resolve");

        let source = "feature/@page.moth";
        let local = resolution_table
            .get(source, "drawing.js")
            .expect("local module-relative provider key should exist");
        let nested = resolution_table
            .get(source, "feature/drawing.js")
            .expect("nested module-relative provider key should exist");
        assert_ne!(
            local.package_id, nested.package_id,
            "accepted provider spellings must retain distinct packages in either clause order"
        );
    }
}

#[test]
fn canonical_multi_entry_discovery_is_deterministic_and_reads_each_source_once() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("page_a")).expect("should create page_a module");
    fs::create_dir_all(src.join("page_b")).expect("should create page_b module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    // Two entry points with independent module-local dependency trees.
    fs::write(
        src.join("page_a/@pageA.moth"),
        "@helper\n@a_only\n#[:pageA]\n",
    )
    .expect("should write pageA");
    fs::write(
        src.join("page_b/@pageB.moth"),
        "@helper\n@b_only\n#[:pageB]\n",
    )
    .expect("should write pageB");
    fs::write(src.join("page_a/helper.moth"), "helper #= 1\n").expect("should write helper A");
    fs::write(src.join("page_b/helper.moth"), "helper #= 2\n").expect("should write helper B");
    fs::write(src.join("page_a/a_only.moth"), "a #= 1\n").expect("should write a_only");
    fs::write(src.join("page_b/b_only.moth"), "b #= 1\n").expect("should write b_only");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let _counter_guard = lock_source_read_counter_tests();
    let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("canonical multi-entry discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    for source in [
        src.join("page_a/@pageA.moth"),
        src.join("page_a/helper.moth"),
        src.join("page_a/a_only.moth"),
        src.join("page_b/@pageB.moth"),
        src.join("page_b/helper.moth"),
        src.join("page_b/b_only.moth"),
    ] {
        let canonical = fs::canonicalize(source).expect("source should canonicalize");
        assert_eq!(
            super::source_loading::source_read_count_for_path_for_test(&canonical),
            1,
            "each selected Moth source should be read exactly once"
        );
    }
    assert_eq!(modules.len(), 2, "expected two discovered modules");

    // Module order must follow deterministic entry-point order.
    let module_names: Vec<_> = modules
        .iter()
        .map(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert_eq!(module_names, vec!["@pageA.moth", "@pageB.moth"]);

    // Per-module input order must be deterministic.
    let module_a_inputs = module_prepared_source_names(modules[0]);
    let module_b_inputs = module_prepared_source_names(modules[1]);

    // Canonical source order is deterministic by logical path (file name within this test).
    assert_eq!(
        module_a_inputs,
        vec!["@pageA.moth", "a_only.moth", "helper.moth"]
    );
    assert_eq!(
        module_b_inputs,
        vec!["@pageB.moth", "b_only.moth", "helper.moth"]
    );
}

#[test]
fn canonical_multi_entry_discovery_calls_provider_once() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("page_a")).expect("should create page_a module");
    fs::create_dir_all(src.join("page_b")).expect("should create page_b module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    // Entry A is plain provider-free; entry B imports a .js file.
    fs::write(src.join("page_a/@pageA.moth"), "a #= 1\n").expect("should write pageA");
    fs::write(
        src.join("page_b/@pageB.moth"),
        "@drawing.js as drawing\n#[:pageB]\n",
    )
    .expect("should write pageB");
    fs::write(src.join("page_b/drawing.js"), "export function draw() {}\n")
        .expect("should write js");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(CountingExternalImportProvider::new(Arc::clone(
        &calls,
    ))));

    let modules =
        discover_modules_for_test_with_providers(&config, &resolver, &style_directives, &providers)
            .expect("provider-backed multi-entry discovery should succeed");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "provider should be called once"
    );
    assert_eq!(modules.len(), 2);

    // Module A has its own source; module B should only contain the Moth entry, not the .js.
    assert_eq!(module_prepared_source_names(modules[0]).len(), 1);
    assert_eq!(
        module_prepared_source_names(modules[1]),
        vec!["@pageB.moth"]
    );
}

#[test]
fn canonical_provider_discovery_reads_and_tokenizes_each_source_once() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("page_a")).expect("should create page_a module");
    fs::create_dir_all(src.join("page_b")).expect("should create page_b module");
    fs::create_dir_all(src.join("shared")).expect("should create shared dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    // Entry A is provider-free; entry B imports a .js file backed by a registered provider.
    fs::write(src.join("page_a/@pageA.moth"), "@helper\n#[:pageA]\n").expect("should write pageA");
    fs::write(src.join("page_a/helper.moth"), "helper #= 1\n")
        .expect("should write module-local helper");
    fs::write(
        src.join("page_b/@pageB.moth"),
        "@drawing.js as drawing\n#[:pageB]\n",
    )
    .expect("should write pageB");
    fs::write(src.join("page_b/drawing.js"), "export function draw() {}\n")
        .expect("should write js");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(CountingExternalImportProvider::new(Arc::clone(
        &calls,
    ))));

    let _counter_guard = lock_source_read_counter_tests();
    let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);

    let modules =
        discover_modules_for_test_with_providers(&config, &resolver, &style_directives, &providers)
            .expect("canonical provider discovery should succeed");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    // Three unique Moth sources: @pageA.moth, page_a/helper.moth, and @pageB.moth.
    // SourceTreeIndex ownership sends each directly to one module queue, which reads each once
    // before header preparation.
    for source in [
        src.join("page_a/@pageA.moth"),
        src.join("page_a/helper.moth"),
        src.join("page_b/@pageB.moth"),
    ] {
        let canonical = fs::canonicalize(source).expect("source should canonicalize");
        assert_eq!(
            super::source_loading::source_read_count_for_path_for_test(&canonical),
            1,
            "each selected Moth source should be read exactly once"
        );
    }

    // The provider-backed import is handled exactly once by header-owned discovery.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "provider should be called once"
    );
    assert_eq!(modules.len(), 2);

    assert!(
        modules
            .iter()
            .all(|module| !module.prepared.contains_moth_template)
    );
}

#[test]
fn unsupported_external_extension_in_multi_entry_preserves_diagnostic_shape() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("page_a")).expect("should create page_a module");
    fs::create_dir_all(src.join("page_b")).expect("should create page_b module");
    fs::create_dir_all(src.join("shared")).expect("should create shared dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    fs::write(src.join("page_a/@pageA.moth"), "a #= 1\n").expect("should write pageA");
    fs::write(
        src.join("page_b/@pageB.moth"),
        "@drawing.js as drawing\n#[:pageB]\n",
    )
    .expect("should write pageB");
    fs::write(src.join("page_b/drawing.js"), "export function draw() {}\n")
        .expect("should write js");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("unsupported .js import should fail discovery"),
        Err(messages) => messages,
    };

    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0021",
        "expected unsupported external extension diagnostic, got {:?}",
        diagnostic
    );
    if let DiagnosticPayload::UnsupportedExternalExtension { path, extension } = &diagnostic.payload
    {
        let path_text = path.to_portable_string(&messages.string_table);
        assert_eq!(path_text, "drawing.js", "unexpected path in diagnostic");
        assert_eq!(
            messages.string_table.resolve(*extension),
            "js",
            "unexpected extension in diagnostic"
        );
    } else {
        panic!(
            "expected UnsupportedExternalExtension payload, got {:?}",
            diagnostic.payload
        );
    }
}

#[test]
fn directory_provider_dependency_calls_provider_once_for_repeated_physical_source() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    // The entry imports the same .js file twice under distinct local aliases, so the physical
    // provider source is reached twice during one traversal.
    fs::write(
        src.join("@page.moth"),
        "@drawing.js as drawing\n@drawing.js as drawing2\n#[:entry]\n",
    )
    .expect("should write entry");
    fs::write(src.join("drawing.js"), "export function draw() {}\n").expect("should write js");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(ResolvingCountingProvider::new(Arc::clone(&calls))));

    discover_modules_for_test_with_providers(&config, &resolver, &style_directives, &providers)
        .expect("repeated provider import should resolve");

    // The provider runs exactly once for one physical provider source, even though two consumers
    // reach it during the traversal. The cache key is the indexed canonical path, so the second
    // reach reuses the first result.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "provider must run exactly once for a repeated physical provider source"
    );
}

#[test]
fn directory_provider_dependency_rejects_cross_module_target() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let feature = src.join("feature");
    fs::create_dir_all(&feature).expect("should create feature module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@feature/private.js as private\n#[:entry]\n",
    )
    .expect("should write entry importing a cross-module provider file");
    fs::write(feature.join("@mod.moth"), "").expect("should write feature root");
    fs::write(feature.join("private.js"), "export function draw() {}\n")
        .expect("should write feature provider file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(CountingExternalImportProvider::new(Arc::clone(
        &calls,
    ))));

    let messages = match discover_modules_for_test_with_providers(
        &config,
        &resolver,
        &style_directives,
        &providers,
    ) {
        Ok(_) => panic!("cross-module provider import should be rejected"),
        Err(messages) => messages,
    };

    // The provider must never be invoked for a cross-module target.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "provider must not run for a cross-module target"
    );

    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0015",
        "expected cross-module import diagnostic, got {:?}",
        diagnostic
    );
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::CrossModuleImportNotExported { .. }
        ),
        "expected CrossModuleImportNotExported payload, got {:?}",
        diagnostic.payload
    );
}

#[test]
fn directory_provider_dependency_missing_target_reports_structured_diagnostic_without_path_probe() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    // The imported .js file does not exist on disk, so it is absent from the source tree index.
    fs::write(
        src.join("@page.moth"),
        "@missing.js as missing\n#[:entry]\n",
    )
    .expect("should write entry importing a missing provider file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(CountingExternalImportProvider::new(Arc::clone(
        &calls,
    ))));

    let messages = match discover_modules_for_test_with_providers(
        &config,
        &resolver,
        &style_directives,
        &providers,
    ) {
        Ok(_) => panic!("missing provider target should fail discovery"),
        Err(messages) => messages,
    };

    // The provider must never be invoked for a target the index could not resolve.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "provider must not run for a missing target"
    );

    // The diagnostic must be a structured import diagnostic, not a filesystem infrastructure
    // error. The old filesystem-backed path canonicalized the candidate and produced a File
    // infrastructure error; the index-based path reports a missing import target through the
    // existing import diagnostic lane, proving no directory-time path probe runs after the
    // index is built.
    assert!(
        messages.error_diagnostics().next().is_some(),
        "missing provider target should surface a structured import diagnostic"
    );
    assert!(
        messages.first_infrastructure_error_for_tests().is_none(),
        "missing provider target must not surface a filesystem infrastructure error"
    );
    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0005",
        "expected missing import target diagnostic, got {:?}",
        diagnostic
    );
}

#[test]
fn canonical_discovery_preserves_cross_module_root_queuing() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
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

    // Entry A depends on its direct child module B, which should queue module B's root.
    fs::write(module_a.join("@pageA.moth"), "@module_b b\n#[:pageA]\n")
        .expect("should write pageA");
    fs::write(module_b.join("@api.moth"), "export:\n    b #= 1\n;\n")
        .expect("should write module_b root");
    fs::write(module_b.join("impl.moth"), "impl #= 1\n").expect("should write module_b impl");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("cross-module root discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(modules.len(), 2);

    // Module A depends on module B's public facade, so module B precedes module A. The still-live
    // donor compiler receives only the provider root that exposes the facade, while module B
    // owns its complete semantic source set.
    let module_a = modules
        .iter()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                .is_some_and(|name| name == "@pageA.moth")
        })
        .expect("module A should be discovered");
    let module_b = modules
        .iter()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                .is_some_and(|name| name == "@api.moth")
        })
        .expect("module B should be discovered");
    let module_a_inputs = module_prepared_source_names(module_a);
    let module_b_inputs = module_prepared_source_names(module_b);

    assert!(
        !module_a_inputs.contains(&"@api.moth".to_string())
            && !module_a_inputs.contains(&"impl.moth".to_string()),
        "the consumer inventory must exclude all provider source files"
    );
    assert!(
        module_b_inputs.contains(&"@api.moth".to_string())
            && !module_b_inputs.contains(&"impl.moth".to_string()),
        "the queued provider module must retain its root without making an unreferenced private file semantic"
    );
}

#[test]
fn scoped_support_package_is_visible_by_name_to_owner_and_sibling_descendant() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let support = src.join("markdown");
    let pages = src.join("pages");
    fs::create_dir_all(&support).expect("should create support package");
    fs::create_dir_all(&pages).expect("should create sibling module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@site.moth"), "@markdown render\n#[:site]\n")
        .expect("should write owner root");
    fs::write(
        support.join("+package.moth"),
        "export:\n    render || -> String:\n        return \"support\"\n    ;\n;\n",
    )
    .expect("should write support root");
    fs::write(pages.join("@page.moth"), "@markdown render\n#[:page]\n")
        .expect("should write sibling module root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("the owner and sibling descendant should resolve the scoped package by name");
    assert_eq!(
        modules.waves().iter().map(Vec::len).sum::<usize>(),
        3,
        "both normal modules and the scoped support provider should receive canonical jobs"
    );
}

#[test]
fn recognized_source_stem_collision_is_ambiguous_without_extension_precedence() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source root");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@HELPER\n#[:page]\n").expect("should write root");
    fs::write(src.join("helper.moth"), "value #= 1\n").expect("should write Moth source");
    fs::write(src.join("Helper.md"), "# Helper\n").expect("should write Markdown source");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("md", crate::builder_surface::SourceFileKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!(
            "case-colliding recognized sources must not resolve by extension or spelling precedence"
        ),
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-IMPORT-0006"
    );
}

#[test]
fn binding_package_and_local_module_prefix_collision_is_ambiguous() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let local_core = src.join("core");
    fs::create_dir_all(&local_core).expect("should create local core module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@core/io input\n#[:page]\n").expect("should write root");
    fs::write(local_core.join("@core.moth"), "#[:local core]\n")
        .expect("should write colliding local module");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("a registered package must not take precedence over a local module prefix"),
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-IMPORT-0006"
    );
}

#[test]
fn directory_source_dependency_rejects_obsolete_relative_form() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source root");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@./helper value\n#[:page]\n").expect("should write root");
    fs::write(src.join("helper.moth"), "value #= 1\n").expect("should write helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("directory source dependencies must reject @./"),
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-IMPORT-0016"
    );
}

#[test]
fn direct_child_private_path_bypass_is_rejected() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let child = src.join("child");
    fs::create_dir_all(&child).expect("should create child module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@child/private value\n#[:page]\n")
        .expect("should write parent root");
    fs::write(child.join("@api.moth"), "export:\n    value #= 1\n;\n")
        .expect("should write child root");
    fs::write(child.join("private.moth"), "value #= 2\n")
        .expect("should write private child source");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("a consumer must not address a child module's private file path"),
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-IMPORT-0015"
    );
}

#[test]
fn stage0_consumes_moth_tokens_into_retained_header_syntax() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@helper\n@intro\n#[:entry]\n").expect("should write entry");
    fs::write(src.join("helper.moth"), "value #= 42\n").expect("should write helper");
    fs::write(src.join("intro.mtf"), "moth template body\n").expect("should write moth template");
    fs::write(src.join("notes.md"), "# Markdown body\n").expect("should write markdown");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", crate::builder_surface::SourceFileKind::MothTemplate);
    source_file_kinds.register("md", crate::builder_surface::SourceFileKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("header discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(modules[0].prepared.semantic.source_file_count, 3);
    assert_eq!(
        module_prepared_source_names(modules[0]),
        vec!["@page.moth", "helper.moth", "intro.mtf"]
    );
    assert!(modules[0].prepared.contains_moth_template);
    let entry_path = fs::canonicalize(src.join("@page.moth")).expect("entry should canonicalize");
    let module_symbols = &modules[0]
        .prepared
        .semantic
        .prepared_header_syntax
        .module_symbols;
    let entry_logical_path = module_symbols
        .canonical_os_path_by_source
        .iter()
        .find_map(|(logical_path, canonical_path)| {
            (canonical_path == &entry_path).then_some(logical_path)
        })
        .expect("entry should retain a source identity");
    assert_eq!(
        modules[0]
            .prepared
            .semantic
            .prepared_header_syntax
            .module_symbols
            .file_dependency_clauses_by_source
            .get(entry_logical_path)
            .map(Vec::len),
        Some(2),
        "the consumed Moth token payload should produce retained entry dependency facts"
    );
}

#[test]
fn canonical_discovery_consumes_moth_tokens_for_every_reachable_file() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let module_a = src.join("module_a");
    let module_b = src.join("module_b");
    fs::create_dir_all(&module_a).expect("should create module_a");
    fs::create_dir_all(&module_b).expect("should create module_b");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    // Two entry points retain independent canonical source ownership. Each module depends on a helper.
    fs::write(module_a.join("@pageA.moth"), "@helperA\n#[:pageA]\n").expect("should write pageA");
    fs::write(module_a.join("helperA.moth"), "a #= 1\n").expect("should write helperA");
    fs::write(module_b.join("@pageB.moth"), "@helperB\n#[:pageB]\n").expect("should write pageB");
    fs::write(module_b.join("helperB.moth"), "b #= 2\n").expect("should write helperB");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("canonical discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(modules.len(), 2);

    assert_eq!(
        module_prepared_source_names(modules[0]),
        vec!["@pageA.moth", "helperA.moth"]
    );
    assert_eq!(
        module_prepared_source_names(modules[1]),
        vec!["@pageB.moth", "helperB.moth"]
    );
    assert!(modules.iter().all(|module| {
        module
            .prepared
            .semantic
            .prepared_header_syntax
            .module_symbols
            .file_dependency_clauses_by_source
            .values()
            .any(|dependencies| !dependencies.is_empty())
    }));
}

// -------------------------
//  Phase 5b: graph-resolved local provider edges
// -------------------------

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

#[test]
fn local_dependency_edge_is_recorded_provider_before_consumer() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, module_a_root, module_b_root) =
        write_cross_module_project(&root);

    let (modules, graph, source_tree_index, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let module_a_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_a_root)
        .expect("module_a root should be a graph node");
    let module_b_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_b_root)
        .expect("module_b root should be a graph node");

    // The dependency flows module_a -> module_b, so the provider (module_b) must precede the
    // consumer (module_a) in the returned inventory order. The edge is provider-before-consumer,
    // never the reverse.
    assert!(
        graph.has_dependency_edge(module_b_id, module_a_id),
        "provider module_b must have an edge into consumer module_a"
    );
    assert!(
        !graph.has_dependency_edge(module_a_id, module_b_id),
        "the consumer must not edge into its provider"
    );

    // The returned modules follow the dependency-ordered wave order: module_b is the provider
    // and must appear in an earlier compile wave than consumer module_a. The inventory
    // preserves wave boundaries so the directory compiler can schedule providers before
    // consumers.
    let waves = modules.waves();
    let provider_wave = waves
        .iter()
        .position(|wave| {
            wave.iter().any(|module| {
                prepared_entry_file_path(&module.prepared.semantic)
                    .expect("prepared module retains its entry file identity")
                    .file_name()
                    == Some(OsStr::new("@api.moth"))
            })
        })
        .expect("module_b should appear in a compile wave");
    let consumer_wave = waves
        .iter()
        .position(|wave| {
            wave.iter().any(|module| {
                prepared_entry_file_path(&module.prepared.semantic)
                    .expect("prepared module retains its entry file identity")
                    .file_name()
                    == Some(OsStr::new("@pageA.moth"))
            })
        })
        .expect("module_a should appear in a compile wave");
    assert!(
        provider_wave < consumer_wave,
        "provider module_b must be in an earlier wave than consumer module_a"
    );
    assert_eq!(
        waves[provider_wave].len(),
        1,
        "the provider is the sole entry in its wave"
    );
    assert_eq!(
        waves[consumer_wave].len(),
        1,
        "the sole consumer is the only entry in its wave"
    );
}

#[test]
fn same_module_dependency_creates_no_project_graph_edge() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    // The single entry module depends on a sibling file inside its own module root.
    fs::write(src.join("@page.moth"), "@helper\n#[:page]\n").expect("should write entry");
    fs::write(src.join("helper.moth"), "value #= 1\n").expect("should write helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let (modules, graph, _source_tree_index, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let entry_root = graph.entry_modules().to_vec();
    assert_eq!(entry_root.len(), 1, "there is one normal entry module");

    // Same-module dependencies create no project-graph edge, so the graph has no edges and one wave.
    let waves = graph.compile_waves().expect("no-edge graph waves cleanly");
    assert_eq!(waves.len(), 1, "no edges means a single ready wave");
    assert_eq!(
        modules.waves().iter().map(|wave| wave.len()).sum::<usize>(),
        1
    );

    // The inventory preserves wave boundaries: the single no-edge entry is the sole module in
    // one ready wave.
    let inventory_waves = modules.waves();
    assert_eq!(
        inventory_waves.len(),
        1,
        "one no-edge entry produces one inventory wave"
    );
    assert_eq!(
        inventory_waves[0].len(),
        1,
        "the singleton wave contains the one no-edge entry"
    );
}

#[test]
fn independent_no_edge_entries_are_grouped_in_one_ready_wave() {
    // Two entry modules with no cross-module dependency edges must be grouped in the same
    // dependency-ready wave; the serial scheduler can then publish them in deterministic order.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let module_a = src.join("module_a");
    let module_b = src.join("module_b");
    fs::create_dir_all(&module_a).expect("should create module_a");
    fs::create_dir_all(&module_b).expect("should create module_b");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    // Two independent entry modules with no cross-module dependencies.
    fs::write(module_a.join("@pageA.moth"), "#[:pageA]\n").expect("should write pageA");
    fs::write(module_b.join("@pageB.moth"), "#[:pageB]\n").expect("should write pageB");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let (modules, graph, _source_tree_index, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    // No cross-module edges means a single ready wave containing both entries.
    let graph_waves = graph.compile_waves().expect("no-edge graph waves cleanly");
    assert_eq!(graph_waves.len(), 1, "no edges means a single ready wave");

    let inventory_waves = modules.waves();
    assert_eq!(
        inventory_waves.len(),
        1,
        "two no-edge entries produce one inventory wave"
    );
    assert_eq!(
        inventory_waves[0].len(),
        2,
        "both no-edge entries are grouped in the same wave"
    );

    // The inventory wave preserves the graph's canonical ModuleId order exactly. Derive the
    // expected entry order from the graph wave rather than assuming a filename sort, then assert
    // the inventory matches it position-for-position. Every node in this no-edge wave is a normal
    // entry, so the graph wave order is the expected entry order.
    let expected_order: Vec<String> = graph_waves[0]
        .iter()
        .map(|module_id| {
            graph
                .node(*module_id)
                .root_file()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    let inventory_order: Vec<String> = inventory_waves[0]
        .iter()
        .map(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(
        inventory_order, expected_order,
        "the inventory wave preserves the graph's canonical ModuleId order"
    );
}

#[test]
fn duplicate_dependency_deduplicates_edge_and_orders_provider_first() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
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
    // The consumer depends on its direct child twice, so the duplicate observation must be
    // idempotent.
    fs::write(
        module_a.join("@pageA.moth"),
        "@module_b\n@module_b\n#[:pageA]\n",
    )
    .expect("should write pageA");
    fs::write(module_b.join("@api.moth"), "export:\n    b #= 1\n;\n")
        .expect("should write module_b root");
    fs::write(module_b.join("impl.moth"), "impl #= 1\n").expect("should write module_b impl");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let (modules, graph, source_tree_index, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let module_a_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&fs::canonicalize(&module_a).unwrap())
        .expect("module_a root should be a graph node");
    let module_b_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&fs::canonicalize(&module_b).unwrap())
        .expect("module_b root should be a graph node");

    // The duplicate observation collapses to one provider-before-consumer edge.
    assert!(graph.has_dependency_edge(module_b_id, module_a_id));
    assert!(!graph.has_dependency_edge(module_a_id, module_b_id));

    // module_b is the provider and must appear in an earlier compile wave than its consumer.
    let waves = graph
        .compile_waves()
        .expect("duplicate-edge graph waves cleanly");
    let provider_wave = waves
        .iter()
        .position(|wave| wave.contains(&module_b_id))
        .expect("module_b should appear in a wave");
    let consumer_a_wave = waves
        .iter()
        .position(|wave| wave.contains(&module_a_id))
        .expect("module_a should appear in a wave");
    assert!(
        provider_wave < consumer_a_wave,
        "the provider must precede its consumer in compile-wave order"
    );

    // The inventory preserves the provider and consumer wave boundary.
    let inventory_waves = modules.waves();
    assert_eq!(
        inventory_waves.len(),
        2,
        "one provider wave and one consumer wave"
    );
    assert_eq!(
        inventory_waves[0].len(),
        1,
        "the provider is the sole entry in the first wave"
    );
    assert!(
        prepared_entry_file_path(&inventory_waves[0][0].prepared.semantic)
            .expect("prepared module retains its entry file identity")
            .file_name()
            .is_some_and(|name| name == "@api.moth"),
        "module_b is the provider in the first wave"
    );
    assert_eq!(
        inventory_waves[1].len(),
        1,
        "the consumer is the sole entry in the second wave"
    );
    assert!(
        prepared_entry_file_path(&inventory_waves[1][0].prepared.semantic)
            .expect("prepared module retains its entry file identity")
            .file_name()
            .is_some_and(|name| name == "@pageA.moth"),
        "module_a is the consumer in the second wave"
    );
}

#[test]
fn dependency_fact_retains_authored_source_location() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, module_a_root, module_b_root) =
        write_cross_module_project(&root);

    let (_modules, graph, source_tree_index, string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let module_a_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_a_root)
        .expect("module_a root should be a graph node");
    let module_b_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_b_root)
        .expect("module_b root should be a graph node");

    let retained_location = graph
        .edge_source_location(module_b_id, module_a_id)
        .expect("the provider-before-consumer edge should retain its authored location");

    // The retained scope is the declaring_source file that authored the structural provider reference.
    let scope_path = retained_location.scope.to_portable_string(&string_table);
    assert!(
        scope_path.contains("@pageA.moth"),
        "retained location scope should name the declaring module root file: {scope_path}"
    );
    // The dependency clause is on the first source line.
    assert_eq!(
        retained_location.start_pos.line_number, 0,
        "retained location should point at the first authored source line"
    );
}

#[test]
fn production_graph_completes_before_scheduling() {
    // Hidden invariant: the production discovery path completes the project module graph before
    // compile-wave scheduling, freezing adjacency into sorted `Vec<ModuleId>` storage. The
    // completed graph schedules cleanly from its frozen adjacency, and any later edge insertion
    // is rejected as mutation after completion.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, module_a_root, module_b_root) =
        write_cross_module_project(&root);

    let (modules, mut graph, source_tree_index, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let module_a_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_a_root)
        .expect("module_a root should be a graph node");
    let module_b_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_b_root)
        .expect("module_b root should be a graph node");

    // The production graph is completed, so scheduling reads its frozen adjacency and reproduces
    // the same provider-before-consumer wave order the inventory already exposes.
    let graph_waves = graph
        .compile_waves()
        .expect("production graph should be completed and schedulable");
    let provider_wave = graph_waves
        .iter()
        .position(|wave| wave.contains(&module_b_id))
        .expect("provider module_b should appear in a wave");
    let consumer_wave = graph_waves
        .iter()
        .position(|wave| wave.contains(&module_a_id))
        .expect("consumer module_a should appear in a wave");
    assert!(
        provider_wave < consumer_wave,
        "frozen adjacency keeps the provider before its consumer"
    );

    // The inventory waves agree with the completed graph's provider-before-consumer order.
    let inventory_waves = modules.waves();
    assert_eq!(
        inventory_waves.len(),
        2,
        "the completed graph produces one provider wave and one consumer wave"
    );

    // Edge insertion after completion is mutation after the graph is frozen, reported as an
    // internal compiler failure rather than silently accepted.
    let mutation_error = graph
        .add_dependency_edge(module_b_id, module_a_id)
        .expect_err("mutation after production completion must be rejected");
    assert_eq!(mutation_error.error_type, ErrorType::Compiler);
    assert!(
        mutation_error.msg.contains("after completion"),
        "mutation error must name the phase violation: {}",
        mutation_error.msg
    );
}

#[test]
fn discovered_modules_carry_both_graph_assigned_identities() {
    // Hidden invariant: directory discovery must preserve both graph identities rather than
    // re-deriving either from an entry path. The dense ID remains the build-owned scheduling and
    // merge key; the stable origin remains the portable semantic identity.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, _module_a_root, _module_b_root) =
        write_cross_module_project(&root);

    let (modules, graph, _source_tree_index, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    assert!(
        modules.waves().iter().map(|wave| wave.len()).sum::<usize>() > 0,
        "the cross-module project should discover at least one normal entry module"
    );

    for module in modules.waves().iter().flatten() {
        let matching_node = graph.nodes().iter().find(|node| {
            node.root_file()
                == prepared_entry_file_path(&module.prepared.semantic)
                    .expect("prepared module retains its entry file identity")
        });
        let matching_node = matching_node.expect(
            "every discovered module entry point must match a graph node's canonical root file",
        );
        assert_eq!(
            module.module_id,
            matching_node.module_id(),
            "discovered module ID must equal its graph-assigned dense identity (entry {:?})",
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity"),
        );
        assert_eq!(
            module.stable_origin,
            *matching_node.stable_origin(),
            "discovered module stable origin must equal its graph-assigned origin (entry {:?})",
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity"),
        );
    }
}

#[test]
fn discovered_module_origin_is_not_rederived_from_a_path_component() {
    // Hidden invariant: the stable origin carried by discovery is the graph-owned value type, not
    // a path-derived fallback. The discovered origins must be distinct `StableModuleOriginIdentity`
    // values keyed by canonical logical module path, and must round-trip through the graph node.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, _module_a_root, _module_b_root) =
        write_cross_module_project(&root);

    let (modules, _graph, _source_tree_index, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    // Each module carries a distinct stable origin keyed by its logical module path. The two
    // cross-module roots have different logical paths, so their origins must differ.
    let origins: Vec<StableModuleOriginIdentity> = modules
        .iter()
        .map(|module| module.stable_origin.clone())
        .collect();
    let unique: std::collections::HashSet<StableModuleOriginIdentity> =
        origins.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        modules.len(),
        "each discovered module must carry its own distinct graph-assigned stable origin"
    );
}

#[test]
fn build_source_origin_lookup_maps_each_owned_file_to_its_node_origin() {
    // Hidden invariant: the source-origin lookup is a direct projection of the central
    // `SourceTreeIndex` ownership through the graph's owned source IDs. Every owned source
    // record's logical identity module origin must equal its containing graph node's stable
    // origin, and no canonical path may appear twice.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, _module_a_root, _module_b_root) =
        write_cross_module_project(&root);

    let (_modules, graph, source_tree_index, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let lookup = graph
        .build_source_origin_lookup(&source_tree_index)
        .expect("the source-origin lookup should build for a valid cross-module project");

    // Every lookup entry's origin must equal the stable origin of the graph node that owns it.
    // The graph node carries no source records; owned source data is resolved through the
    // retained central index, so the lookup must cover exactly the index's owned source IDs.
    for node in graph.nodes() {
        for source_id in source_tree_index.owned_source_indices(node.module_id()) {
            let record = source_tree_index.source(*source_id);
            let lookup_origin = lookup
                .get(record.canonical_path())
                .expect("every owned source entry must be present in the lookup");
            assert_eq!(
                lookup_origin,
                node.stable_origin(),
                "an owned source entry's lookup origin must equal its containing node origin (path: {:?})",
                record.canonical_path().display(),
            );
        }
    }

    // No canonical path may appear under two different origins: the lookup is a function, not a
    // relation. Duplicates would have failed inside `build_source_origin_lookup`, so reaching
    // here with every entry validated confirms single-ownership.
    let unique_paths: HashSet<&std::path::Path> = lookup.keys().map(|p| p.as_path()).collect();
    let total_entries: usize = graph
        .nodes()
        .iter()
        .map(|node| {
            source_tree_index
                .owned_source_indices(node.module_id())
                .len()
        })
        .sum();
    assert_eq!(
        unique_paths.len(),
        total_entries,
        "every owned source path must be unique across all graph nodes"
    );
}

#[test]
fn canonical_module_job_excludes_cross_module_donor_sources() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let provider = src.join("a_provider");
    fs::create_dir_all(&provider).expect("should create provider module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@z_local\n@a_provider value\n#[:entry]\n",
    )
    .expect("should write consumer root");
    fs::write(src.join("z_local.moth"), "local #= 1\n").expect("should write local source");
    fs::write(provider.join("@api.moth"), "export:\n    value #= 1\n;\n")
        .expect("should write provider root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let consumer = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                == Some(OsStr::new("@page.moth"))
        })
        .expect("consumer module should be discovered");
    let input_names = module_prepared_source_names(consumer);

    assert_eq!(
        input_names,
        vec!["@page.moth", "z_local.moth"],
        "the consumer job must contain only its canonical prepared sources; the provider reaches binding through its completed interface"
    );
}

#[test]
fn indexed_namespace_rejects_direct_entry_root_dependency() {
    // Path components starting with `@` are now rejected by the path parser before
    // namespace resolution. The `@` introducer is consumed by the lexer, so any
    // component starting with `@` is a `@@` form that has no valid dependency meaning.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source root");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@@page symbol\n#[:page]\n")
        .expect("should write entry root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("direct entry-root @@page dependency must be rejected by the path parser"),
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-SYNTAX-0018"
    );
}

#[test]
fn indexed_namespace_rejects_direct_nested_child_root_dependency() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let child = src.join("child");
    fs::create_dir_all(&child).expect("should create child module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@child/@home symbol\n#[:page]\n")
        .expect("should write parent root");
    fs::write(child.join("@home.moth"), "export:\n    symbol #= 1\n;\n")
        .expect("should write child root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => {
            panic!("direct nested-child root dependencies must be rejected by the path parser")
        }
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-SYNTAX-0018"
    );
}

#[test]
fn provider_binding_index_rejects_duplicate_shell_edges() {
    let shell = DependencyShellId::new(SourceId::from_index(0), 0);
    let edges = vec![
        ResolvedDependencyEdge {
            provider_module_id: ModuleId::from_index(1),
            consumer_module_id: ModuleId::from_index(0),
            dependency_shell_id: shell,
            graph_location: SourceLocation::default(),
        },
        ResolvedDependencyEdge {
            provider_module_id: ModuleId::from_index(2),
            consumer_module_id: ModuleId::from_index(0),
            dependency_shell_id: shell,
            graph_location: SourceLocation::default(),
        },
    ];

    let error = super::compilation::build_provider_binding_index(&edges)
        .expect_err("one retained shell must resolve to exactly one provider edge");

    assert!(
        error.msg.contains("more than one provider edge"),
        "unexpected error: {}",
        error.msg
    );
}

#[test]
fn source_package_dependency_index_rejects_cross_category_or_duplicate_shells() {
    let shell = DependencyShellId::new(SourceId::from_index(0), 0);
    let provider_edge = ResolvedDependencyEdge {
        provider_module_id: ModuleId::from_index(1),
        consumer_module_id: ModuleId::from_index(0),
        dependency_shell_id: shell,
        graph_location: SourceLocation::default(),
    };
    let provider_binding_index = super::compilation::build_provider_binding_index(&[provider_edge])
        .expect("one provider edge should index");

    let package_dependency = ResolvedSourcePackageDependency {
        consumer_module_id: ModuleId::from_index(0),
        dependency_prefix: "markdown".to_owned(),
        dependency_shell_id: shell,
    };

    let error = super::compilation::build_source_package_dependency_index(
        &provider_binding_index,
        &[package_dependency],
    )
    .expect_err("one shell cannot address both a provider module and a source package");

    assert!(
        error
            .msg
            .contains("both a provider module and a source package"),
        "unexpected error: {}",
        error.msg
    );
}

// ---------------------------------------------------------------------------
// R5C2A package ordering and registry invariants
// ---------------------------------------------------------------------------

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
fn package_ordering_is_deterministic_across_reversed_discovery_order() {
    let prefixes = vec!["a".to_owned(), "b".to_owned()];
    let dependencies = dependency_prefixes(&[&[], &["a"]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("acyclic package graph orders");
    assert_eq!(order, vec![0, 1]);
    let prefixes_in_order = order
        .iter()
        .map(|index| prefixes[*index].as_str())
        .collect::<Vec<_>>();
    assert_eq!(prefixes_in_order, vec!["a", "b"]);

    // The same graph presented in reversed discovery order must yield the same prefix sequence.
    let prefixes = vec!["b".to_owned(), "a".to_owned()];
    let dependencies = dependency_prefixes(&[&["a"], &[]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("acyclic package graph orders");
    assert_eq!(order, vec![1, 0]);
    let prefixes_in_order = order
        .iter()
        .map(|index| prefixes[*index].as_str())
        .collect::<Vec<_>>();
    assert_eq!(prefixes_in_order, vec!["a", "b"]);

    // Independent packages tie-break by prefix, so reversed discovery order still yields the
    // same prefix sequence.
    let prefixes = vec!["a".to_owned(), "b".to_owned()];
    let dependencies = dependency_prefixes(&[&[], &[]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("independent package graph orders");
    assert_eq!(order, vec![0, 1]);

    let prefixes = vec!["b".to_owned(), "a".to_owned()];
    let dependencies = dependency_prefixes(&[&[], &[]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("independent package graph orders");
    assert_eq!(order, vec![1, 0]);
}

#[test]
fn package_ordering_tie_breaks_by_discovery_prefix_order() {
    let prefixes = vec!["z".to_owned(), "a".to_owned(), "m".to_owned()];
    let dependencies = dependency_prefixes(&[&[], &[], &[]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("independent packages order");
    assert_eq!(order, vec![1, 2, 0], "ready packages leave in prefix order");
}

#[test]
fn package_ordering_orders_diamond_dependencies_once() {
    let prefixes = vec![
        "a".to_owned(),
        "b".to_owned(),
        "c".to_owned(),
        "d".to_owned(),
    ];
    let dependencies = dependency_prefixes(&[&[], &["a"], &["a"], &["b", "c"]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("diamond package graph orders");
    assert_eq!(order, vec![0, 1, 2, 3]);
}

#[test]
fn package_ordering_rejects_cycles_and_unknown_prefixes() {
    let prefixes = vec!["a".to_owned(), "b".to_owned()];
    let dependencies = dependency_prefixes(&[&["b"], &["a"]]);
    let error = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect_err("a package dependency cycle is malformed graph metadata");
    assert!(error.msg.contains("dependency cycle"));

    let prefixes = vec!["a".to_owned()];
    let dependencies = dependency_prefixes(&[&["missing"]]);
    let error = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect_err("an unknown provider prefix is malformed graph metadata");
    assert!(error.msg.contains("unindexed source package @missing"));
}

#[test]
fn completed_package_registry_rejects_duplicate_prefix() {
    let mut registry = CompletedSourcePackageRegistry::new();
    registry
        .publish(compiled_package("markdown"), &[])
        .expect("first package publishes");

    let error = registry
        .publish(compiled_package("markdown"), &[])
        .expect_err("one prefix must index exactly one completed package");
    assert!(error.msg.contains("completed more than once"));
}

#[test]
fn completed_package_registry_records_direct_dependency_edges_once() {
    let mut registry = CompletedSourcePackageRegistry::new();
    let a = registry
        .publish(compiled_package("a"), &[])
        .expect("provider package publishes");
    let b = registry
        .publish(compiled_package("b"), &["a".to_owned()])
        .expect("consumer package publishes");

    registry
        .validate_dependency_edges()
        .expect("dependency-first publication order is valid");
    assert_eq!(
        registry.provider_packages(b).expect("b has provider edges"),
        &[a]
    );
    assert_eq!(
        registry.consumer_packages(a).expect("a has consumer edges"),
        &[b]
    );
    assert_eq!(registry.by_prefix("a"), Some(a));
    assert_eq!(registry.by_prefix("b"), Some(b));
    assert_eq!(registry.by_prefix("missing"), None);
}

#[test]
fn completed_package_registry_rejects_duplicate_dependency_input_without_mutation() {
    let mut registry = CompletedSourcePackageRegistry::new();
    let provider = registry
        .publish(compiled_package("provider"), &[])
        .expect("provider package publishes");

    let error = registry
        .publish(
            compiled_package("consumer"),
            &["provider".to_owned(), "provider".to_owned()],
        )
        .expect_err("duplicate provider input must be rejected before publication");

    assert!(error.msg.contains("more than once"));
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.by_prefix("consumer"), None);
    assert!(
        registry
            .consumer_packages(provider)
            .expect("provider has a consumer lane")
            .is_empty()
    );
}

#[test]
fn completed_package_registry_rejects_self_dependency_before_publication() {
    let mut registry = CompletedSourcePackageRegistry::new();
    let error = registry
        .publish(compiled_package("a"), &["a".to_owned()])
        .expect_err("a package must never depend on its own not-yet-published prefix");
    assert!(error.msg.contains("unindexed source package @a"));
}

#[test]
fn module_package_dependency_index_walks_only_direct_dependencies() {
    let mut registry = CompletedSourcePackageRegistry::new();
    let a = registry
        .publish(compiled_package("a"), &[])
        .expect("provider package publishes");
    let b = registry
        .publish(compiled_package("b"), &["a".to_owned()])
        .expect("consumer package publishes");

    let consumer_module_id = ModuleId::from_index(5);
    let shell = DependencyShellId::new(SourceId::from_index(0), 0);
    let dependencies = vec![ResolvedSourcePackageDependency {
        consumer_module_id,
        dependency_prefix: "b".to_owned(),
        dependency_shell_id: shell,
    }];

    let index = super::compilation::build_module_package_dependency_index(&dependencies, &registry)
        .expect("direct dependencies index");
    assert_eq!(
        index.len(),
        1,
        "readiness must only visit direct package dependencies"
    );
    assert_eq!(
        index.get(&consumer_module_id),
        Some(&vec![b]),
        "the module depends on package b, not on transitive provider a"
    );
    assert!(a != b);
    assert!(
        !index.contains_key(&ModuleId::from_index(6)),
        "modules without package dependencies must not appear in the readiness index"
    );
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn synthetic_traversal_prepares_retained_clauses_without_a_token_rescan() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::create_dir_all(root.join("utils")).expect("should create utils directory");
    fs::write(root.join("main.moth"), "@utils/helper greet\ngreet()\n")
        .expect("should write main file");
    fs::write(
        root.join("utils/helper.moth"),
        "greet||:\n    io.line([: [\"hello\"]])\n;\n",
    )
    .expect("should write helper file");

    let _counter_capture =
        crate::compiler_frontend::instrumentation::capture_frontend_counters_for_test();
    crate::compiler_frontend::instrumentation::reset_frontend_counters();
    let counter_guard =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");

    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let config = Config::new(root.clone());
    let resolver = configured_resolver(&config);
    let mut external_packages = ExternalPackageRegistry::new();
    let external_import_providers = crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::empty();
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

    let collected = super::source_discovery::collect_reachable_input_files(
        &root.join("main.moth"),
        &resolver,
        &style_directives,
        &mut external_imports,
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut ResourceInputRegistry::new(),
        &mut string_table,
    )
    .expect("synthetic single-file traversal should succeed");

    assert_eq!(
        collected.input_files.len(),
        2,
        "main file and its helper dependency must both be reachable"
    );

    crate::compiler_frontend::instrumentation::log_frontend_counters();
    let observations = counter_guard.finish();
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };

    assert_eq!(
        counter_value("token_rescan_count"),
        0.0,
        "Stage 0 must consume retained clause facts and never rescan tokens"
    );
    assert_eq!(
        counter_value("dependency_clause_count"),
        1.0,
        "the single authored clause must be counted once"
    );
    assert_eq!(
        counter_value("retained_shell_count"),
        1.0,
        "one authored clause owns one retained shell"
    );
    assert_eq!(
        counter_value("resolved_source_package_clause_count"),
        1.0,
        "the helper dependency binds as an extensionless source clause"
    );
    assert_eq!(
        counter_value("resolved_provider_clause_count"),
        0.0,
        "no explicit-extension provider clause is bound in this traversal"
    );
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn directory_discovery_counts_resolved_clauses_by_language_family() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let entry_source = "@docs/intro\n@child greet\n@core/io line\n@drawing.js draw\n#[:entry]\n";
    let intro_source = "intro #= \"intro\"\n";
    let ownership_source = "ownership #= \"ownership\"\n";
    let child_source = "export:\n    greet || -> String:\n        return \"hi\"\n    ;\n;\n";
    fs::create_dir_all(src.join("docs/guides")).expect("should create docs folders");
    fs::create_dir_all(src.join("child")).expect("should create child module dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), entry_source).expect("should write entry");
    fs::write(src.join("docs/intro.moth"), intro_source).expect("should write intro");
    fs::write(src.join("docs/guides/ownership.moth"), ownership_source)
        .expect("should write ownership");
    fs::write(src.join("child/@mod.moth"), child_source).expect("should write child module root");
    fs::write(src.join("drawing.js"), "export function draw() {}\n")
        .expect("should write drawing provider file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(CountingExternalImportProvider::new(Arc::clone(
        &calls,
    ))));

    // Derive the expected retained token volume outside the production counter window.
    let mut expected_token_string_table = StringTable::new();
    let expected_token_count = [entry_source, intro_source, child_source]
        .into_iter()
        .enumerate()
        .map(|(index, source)| {
            let scope =
                crate::compiler_frontend::symbols::interned_path::InternedPath::from_single_str(
                    &format!("counter-fixture-{index}.moth"),
                    &mut expected_token_string_table,
                );
            crate::compiler_frontend::tokenizer::lexer::tokenize(
                source,
                &scope,
                crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode::SourceFile,
                &style_directives,
                &mut expected_token_string_table,
                None,
            )
            .expect("counter fixture source should tokenize")
            .length
        })
        .sum::<usize>() as f64;

    let _counter_capture =
        crate::compiler_frontend::instrumentation::capture_frontend_counters_for_test();
    crate::compiler_frontend::instrumentation::reset_frontend_counters();
    let counter_guard =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");

    let modules =
        discover_modules_for_test_with_providers(&config, &resolver, &style_directives, &providers)
            .expect("directory discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(
        modules.len(),
        2,
        "entry and child module must both be discovered"
    );
    let selected_source_count = modules
        .iter()
        .map(|module| module.prepared.semantic.source_file_count)
        .sum::<usize>() as f64;

    crate::compiler_frontend::instrumentation::log_frontend_counters();
    let observations = counter_guard.finish();
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };

    assert_eq!(
        counter_value("token_rescan_count"),
        0.0,
        "Stage 0 must consume retained clause facts and never rescan tokens"
    );
    assert_eq!(
        counter_value("file_preparation_pass_count"),
        selected_source_count,
        "each selected directory source must enter preparation once"
    );
    assert_eq!(
        counter_value("prepared_file_count"),
        selected_source_count,
        "each selected directory source must become one retained prepared output"
    );
    assert_eq!(
        counter_value("token_count"),
        expected_token_count,
        "successful aggregation must count the tokens retained by all selected sources"
    );
    assert_eq!(
        counter_value("dependency_clause_count"),
        4.0,
        "four authored clauses must be counted once each"
    );
    assert_eq!(
        counter_value("retained_shell_count"),
        4.0,
        "one authored clause owns one retained shell"
    );
    assert_eq!(
        counter_value("resolved_source_package_clause_count"),
        3.0,
        "one same-module clause, one cross-module clause and one virtual package clause resolve as extensionless source clauses"
    );
    assert_eq!(
        counter_value("resolved_provider_clause_count"),
        1.0,
        "the explicit-extension drawing.js clause resolves through a registered provider"
    );
}
mod file_reference_resolution_tests {
    use super::super::file_reference_resolution::{
        SingleFileReferenceOutcome, SingleFileReferenceResolver, stable_case_mismatch,
    };
    use super::super::resource_inputs::ResourceInputRegistry;
    use crate::builder_surface::SourceFileKind;
    use crate::compiler_frontend::compiler_messages::InvalidCompileTimePathReason;
    use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
    use crate::compiler_frontend::paths::file_references::{
        PreparedFileReference, PreparedFileReferenceClass,
    };
    use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
    use crate::compiler_frontend::source::SourceId;
    use crate::compiler_frontend::symbols::interned_path::InternedPath;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use std::fs;

    #[test]
    fn case_mismatch_evidence_is_sorted_and_unicode_lowercased() {
        let mut ambiguous = vec!["data".to_owned(), "Data".to_owned()];
        assert_eq!(
            stable_case_mismatch("DATA", &mut ambiguous),
            Some("Data".to_owned())
        );

        let mut unicode = vec!["Éclair".to_owned()];
        assert_eq!(
            stable_case_mismatch("éCLAIR", &mut unicode),
            Some("Éclair".to_owned())
        );
    }

    #[test]
    fn synthetic_physical_resolution_cache_reuses_settled_outcome() {
        let temp = tempfile::tempdir().expect("test root should be created");
        let resource_directory = temp.path().join("assets");
        fs::create_dir_all(&resource_directory).expect("resource directory should be created");
        let resource_path = resource_directory.join("logo.svg");
        fs::write(&resource_path, "resource").expect("resource should be created");

        let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
        source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
        let mut resource_inputs = ResourceInputRegistry::new();
        let canonical_root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
        let mut resolver = SingleFileReferenceResolver::new(
            canonical_root.clone(),
            &source_file_kinds,
            &mut resource_inputs,
        );
        let mut strings = StringTable::new();
        let mut path_syntax = PathSyntaxTable::new();
        let path = InternedPath::from_single_str("assets/logo.svg", &mut strings);
        let path_syntax_id = path_syntax.push(path.clone(), SourceLocation::default());
        let reference = PreparedFileReference {
            source_file: Some(SourceId::from_index(0)),
            path_syntax: path_syntax_id,
            location: SourceLocation::default(),
            class: PreparedFileReferenceClass::ResourceFile,
        };

        let owner = canonical_root.join("main.moth");
        let first = resolver
            .resolve(&owner, &path_syntax, &reference, &mut strings)
            .expect("first resource reference should resolve");
        let second = resolver
            .resolve(&owner, &path_syntax, &reference, &mut strings)
            .expect("repeated resource reference should resolve");

        assert!(
            matches!(first.outcome, SingleFileReferenceOutcome::Resource { .. }),
            "first outcome: {:?}",
            first.outcome
        );
        assert!(matches!(
            second.outcome,
            SingleFileReferenceOutcome::Resource { .. }
        ));
        assert_eq!(resource_inputs.records().len(), 1);
    }

    #[test]
    fn synthetic_moth_value_skips_physical_resolution_for_missing_target() {
        let temp = tempfile::tempdir().expect("test root should be created");
        let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
        let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
        let mut resource_inputs = ResourceInputRegistry::new();
        let mut resolver = SingleFileReferenceResolver::new(
            root.clone(),
            &source_file_kinds,
            &mut resource_inputs,
        );
        let mut strings = StringTable::new();
        let mut path_syntax = PathSyntaxTable::new();
        let path = InternedPath::from_single_str("missing.moth", &mut strings);
        let path_syntax_id = path_syntax.push(path.clone(), SourceLocation::default());
        let reference = PreparedFileReference {
            source_file: Some(SourceId::from_index(0)),
            path_syntax: path_syntax_id,
            location: SourceLocation::default(),
            class: PreparedFileReferenceClass::SourceKindNoFileValue,
        };

        let resolved = resolver
            .resolve(
                &root.join("main.moth"),
                &path_syntax,
                &reference,
                &mut strings,
            )
            .expect("a .moth value should be identified without a physical target");

        assert!(matches!(
            resolved.outcome,
            SingleFileReferenceOutcome::IdentifiedSourceKind
        ));
        assert!(resource_inputs.records().is_empty());
        assert!(resource_inputs.missing_watch_interests().is_empty());
    }

    #[test]
    fn synthetic_not_a_directory_is_a_typed_path_failure() {
        let temp = tempfile::tempdir().expect("test root should be created");
        let blocker = temp.path().join("not_a_directory");
        fs::write(&blocker, "regular file").expect("blocker file should be created");

        let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
        source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
        let mut resource_inputs = ResourceInputRegistry::new();
        let mut resolver = SingleFileReferenceResolver::new(
            fs::canonicalize(temp.path()).expect("test root should canonicalize"),
            &source_file_kinds,
            &mut resource_inputs,
        );
        let mut strings = StringTable::new();
        let mut path_syntax = PathSyntaxTable::new();
        let path = InternedPath::from_single_str("not_a_directory/value.mtf", &mut strings);
        let path_syntax_id = path_syntax.push(path.clone(), SourceLocation::default());
        let reference = PreparedFileReference {
            source_file: Some(SourceId::from_index(0)),
            path_syntax: path_syntax_id,
            location: SourceLocation::default(),
            class: PreparedFileReferenceClass::ContentSource,
        };
        let resolved = resolver
            .resolve(
                &temp.path().join("main.moth"),
                &path_syntax,
                &reference,
                &mut strings,
            )
            .expect("NotADirectory should remain a typed path outcome");
        assert!(matches!(
            resolved.outcome,
            SingleFileReferenceOutcome::Diagnostic(diagnostic)
                if matches!(
                    diagnostic.payload,
                    crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
                        reason: InvalidCompileTimePathReason::TargetNotRegular,
                        ..
                    }
                )
        ));
        assert!(resource_inputs.missing_watch_interests().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn synthetic_in_owner_dangling_alias_watches_resolved_physical_prefix() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("test root should be created");
        let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
        let real = root.join("real");
        fs::create_dir_all(&real).expect("real directory should be created");
        symlink(real.join("missing-target"), root.join("alias"))
            .expect("dangling in-owner alias should be created");

        let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
        source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
        let mut resource_inputs = ResourceInputRegistry::new();
        let mut resolver = SingleFileReferenceResolver::new(
            root.clone(),
            &source_file_kinds,
            &mut resource_inputs,
        );
        let mut strings = StringTable::new();
        let mut path_syntax = PathSyntaxTable::new();
        let path = InternedPath::from_single_str("alias/leaf.svg", &mut strings);
        let path_syntax_id = path_syntax.push(path.clone(), SourceLocation::default());
        let reference = PreparedFileReference {
            source_file: Some(SourceId::from_index(0)),
            path_syntax: path_syntax_id,
            location: SourceLocation::default(),
            class: PreparedFileReferenceClass::ResourceFile,
        };

        let owner = root.join("main.moth");
        for _ in 0..2 {
            let resolved = resolver
                .resolve(&owner, &path_syntax, &reference, &mut strings)
                .expect("in-owner dangling alias should settle");
            assert!(matches!(
                resolved.outcome,
                SingleFileReferenceOutcome::Diagnostic(diagnostic)
                    if matches!(
                        diagnostic.payload,
                        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
                            reason: InvalidCompileTimePathReason::MissingTarget,
                            ..
                        }
                    )
            ));
        }

        assert_eq!(resource_inputs.missing_watch_interests().len(), 1);
        assert_eq!(
            resource_inputs.missing_watch_interests()[0].canonical_path(),
            real.join("missing-target")
        );
    }

    #[cfg(unix)]
    #[test]
    fn synthetic_dangling_alias_resolves_nested_symlink_targets_before_parent_segments() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("test root should be created");
        let outside_temp = tempfile::tempdir().expect("outside root should be created");
        let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
        let outside_root =
            fs::canonicalize(outside_temp.path()).expect("outside root should canonicalize");
        fs::create_dir_all(root.join("inside")).expect("inside directory should be created");
        symlink(&outside_root, root.join("inside/redirect"))
            .expect("nested external target should be created");
        symlink("inside/redirect/..", root.join("alias"))
            .expect("nested dangling alias should be created");

        let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
        source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
        let mut resource_inputs = ResourceInputRegistry::new();
        let mut resolver = SingleFileReferenceResolver::new(
            root.clone(),
            &source_file_kinds,
            &mut resource_inputs,
        );
        let mut strings = StringTable::new();
        let mut path_syntax = PathSyntaxTable::new();
        let path = InternedPath::from_single_str("alias/missing.svg", &mut strings);
        let path_syntax_id = path_syntax.push(path.clone(), SourceLocation::default());
        let reference = PreparedFileReference {
            source_file: Some(SourceId::from_index(0)),
            path_syntax: path_syntax_id,
            location: SourceLocation::default(),
            class: PreparedFileReferenceClass::ResourceFile,
        };

        let resolved = resolver
            .resolve(
                &root.join("main.moth"),
                &path_syntax,
                &reference,
                &mut strings,
            )
            .expect("nested dangling alias should settle");
        assert!(matches!(
            resolved.outcome,
            SingleFileReferenceOutcome::Diagnostic(diagnostic)
                if matches!(
                    diagnostic.payload,
                    crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
                        reason: InvalidCompileTimePathReason::EscapesSymlink,
                        ..
                    }
                )
        ));
        assert!(resource_inputs.missing_watch_interests().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn synthetic_dangling_symlink_cycles_and_excessive_depth_are_infrastructure_errors() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("test root should be created");
        let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
        symlink(root.join("cycle-b"), root.join("cycle-a"))
            .expect("first cycle link should be created");
        symlink(root.join("cycle-a"), root.join("cycle-b"))
            .expect("second cycle link should be created");
        for index in 0..=41 {
            let target = if index == 41 {
                root.join("missing-target")
            } else {
                root.join(format!("deep-{}", index + 1))
            };
            symlink(target, root.join(format!("deep-{index}")))
                .expect("deep symlink should be created");
        }

        let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
        source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
        let mut resource_inputs = ResourceInputRegistry::new();
        let mut resolver = SingleFileReferenceResolver::new(
            root.clone(),
            &source_file_kinds,
            &mut resource_inputs,
        );
        let mut strings = StringTable::new();
        let mut path_syntax = PathSyntaxTable::new();

        let resolve = |resolver: &mut SingleFileReferenceResolver<'_>,
                       path: &str,
                       strings: &mut StringTable,
                       path_syntax: &mut PathSyntaxTable| {
            let authored_path = InternedPath::from_single_str(path, strings);
            let path_syntax_id = path_syntax.push(authored_path.clone(), SourceLocation::default());
            let reference = PreparedFileReference {
                source_file: Some(SourceId::from_index(0)),
                path_syntax: path_syntax_id,
                location: SourceLocation::default(),
                class: PreparedFileReferenceClass::ResourceFile,
            };
            resolver.resolve(&root.join("main.moth"), path_syntax, &reference, strings)
        };

        assert!(
            resolve(
                &mut resolver,
                "cycle-a/leaf.svg",
                &mut strings,
                &mut path_syntax
            )
            .is_err()
        );
        assert!(
            resolve(
                &mut resolver,
                "deep-0/leaf.svg",
                &mut strings,
                &mut path_syntax
            )
            .is_err()
        );
        assert!(resource_inputs.missing_watch_interests().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn synthetic_rejects_support_and_missing_symlink_boundaries_without_watches() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("test root should be created");
        let outside_temp = tempfile::tempdir().expect("outside root should be created");
        let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
        let support_root = root.join("support");
        let child_root = root.join("child");
        let outside_root =
            fs::canonicalize(outside_temp.path()).expect("outside root should canonicalize");
        fs::create_dir_all(&support_root).expect("support root should be created");
        fs::create_dir_all(&child_root).expect("child root should be created");
        fs::write(support_root.join("+support.moth"), "support root")
            .expect("support root file should be created");
        fs::write(child_root.join("@child.moth"), "child root")
            .expect("child root file should be created");
        fs::write(support_root.join("existing.mtf"), "template")
            .expect("support content should be created");
        fs::write(support_root.join("existing.svg"), "resource")
            .expect("support resource should be created");
        symlink(&support_root, root.join("support-alias"))
            .expect("support alias should be created");
        symlink(&child_root, root.join("child-alias")).expect("child alias should be created");
        symlink(&outside_root, root.join("outside-alias"))
            .expect("outside alias should be created");
        symlink(
            outside_root.join("missing-target"),
            root.join("dangling-external"),
        )
        .expect("dangling external alias should be created");
        symlink(
            child_root.join("missing-target"),
            root.join("dangling-child"),
        )
        .expect("dangling child alias should be created");
        symlink(
            support_root.join("missing-target"),
            root.join("dangling-support"),
        )
        .expect("dangling support alias should be created");

        let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
        source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
        let mut resource_inputs = ResourceInputRegistry::new();
        let mut resolver = SingleFileReferenceResolver::new(
            root.clone(),
            &source_file_kinds,
            &mut resource_inputs,
        );
        let mut strings = StringTable::new();
        let mut path_syntax = PathSyntaxTable::new();

        let resolve = |resolver: &mut SingleFileReferenceResolver<'_>,
                       path: &str,
                       class: PreparedFileReferenceClass,
                       strings: &mut StringTable,
                       path_syntax: &mut PathSyntaxTable| {
            let authored_path = InternedPath::from_single_str(path, strings);
            let path_syntax_id = path_syntax.push(authored_path.clone(), SourceLocation::default());
            let reference = PreparedFileReference {
                source_file: Some(SourceId::from_index(0)),
                path_syntax: path_syntax_id,
                location: SourceLocation::default(),
                class,
            };
            resolver
                .resolve(&root.join("main.moth"), path_syntax, &reference, strings)
                .expect("synthetic reference should settle")
        };

        for (path, class) in [
            (
                "support/existing.mtf",
                PreparedFileReferenceClass::ContentSource,
            ),
            (
                "support/existing.svg",
                PreparedFileReferenceClass::ResourceFile,
            ),
            (
                "support/missing.mtf",
                PreparedFileReferenceClass::ContentSource,
            ),
            (
                "support/missing.svg",
                PreparedFileReferenceClass::ResourceFile,
            ),
            (
                "support-alias/missing.mtf",
                PreparedFileReferenceClass::ContentSource,
            ),
            (
                "child-alias/missing.svg",
                PreparedFileReferenceClass::ResourceFile,
            ),
            (
                "dangling-child/missing.svg",
                PreparedFileReferenceClass::ResourceFile,
            ),
            (
                "dangling-support/missing.mtf",
                PreparedFileReferenceClass::ContentSource,
            ),
        ] {
            let resolved = resolve(&mut resolver, path, class, &mut strings, &mut path_syntax);
            assert!(
                matches!(
                    resolved.outcome,
                    SingleFileReferenceOutcome::Diagnostic(diagnostic)
                        if matches!(
                            diagnostic.payload,
                            crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
                                reason: InvalidCompileTimePathReason::EscapesModuleBoundary,
                                ..
                            }
                        )
                ),
                "expected module-boundary rejection for {path}"
            );
        }

        for path in [
            "outside-alias/missing.mtf",
            "outside-alias/missing.svg",
            "dangling-external/missing.mtf",
            "dangling-external/missing.svg",
        ] {
            let resolved = resolve(
                &mut resolver,
                path,
                PreparedFileReferenceClass::ResourceFile,
                &mut strings,
                &mut path_syntax,
            );
            assert!(
                matches!(
                    resolved.outcome,
                    SingleFileReferenceOutcome::Diagnostic(diagnostic)
                        if matches!(
                            diagnostic.payload,
                            crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCompileTimePath {
                                reason: InvalidCompileTimePathReason::EscapesSymlink,
                                ..
                            }
                        )
                ),
                "expected symlink escape rejection for {path}"
            );
        }
        assert!(resource_inputs.missing_watch_interests().is_empty());
    }
}
