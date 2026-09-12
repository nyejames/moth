#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
use super::*;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

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
    let mut path_fork = PathInternerFork::empty();
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
    let mut path_fork = PathInternerFork::empty();
    let expected_token_count = [entry_source, intro_source, child_source]
        .into_iter()
        .enumerate()
        .map(|(index, source)| {
            let scope =
                path_fork.try_intern_portable_path(&format!("counter-fixture-{index}.moth"), &mut expected_token_string_table).expect("test path fits");
            let mut counter_span_builder = ExtendedSpanBuilder::new();
            crate::compiler_frontend::tokenizer::lexer::tokenize(
                source,
                &scope,
                crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode::SourceFile,
                &style_directives,
                &mut expected_token_string_table,
                crate::compiler_frontend::source::SourceId::COMPILATION_ROOT,
                &mut counter_span_builder,
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
