use super::prepared_source::PreparedSourceInput;
use super::prepared_source_store::{MothScanOrigin, PreparedSourceStore};
use super::*;
use crate::build_system::build::BackendBuilder;
use crate::build_system::create_project_modules::module_namespace::{
    DirectoryImportResolution, ModuleNamespaceSet, ResolvedImport,
};
use crate::build_system::create_project_modules::resolve_project_entry_root;
use crate::build_system::create_project_modules::source_package_discovery::build_source_package_boundary_indexes;
use crate::build_system::project_config::{
    ProjectConfigParseServices, load_project_config, parse_project_config_file,
};
use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::provider::{
    ExternalFileExtension, ExternalImportProvider, ExternalImportProviderContext,
    ExternalImportProviderKind, ExternalImportRequest, RequiredRuntimeImport,
    ResolvedExternalImport, RuntimeAssetIdentity,
};
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::compiler_frontend::compiler_errors::{CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terse};
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticCategory, DiagnosticPayload,
    InvalidAssignmentTargetReason, InvalidConfigReason, InvalidImportClauseReason,
    InvalidPackageFolderReason,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::{ExternalFunctionId, ExternalTypeId};
use crate::compiler_frontend::paths::const_paths::StructuralProviderReference;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::temp_dir;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Serializes tests that reset and read the process-global `SOURCE_READ_COUNT_FOR_TEST` counter.
///
/// WHY: source-read counting uses one global atomic and one global tracked-prefix slot. Parallel
/// test execution would otherwise let one test's reset/prefix overwrite another's mid-snapshot, so
/// every test that asserts on `source_read_count_for_test` holds this lock for its whole window.
#[cfg(test)]
static SOURCE_READ_COUNTER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        &project_root,
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
        source_tree_index.module_roots().clone(),
    )
    .expect("project path resolver should build")
}

fn test_style_directives() -> StyleDirectiveRegistry {
    StyleDirectiveRegistry::built_ins()
}

fn parse_project_config_for_test(
    config: &mut Config,
    config_path: &std::path::Path,
    style_directives: &StyleDirectiveRegistry,
) -> Result<(), CompilerMessages> {
    let frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    let mut string_table = StringTable::new();
    let services = ProjectConfigParseServices {
        style_directives,
        frontend_surface: &frontend_surface,
    };
    parse_project_config_file(config, config_path, &services, &mut string_table)
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
    let services = ProjectConfigParseServices {
        style_directives,
        frontend_surface: &frontend_surface,
    };
    parse_project_config_file(config, config_path, &services, &mut string_table)
}

fn parse_project_config_for_test_with_packages(
    config: &mut Config,
    config_path: &std::path::Path,
    style_directives: &StyleDirectiveRegistry,
    frontend_surface: &crate::builder_surface::BuilderSurface,
) -> Result<(), CompilerMessages> {
    let mut string_table = StringTable::new();
    let services = ProjectConfigParseServices {
        style_directives,
        frontend_surface,
    };
    parse_project_config_file(config, config_path, &services, &mut string_table)
}

fn discover_modules_for_test(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> Result<ModuleEntryCompileWaves, CompilerMessages> {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        &project_root,
        config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )?;
    let mut project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let mut external_packages = ExternalPackageRegistry::new();
    let external_import_providers =
        crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::empty();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_import_resolution_table =
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
    let mut external_imports = super::reachable_file_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: &external_import_providers,
        cache: &mut external_import_cache,
        resolution_table: &mut external_import_resolution_table,
    };
    discover_all_modules_in_project(
        config,
        resolver,
        &mut project_module_graph,
        style_directives,
        &mut external_imports,
        DirectoryImportResolution::project(&module_namespace_set, &source_tree_index),
        &mut string_table,
    )
}

fn discover_modules_for_test_with_providers(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_import_providers: &ExternalImportProviderRegistry,
) -> Result<ModuleEntryCompileWaves, CompilerMessages> {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        &project_root,
        config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        external_import_providers,
        &mut string_table,
    )?;
    let mut project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let mut external_packages = ExternalPackageRegistry::new();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_import_resolution_table =
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
    let mut external_imports = super::reachable_file_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: external_import_providers,
        cache: &mut external_import_cache,
        resolution_table: &mut external_import_resolution_table,
    };

    discover_all_modules_in_project(
        config,
        resolver,
        &mut project_module_graph,
        style_directives,
        &mut external_imports,
        DirectoryImportResolution::project(&module_namespace_set, &source_tree_index),
        &mut string_table,
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
    body: impl FnOnce(&DirectoryImportResolution, &mut StringTable),
) {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let external_import_providers =
        crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default();
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        &project_root,
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
    let resolution = DirectoryImportResolution::project(&module_namespace_set, &source_tree_index);
    body(&resolution, &mut string_table);
}

/// Build a grouped `StructuralProviderReference` whose interned path is the provider prefix plus
/// the requested item, matching how the tokenizer expands grouped import syntax.
fn grouped_provider(
    path_segments: &[&str],
    string_table: &mut StringTable,
) -> StructuralProviderReference {
    let mut path = crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    for segment in path_segments {
        path.push_str(segment, string_table);
    }
    StructuralProviderReference {
        path,
        path_location: SourceLocation::default(),
        from_grouped: true,
    }
}

#[test]
fn grouped_import_resolves_same_module_item_file() {
    let root = temp_dir("grouped_same_module_item_file");
    let src = root.join("src");
    fs::create_dir_all(src.join("choices")).expect("should create choices dir");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @choices { Response }\n#[:entry]\n",
    )
    .expect("should write entry");
    fs::write(
        src.join("choices/Response.moth"),
        "Response :: Success, ;\n",
    )
    .expect("should write same-module item file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    let importer = fs::canonicalize(src.join("#page.moth")).expect("importer should canonicalize");

    with_namespace_resolution(
        &config,
        &resolver,
        &crate::builder_surface::SourcePackageRegistry::default(),
        |resolution, string_table| {
            let provider = grouped_provider(&["choices", "Response"], string_table);
            let resolved = resolution
                .resolve_import(&provider, &importer, string_table)
                .expect("a grouped same-module item file should resolve");
            match resolved {
                ResolvedImport::SameModuleSource { canonical_path, .. } => {
                    assert!(
                        canonical_path.ends_with("choices/Response.moth"),
                        "expected the same-module item file, got {:?}",
                        canonical_path
                    );
                }
                other => panic!("expected SameModuleSource, got {:?}", other),
            }
        },
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn grouped_import_resolves_cross_module_child_facade() {
    let root = temp_dir("grouped_cross_module_child_facade");
    let src = root.join("src");
    fs::create_dir_all(src.join("child")).expect("should create child module dir");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @child { greet }\n#[:entry]\n",
    )
    .expect("should write entry");
    fs::write(
        src.join("child/#mod.moth"),
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
    let importer = fs::canonicalize(src.join("#page.moth")).expect("importer should canonicalize");

    with_namespace_resolution(
        &config,
        &resolver,
        &crate::builder_surface::SourcePackageRegistry::default(),
        |resolution, string_table| {
            let provider = grouped_provider(&["child", "greet"], string_table);
            let resolved = resolution
                .resolve_import(&provider, &importer, string_table)
                .expect("a grouped child-module facade should resolve");
            match resolved {
                ResolvedImport::CrossModule { root_file, .. } => {
                    assert!(
                        root_file.ends_with("child/#mod.moth"),
                        "expected the child module facade root file, got {:?}",
                        root_file
                    );
                }
                other => panic!("expected CrossModule, got {:?}", other),
            }
        },
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn grouped_import_resolves_source_package_facade() {
    let root = temp_dir("grouped_source_package_facade");
    let src = root.join("src");
    let package_root = root.join("builder/helper");
    fs::create_dir_all(&src).expect("should create src dir");
    fs::create_dir_all(&package_root).expect("should create helper package dir");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @helper { add }\n#[:entry]\n",
    )
    .expect("should write entry");
    fs::write(
        package_root.join("#mod.moth"),
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
    let importer = fs::canonicalize(src.join("#page.moth")).expect("importer should canonicalize");

    with_namespace_resolution(
        &config,
        &resolver,
        &source_packages,
        |resolution, string_table| {
            let provider = grouped_provider(&["helper", "add"], string_table);
            let resolved = resolution
                .resolve_import(&provider, &importer, string_table)
                .expect("a grouped source-package facade should resolve");
            match resolved {
                ResolvedImport::SourcePackageSurface { root_file, .. } => {
                    assert!(
                        root_file.ends_with("helper/#mod.moth"),
                        "expected the helper package facade root file, got {:?}",
                        root_file
                    );
                }
                other => panic!("expected SourcePackageSurface, got {:?}", other),
            }
        },
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

/// Discover modules and return the populated project module graph plus the shared string table
/// so focused Phase 5b invariant tests can inspect inserted edges and retained source locations.
fn discover_modules_and_graph_for_test(
    config: &Config,
    resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
) -> (
    ModuleEntryCompileWaves,
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
        &project_root,
        config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("source tree index should build");
    let mut project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let mut external_packages = ExternalPackageRegistry::new();
    let external_import_providers =
        crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::empty();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_import_resolution_table =
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
    let mut external_imports = super::reachable_file_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: &external_import_providers,
        cache: &mut external_import_cache,
        resolution_table: &mut external_import_resolution_table,
    };

    let modules = discover_all_modules_in_project(
        config,
        resolver,
        &mut project_module_graph,
        style_directives,
        &mut external_imports,
        DirectoryImportResolution::project(&module_namespace_set, &source_tree_index),
        &mut string_table,
    )
    .expect("module discovery should pass for focused graph-edge tests");

    (
        modules,
        project_module_graph,
        source_tree_index,
        string_table,
    )
}

fn rendered_first_error(messages: &CompilerMessages) -> String {
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("expected one diagnostic");
    terse::format_terse_diagnostic_with_context(
        diagnostic,
        DiagnosticRenderContext::new(&messages.string_table),
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
) -> Result<ModuleEntryCompileWaves, CompilerMessages> {
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
    let root = temp_dir("source_tree_index_outputs");
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
        fs::write(directory.join("#skipped.moth"), "").expect("should write skipped root");
    }

    fs::write(entry_root.join("#home.moth"), "").expect("should write entry root");
    fs::write(entry_root.join("ordinary.moth"), "").expect("should write ordinary source");
    fs::write(nested.join("#nested.moth"), "").expect("should write nested root");

    let mut config = Config::new(root.clone());
    config.dev_folder = PathBuf::from("scratch");
    config.release_folder = PathBuf::from("generated");
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();

    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root.clone(),
        &canonical_root,
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
    assert!(entry_root_files[0].ends_with("#home.moth"));
    assert_eq!(index.stats().dirs_visited, 2);
    assert_eq!(index.stats().dirs_skipped, 10);
    assert_eq!(index.stats().files_seen, 3);
    assert_eq!(index.stats().hash_root_files_seen, 2);
    assert_eq!(index.stats().module_roots_found, 2);

    let root_directories = index
        .module_roots()
        .root_directories()
        .map(|path| path.file_name().and_then(OsStr::to_str).unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        root_directories[0],
        canonical_entry_root
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap()
    );
    assert_eq!(root_directories[1], "nested");

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn source_tree_index_ignores_collision_in_fixed_skipped_directory() {
    let root = temp_dir("source_tree_index_fixed_skipped_collision");
    let entry_root = root.clone();

    // Fixed-skipped directory with collision-shaped contents.
    // The canonical `skipped_directory_collision_ignored` integration case covers the
    // configured-skip path; this unit retains the fixed-skip policy fact.
    let target_dir = entry_root.join("target");
    fs::create_dir_all(target_dir.join("helper")).expect("should create target/helper");
    fs::write(target_dir.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");

    // Real module root that should be discovered.
    let nested = entry_root.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module");
    fs::write(entry_root.join("#home.moth"), "").expect("should write entry root");
    fs::write(nested.join("#nested.moth"), "").expect("should write nested root");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();

    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        &canonical_root,
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn source_tree_index_ignores_package_prefix_collision_in_skipped_directory() {
    let root = temp_dir("source_tree_index_skipped_prefix_collision");
    let entry_root = root.join("src");
    fs::create_dir_all(&entry_root).expect("should create entry root");

    // Fixed-skipped directory whose name matches a source-backed package prefix.
    // Under the skip policy this folder is not importable, so no prefix collision.
    fs::create_dir_all(entry_root.join("target")).expect("should create target folder");
    fs::write(entry_root.join("#home.moth"), "").expect("should write entry root");

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
        &canonical_root,
        &config,
        &source_packages,
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("skipped folder matching a package prefix must not trigger prefix collision");

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn source_tree_index_detects_collision_in_non_skipped_directory() {
    let root = temp_dir("source_tree_index_non_skipped_collision");
    let entry_root = root.join("src");
    fs::create_dir_all(entry_root.join("helper")).expect("should create helper folder");
    fs::write(entry_root.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");
    fs::write(entry_root.join("#home.moth"), "").expect("should write entry root");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();

    let messages = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        &canonical_root,
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn bounded_module_roots_for_single_file_indexes_nested_roots_with_ignored_directories() {
    let root = temp_dir("bounded_single_file_nested_ignored");
    let module_dir = root.join("module");
    let nested = module_dir.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module");

    // Ignored directory with collision-shaped contents.
    let target_dir = module_dir.join("target");
    fs::create_dir_all(target_dir.join("helper")).expect("should create target/helper");
    fs::write(target_dir.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");

    fs::write(module_dir.join("#home.moth"), "").expect("should write entry root");
    fs::write(nested.join("#nested.moth"), "").expect("should write nested root");

    let config = Config::new(root.clone());
    let entry_file = fs::canonicalize(module_dir.join("#home.moth")).unwrap();
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
        .expect("single-file hash root should index its tree without collision errors");

    let root_directories = module_roots
        .root_directories()
        .map(|path| path.file_name().and_then(OsStr::to_str).unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(root_directories.len(), 2);
    assert!(root_directories.contains(&"module"));
    assert!(root_directories.contains(&"nested"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn bounded_module_roots_for_single_file_rejects_import_name_collisions() {
    let root = temp_dir("bounded_single_file_collision");
    let module_dir = root.join("module");
    fs::create_dir_all(module_dir.join("helper")).expect("should create helper directory");
    fs::write(module_dir.join("helper.moth"), "helper #= 1\n")
        .expect("should write colliding source file");
    fs::write(module_dir.join("#home.moth"), "").expect("should write entry root");

    let config = Config::new(root.clone());
    let entry_file = fs::canonicalize(module_dir.join("#home.moth")).unwrap();
    let mut string_table = StringTable::new();

    let messages = super::source_tree_index::SourceTreeIndex::bounded_module_roots_for_single_file(
        &entry_file,
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect_err("single-file hash roots should reject real import-name collisions");

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourceFileFolderCollision { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn source_tree_index_rejects_duplicate_hash_root_files() {
    let root = temp_dir("source_tree_index_duplicate_roots");
    let entry_root = root.join("src");
    fs::create_dir_all(&entry_root).expect("should create entry root");
    fs::write(entry_root.join("#home.moth"), "").expect("should write page root");
    fs::write(entry_root.join("#layout.moth"), "").expect("should write layout root");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();
    let messages = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        &canonical_root,
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect_err("a module directory may contain only one hash root");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn project_path_resolver_consumes_source_tree_module_roots() {
    let root = temp_dir("source_tree_index_resolver_consumption");
    let entry_root = root.join("src");
    let nested = entry_root.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module directory");
    fs::write(entry_root.join("#home.moth"), "").expect("should write entry root");
    fs::write(nested.join("#nested.moth"), "").expect("should write nested root");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    let mut string_table = StringTable::new();
    let setup = super::project_roots::build_project_path_resolver_with_index(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &crate::compiler_frontend::external_packages::ExternalPackageRegistry::new(),
        &mut string_table,
    )
    .expect("resolver setup should build from prepared roots");
    let resolver = setup.resolver;

    let mut import_path = crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    import_path.push_str("nested", &mut string_table);
    import_path.push_str("identity", &mut string_table);
    let resolved = resolver
        .resolve_import_to_source_file_with_public_surface_fallback(
            &import_path,
            &entry_root.join("#home.moth"),
            &mut string_table,
        )
        .expect("prepared nested module root should resolve its public surface");
    assert_eq!(
        resolved.path,
        fs::canonicalize(nested.join("#nested.moth")).unwrap()
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[derive(Debug)]
struct CountingExternalImportProvider {
    calls: Arc<AtomicUsize>,
    extensions: Vec<ExternalFileExtension>,
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
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: request.canonical_source_path,
                asset_kind: "js".to_owned(),
            }),
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
    let root = temp_dir("config_constants");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "entry_root #= \"src\"\ndev_folder #= \"dev\"\noutput_folder #= \"release\"\nname #= \"docs\"\nversion #= \"1.2.3\"\nproject #= \"html\"\npage_url_style #= \"trailing_slash\"\nredirect_index_html #= true\npackage_folders #= { \"lib\", \"packages\" }\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test_with_html_keys(&mut config, &config_path, &style_directives)
        .expect("config should parse");

    assert_eq!(config.entry_root, PathBuf::from("src"));
    assert_eq!(config.dev_folder, PathBuf::from("dev"));
    assert_eq!(config.release_folder, PathBuf::from("release"));
    assert_eq!(config.project_name, "docs");
    assert_eq!(config.version, "1.2.3");
    assert_eq!(config.settings.get("project"), Some(&"html".to_string()));
    assert_eq!(
        config.settings.get("page_url_style"),
        Some(&"trailing_slash".to_string())
    );
    assert_eq!(
        config.settings.get("redirect_index_html"),
        Some(&"true".to_string())
    );
    assert_eq!(
        config.package_folders,
        vec![PathBuf::from("lib"), PathBuf::from("packages")]
    );
    assert!(
        config.has_explicit_package_folders,
        "package_folders should be marked as explicitly configured"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn loads_canonical_config_file_from_project_root() {
    let root = temp_dir("canonical_config_lookup");
    fs::create_dir_all(&root).expect("should create root dir");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    let services = ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
    };
    let mut string_table = StringTable::new();

    load_project_config(&mut config, &services, &mut string_table)
        .expect("canonical config should load");

    assert_eq!(config.config_file_path(), root.join("config.moth"));
    assert_eq!(config.entry_root, PathBuf::from("src"));

    fs::remove_dir_all(&root).expect("should remove root dir");
}

#[test]
fn rejects_direct_canonical_config_import_paths() {
    let mut string_table = StringTable::new();

    for import_path in ["config", "config.moth"] {
        let path = crate::compiler_frontend::symbols::interned_path::InternedPath::from_single_str(
            import_path,
            &mut string_table,
        );

        assert!(
            crate::compiler_frontend::source_packages::root_file::import_path_references_config_file(
                &path,
                false,
                &string_table,
            ),
            "direct config import should be treated as a special file: {import_path}"
        );
    }

    let mut nested_source_path =
        crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    nested_source_path.push_str("config", &mut string_table);
    nested_source_path.push_str("init_config", &mut string_table);

    assert!(
        !crate::compiler_frontend::source_packages::root_file::import_path_references_config_file(
            &nested_source_path,
            false,
            &string_table,
        ),
        "a folder named config must remain a valid source path prefix"
    );

    let mut grouped_config_path =
        crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    grouped_config_path.push_str("config", &mut string_table);
    grouped_config_path.push_str("project", &mut string_table);

    assert!(
        crate::compiler_frontend::source_packages::root_file::import_path_references_config_file(
            &grouped_config_path,
            true,
            &string_table,
        ),
        "a grouped import must classify its source component as config"
    );
}

#[test]
fn rejects_unknown_config_key() {
    let root = temp_dir("config_unknown_key");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "custom_key #= \"custom_value\"\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    let DiagnosticPayload::InvalidConfig {
        reason: InvalidConfigReason::UnknownKey { key },
        ..
    } = &diagnostic.payload
    else {
        panic!(
            "expected UnknownKey diagnostic, got: {:?}",
            diagnostic.payload
        );
    };
    assert_eq!(
        messages.string_table.resolve(*key),
        "custom_key",
        "UnknownKey should retain the authored key name in the structured payload"
    );

    // The key occupies columns 1 through 10. The initializer begins later on the same line, so
    // this exact span protects the Stage 0 key-location handoff.
    assert_eq!(
        diagnostic
            .primary_location
            .scope
            .to_path_buf(&messages.string_table),
        config_path.as_path(),
        "UnknownKey should point at the authored config file scope"
    );
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, 1,
        "UnknownKey should start at the authored key name"
    );
    assert_eq!(
        diagnostic.primary_location.end_pos.char_column, 10,
        "UnknownKey should end at the authored key name span, not the initializer value"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_config_plain_and_mutable_bindings() {
    // Both `=` and `~=` produce the same `PlainBindingUnsupported` reason. The canonical
    // `config_plain_project_rejected` and `config_mutable_key_rejected` cases cover the
    // user-visible rejection; this unit retains the typed reason for both binding modes.
    for (operator, label) in [("=", "plain"), ("~=", "mutable")] {
        let root = temp_dir(&format!("config_{label}_binding_rejected"));
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
    let root = temp_dir("config_hash_binding");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "entry_root #= \"src\"\nproject_name #String = \"docs\"\nversion #= \"1.0\"\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("config should parse");

    assert_eq!(config.entry_root, PathBuf::from("src"));
    assert_eq!(config.project_name, "docs");
    assert_eq!(config.version, "1.0");

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_config_function_declarations() {
    let root = temp_dir("config_function_rejected");
    fs::create_dir_all(&root).expect("should create root dir");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_config_type_declarations() {
    let cases = [
        (
            "struct",
            "Config = |\n    value String,\n|\nentry_root #= \"src\"\n",
        ),
        ("choice", "Mode ::\n    Ready,\n;\nentry_root #= \"src\"\n"),
        (
            "alias",
            "EntryRoot as String\nentry_root #EntryRoot = \"src\"\n",
        ),
    ];

    for (case_name, source) in cases {
        let root = temp_dir(&format!("config_{case_name}_accepted"));
        fs::create_dir_all(&root).expect("should create root dir");
        let config_path = root.join(settings::CONFIG_FILE_NAME);

        fs::write(&config_path, source).expect("should write config");

        let mut config = Config::new(root.clone());
        let style_directives = test_style_directives();
        parse_project_config_for_test(&mut config, &config_path, &style_directives)
            .expect("config should accept type declarations");

        assert_eq!(
            config.entry_root,
            PathBuf::from("src"),
            "config key should be parsed for {case_name}"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

#[test]
fn rejects_config_standalone_template() {
    let root = temp_dir("config_standalone_template_rejected");
    fs::create_dir_all(&root).expect("should create root dir");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_config_const_page_fragment() {
    let root = temp_dir("config_const_fragment_rejected");
    fs::create_dir_all(&root).expect("should create root dir");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_project_local_config_import_even_when_module_root_exists() {
    let root = temp_dir("config_project_local_import_rejected");
    fs::create_dir_all(&root).expect("should create root dir");
    fs::create_dir_all(root.join("settings")).expect("should create settings module");
    fs::write(root.join("settings/#mod.moth"), "value #= \"src\"\n")
        .expect("should write settings root");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "import @settings { value }\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigImportRootViolation,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn configured_package_folder_registers_project_local_source_metadata() {
    let root = temp_dir("configured_project_local_package_metadata");
    let package_root = root.join("packages/widgets");
    fs::create_dir_all(&package_root).expect("should create project-local package");

    let mut config = Config::new(root.clone());
    config.package_folders = vec![PathBuf::from("packages")];
    config.has_explicit_package_folders = true;

    let mut string_table = StringTable::new();
    let discovered = super::source_package_discovery::discover_project_local_source_packages(
        &config,
        &root,
        &mut string_table,
    )
    .expect("configured package folder should be discovered");
    let package = discovered
        .get_root("widgets")
        .expect("widgets package should be registered");

    assert_eq!(
        package.metadata,
        crate::builder_surface::PackageMetadata::source(
            crate::builder_surface::PackageOrigin::ProjectLocal,
        )
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_config_imported_builder_source_package_constant() {
    let root = temp_dir("config_builder_package_constant");
    let package_root = root.join("builder/defaults");
    fs::create_dir_all(&package_root).expect("should create Builder package");
    fs::write(
        package_root.join("#mod.moth"),
        "export:\n    default_entry_root #= \"src\"\n;\n",
    )
    .expect("should write builder root");
    let config_path = root.join(settings::CONFIG_FILE_NAME);
    fs::write(
        &config_path,
        "import @defaults { default_entry_root }\nentry_root #= default_entry_root\n",
    )
    .expect("should write config");

    let mut frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "defaults",
        package_root,
        PackageOrigin::Builder,
    );

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test_with_packages(
        &mut config,
        &config_path,
        &style_directives,
        &frontend_surface,
    )
    .expect("config should resolve builder source-backed package constant");

    assert_eq!(config.entry_root, PathBuf::from("src"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_config_imported_constant_that_depends_on_imported_constant() {
    let root = temp_dir("config_builder_package_constant_chain");
    let package_root = root.join("builder/defaults");
    fs::create_dir_all(&package_root).expect("should create Builder package");
    fs::write(
        package_root.join("#mod.moth"),
        "root_folder #= \"src\"\nexport:\n    default_entry_root #= root_folder\n;\n",
    )
    .expect("should write builder root");
    let config_path = root.join(settings::CONFIG_FILE_NAME);
    fs::write(
        &config_path,
        "import @defaults { default_entry_root }\nentry_root #= default_entry_root\n",
    )
    .expect("should write config");

    let mut frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "defaults",
        package_root,
        PackageOrigin::Builder,
    );

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test_with_packages(
        &mut config,
        &config_path,
        &style_directives,
        &frontend_surface,
    )
    .expect("config should resolve imported constant dependency");

    assert_eq!(config.entry_root, PathBuf::from("src"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_config_imported_constant_reexported_from_builder_source_package_file() {
    let root = temp_dir("config_builder_package_reexport");
    let package_root = root.join("builder/defaults");
    fs::create_dir_all(&package_root).expect("should create Builder package");
    fs::write(
        package_root.join("#mod.moth"),
        "import @./values { root_folder as internal_root }\n\nexport:\n    default_entry_root #= internal_root\n;\n",
    )
    .expect("should write builder root");
    fs::write(package_root.join("values.moth"), "root_folder #= \"src\"\n")
        .expect("should write builder support file");
    let config_path = root.join(settings::CONFIG_FILE_NAME);
    fs::write(
        &config_path,
        "import @defaults { default_entry_root }\nentry_root #= default_entry_root\n",
    )
    .expect("should write config");

    let mut frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "defaults",
        package_root,
        PackageOrigin::Builder,
    );

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test_with_packages(
        &mut config,
        &config_path,
        &style_directives,
        &frontend_surface,
    )
    .expect("config should resolve re-exported builder source-backed package constant");

    assert_eq!(config.entry_root, PathBuf::from("src"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_config_imported_type_declarations_as_support_surface() {
    let root = temp_dir("config_builder_package_type_alias");
    let package_root = root.join("builder/defaults");
    fs::create_dir_all(&package_root).expect("should create Builder package");
    fs::write(
        package_root.join("#mod.moth"),
        "export:\n    EntryRoot as String\n    Config = |\n        value String,\n    |\n    Mode ::\n        Ready,\n    ;\n    default_entry_root #= \"src\"\n;\n",
    )
    .expect("should write builder root");
    let config_path = root.join(settings::CONFIG_FILE_NAME);
    fs::write(
        &config_path,
        "import @defaults { EntryRoot, Config, Mode, default_entry_root }\nentry_root #EntryRoot = default_entry_root\n",
    )
    .expect("should write config");

    let mut frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "defaults",
        package_root,
        PackageOrigin::Builder,
    );

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test_with_packages(
        &mut config,
        &config_path,
        &style_directives,
        &frontend_surface,
    )
    .expect("config should allow imported type declarations as support surface");

    assert_eq!(config.entry_root, PathBuf::from("src"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn imported_config_support_duplicate_keeps_normal_duplicate_diagnostic() {
    let root = temp_dir("config_builder_package_duplicate");
    let package_root = root.join("builder/defaults");
    fs::create_dir_all(&package_root).expect("should create Builder package");
    fs::write(
        package_root.join("#mod.moth"),
        "default_entry_root #= \"src\"\ndefault_entry_root #= \"app\"\n",
    )
    .expect("should write duplicate builder root");
    let config_path = root.join(settings::CONFIG_FILE_NAME);
    fs::write(
        &config_path,
        "import @defaults { default_entry_root }\nentry_root #= default_entry_root\n",
    )
    .expect("should write config");

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
    .expect_err("duplicate imported support declarations should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::DuplicateDeclaration { .. }
        ),
        "expected normal duplicate declaration diagnostic, got: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn package_prefix_collision_with_entry_root_folder_rejected() {
    let root = temp_dir("entry_root_lib_collision");
    fs::create_dir_all(root.join("src/helper")).expect("should create src/helper");
    fs::create_dir_all(root.join("lib/helper")).expect("should create lib/helper");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/helper/#mod.moth"), "foo #= 1\n").expect("should write root");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

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
        "entry-root folder colliding with source-backed package prefix should fail"
    );
    let messages = result.expect_err("checked above");
    let error_text = rendered_first_error(&messages);
    assert!(
        error_text.contains("collides") || error_text.contains("Ambiguous"),
        "error should mention collision or ambiguity: {error_text}"
    );
    assert_has_config_error(&messages);
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::EntryRootPackagePrefixCollision { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_package_folder_absolute_path_entry() {
    let root = temp_dir("invalid_package_folders_absolute");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "package_folders #= { \"/absolute/lib\" }\n")
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
                reason: InvalidConfigReason::InvalidPackageFolder {
                    reason: InvalidPackageFolderReason::AbsolutePath,
                    ..
                },
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_package_folder_parent_directory_entry() {
    let root = temp_dir("invalid_package_folders_dotdot");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "package_folders #= { \"../lib\" }\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::InvalidPackageFolder {
                    reason: InvalidPackageFolderReason::ParentDirectorySegment,
                    ..
                },
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_duplicate_package_folder_entries() {
    let root = temp_dir("duplicate_package_folders");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "package_folders #= { \"lib\", \"lib\" }\n")
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
                reason: InvalidConfigReason::DuplicatePackageFolder { .. },
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_nested_package_folder_entry() {
    let root = temp_dir("invalid_package_folders_nested");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "package_folders #= { \"lib/helpers\" }\n")
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
                reason: InvalidConfigReason::InvalidPackageFolder {
                    reason: InvalidPackageFolderReason::NestedPath,
                    ..
                },
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn missing_default_package_folder_is_ignored() {
    let root = temp_dir("missing_default_lib_ignored");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    assert!(
        !config.has_explicit_package_folders,
        "default package folders should not be marked explicit"
    );

    let mut string_table = StringTable::new();
    let resolver = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect("resolver should build even when default /lib is missing");

    assert!(
        resolver.source_package_roots().is_empty(),
        "no source-backed packages should be discovered when default /lib is missing"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_config_const_record_field_projection() {
    let root = temp_dir("config_const_record_projection");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "Defaults = |\n    entry_root String = \"src\",\n|\n\nentry_root #= Defaults().entry_root\n",
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn malformed_import_syntax_keeps_precise_location_during_module_discovery() {
    let root = temp_dir("malformed_import_location");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "import\n#[:ok]\n").expect("should write malformed entry");

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
        Ok(_) => panic!("malformed import should fail discovery"),
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
        src.join("#page.moth")
            .canonicalize()
            .expect("entry file path should canonicalize")
    );
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 1);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidImportClause {
                reason: InvalidImportClauseReason::ExpectedPath,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn config_import_parse_failure_keeps_precise_location_in_compiler_messages() {
    let root = temp_dir("config_import_location");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);
    fs::write(&config_path, "import\n").expect("should write malformed config");

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
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 0);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidImportClause {
                reason: InvalidImportClauseReason::ExpectedPath,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn discover_modules_uses_reachable_files_only() {
    let root = temp_dir("reachable_only");
    let src = root.join("src");
    fs::create_dir_all(src.join("libs")).expect("should create libs folder");
    fs::create_dir_all(src.join("styles")).expect("should create styles folder");
    fs::create_dir_all(src.join("docs")).expect("should create docs folder");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::create_dir_all(src.join("errors")).expect("should create errors folder");
    fs::write(
        src.join("#page.moth"),
        "import @libs/html { basic }\n#[:ok]\n",
    )
    .expect("should write entry");
    fs::write(src.join("errors/#404.moth"), "#[:404]\n").expect("should write 404");
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
        .find(|module| module.entry_point.file_name() == Some(OsStr::new("#page.moth")))
        .expect("should include #page module");
    let page_paths = page_module
        .input_files
        .iter()
        .map(|file| {
            file.source_path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect::<HashSet<_>>();

    assert!(page_paths.contains("#page.moth"));
    assert!(page_paths.contains("html.moth"));
    assert!(!page_paths.contains("outdated.moth"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn discover_modules_resolves_relative_child_imports() {
    let root = temp_dir("relative_imports");
    let src = root.join("src");
    fs::create_dir_all(src.join("components")).expect("should create components folder");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @components/widget\nio.line([: [\"page\"]])\n",
    )
    .expect("should write page");
    fs::write(
        src.join("components/widget.moth"),
        "import @components/common\nio.line([: [\"widget\"]])\n",
    )
    .expect("should write widget file");
    fs::write(
        src.join("components/common.moth"),
        "io.line([: [\"common\"]])\n",
    )
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

    let discovered = modules[0]
        .input_files
        .iter()
        .map(|file| {
            file.source_path()
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect::<HashSet<_>>();

    assert!(discovered.contains("#page.moth"));
    assert!(discovered.contains("widget.moth"));
    assert!(discovered.contains("common.moth"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn entry_root_fallback_wins_for_unmatched_non_relative_imports() {
    let root = temp_dir("entry_root_fallback");
    let src = root.join("src");
    let lib = root.join("lib");
    fs::create_dir_all(src.join("helpers")).expect("should create source helpers");
    fs::create_dir_all(lib.join("helpers")).expect("should create root-folder helpers");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @helpers/theme\nio.line([: [\"page\"]])\n",
    )
    .expect("should write page");
    fs::write(
        src.join("helpers/theme.moth"),
        "io.line([: [\"source\"]])\n",
    )
    .expect("should write source");
    fs::write(
        lib.join("helpers/theme.moth"),
        "io.line([: [\"package\"]])\n",
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
    let discovered_paths = modules[0]
        .input_files
        .iter()
        .map(|file| file.source_path().to_path_buf())
        .collect::<HashSet<_>>();

    assert!(
        discovered_paths.contains(&source_theme),
        "unmatched non-relative imports should fall back to the entry root"
    );
    assert!(
        !discovered_paths.contains(&package_theme),
        "entry-root fallback must not also pull in the same-stem package file"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn discover_all_modules_finds_multiple_hash_entries_per_root() {
    let root = temp_dir("multi_hash_entries");
    let src = root.join("src");
    fs::create_dir_all(src.join("nested")).expect("should create nested folder");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "io.line([: [\"page\"]])\n").expect("should write #page");
    fs::create_dir_all(src.join("layout")).expect("should create layout folder");
    fs::write(
        src.join("layout/#layout.moth"),
        "io.line([: [\"layout\"]])\n",
    )
    .expect("should write #layout");
    fs::write(src.join("nested/#lib.moth"), "io.line([: [\"lib\"]])\n")
        .expect("should write nested #lib");
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
            module
                .entry_point
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect::<HashSet<_>>();

    assert!(entry_names.contains("#page.moth"));
    assert!(entry_names.contains("#layout.moth"));
    assert!(entry_names.contains("#lib.moth"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_folded_template_initializer_for_compile_time_config_binding() {
    let root = temp_dir("config_folded_template");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "project #= [:html]\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("folded template initializer should be accepted");

    assert_eq!(
        config.settings.get("project"),
        Some(&"html".to_string()),
        "folded template should become config string value"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_config_local_reference_to_earlier_private_const() {
    let root = temp_dir("config_local_reference");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "output_folder #= \"release\"\ndev_folder #= output_folder\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("config with private const reference should succeed");

    assert_eq!(
        config.release_folder,
        PathBuf::from("release"),
        "output_folder should be set"
    );
    assert_eq!(
        config.dev_folder,
        PathBuf::from("release"),
        "dev_folder should resolve through private const reference"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_config_unresolved_local_reference() {
    let root = temp_dir("config_unresolved_local_reference");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "entry_root #= \"src\"\nproject #= missing_value\n",
    )
    .expect("should write config");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_config_non_compile_time_constant_value() {
    let root = temp_dir("config_non_foldable");
    fs::create_dir_all(&root).expect("should create root dir");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_duplicate_plain_config_bindings_before_config_validation() {
    let root = temp_dir("config_duplicate_private");
    fs::create_dir_all(&root).expect("should create root dir");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_config_non_key_private_helper() {
    let root = temp_dir("config_non_key_helper");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "helper #= \"src\"\nentry_root #= helper\n")
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
                reason: InvalidConfigReason::UnknownKey { .. },
                ..
            }
        ),
        "expected unknown key diagnostic for non-key helper, got: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_config_runtime_call_in_value() {
    let root = temp_dir("config_runtime_call");
    fs::create_dir_all(&root).expect("should create root dir");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

// ── Config value shape enforcement tests ──────────────────────────────────────

#[test]
fn accepts_valid_bool_config_keys() {
    let root = temp_dir("config_bool_shape_ok");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "redirect_index_html #= false\nhtml_inject_core_css #= true\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test_with_html_keys(&mut config, &config_path, &style_directives)
        .expect("valid boolean config values should parse");

    assert_eq!(
        config.settings.get("redirect_index_html"),
        Some(&"false".to_string())
    );
    assert_eq!(
        config.settings.get("html_inject_core_css"),
        Some(&"true".to_string())
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_core_string_key_with_bool_value() {
    let root = temp_dir("config_string_shape_bool_rejected");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "entry_root #= true\n").expect("should write config");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_backend_bool_key_with_string_value() {
    let root = temp_dir("config_bool_shape_string_rejected");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "redirect_index_html #= \"false\"\n").expect("should write config");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_package_folders_with_bool_value() {
    let root = temp_dir("config_package_folders_bool_rejected");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "package_folders #= true\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    let messages = parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect_err("config should fail");

    let diagnostic = first_error_diagnostic(&messages);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::UnsupportedPackageFoldersValue,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_package_folders_single_string() {
    let root = temp_dir("config_package_folders_single_string");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(&config_path, "package_folders #= \"lib\"\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("single-string package_folders should parse");

    assert_eq!(config.package_folders, vec![PathBuf::from("lib")]);
    assert!(config.has_explicit_package_folders);

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn accepts_config_local_reference_after_shape_enforcement() {
    let root = temp_dir("config_local_ref_after_shape");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "entry_root #= \"src\"\ndev_folder #= entry_root\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &config_path, &style_directives)
        .expect("config with local const reference should succeed");

    assert_eq!(
        config.entry_root,
        PathBuf::from("src"),
        "entry_root should be set"
    );
    assert_eq!(
        config.dev_folder,
        PathBuf::from("src"),
        "dev_folder should resolve through private const reference"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn detects_duplicate_top_level_config_constants() {
    let root = temp_dir("config_duplicate_top_level_constants");
    fs::create_dir_all(&root).expect("should create root dir");
    let config_path = root.join(settings::CONFIG_FILE_NAME);

    fs::write(
        &config_path,
        "entry_root #= \"other\"\nentry_root #= \"src\"\n",
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

// ── Canonical config identity tests ──────────────────────────────────────────

#[test]
fn authored_config_keeps_non_canonical_spelling_in_duplicate_diagnostic() {
    // The caller-provided config path spelling is preserved as the authored source-location
    // identity even when it is non-canonical. The resolver directory comes only from the
    // canonical config parent, while diagnostics keep the authored spelling.
    let root = temp_dir("config_non_canonical_spelling");
    fs::create_dir_all(&root).expect("should create root dir");
    fs::create_dir_all(root.join("sub")).expect("should create sub dir");
    let config_path = root.join("config.moth");
    fs::write(
        &config_path,
        "entry_root #= \"src\"\nentry_root #= \"other\"\n",
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn authored_config_resolver_uses_canonical_parent_for_noncanonical_spelling() {
    // A non-canonical config path that detours through a sibling directory must still derive
    // the resolver directory from the canonical config parent and apply the config value.
    let root = temp_dir("config_relative_parent_spelling");
    fs::create_dir_all(&root).expect("should create root dir");
    fs::create_dir_all(root.join("sub")).expect("should create sub dir");
    let config_path = root.join("config.moth");
    fs::write(&config_path, "entry_root #= \"src\"\n").expect("should write config");

    let non_canonical_config_path = root.join("sub").join("..").join("config.moth");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(&mut config, &non_canonical_config_path, &style_directives)
        .expect("non-canonical authored spelling should resolve and apply config");

    assert_eq!(config.entry_root, PathBuf::from("src"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn project_local_lib_directory_is_discovered_as_source_package_root() {
    let root = temp_dir("project_local_lib");
    fs::create_dir_all(&root).expect("should create root dir");
    fs::create_dir_all(root.join("lib/helper")).expect("should create lib/helper");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/helper/#mod.moth"), "foo #= 1\n").expect("should write root");
    fs::write(root.join("lib/helper/utils.moth"), "bar #= 2\n").expect("should write lib file");
    fs::write(root.join("config.moth"), "").expect("should write config");

    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();
    let resolver = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect("resolver should build");

    // Import path `@helper/utils` should resolve to the project-local lib root.
    let mut path = crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    path.push_str("helper", &mut string_table);
    path.push_str("utils", &mut string_table);

    let importer = root.join("src/#page.moth");
    let resolved = resolver
        .resolve_import_to_source_file(&path, &importer, &mut string_table)
        .expect("should resolve source-backed package import")
        .path;

    assert_eq!(
        resolved,
        fs::canonicalize(root.join("lib/helper/utils.moth")).unwrap(),
        "should resolve to project-local lib directory"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn package_prefix_collision_with_builder_package_rejected() {
    let root = temp_dir("lib_collision");
    fs::create_dir_all(&root).expect("should create root dir");
    fs::create_dir_all(root.join("lib/html")).expect("should create lib/html");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/html/#mod.moth"), "foo #= 1\n").expect("should write root");
    fs::write(root.join("config.moth"), "").expect("should write config");

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

    assert!(
        result.is_err(),
        "should fail when Builder and ProjectLocal package prefixes collide"
    );
    let messages = result.expect_err("checked above");
    let error_text = rendered_first_error(&messages);
    assert!(
        error_text.contains("collide") || error_text.contains("html"),
        "error should mention collision: {error_text}"
    );
    assert_has_config_error(&messages);
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourcePackageBuilderPrefixCollision { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn configured_package_folder_is_discovered_as_source_package_root() {
    let root = temp_dir("project_local_custom_package_folder");
    fs::create_dir_all(&root).expect("should create root dir");
    fs::create_dir_all(root.join("packages/helper")).expect("should create packages/helper");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("packages/helper/#mod.moth"), "foo #= 1\n").expect("should write root");
    fs::write(root.join("packages/helper/utils.moth"), "bar #= 2\n")
        .expect("should write lib file");
    fs::write(
        root.join("config.moth"),
        "package_folders #= { \"packages\" }\n",
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
    let resolver = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect("resolver should build");

    let mut path = crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    path.push_str("helper", &mut string_table);
    path.push_str("utils", &mut string_table);

    let importer = root.join("src/#page.moth");
    let resolved = resolver
        .resolve_import_to_source_file(&path, &importer, &mut string_table)
        .expect("should resolve source-backed package import")
        .path;

    assert_eq!(
        resolved,
        fs::canonicalize(root.join("packages/helper/utils.moth")).unwrap(),
        "should resolve to configured package folder"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn missing_explicit_package_folder_is_error() {
    let root = temp_dir("missing_explicit_package_folder");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(
        root.join("config.moth"),
        "package_folders #= { \"packages\" }\n",
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
        "missing explicitly configured package folder should fail"
    );
    let messages = result.expect_err("checked above");
    let error_text = rendered_first_error(&messages);
    assert!(
        error_text.contains("Configured package folder 'packages' does not exist"),
        "unexpected error message: {error_text}"
    );
    assert_has_config_error(&messages);
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::ConfiguredPackageFolderMissing { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn explicit_package_folder_must_be_directory() {
    let root = temp_dir("package_folder_not_directory");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("packages"), "").expect("should write file in place of folder");
    fs::write(
        root.join("config.moth"),
        "package_folders #= { \"packages\" }\n",
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

    let messages = result.expect_err("package scan root file should fail");
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::ConfiguredPackageFolderNotDirectory { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn source_package_requires_one_generic_hash_root() {
    let root = temp_dir("source_package_missing_root");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::create_dir_all(root.join("lib/helper")).expect("should create lib/helper");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

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

    let messages = result.expect_err("source-backed package without a hash root should fail");
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourcePackageMissingRoot { .. }
    ));
    let error_text = rendered_first_error(&messages);
    assert!(error_text.contains("#*.moth"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn source_package_accepts_cosmetic_hash_root_name() {
    let root = temp_dir("source_package_cosmetic_root");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::create_dir_all(root.join("lib/helper")).expect("should create lib/helper");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/helper/#package.moth"), "foo #= 1\n")
        .expect("should write cosmetic root");
    fs::write(root.join("lib/helper/utils.moth"), "bar #= 2\n")
        .expect("should write package source");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let resolver = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect("cosmetic source-backed package root should pass Stage 0 preflight");

    let mut path = crate::compiler_frontend::symbols::interned_path::InternedPath::new();
    path.push_str("helper", &mut string_table);
    let importer = root.join("src/#page.moth");
    let resolved = resolver
        .resolve_import_to_source_file_with_public_surface_fallback(
            &path,
            &importer,
            &mut string_table,
        )
        .expect("source-backed package folder import should resolve through the root pipeline");

    assert_eq!(
        resolved.path,
        fs::canonicalize(root.join("lib/helper/#package.moth")).unwrap()
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn source_package_rejects_multiple_generic_hash_roots() {
    let root = temp_dir("source_package_multiple_roots");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::create_dir_all(root.join("lib/helper")).expect("should create lib/helper");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/helper/#first.moth"), "foo #= 1\n").expect("should write first root");
    fs::write(root.join("lib/helper/#second.moth"), "bar #= 2\n")
        .expect("should write second root");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

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

    let messages = result.expect_err("multiple source-backed package roots should fail preflight");
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourcePackageMultipleRoots { .. }
    ));
    let error_text = rendered_first_error(&messages);
    assert!(error_text.contains("#first.moth"));
    assert!(error_text.contains("#second.moth"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn package_prefix_collision_across_scan_roots_rejected() {
    let root = temp_dir("duplicate_package_prefixes");
    fs::create_dir_all(root.join("lib/helper")).expect("should create lib/helper");
    fs::create_dir_all(root.join("vendor/helper")).expect("should create vendor/helper");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/helper/#mod.moth"), "foo #= 1\n").expect("should write root");
    fs::write(root.join("vendor/helper/#mod.moth"), "bar #= 2\n").expect("should write root");
    fs::write(
        root.join("config.moth"),
        "package_folders #= { \"lib\", \"vendor\" }\n",
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
        "same source-backed package prefix discovered from two configured folders should fail"
    );
    let messages = result.expect_err("checked above");
    let error_text = rendered_first_error(&messages);
    assert!(
        error_text.contains("Configured package folder collision"),
        "unexpected error message: {error_text}"
    );
    assert_has_config_error(&messages);
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourcePackagePrefixCollision { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn entry_root_requires_at_least_one_root_entry_file() {
    let root = temp_dir("entry_root_without_entries");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

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
        panic!("entry root without #*.moth entries should fail");
    };

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::NoRootModuleEntries { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

// ── Phase 4 project-structure collision tests ─────────────────────────────────

#[test]
fn rejects_moth_file_and_folder_collision_in_same_directory() {
    let root = temp_dir("moth_folder_collision");
    fs::create_dir_all(root.join("src/UI")).expect("should create src/UI");
    fs::write(root.join("src/UI/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("src/ui.moth"), "y ~= 2\n").expect("should write colliding file");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_template_file_and_folder_collision_in_same_directory() {
    let root = temp_dir("template_folder_collision");
    fs::create_dir_all(root.join("src/ui")).expect("should create src/ui");
    fs::write(root.join("src/ui/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("src/ui.mtf"), "template\n").expect("should write colliding file");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn allows_same_stem_in_different_directories() {
    let root = temp_dir("same_stem_different_dirs");
    fs::create_dir_all(root.join("src/components")).expect("should create src/components");
    fs::create_dir_all(root.join("src/pages")).expect("should create src/pages");
    fs::write(root.join("src/components/card.moth"), "x ~= 1\n").expect("should write card");
    fs::write(root.join("src/pages/card.moth"), "y ~= 2\n").expect("should write another card");
    fs::write(root.join("src/#page.moth"), "z ~= 3\n").expect("should write entry");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_collision_with_empty_folder() {
    let root = temp_dir("collision_empty_folder");
    fs::create_dir_all(root.join("src/helper")).expect("should create src/helper");
    fs::write(root.join("src/helper.moth"), "x ~= 1\n").expect("should write colliding file");
    fs::write(root.join("src/#page.moth"), "y ~= 2\n").expect("should write entry");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn js_file_with_same_stem_as_folder_does_not_trigger_collision() {
    let root = temp_dir("js_same_stem_no_collision");
    fs::create_dir_all(root.join("src/helper")).expect("should create src/helper");
    fs::write(root.join("src/helper.js"), "// js\n").expect("should write js file");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn rejects_moth_file_and_folder_collision_in_source_package() {
    let root = temp_dir("source_package_moth_folder_collision");
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::create_dir_all(root.join("lib/helper/ui")).expect("should create lib/helper/ui");
    fs::write(root.join("src/#page.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("lib/helper/#mod.moth"), "value #= 1\n").expect("should write root");
    fs::write(root.join("lib/helper/ui.moth"), "value #= 2\n")
        .expect("should write colliding package file");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\npackage_folders #= { \"lib\" }\n",
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
        "source-backed package ui.moth + ui/ collision should be rejected"
    );
    let messages = result.expect_err("checked above");
    assert_has_config_error(&messages);
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourceFileFolderCollision { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn unsupported_js_import_without_provider_reports_moth_import_0021() {
    let root = temp_dir("unsupported_js_import");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    // Entry file imports a .js file explicitly.
    fs::write(src.join("#page.moth"), "import @./drawing.js\n#[:ok]\n")
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
        assert_eq!(path_text, "./drawing.js", "unexpected path in diagnostic");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn explicit_moth_extension_still_reports_moth_import_0020() {
    let root = temp_dir("explicit_moth_extension");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "import @helper.moth\n#[:ok]\n").expect("should write entry");

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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn unsupported_moth_template_import_without_builder_support_reports_moth_import_0025() {
    let root = temp_dir("unsupported_moth_template_import");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "import @intro\n#[:ok]\n").expect("should write entry");
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
        Ok(_) => panic!("unsupported .mtf import should fail discovery"),
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn direct_moth_template_extension_import_reports_moth_import_0024() {
    let root = temp_dir("direct_moth_template_extension");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "import @intro.mtf\n#[:ok]\n").expect("should write entry");
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
        Ok(_) => panic!("direct .mtf import should fail discovery"),
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn moth_template_files_are_reachable_without_import_scanning() {
    let root = temp_dir("moth_template_no_import_scanning");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "import @intro\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.mtf"), "import @missing\n").expect("should write moth template file");

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
        .expect(".mtf body text must not be scanned for imports");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    let input_paths: HashSet<_> = modules[0]
        .input_files
        .iter()
        .map(|input| input.source_path().file_name().unwrap().to_owned())
        .collect();
    assert!(input_paths.contains(OsStr::new("#page.moth")));
    assert!(input_paths.contains(OsStr::new("intro.mtf")));

    let moth_template_input = modules[0]
        .input_files
        .iter()
        .find(|input| input.source_path().file_name() == Some(OsStr::new("intro.mtf")))
        .expect("intro.mtf should be in discovered inputs");
    assert!(matches!(
        moth_template_input,
        PreparedSourceInput::MothTemplate { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn reachable_moth_template_queues_same_directory_root_file() {
    let root = temp_dir("moth_template_same_directory_root");
    let src = root.join("src");
    let docs = src.join("docs");
    fs::create_dir_all(&docs).expect("should create docs dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "import @docs\n#[:ok]\n").expect("should write entry");
    fs::write(docs.join("intro.mtf"), "hello\n").expect("should write moth template file");
    fs::write(
        docs.join("#docs.moth"),
        "import @intro\ntitle #= \"Docs\"\n",
    )
    .expect("should write root");

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
        .expect("reachable .mtf should discover same-directory hash root");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    // The entry module imports a Moth template file from the `docs` module root, so `docs` is its
    // provider and precedes it in the returned inventory order. Find the entry module by its
    // root file rather than assuming index 0.
    let entry_module = modules
        .iter()
        .find(|module| {
            module
                .entry_point
                .file_name()
                .is_some_and(|name| name == "#page.moth")
        })
        .expect("entry module should be discovered");
    let input_paths: HashSet<_> = entry_module
        .input_files
        .iter()
        .map(|input| input.source_path().file_name().unwrap().to_owned())
        .collect();
    assert!(input_paths.contains(OsStr::new("#page.moth")));
    assert!(!input_paths.contains(OsStr::new("intro.mtf")));
    assert!(!input_paths.contains(OsStr::new("#docs.moth")));

    let docs_module = modules
        .iter()
        .find(|module| {
            module
                .entry_point
                .file_name()
                .is_some_and(|name| name == "#docs.moth")
        })
        .expect("docs provider module should be discovered");

    let moth_template_input = docs_module
        .input_files
        .iter()
        .find(|input| input.source_path().file_name() == Some(OsStr::new("intro.mtf")))
        .expect("intro.mtf should be in discovered inputs");
    assert!(matches!(
        moth_template_input,
        PreparedSourceInput::MothTemplate { .. }
    ));

    let root_input = docs_module
        .input_files
        .iter()
        .find(|input| input.source_path().file_name() == Some(OsStr::new("#docs.moth")))
        .expect("#docs.moth should be in discovered inputs");
    assert!(matches!(root_input, PreparedSourceInput::Moth { .. }));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn unimported_moth_template_file_under_entry_root_is_ignored() {
    let root = temp_dir("unimported_moth_template_ignored");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "#[:ok]\n").expect("should write entry");
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
        .expect("unimported .mtf file should not affect discovery");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(modules[0].input_files.len(), 1);
    assert_eq!(
        modules[0].input_files[0].source_path().file_name().unwrap(),
        OsStr::new("#page.moth")
    );
    assert!(matches!(
        modules[0].input_files[0],
        PreparedSourceInput::Moth { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn extensionless_moth_import_and_virtual_package_import_still_work() {
    let root = temp_dir("extensionless_and_virtual");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    // Normal extensionless imports still resolve as Moth source files, while virtual package
    // imports continue to stay out of Stage 0 filesystem traversal.
    fs::write(
        src.join("#page.moth"),
        "import @helper\nimport @core/io { line }\n#[:ok]\n",
    )
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

    let discovered = modules[0]
        .input_files
        .iter()
        .map(|file| {
            file.source_path()
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect::<HashSet<_>>();

    assert!(discovered.contains("#page.moth"));
    assert!(discovered.contains("helper.moth"));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn reachable_file_discovery_markdown_files_are_reachable_without_import_scanning() {
    let root = temp_dir("markdown_no_import_scanning");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "import @intro\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "import @missing\n").expect("should write markdown file");

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
        .expect(".md body text must not be scanned for imports");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    let input_paths: HashSet<_> = modules[0]
        .input_files
        .iter()
        .map(|input| input.source_path().file_name().unwrap().to_owned())
        .collect();
    assert!(input_paths.contains(OsStr::new("#page.moth")));
    assert!(input_paths.contains(OsStr::new("intro.md")));

    let markdown_input = modules[0]
        .input_files
        .iter()
        .find(|input| input.source_path().file_name() == Some(OsStr::new("intro.md")))
        .expect("intro.md should be in discovered inputs");
    assert!(matches!(
        markdown_input,
        PreparedSourceInput::PlainMarkdown { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn reachable_file_discovery_markdown_does_not_queue_unrelated_module_root_file() {
    let root = temp_dir("markdown_no_unrelated_module_root");
    let src = root.join("src");
    fs::create_dir_all(src.join("other")).expect("should create other module dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "import @intro\n#[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "hello\n").expect("should write markdown file");
    fs::write(src.join("other/#other.moth"), "export:\n    x #= 1\n;\n")
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

    let input_paths: HashSet<_> = modules[0]
        .input_files
        .iter()
        .map(|input| input.source_path().file_name().unwrap().to_owned())
        .collect();
    assert!(input_paths.contains(OsStr::new("#page.moth")));
    assert!(input_paths.contains(OsStr::new("intro.md")));
    assert!(!input_paths.contains(OsStr::new("#other.moth")));

    let markdown_input = modules[0]
        .input_files
        .iter()
        .find(|input| input.source_path().file_name() == Some(OsStr::new("intro.md")))
        .expect("intro.md should be in discovered inputs");
    assert!(matches!(
        markdown_input,
        PreparedSourceInput::PlainMarkdown { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn reachable_file_discovery_unimported_markdown_file_is_ignored() {
    let root = temp_dir("unimported_markdown_ignored");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "#[:ok]\n").expect("should write entry");
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
        .expect("unimported .md file should not affect discovery");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(modules[0].input_files.len(), 1);
    assert_eq!(
        modules[0].input_files[0].source_path().file_name().unwrap(),
        OsStr::new("#page.moth")
    );
    assert!(matches!(
        modules[0].input_files[0],
        PreparedSourceInput::Moth { .. }
    ));

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn reachable_file_discovery_direct_markdown_extension_import_reports_moth_import_0024() {
    let root = temp_dir("direct_markdown_extension");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "import @intro.md\n#[:ok]\n").expect("should write entry");
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
        Ok(_) => panic!("direct .md import should fail discovery"),
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
            "unexpected import path in explicit source extension diagnostic"
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn reachable_file_discovery_unsupported_markdown_import_reports_moth_import_0025() {
    let root = temp_dir("unsupported_markdown_import");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("#page.moth"), "import @intro\n#[:ok]\n").expect("should write entry");
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
        Ok(_) => panic!("unsupported .md import should fail discovery"),
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
            "unexpected import path in unsupported source file kind diagnostic"
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn stage0_reuses_scanned_moth_source_when_assembling_input_files() {
    let root = temp_dir("stage0_reuses_scanned_moth_source");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "import @helper\n#[:entry]\n").expect("should write entry");
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

    let _counter_guard = SOURCE_READ_COUNTER_TEST_LOCK
        .lock()
        .expect("source read counter test lock poisoned");
    let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);
    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(modules.len(), 1);
    assert_eq!(
        super::source_loading::source_read_count_for_test(),
        2,
        "entry and helper .moth files should each be read once during import scanning"
    );
    assert_eq!(modules[0].input_files.len(), 2);
    assert!(
        modules[0]
            .input_files
            .iter()
            .any(|input| input.source_code().contains("#[:entry]"))
    );
    assert!(
        modules[0]
            .input_files
            .iter()
            .any(|input| input.source_code().contains("message #="))
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn project_prepared_source_store_transitions_one_dense_source_slot_once() {
    let root = temp_dir("project_prepared_source_store_dense_slot");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "value #= 1\n").expect("should write entry");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let project_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let entry_root = fs::canonicalize(&src).expect("entry root should canonicalize");
    let entry_path = fs::canonicalize(src.join("#page.moth")).expect("entry should canonicalize");
    let mut string_table = StringTable::new();
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        &project_root,
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("source tree index should build");
    let source_id = source_tree_index
        .source_id_for_canonical_path(&entry_path)
        .expect("entry should have a dense source ID");
    let mut store = PreparedSourceStore::new(source_tree_index.source_count());

    let Ok((_, first_origin)) = store.prepare_or_get_project_moth_imports(
        source_id,
        &entry_path,
        &style_directives,
        &mut string_table,
    ) else {
        panic!("first preparation should succeed");
    };
    let Ok((_, second_origin)) = store.prepare_or_get_project_moth_imports(
        source_id,
        &entry_path,
        &style_directives,
        &mut string_table,
    ) else {
        panic!("second preparation should reuse the same slot");
    };

    assert!(matches!(first_origin, MothScanOrigin::FreshRead { .. }));
    assert_eq!(second_origin, MothScanOrigin::ReusedFromStore);

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn stage0_loads_asset_sources_and_preserves_deterministic_input_order() {
    let root = temp_dir("stage0_asset_source_loading_order");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @intro\nimport @notes\n#[:entry]\n",
    )
    .expect("should write entry");
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
    let input_files = &modules[0].input_files;
    let input_names = input_files
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

    assert_eq!(input_names, vec!["#page.moth", "intro.mtf", "notes.md"]);
    assert!(matches!(
        input_files[1],
        PreparedSourceInput::MothTemplate { .. }
    ));
    assert_eq!(input_files[1].source_code(), "moth template body\n");
    assert!(matches!(
        input_files[2],
        PreparedSourceInput::PlainMarkdown { .. }
    ));
    assert_eq!(input_files[2].source_code(), "# Markdown body\n");

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn stage0_parallel_missing_source_loading_preserves_input_order() {
    let root = temp_dir("stage0_parallel_missing_source_order");
    fs::create_dir_all(&root).expect("should create root dir");

    let source_paths = (0..super::reachable_file_discovery::STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES)
        .map(|index| {
            let path = root.join(format!("asset_{index}.md"));
            fs::write(&path, format!("# Asset {index}\n")).expect("should write markdown asset");
            path
        })
        .collect::<Vec<_>>();
    let mut string_table = StringTable::new();

    let input_files = super::reachable_file_discovery::load_missing_source_paths_for_test(
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
    let expected_names = (0
        ..super::reachable_file_discovery::STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES)
        .map(|index| format!("asset_{index}.md"))
        .collect::<Vec<_>>();

    assert_eq!(loaded_names, expected_names);
    for (index, input_file) in input_files.iter().enumerate() {
        assert_eq!(input_file.source_code(), format!("# Asset {index}\n"));
        assert!(matches!(
            input_file,
            PreparedSourceInput::PlainMarkdown { .. }
        ));
    }

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn stage0_missing_source_load_preserves_file_error_shape() {
    let root = temp_dir("stage0_missing_source_load_error");
    fs::create_dir_all(&root).expect("should create root dir");
    let missing_source = root.join("missing.md");
    let mut string_table = StringTable::new();

    let messages = super::reachable_file_discovery::load_missing_source_path_for_test(
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn provider_backed_imports_are_resolved_without_becoming_source_inputs() {
    let root = temp_dir("provider_imports_not_source_inputs");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "import @./drawing.js\n#[:entry]\n")
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
    assert_eq!(modules[0].input_files.len(), 1);
    assert_eq!(
        modules[0].input_files[0].source_path().file_name().unwrap(),
        OsStr::new("#page.moth")
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn provider_free_multi_entry_discovery_is_deterministic_and_uses_parallel_path() {
    let root = temp_dir("provider_free_multi_entry_deterministic");
    let src = root.join("src");
    fs::create_dir_all(src.join("page_a")).expect("should create page_a module");
    fs::create_dir_all(src.join("page_b")).expect("should create page_b module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    // Two entry points with independent module-local dependency trees.
    fs::write(
        src.join("page_a/#pageA.moth"),
        "import @helper\nimport @a_only\n#[:pageA]\n",
    )
    .expect("should write pageA");
    fs::write(
        src.join("page_b/#pageB.moth"),
        "import @helper\nimport @b_only\n#[:pageB]\n",
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

    let _counter_guard = SOURCE_READ_COUNTER_TEST_LOCK
        .lock()
        .expect("source read counter test lock poisoned");
    let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("provider-free multi-entry discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(
        super::source_loading::source_read_count_for_test(),
        6,
        "provider-free classification should read each unique Moth source once and share the source cache with module discovery"
    );
    assert_eq!(modules.len(), 2, "expected two discovered modules");

    // Module order must follow deterministic entry-point order.
    let module_names: Vec<_> = modules
        .iter()
        .map(|module| {
            module
                .entry_point
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert_eq!(module_names, vec!["#pageA.moth", "#pageB.moth"]);

    // Per-module input order must be deterministic.
    let module_a_inputs = modules[0]
        .input_files
        .iter()
        .map(|input| {
            input
                .source_path()
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();
    let module_b_inputs = modules[1]
        .input_files
        .iter()
        .map(|input| {
            input
                .source_path()
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();

    // Reachable files are collected into a `BTreeSet`, so per-module order is deterministic by
    // canonical path (file name within this test).
    assert_eq!(
        module_a_inputs,
        vec!["#pageA.moth", "a_only.moth", "helper.moth"]
    );
    assert_eq!(
        module_b_inputs,
        vec!["#pageB.moth", "b_only.moth", "helper.moth"]
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn provider_backed_import_in_multi_entry_falls_back_to_serial_and_calls_provider() {
    let root = temp_dir("provider_backed_multi_entry_fallback");
    let src = root.join("src");
    fs::create_dir_all(src.join("page_a")).expect("should create page_a module");
    fs::create_dir_all(src.join("page_b")).expect("should create page_b module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    // Entry A is plain provider-free; entry B imports a .js file.
    fs::write(src.join("page_a/#pageA.moth"), "a #= 1\n").expect("should write pageA");
    fs::write(
        src.join("page_b/#pageB.moth"),
        "import @./drawing.js\n#[:pageB]\n",
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
            .expect("provider-backed multi-entry discovery should fall back and succeed");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "provider should be called once"
    );
    assert_eq!(modules.len(), 2);

    // Module A has its own input; module B should only contain the Moth entry, not the .js.
    assert_eq!(modules[0].input_files.len(), 1);
    assert_eq!(modules[1].input_files.len(), 1);
    assert_eq!(
        modules[1].input_files[0].source_path().file_name().unwrap(),
        OsStr::new("#pageB.moth")
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn provider_required_replay_reuses_classification_cache_without_retokenizing() {
    let root = temp_dir("provider_required_replay_cache");
    let src = root.join("src");
    fs::create_dir_all(src.join("page_a")).expect("should create page_a module");
    fs::create_dir_all(src.join("page_b")).expect("should create page_b module");
    fs::create_dir_all(src.join("shared")).expect("should create shared dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    // Entry A is provider-free; entry B imports a .js file backed by a registered provider.
    fs::write(
        src.join("page_a/#pageA.moth"),
        "import @helper\n#[:pageA]\n",
    )
    .expect("should write pageA");
    fs::write(src.join("page_a/helper.moth"), "helper #= 1\n")
        .expect("should write module-local helper");
    fs::write(
        src.join("page_b/#pageB.moth"),
        "import @./drawing.js\n#[:pageB]\n",
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

    let _counter_guard = SOURCE_READ_COUNTER_TEST_LOCK
        .lock()
        .expect("source read counter test lock poisoned");
    let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);

    let modules =
        discover_modules_for_test_with_providers(&config, &resolver, &style_directives, &providers)
            .expect("provider-required replay should succeed");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    // Three unique Moth sources: #pageA.moth, shared/helper.moth, and #pageB.moth.
    // Classification reads each once while completing the full local traversal. The serial
    // provider-capable replay must reuse that retained cache, so each .moth is read exactly once
    // and never re-tokenized.
    assert_eq!(
        super::source_loading::source_read_count_for_test(),
        3,
        "provider-required replay should reuse the classification cache without re-reading .moth files"
    );

    // The provider-backed import is handled exactly once during serial replay.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "provider should be called once during replay"
    );
    assert_eq!(modules.len(), 2);

    // Every Moth input carries its retained Stage 0 token stream by type, proving the
    // replayed inputs consumed retained tokens rather than a second scan path.
    for module in &modules {
        for input in &module.input_files {
            match input {
                PreparedSourceInput::Moth {
                    source_path,
                    tokens,
                    ..
                } => {
                    assert!(
                        !tokens.tokens.is_empty(),
                        "replayed Moth file {:?} should carry retained tokens",
                        source_path.file_name(),
                    );
                }
                _ => panic!("provider-required replay should only produce Moth files"),
            }
        }
    }

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn unsupported_external_extension_in_multi_entry_preserves_diagnostic_shape() {
    let root = temp_dir("unsupported_extension_multi_entry");
    let src = root.join("src");
    fs::create_dir_all(src.join("page_a")).expect("should create page_a module");
    fs::create_dir_all(src.join("page_b")).expect("should create page_b module");
    fs::create_dir_all(src.join("shared")).expect("should create shared dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    fs::write(src.join("page_a/#pageA.moth"), "a #= 1\n").expect("should write pageA");
    fs::write(
        src.join("page_b/#pageB.moth"),
        "import @./drawing.js\n#[:pageB]\n",
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
        assert_eq!(path_text, "./drawing.js", "unexpected path in diagnostic");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn directory_provider_import_calls_provider_once_for_repeated_physical_source() {
    let root = temp_dir("provider_exact_once_repeated_source");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    // The entry imports the same .js file twice under distinct local aliases, so the physical
    // provider source is reached twice during one traversal.
    fs::write(
        src.join("#page.moth"),
        "import @./drawing.js\nimport @./drawing.js as drawing2\n#[:entry]\n",
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

    // The provider runs exactly once for one physical provider source, even though two importers
    // reach it during the traversal. The cache key is the indexed canonical path, so the second
    // reach reuses the first result.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "provider must run exactly once for a repeated physical provider source"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn directory_provider_import_rejects_cross_module_target() {
    let root = temp_dir("provider_cross_module_rejected");
    let src = root.join("src");
    let feature = src.join("feature");
    fs::create_dir_all(&feature).expect("should create feature module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @feature/private.js\n#[:entry]\n",
    )
    .expect("should write entry importing a cross-module provider file");
    fs::write(feature.join("#mod.moth"), "").expect("should write feature root");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn directory_provider_import_missing_target_reports_structured_diagnostic_without_path_probe() {
    let root = temp_dir("provider_missing_target_no_probe");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    // The imported .js file does not exist on disk, so it is absent from the source tree index.
    fs::write(src.join("#page.moth"), "import @./missing.js\n#[:entry]\n")
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn provider_free_parallel_preserves_cross_module_root_queuing() {
    let root = temp_dir("provider_free_cross_module_root");
    let src = root.join("src");
    let module_a = src.join("module_a");
    let module_b = module_a.join("module_b");
    fs::create_dir_all(&module_a).expect("should create module_a");
    fs::create_dir_all(&module_b).expect("should create module_b");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    // Entry A imports its direct child module B, which should queue module B's root.
    fs::write(
        module_a.join("#pageA.moth"),
        "import @module_b { b }\n#[:pageA]\n",
    )
    .expect("should write pageA");
    fs::write(module_b.join("#api.moth"), "export:\n    b #= 1\n;\n")
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

    // Module A imports module B's public facade, so module B precedes module A. The still-live
    // donor compiler receives only the provider root that exposes the facade, while module B
    // owns its complete semantic source set.
    let module_a = modules
        .iter()
        .find(|module| {
            module
                .entry_point
                .file_name()
                .is_some_and(|name| name == "#pageA.moth")
        })
        .expect("module A should be discovered");
    let module_b = modules
        .iter()
        .find(|module| {
            module
                .entry_point
                .file_name()
                .is_some_and(|name| name == "#api.moth")
        })
        .expect("module B should be discovered");
    let module_a_inputs = module_a
        .input_files
        .iter()
        .map(|input| {
            input
                .source_path()
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();
    let module_b_inputs = module_b
        .input_files
        .iter()
        .map(|input| {
            input
                .source_path()
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();

    assert!(
        !module_a_inputs.contains(&"#api.moth".to_string())
            && !module_a_inputs.contains(&"impl.moth".to_string()),
        "the consumer inventory must exclude all provider source files"
    );
    assert!(
        module_b_inputs.contains(&"#api.moth".to_string())
            && !module_b_inputs.contains(&"impl.moth".to_string()),
        "the queued provider module must retain its root without making an unreferenced private file semantic"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn scoped_support_package_is_visible_by_name_to_owner_and_sibling_descendant() {
    let root = temp_dir("indexed_namespace_support_visibility");
    let src = root.join("src");
    let support = src.join("markdown");
    let pages = src.join("pages");
    fs::create_dir_all(&support).expect("should create support package");
    fs::create_dir_all(&pages).expect("should create sibling module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#site.moth"),
        "import @markdown { render }\n#[:site]\n",
    )
    .expect("should write owner root");
    fs::write(
        support.join("+package.moth"),
        "export:\n    render || -> String:\n        return \"support\"\n    ;\n;\n",
    )
    .expect("should write support root");
    fs::write(
        pages.join("#page.moth"),
        "import @markdown { render }\n#[:page]\n",
    )
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn recognized_source_stem_collision_is_ambiguous_without_extension_precedence() {
    let root = temp_dir("indexed_namespace_source_stem_collision");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source root");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "import @HELPER\n#[:page]\n").expect("should write root");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn binding_package_and_local_module_prefix_collision_is_ambiguous() {
    let root = temp_dir("indexed_namespace_binding_package_collision");
    let src = root.join("src");
    let local_core = src.join("core");
    fs::create_dir_all(&local_core).expect("should create local core module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @core/io { input }\n#[:page]\n",
    )
    .expect("should write root");
    fs::write(local_core.join("#core.moth"), "#[:local core]\n")
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn directory_source_import_rejects_obsolete_relative_form() {
    let root = temp_dir("indexed_namespace_relative_import_rejected");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source root");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @./helper { value }\n#[:page]\n",
    )
    .expect("should write root");
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
        Ok(_) => panic!("directory source imports must reject @./"),
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-IMPORT-0007"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn direct_child_private_path_bypass_is_rejected() {
    let root = temp_dir("indexed_namespace_child_private_bypass");
    let src = root.join("src");
    let child = src.join("child");
    fs::create_dir_all(&child).expect("should create child module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @child/private { value }\n#[:page]\n",
    )
    .expect("should write parent root");
    fs::write(child.join("#api.moth"), "export:\n    value #= 1\n;\n")
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn stage0_retains_moth_tokens_and_leaves_non_tokenized_sources_without_tokens() {
    let root = temp_dir("stage0_retained_tokens");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @helper\nimport @intro\n#[:entry]\n",
    )
    .expect("should write entry");
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
        .expect("retained-token discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    let input_files = &modules[0].input_files;

    // Every Moth input carries the retained Stage 0 token stream by type; Moth template and
    // PlainMarkdown variants cannot carry tokens, so the invalid state is unrepresentable.
    for input in input_files.iter() {
        match input {
            PreparedSourceInput::Moth {
                source_path,
                tokens,
                ..
            } => {
                assert!(
                    !tokens.tokens.is_empty(),
                    "retained token stream for {:?} should not be empty",
                    source_path.file_name(),
                );
            }
            PreparedSourceInput::MothTemplate { source_path, .. }
            | PreparedSourceInput::PlainMarkdown { source_path, .. } => {
                // Non-Moth sources have no retained token stream by construction.
                let _ = source_path;
            }
        }
    }

    // The retained Moth token for the entry file should contain the import path token,
    // proving the Stage 0 lexical pass produced the tokens that header parsing will consume.
    let entry_input = input_files
        .iter()
        .find(|input| match input {
            PreparedSourceInput::Moth { source_path, .. } => source_path
                .file_name()
                .is_some_and(|name| name == "#page.moth"),
            _ => false,
        })
        .expect("entry file should be in the input set");
    let entry_tokens = match entry_input {
        PreparedSourceInput::Moth { tokens, .. } => tokens,
        _ => unreachable!("entry file should be a Moth prepared input"),
    };
    assert!(
        entry_tokens.tokens.iter().any(|token| matches!(
            token.kind,
            crate::compiler_frontend::tokenizer::tokens::TokenKind::Import
        )),
        "retained entry tokens should contain the Import token from Stage 0 lexing"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn provider_free_parallel_retains_moth_tokens_for_every_reachable_file() {
    let root = temp_dir("provider_free_retained_tokens");
    let src = root.join("src");
    let module_a = src.join("module_a");
    let module_b = src.join("module_b");
    fs::create_dir_all(&module_a).expect("should create module_a");
    fs::create_dir_all(&module_b).expect("should create module_b");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");

    // Two entry points exercise the provider-free parallel path. Each module imports a helper.
    fs::write(module_a.join("#pageA.moth"), "import @helperA\n#[:pageA]\n")
        .expect("should write pageA");
    fs::write(module_a.join("helperA.moth"), "a #= 1\n").expect("should write helperA");
    fs::write(module_b.join("#pageB.moth"), "import @helperB\n#[:pageB]\n")
        .expect("should write pageB");
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
        .expect("provider-free parallel discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(modules.len(), 2);

    // Every Moth input in both modules must carry retained Stage 0 tokens by type,
    // proving the provider-free parallel path reuses classification tokens without re-tokenizing.
    for module in &modules {
        for input in &module.input_files {
            match input {
                PreparedSourceInput::Moth {
                    source_path,
                    tokens,
                    ..
                } => {
                    assert!(
                        !tokens.tokens.is_empty(),
                        "retained tokens for {:?} should not be empty",
                        source_path.file_name(),
                    );
                }
                _ => panic!("test should only produce Moth files"),
            }
        }
    }

    fs::remove_dir_all(&root).expect("should remove temp root");
}

// -------------------------
//  Phase 5b: graph-resolved local provider edges
// -------------------------

/// Write a two-module project where `module_a` imports its direct child `module_b`, plus the
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
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        module_a.join("#pageA.moth"),
        "import @module_b\n#[:pageA]\n",
    )
    .expect("should write pageA");
    fs::write(module_b.join("#api.moth"), "export:\n    b #= 1\n;\n")
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
    let root = temp_dir("phase5b_provider_before_consumer");
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

    // The import flows module_a -> module_b, so the provider (module_b) must precede the
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
            wave.iter()
                .any(|module| module.entry_point.file_name() == Some(OsStr::new("#api.moth")))
        })
        .expect("module_b should appear in a compile wave");
    let consumer_wave = waves
        .iter()
        .position(|wave| {
            wave.iter()
                .any(|module| module.entry_point.file_name() == Some(OsStr::new("#pageA.moth")))
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn same_module_import_creates_no_project_graph_edge() {
    let root = temp_dir("phase5b_same_module_no_edge");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    // The single entry module imports a sibling file inside its own module root.
    fs::write(src.join("#page.moth"), "import @helper\n#[:page]\n").expect("should write entry");
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

    // Same-module imports create no project-graph edge, so the graph has no edges and one wave.
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn independent_no_edge_entries_are_grouped_in_one_ready_wave() {
    // Two entry modules with no cross-module dependency edges must be grouped in the same
    // compile wave so the directory compiler can compile them in parallel within that wave.
    let root = temp_dir("phase5c_no_edge_same_wave");
    let src = root.join("src");
    let module_a = src.join("module_a");
    let module_b = src.join("module_b");
    fs::create_dir_all(&module_a).expect("should create module_a");
    fs::create_dir_all(&module_b).expect("should create module_b");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    // Two independent entry modules with no cross-module imports.
    fs::write(module_a.join("#pageA.moth"), "#[:pageA]\n").expect("should write pageA");
    fs::write(module_b.join("#pageB.moth"), "#[:pageB]\n").expect("should write pageB");

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
            module
                .entry_point
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn duplicate_import_deduplicates_edge_and_orders_provider_first() {
    let root = temp_dir("phase5b_duplicate_edge");
    let src = root.join("src");
    let module_a = src.join("module_a");
    let module_b = module_a.join("module_b");
    fs::create_dir_all(&module_a).expect("should create module_a");
    fs::create_dir_all(&module_b).expect("should create module_b");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    // The consumer imports its direct child twice, so the duplicate observation must be
    // idempotent.
    fs::write(
        module_a.join("#pageA.moth"),
        "import @module_b\nimport @module_b\n#[:pageA]\n",
    )
    .expect("should write pageA");
    fs::write(module_b.join("#api.moth"), "export:\n    b #= 1\n;\n")
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
        inventory_waves[0][0]
            .entry_point
            .file_name()
            .is_some_and(|name| name == "#api.moth"),
        "module_b is the provider in the first wave"
    );
    assert_eq!(
        inventory_waves[1].len(),
        1,
        "the consumer is the sole entry in the second wave"
    );
    assert!(
        inventory_waves[1][0]
            .entry_point
            .file_name()
            .is_some_and(|name| name == "#pageA.moth"),
        "module_a is the consumer in the second wave"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn dependency_fact_retains_authored_source_location() {
    let root = temp_dir("phase5b_source_location_retention");
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

    // The retained scope is the importer file that authored the structural provider reference.
    let scope_path = retained_location.scope.to_portable_string(&string_table);
    assert!(
        scope_path.contains("#pageA.moth"),
        "retained location scope should name the importing module root file: {scope_path}"
    );
    // The import clause is on the first source line.
    assert_eq!(
        retained_location.start_pos.line_number, 0,
        "retained location should point at the first authored source line"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn production_graph_completes_before_scheduling() {
    // Hidden invariant: the production discovery path completes the project module graph before
    // compile-wave scheduling, freezing adjacency into sorted `Vec<ModuleId>` storage. The
    // completed graph schedules cleanly from its frozen adjacency, and any later edge insertion
    // is rejected as mutation after completion.
    let root = temp_dir("r4e1_production_completion");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn discovered_modules_carry_both_graph_assigned_identities() {
    // Hidden invariant: directory discovery must preserve both graph identities rather than
    // re-deriving either from an entry path. The dense ID remains the build-owned scheduling and
    // merge key; the stable origin remains the portable semantic identity.
    let root = temp_dir("phase7a_origin_preservation");
    let (config, resolver, style_directives, _module_a_root, _module_b_root) =
        write_cross_module_project(&root);

    let (modules, graph, _source_tree_index, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    assert!(
        modules.waves().iter().map(|wave| wave.len()).sum::<usize>() > 0,
        "the cross-module project should discover at least one normal entry module"
    );

    for module in modules.waves().iter().flatten() {
        let matching_node = graph
            .nodes()
            .iter()
            .find(|node| node.root_file() == module.entry_point);
        let matching_node = matching_node.expect(
            "every discovered module entry point must match a graph node's canonical root file",
        );
        assert_eq!(
            module.module_id,
            matching_node.module_id(),
            "discovered module ID must equal its graph-assigned dense identity (entry {:?})",
            module.entry_point,
        );
        assert_eq!(
            module.stable_origin,
            *matching_node.stable_origin(),
            "discovered module stable origin must equal its graph-assigned origin (entry {:?})",
            module.entry_point,
        );
    }

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn discovered_module_origin_is_not_rederived_from_a_path_component() {
    // Hidden invariant: the stable origin carried by discovery is the graph-owned value type, not
    // a path-derived fallback. The discovered origins must be distinct `StableModuleOriginIdentity`
    // values keyed by canonical logical module path, and must round-trip through the graph node.
    let root = temp_dir("phase7a_origin_identity_values");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn build_source_origin_lookup_maps_each_owned_file_to_its_node_origin() {
    // Hidden invariant: the source-origin lookup is a direct projection of the central
    // `SourceTreeIndex` ownership through the graph's owned source IDs. Every owned source
    // record's logical identity module origin must equal its containing graph node's stable
    // origin, and no canonical path may appear twice.
    let root = temp_dir("source_origin_lookup_node_origin_alignment");
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
        for source_id in source_tree_index.owned_source_ids(node.module_id()) {
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
        .map(|node| source_tree_index.owned_source_ids(node.module_id()).len())
        .sum();
    assert_eq!(
        unique_paths.len(),
        total_entries,
        "every owned source path must be unique across all graph nodes"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

// ---------------------------------------------------------------------------
// R4D1: project semantic-source-set focused tests
// ---------------------------------------------------------------------------

struct DiscoveredSemanticSet {
    set: super::semantic_source_set::SemanticSourceSet,
    source_tree_index: super::source_tree_index::SourceTreeIndex,
}

/// Build the project context (source tree index, graph, namespace set) and return the semantic
/// source set plus its owning index for one entry without consuming it through input assembly.
fn discover_semantic_set_for_entry(
    config: &Config,
    style_directives: &StyleDirectiveRegistry,
    entry_path: &Path,
) -> Option<DiscoveredSemanticSet> {
    discover_semantic_set_for_entry_with_inputs(
        config,
        style_directives,
        entry_path,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &ExternalImportProviderRegistry::default(),
    )
}

fn discover_semantic_set_for_entry_with_inputs(
    config: &Config,
    style_directives: &StyleDirectiveRegistry,
    entry_path: &Path,
    source_packages: &crate::builder_surface::SourcePackageRegistry,
    external_import_providers: &ExternalImportProviderRegistry,
) -> Option<DiscoveredSemanticSet> {
    discover_semantic_set_result_for_entry_with_inputs(
        config,
        style_directives,
        entry_path,
        source_packages,
        external_import_providers,
    )
    .expect("semantic source set discovery should pass")
}

fn discover_semantic_set_result_for_entry_with_inputs(
    config: &Config,
    style_directives: &StyleDirectiveRegistry,
    entry_path: &Path,
    source_packages: &crate::builder_surface::SourcePackageRegistry,
    external_import_providers: &ExternalImportProviderRegistry,
) -> Result<Option<DiscoveredSemanticSet>, CompilerMessages> {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(config)).expect("entry root should resolve");
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::default();
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root.clone(),
        &project_root,
        config,
        source_packages,
        &source_file_kinds,
        external_import_providers,
        &mut string_table,
    )?;
    let project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let mut external_packages = ExternalPackageRegistry::new();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_import_resolution_table =
        crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
    let source_package_boundary_indexes = build_source_package_boundary_indexes(
        source_packages,
        &source_file_kinds,
        external_import_providers,
        &mut string_table,
    )?;
    let prepared_source_package_roots =
        source_package_boundary_indexes.prepared_source_package_roots();
    let resolver = ProjectPathResolver::new_with_module_roots(
        project_root,
        entry_root,
        prepared_source_package_roots,
        &source_file_kinds,
        source_tree_index.module_roots().clone(),
    )
    .expect("resolver should build");
    let module_namespace_set = ModuleNamespaceSet::build(
        &source_tree_index,
        &project_module_graph,
        source_package_boundary_indexes,
        &external_packages,
    );
    let mut external_imports = super::reachable_file_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: external_import_providers,
        cache: &mut external_import_cache,
        resolution_table: &mut external_import_resolution_table,
    };

    // Source packages now compile as separate indexed graphs. Validate their semantic imports
    // through the same package-boundary namespace before discovering the project consumer set.
    // The project traversal sees only each package facade and cannot inspect private package
    // sources by design.
    for (import_prefix, package_index) in module_namespace_set.source_package_boundaries() {
        let mut package_graph =
            super::project_module_graph::ProjectModuleGraph::from_source_tree_index(package_index);
        let package_resolver = resolver.for_source_package_boundary(
            package_index.entry_root().to_path_buf(),
            package_index
                .module_identities()
                .derive_compilation_root_table(),
        );
        super::module_inventory::discover_all_modules_in_package(
            config,
            &package_resolver,
            &mut package_graph,
            style_directives,
            &mut external_imports,
            DirectoryImportResolution::package(&module_namespace_set, import_prefix, package_index),
            &mut string_table,
        )?;
    }

    super::reachable_file_discovery::discover_semantic_source_set_for_test(
        entry_path,
        &resolver,
        style_directives,
        &mut external_imports,
        DirectoryImportResolution::project(&module_namespace_set, &source_tree_index),
        &mut string_table,
    )
    .map(|set| {
        set.map(|set| DiscoveredSemanticSet {
            set,
            source_tree_index,
        })
    })
}

#[test]
fn semantic_source_set_contains_only_same_owner_sources_in_source_id_order() {
    let root = temp_dir("semantic_set_same_owner_order");
    let src = root.join("src");
    fs::create_dir_all(src.join("helpers")).expect("should create helpers dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    // The entry imports two same-module sources in reverse SourceId order so the test proves the
    // set sorts by SourceId, not by import declaration or traversal order.
    fs::write(
        src.join("#page.moth"),
        "import @helpers/util\nimport @top\n#[:entry]\n",
    )
    .expect("should write entry");
    fs::write(src.join("top.moth"), "top #= 1\n").expect("should write top");
    fs::write(src.join("helpers/util.moth"), "util #= 1\n").expect("should write util");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let entry_path = fs::canonicalize(src.join("#page.moth")).expect("entry should canonicalize");
    let discovered = discover_semantic_set_for_entry(&config, &style_directives, &entry_path)
        .expect("semantic set should be built for a directory entry");

    let source_ids = discovered.set.source_ids();
    let entry_module_id = discovered.set.entry_module_id();

    // The entry module root file is always the first member (seeded).
    assert_eq!(
        source_ids.len(),
        3,
        "entry root plus two same-owner sources"
    );

    // SourceIds are in ascending (portable logical identity) order.
    let mut sorted = source_ids.clone();
    sorted.sort();
    assert_eq!(source_ids, sorted, "SourceIds must be in ascending order");

    // Every member is owned by the entry module.
    for source_id in &source_ids {
        let record = discovered.source_tree_index.source(*source_id);
        let ownership = record.ownership();
        assert!(
            matches!(ownership, super::source_tree_index::SourceOwnership::Owned(owner) if owner == entry_module_id),
            "every semantic member must be owned by the entry module"
        );
    }

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn semantic_input_assembly_excludes_cross_module_donor_sources() {
    let root = temp_dir("semantic_set_drives_input_assembly");
    let src = root.join("src");
    let provider = src.join("a_provider");
    fs::create_dir_all(&provider).expect("should create provider module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @z_local\nimport @a_provider { value }\n#[:entry]\n",
    )
    .expect("should write consumer root");
    fs::write(src.join("z_local.moth"), "local #= 1\n").expect("should write local source");
    fs::write(provider.join("#api.moth"), "export:\n    value #= 1\n;\n")
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
        .find(|module| module.entry_point.file_name() == Some(OsStr::new("#page.moth")))
        .expect("consumer module should be discovered");
    let input_names = consumer
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
        .collect::<Vec<_>>();

    assert_eq!(
        input_names,
        vec!["#page.moth", "z_local.moth"],
        "the consumer job must contain only its SourceId-ordered semantic members; the provider reaches binding through its completed interface"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn semantic_source_sets_for_distinct_modules_are_disjoint() {
    let root = temp_dir("semantic_sets_disjoint");
    let src = root.join("src");
    let module_a = src.join("a");
    let module_b = src.join("b");
    fs::create_dir_all(&module_a).expect("should create module A");
    fs::create_dir_all(&module_b).expect("should create module B");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    for (module, root_name, value) in [(&module_a, "#a.moth", "a"), (&module_b, "#b.moth", "b")] {
        fs::write(module.join(root_name), "import @helper\n").expect("should write module root");
        fs::write(module.join("helper.moth"), format!("{value} #= 1\n"))
            .expect("should write module helper");
    }

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let entry_a = fs::canonicalize(module_a.join("#a.moth")).expect("entry A should canonicalize");
    let entry_b = fs::canonicalize(module_b.join("#b.moth")).expect("entry B should canonicalize");
    let set_a = discover_semantic_set_for_entry(&config, &style_directives, &entry_a)
        .expect("module A set should be built")
        .set
        .source_ids()
        .into_iter()
        .collect::<HashSet<_>>();
    let set_b = discover_semantic_set_for_entry(&config, &style_directives, &entry_b)
        .expect("module B set should be built")
        .set
        .source_ids()
        .into_iter()
        .collect::<HashSet<_>>();

    assert!(
        set_a.is_disjoint(&set_b),
        "one project SourceId must not belong to two module semantic sets"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn semantic_source_set_excludes_cross_module_implementation_sources() {
    let root = temp_dir("semantic_set_no_cross_module_leakage");
    let (config, _resolver, style_directives, _module_a_root, _module_b_root) =
        write_cross_module_project(&root);

    let src = root.join("src");
    let entry_path =
        fs::canonicalize(src.join("module_a/#pageA.moth")).expect("entry should canonicalize");
    let discovered = discover_semantic_set_for_entry(&config, &style_directives, &entry_path)
        .expect("semantic set should be built");

    let source_ids = discovered.set.source_ids();

    // The entry module A owns only #pageA.moth. Module B's root (#api.moth) is a cross-module
    // provider root queued as an interface input; its impl file (impl.moth) is never imported.
    // Neither B source enters A's semantic set.
    assert_eq!(
        source_ids.len(),
        1,
        "module A's semantic set must contain only its own root, not module B's sources"
    );

    // Verify the set member is the entry root file, not module B's root or impl.
    let member_record = discovered.source_tree_index.source(source_ids[0]);
    assert_eq!(
        member_record.canonical_path(),
        entry_path,
        "the sole semantic member must be the entry root file"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn semantic_source_set_excludes_provider_owned_sources() {
    let root = temp_dir("semantic_set_no_provider");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "import @./drawing.js\n#[:entry]\n")
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

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(CountingExternalImportProvider::new(Arc::clone(
        &calls,
    ))));

    let entry_path = fs::canonicalize(src.join("#page.moth")).expect("entry should canonicalize");
    let discovered = discover_semantic_set_for_entry_with_inputs(
        &config,
        &style_directives,
        &entry_path,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &providers,
    )
    .expect("semantic set should be built");

    let source_ids = discovered.set.source_ids();

    // The provider-owned .js file is never a compiler-semantic source and never enters the
    // project semantic set. Only the entry root .moth file is a member.
    assert_eq!(
        source_ids.len(),
        1,
        "provider-owned files must not enter the project semantic set"
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "provider resolution must remain an explicit external edge"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn project_semantic_source_set_excludes_source_package_sources() {
    let root = temp_dir("semantic_set_no_source_package");
    let src = root.join("src");
    let package_root = root.join("packages/shared");
    fs::create_dir_all(&src).expect("should create src dir");
    fs::create_dir_all(&package_root).expect("should create package dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @shared { value }\n#[:entry]\n",
    )
    .expect("should write entry");
    fs::write(
        package_root.join("#api.moth"),
        "export:\n    value #= 1\n;\n",
    )
    .expect("should write package root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::default();
    source_packages.register_filesystem_root(
        "shared",
        fs::canonicalize(&package_root).expect("package root should canonicalize"),
        PackageOrigin::Builder,
    );

    let entry_path = fs::canonicalize(src.join("#page.moth")).expect("entry should canonicalize");
    let discovered = discover_semantic_set_for_entry_with_inputs(
        &config,
        &style_directives,
        &entry_path,
        &source_packages,
        &ExternalImportProviderRegistry::default(),
    )
    .expect("semantic set should be built");

    assert_eq!(
        discovered.set.source_ids().len(),
        1,
        "source-package facade and implementation sources must not enter the project set"
    );
    assert_eq!(
        discovered
            .source_tree_index
            .source(discovered.set.source_ids()[0])
            .canonical_path(),
        entry_path,
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn semantic_source_set_preserves_unchanged_source_read_count() {
    let root = temp_dir("semantic_set_read_count");
    let src = root.join("src");
    fs::create_dir_all(src.join("helpers")).expect("should create helpers dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "import @helpers/util\n#[:entry]\n")
        .expect("should write entry");
    fs::write(src.join("helpers/util.moth"), "util #= 1\n").expect("should write util");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let _counter_guard = SOURCE_READ_COUNTER_TEST_LOCK
        .lock()
        .expect("source read counter test lock poisoned");
    let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");
    super::source_loading::reset_source_read_count_for_test(&canonical_root);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    // The semantic set does not add reads or tokenizations: the entry and helper .moth files are
    // each read once during import scanning, matching the pre-set behavior.
    assert_eq!(
        super::source_loading::source_read_count_for_test(),
        2,
        "semantic set assembly must not change source read counts"
    );
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].input_files.len(), 2);

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn indexed_namespace_rejects_direct_entry_root_import() {
    for import_path in ["#page", "#page.moth"] {
        let root = temp_dir("indexed_namespace_direct_entry_root_rejected");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create source root");

        fs::write(
            root.join(settings::CONFIG_FILE_NAME),
            "entry_root #= \"src\"\n",
        )
        .expect("should write config");
        fs::write(
            src.join("#page.moth"),
            format!("import @{import_path} {{ symbol }}\n#[:page]\n"),
        )
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
            Ok(_) => panic!(
                "direct entry-root import {import_path} must use the direct-special diagnostic"
            ),
            Err(messages) => messages,
        };
        assert_eq!(
            first_error_diagnostic(&messages).kind.code(),
            "MOTH-IMPORT-0008"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

#[test]
fn indexed_namespace_rejects_direct_nested_child_root_import() {
    let root = temp_dir("indexed_namespace_direct_nested_child_root_rejected");
    let src = root.join("src");
    let child = src.join("child");
    fs::create_dir_all(&child).expect("should create child module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @child/#home { symbol }\n#[:page]\n",
    )
    .expect("should write parent root");
    fs::write(child.join("#home.moth"), "export:\n    symbol #= 1\n;\n")
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
        Ok(_) => panic!(
            "direct nested-child root imports must be rejected as direct special file imports"
        ),
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-IMPORT-0008"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn indexed_namespace_rejects_source_package_internal_direct_root_import() {
    let root = temp_dir("indexed_namespace_source_package_direct_mod_rejected");
    let src = root.join("src");
    let package_root = root.join("packages/helper");
    fs::create_dir_all(&src).expect("should create source root");
    fs::create_dir_all(&package_root).expect("should create source package root");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "entry_root #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(
        src.join("#page.moth"),
        "import @helper { use_public }\n\nio.line([: [use_public]])\n",
    )
    .expect("should write entry root");
    fs::write(
        package_root.join("#mod.moth"),
        "public_symbol #= \"public\"\nimport @sibling { use_public as internal_use_public }\nexport:\n    use_public #= internal_use_public\n;\n",
    )
    .expect("should write package module root");
    fs::write(
        package_root.join("sibling.moth"),
        "import @#mod { public_symbol }\n\nuse_public #= public_symbol\n",
    )
    .expect("should write package-internal source importing its own root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
    source_packages.register_filesystem_root(
        "helper",
        fs::canonicalize(&package_root).expect("package root should canonicalize"),
        PackageOrigin::Builder,
    );
    let entry_path = fs::canonicalize(src.join("#page.moth")).expect("entry should canonicalize");

    let messages = match discover_semantic_set_result_for_entry_with_inputs(
        &config,
        &style_directives,
        &entry_path,
        &source_packages,
        &ExternalImportProviderRegistry::default(),
    ) {
        Ok(_) => panic!("source-package-internal direct root imports must be rejected"),
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-IMPORT-0008"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}
