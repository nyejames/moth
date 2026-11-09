use super::create_project_modules_tests::{diagnosed_reports, successful_compilation_value};
use super::*;

use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::{
    CrossModuleImportNotExportedView, InvalidImportPathCode, InvalidImportPathView,
    MissingImportTargetView, UnsupportedExternalExtensionView,
};
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
#[test]
fn stage0_reuses_scanned_moth_source_when_assembling_input_files() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);
    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(modules.len(), 1);
    for source in [src.join("@page.moth"), src.join("helper.moth")] {
        let canonical = fs::canonicalize(source).expect("source should canonicalize");
        assert_eq!(
            super::source_loading_test_support::source_read_count_for_path_for_test(&canonical),
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
    assert!(!should_parallelize_owned_source_preparation(15));
    assert!(should_parallelize_owned_source_preparation(16));

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);
    crate::compiler_frontend::pipeline_test_support::reset_file_frontend_prepare_count_for_test(
        &canonical_root,
    );

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
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
        let canonical = fs::canonicalize(source_path).expect("source should canonicalize");
        let expected_reads = if unreachable_names
            .iter()
            .any(|name| source_path.ends_with(name))
        {
            0
        } else {
            1
        };
        assert_eq!(
            super::source_loading_test_support::source_read_count_for_path_for_test(&canonical),
            expected_reads,
            "selected sources should be read once while unselected sources stay unread"
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
            crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(
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
            crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(
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

    let prepared_header_syntax = &module.prepared.semantic.prepared_header_syntax;
    let module_symbols = &prepared_header_syntax.module_symbols;
    let reachable_header = prepared_header_syntax
        .headers
        .iter()
        .find(|header| {
            let source_path = module_symbols
                .source_paths_by_source_id
                .get(&header.tokens.source())
                .copied()
                .map(|path| {
                    module.prepared.semantic.path_fork.render_portable(
                        path,
                        &module.prepared.semantic.string_table,
                        &mut Vec::new(),
                    )
                });
            source_path
                .as_deref()
                .and_then(|path| Path::new(path).file_name())
                .and_then(|name| name.to_str())
                == Some("reachable.moth")
        })
        .expect("reachable source should retain one header stream");
    let reachable_owner = prepared_header_syntax
        .source_token_owners
        .get(&reachable_header.tokens.source())
        .expect("reachable source should retain its token owner");
    let reachable_table = reachable_owner
        .tokens_ref()
        .path_syntax_arc()
        .expect("reachable source should retain its path table");
    assert!(
        reachable_table.paths().iter().any(|path| {
            module.prepared.semantic.path_fork.render_portable(
                path.root,
                &module.prepared.semantic.string_table,
                &mut Vec::new(),
            ) == "leaf"
        }),
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
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

    let (source_files, input_files) = load_missing_source_paths_for_test(
        source_paths,
        crate::builder_surface::SourceFileKind::PlainMarkdown,
        &mut string_table,
    )
    .expect("parallel missing source loading should pass");

    let loaded_names = input_files
        .iter()
        .map(|input| {
            source_files
                .get(input.source_id())
                .and_then(|identity| identity.canonical_os_path.as_deref())
                .and_then(Path::to_str)
                .and_then(|path| Path::new(path).file_name())
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
        assert!(
            matches!(input_file.source, PreparedSourceKind::Deferred),
            "missing-source loading should produce deferred inputs"
        );
        let source_code = source_files
            .retained_text(input_file.source_id())
            .expect("loaded source should retain its snapshot");
        let expected_source = format!("# Asset {index}\n");
        assert_eq!(source_code, expected_source);
    }
}

#[test]
fn stage0_missing_source_load_preserves_file_error_shape() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let missing_source = root.join("missing.md");
    let error = load_missing_source_path_for_test(
        missing_source.clone(),
        crate::builder_surface::SourceFileKind::PlainMarkdown,
    )
    .expect_err("missing source read should fail");

    assert!(
        error
            .message
            .contains("Error reading file when adding new moth files to parse"),
        "unexpected infrastructure message: {}",
        error.message,
    );
    let host_path = error
        .host_path
        .as_deref()
        .expect("file errors should retain their host path");
    assert!(
        host_path.ends_with("missing.md"),
        "missing source path should be preserved in the diagnostic host path: {host_path:?}"
    );
}

#[test]
fn stage0_serial_missing_source_load_retains_siblings_and_finalizes_failures() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let first_missing = root.join("z_first_missing.md");
    let loaded = root.join("a_loaded.md");
    let second_missing = root.join("y_second_missing.md");
    fs::write(&loaded, "# Loaded sibling\n").expect("should write loaded source");

    let mut string_table = StringTable::new();
    let (prepared_failure, source_files) =
        match load_missing_source_paths_with_registered_paths_for_test(
            vec![
                first_missing.clone(),
                loaded.clone(),
                second_missing.clone(),
            ],
            SourceFileKind::PlainMarkdown,
            &mut string_table,
        ) {
            Ok(_) => panic!("a serial missing-source failure should be reported"),
            Err((failure, Some(source_files))) => (failure, *source_files),
            Err((failure, None)) => {
                panic!("source loading should retain its finalized source owner: {failure:?}")
            }
        };

    let first_missing_id = source_files
        .get_by_canonical_path(&first_missing)
        .expect("first missing source should retain its registration slot")
        .id;
    let loaded_id = source_files
        .get_by_canonical_path(&loaded)
        .expect("loaded sibling should retain its registration slot")
        .id;
    let second_missing_id = source_files
        .get_by_canonical_path(&second_missing)
        .expect("second missing source should retain its registration slot")
        .id;

    assert_eq!(
        source_files.retained_text(loaded_id),
        Some("# Loaded sibling\n"),
        "a successful sibling must be retained before the terminal read failure is published"
    );
    assert!(source_files.retained_text(first_missing_id).is_none());
    assert!(source_files.retained_text(second_missing_id).is_none());
    assert!(source_files.source_load_error(first_missing_id).is_some());
    assert!(source_files.source_load_error(second_missing_id).is_some());

    let boundary = prepared_failure_result::<()>(
        prepared_failure,
        Some(Arc::new(source_files)),
        &string_table,
    );
    let Err(errors) = boundary else {
        panic!("source loading should retain its infrastructure causes");
    };
    let error = errors
        .iter()
        .next()
        .expect("the first read failure should retain the infrastructure error lane");
    let host_path = error
        .host_path
        .as_deref()
        .expect("file errors should retain their host path");
    assert!(
        host_path.ends_with("z_first_missing.md"),
        "the first input-order read failure should be reported first: {host_path:?}"
    );
}

#[test]
fn stage0_parallel_missing_source_load_retains_siblings_and_finalizes_failures() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let source_count = super::source_discovery::STAGE0_PARALLEL_SOURCE_LOAD_MIN_FILES;
    let source_paths = (0..source_count)
        .map(|index| {
            let path = root.join(format!("asset_{index}.md"));
            if index != 1 && index != source_count - 1 {
                fs::write(&path, format!("# Asset {index}\n"))
                    .expect("should write parallel source");
            }
            path
        })
        .collect::<Vec<_>>();
    let mut string_table = StringTable::new();

    let (prepared_failure, source_files) =
        match load_missing_source_paths_with_registered_paths_for_test(
            source_paths.clone(),
            SourceFileKind::PlainMarkdown,
            &mut string_table,
        ) {
            Ok(_) => panic!("parallel missing-source failures should be reported"),
            Err((failure, Some(source_files))) => (failure, *source_files),
            Err((failure, None)) => {
                panic!("parallel loading should retain its finalized source owner: {failure:?}")
            }
        };
    let mut source_ids = Vec::with_capacity(source_paths.len());
    for (index, path) in source_paths.iter().enumerate() {
        let source_id = source_files
            .get_by_canonical_path(path)
            .expect("every parallel source should retain its registration slot")
            .id;
        source_ids.push(source_id);
        if index == 1 || index == source_count - 1 {
            assert!(
                source_files.retained_text(source_id).is_none(),
                "failed source {index} must not create a loaded record"
            );
            assert!(
                source_files.source_load_error(source_id).is_some(),
                "failed source {index} must finalize its slot"
            );
        } else {
            assert_eq!(
                source_files.retained_text(source_id),
                Some(format!("# Asset {index}\n").as_str()),
                "successful source {index} must survive a sibling failure"
            );
            assert!(source_files.source_load_error(source_id).is_none());
        }
    }

    let boundary = prepared_failure_result::<()>(
        prepared_failure,
        Some(Arc::new(source_files)),
        &string_table,
    );
    let Err(errors) = boundary else {
        panic!("parallel source loading should retain its infrastructure causes");
    };
    let error = errors
        .iter()
        .next()
        .expect("the parallel read failure should retain the infrastructure error lane");
    let host_path = error
        .host_path
        .as_deref()
        .expect("file errors should retain their host path");
    assert!(
        host_path.ends_with("asset_1.md"),
        "parallel failures should be reported in deterministic input order: {host_path:?}"
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let modules = successful_compilation_value(discover_modules_for_test_with_providers(
        &config,
        &resolver,
        &style_directives,
        &providers,
    ));
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();

    super::source_discovery::collect_reachable_input_files(
        &nested_entry,
        &super::module_namespace::ModuleNamespaceSet::default(),
        &resolver,
        &style_directives,
        &mut external_imports,
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut ResourceInputRegistry::new(),
        &mut path_fork,
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
            "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
        let mut path_fork =
            crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();

        super::source_discovery::collect_reachable_input_files(
            &nested_entry,
            &super::module_namespace::ModuleNamespaceSet::default(),
            &resolver,
            &style_directives,
            &mut external_imports,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &mut ResourceInputRegistry::new(),
            &mut path_fork,
            &mut string_table,
        )
        .expect("both nested provider clauses should resolve");

        let source = "src/feature/@page.moth";
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
fn staging_identities_stay_outside_the_traversal_range() {
    use super::source_discovery::{PROVIDER_SOURCE_ID_BASE, ProviderOwnedSourceStaging};
    use crate::builder_surface::external_import_providers::provider_sources::ProviderSourceRegistrar;

    let mut staging = ProviderOwnedSourceStaging::new();
    let canonical = PathBuf::from("/fixture/drawing.js");
    let first = staging
        .register_provider_source(&canonical, "drawing.js", "export const x = 1;\n".to_owned())
        .expect("first provider source registers");
    let first_id = first.source();
    let second = staging
        .register_provider_source(
            Path::new("/fixture/other.js"),
            "other.js",
            "export const y = 2;\n".to_owned(),
        )
        .expect("second provider source registers");
    assert_ne!(first_id, second.source());
    assert!(first_id.index() >= PROVIDER_SOURCE_ID_BASE);
    assert!(second.source().index() >= PROVIDER_SOURCE_ID_BASE);
    assert_eq!(staging.provisional_identities().count(), 2);
    assert_eq!(staging.registration_rows().count(), 2);
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
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
            super::source_loading_test_support::source_read_count_for_path_for_test(&canonical),
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let modules = successful_compilation_value(discover_modules_for_test_with_providers(
        &config,
        &resolver,
        &style_directives,
        &providers,
    ));
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);

    let modules = successful_compilation_value(discover_modules_for_test_with_providers(
        &config,
        &resolver,
        &style_directives,
        &providers,
    ));
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
            super::source_loading_test_support::source_read_count_for_path_for_test(&canonical),
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);

    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0021",
        "expected unsupported external extension diagnostic, got {}",
        diagnostic.descriptor().code
    );
    let facts = diagnostic
        .family_view::<UnsupportedExternalExtensionView>()
        .expect("unsupported external extension should retain typed path facts");
    let (report, _) = diagnosed_reports(&failure)
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("unsupported external extension should retain its error report");
    let context = DiagnosticRenderContext::for_report(report);
    assert_eq!(
        context.render_path(facts.path),
        "drawing.js",
        "unexpected path in diagnostic"
    );
    assert_eq!(
        context.string_table.resolve(facts.extension),
        "js",
        "unexpected extension in diagnostic"
    );
}

#[test]
fn directory_provider_dependency_calls_provider_once_for_repeated_physical_source() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    successful_compilation_value(discover_modules_for_test_with_providers(
        &config,
        &resolver,
        &style_directives,
        &providers,
    ));

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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let failure =
        discover_modules_for_test_with_providers(&config, &resolver, &style_directives, &providers);

    // The provider must never be invoked for a cross-module target.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "provider must not run for a cross-module target"
    );

    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0015",
        "expected cross-module import diagnostic, got {}",
        diagnostic.descriptor().code
    );
    diagnostic
        .family_view::<CrossModuleImportNotExportedView>()
        .expect("cross-module import should retain typed symbol-path facts");
}

#[test]
fn directory_provider_dependency_missing_target_reports_structured_diagnostic_without_path_probe() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let failure =
        discover_modules_for_test_with_providers(&config, &resolver, &style_directives, &providers);

    // The provider must never be invoked for a target the index could not resolve.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "provider must not run for a missing target"
    );

    // The diagnostic must be a structured import diagnostic, not a filesystem infrastructure
    // error. The index-based lookup reports a missing import target through the existing
    // import diagnostic lane, which proves no directory-time path probe runs after the
    // index is built.
    assert!(
        diagnosed_reports(&failure)
            .iter_severity(DiagnosticSeverity::Error)
            .next()
            .is_some(),
        "missing provider target should surface a structured import diagnostic"
    );
    assert!(
        matches!(&failure, Ok(CompilationOutcome::Diagnosed(_))),
        "missing provider target must remain a diagnosed result"
    );
    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0005",
        "expected missing import target diagnostic, got {}",
        diagnostic.descriptor().code
    );
    assert_primary_span_text(&failure, &src.join("@page.moth"), "@missing.js");
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);
    assert_eq!(
        first_error_diagnostic(&failure).descriptor().code,
        "MOTH-IMPORT-0006"
    );
    assert_primary_span_text(&failure, &src.join("@page.moth"), "@HELPER");
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);
    assert_eq!(
        first_error_diagnostic(&failure).descriptor().code,
        "MOTH-IMPORT-0006"
    );
}

#[test]
fn binding_package_case_variant_import_with_local_file_is_ambiguous() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let local_core = src.join("core");
    fs::create_dir_all(&local_core).expect("should create local core folder");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@core/IO\n#[:[IO.value]]\n").expect("should write root");
    // Package identities reject case-only overlaps, so `@core/IO` can't select this local file
    // just because the registered `@core/io` lookup is exact.
    fs::write(local_core.join("IO.moth"), "value #= 1\n").expect("should write local file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);
    assert_eq!(
        first_error_diagnostic(&failure).descriptor().code,
        "MOTH-IMPORT-0006"
    );
    assert_primary_span_text(&failure, &src.join("@page.moth"), "@core/IO");
}

#[test]
fn directory_source_dependency_rejects_obsolete_relative_form() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source root");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);
    assert_eq!(
        first_error_diagnostic(&failure).descriptor().code,
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);
    assert_eq!(
        first_error_diagnostic(&failure).descriptor().code,
        "MOTH-IMPORT-0015"
    );
}

#[test]
fn module_import_through_hidden_directory_reports_hidden_reason() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let hidden = src.join("_private/child");
    fs::create_dir_all(&hidden).expect("should create hidden child");
    fs::write(hidden.join("@child.moth"), "export:\n    value #= 1\n;\n")
        .expect("should write hidden root");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@_private/child value\n#[:page]\n")
        .expect("should write page");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);
    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0016",
        "a hidden-directory traversal must use the invalid-path diagnostic, got {}",
        diagnostic.descriptor().code
    );
    assert_eq!(
        diagnostic
            .family_view::<InvalidImportPathView>()
            .expect("hidden import should retain typed invalid-path facts")
            .reason,
        InvalidImportPathCode::HiddenDirectory
    );
    assert_primary_span_text(&failure, &src.join("@page.moth"), "@_private/child");
}

#[test]
fn single_component_hidden_import_remains_ambiguous_missing_target() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("_private")).expect("should create hidden directory");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@_private value\n#[:page]\n").expect("should write page");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);
    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0005",
        "a single hidden component may name a file or directory and must stay missing"
    );
    assert!(
        diagnostic
            .family_view::<MissingImportTargetView>()
            .is_some()
    );
}

#[test]
fn underscore_prefixed_helper_file_remains_importable() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@_helper value\n#[:page]\n").expect("should write page");
    fs::write(src.join("_helper.moth"), "value #= 1\n").expect("should write helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
}

#[test]
fn selected_hidden_root_can_import_its_visible_child_module() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let selected = _tmp_root.path().join("_prototype");
    let child = selected.join("visible");
    fs::create_dir_all(&child).expect("should create visible child");
    fs::write(
        selected.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(selected.join("@site.moth"), "@visible v\n#[:site]\n").expect("should write root");
    fs::write(child.join("@mod.moth"), "export:\n    v #= 1\n;\n")
        .expect("should write child root");

    let mut config = Config::new(selected.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &selected.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
}

#[test]
fn stage0_consumes_moth_tokens_into_retained_header_syntax() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let (modules, source_files) = successful_compilation_value(
        discover_modules_and_source_files_for_test(&config, &resolver, &style_directives),
    );
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
        .source_ids_by_source
        .iter()
        .find_map(|(logical_path, source_id)| {
            source_files
                .get(*source_id)
                .and_then(|record| record.canonical_os_path.as_ref())
                .filter(|canonical_path| canonical_path.to_path_buf() == entry_path)
                .map(|_| logical_path)
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
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
//  Qualified-reference provider diagnostics
// -------------------------

/// Write one single-module project whose entry body is supplied verbatim.
fn write_qualified_reference_probe_project(
    root: &std::path::Path,
    entry_body: &str,
    extra_files: &[(&str, &str)],
) -> (
    Config,
    ProjectPathResolver,
    StyleDirectiveRegistry,
    std::path::PathBuf,
) {
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), entry_body).expect("should write entry");
    for (relative_path, contents) in extra_files {
        let path = src.join(relative_path);
        fs::create_dir_all(path.parent().expect("extra file has a parent"))
            .expect("should create extra file parent");
        fs::write(path, contents).expect("should write extra file");
    }

    let mut config = Config::new(root.to_path_buf());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    (config, resolver, style_directives, src)
}

#[test]
fn resource_path_qualified_occurrence_uses_the_provider_lane_without_reading_the_resource() {
    // An authored resource path without a selection stays a file value. Adding a selection must
    // not reinterpret the resource as a namespace: it enters the same provider lane as the
    // explicit direct-selection clause and never becomes a resource read.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, src) = write_qualified_reference_probe_project(
        &root,
        "logo = @images/logo.svg name\n#[:entry]\n",
        &[("images/logo.svg", "<svg/>")],
    );

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);
    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0021",
        "a selected resource path must use the unsupported-provider diagnostic, got {}",
        diagnostic.descriptor().code
    );
    diagnostic
        .family_view::<UnsupportedExternalExtensionView>()
        .expect("the provider lane retains typed extension facts");
    assert_primary_span_text(&failure, &src.join("@page.moth"), "@images/logo.svg");

    // The authored clause form produces the same diagnostic and span for the same path.
    let _clause_root = tempfile::tempdir().expect("should create temp dir");
    let clause_root = _clause_root.path().to_path_buf();
    let (clause_config, clause_resolver, clause_directives, clause_src) =
        write_qualified_reference_probe_project(
            &clause_root,
            "@images/logo.svg name\n#[:entry]\n",
            &[("images/logo.svg", "<svg/>")],
        );
    let clause_failure =
        discover_modules_for_test(&clause_config, &clause_resolver, &clause_directives);
    let clause_diagnostic = first_error_diagnostic(&clause_failure);
    assert_eq!(
        clause_diagnostic.descriptor().code,
        diagnostic.descriptor().code,
        "the occurrence and clause forms share the provider error lane"
    );
    assert_primary_span_text(
        &clause_failure,
        &clause_src.join("@page.moth"),
        "@images/logo.svg",
    );
}

#[test]
fn explicit_moth_extension_qualified_occurrence_keeps_the_source_extension_diagnostic() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, src) = write_qualified_reference_probe_project(
        &root,
        "value = @helper.moth answer\n#[:entry]\n",
        &[("helper.moth", "answer #= 1\n")],
    );

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);
    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0020",
        "an explicit .moth source path keeps its dependency diagnostic, got {}",
        diagnostic.descriptor().code
    );
    assert_primary_span_text(&failure, &src.join("@page.moth"), "@helper.moth");

    let _clause_root = tempfile::tempdir().expect("should create temp dir");
    let clause_root = _clause_root.path().to_path_buf();
    let (clause_config, clause_resolver, clause_directives, _clause_src) =
        write_qualified_reference_probe_project(
            &clause_root,
            "@helper.moth answer\n#[:entry]\n",
            &[("helper.moth", "answer #= 1\n")],
        );
    let clause_failure =
        discover_modules_for_test(&clause_config, &clause_resolver, &clause_directives);
    assert_eq!(
        first_error_diagnostic(&clause_failure).descriptor().code,
        diagnostic.descriptor().code,
    );
}

#[test]
fn entry_only_child_qualified_occurrence_matches_the_clause_error_lane() {
    let write_entry_only_child_fixture = |root: &std::path::Path, body: &str| {
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create src");
        fs::write(
            root.join(settings::CONFIG_FILE_NAME),
            "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
        )
        .expect("should write config");
        fs::write(src.join("@page.moth"), body).expect("should write entry");
        let child = src.join("child");
        fs::create_dir_all(&child).expect("should create child");
        fs::write(child.join("@entry.moth"), "hidden #= 1\n").expect("should write child entry");

        let mut config = Config::new(root.to_path_buf());
        let style_directives = test_style_directives();
        parse_project_config_for_test(
            &mut config,
            &root.join(settings::CONFIG_FILE_NAME),
            &style_directives,
        )
        .expect("config should parse");
        let resolver = configured_resolver(&config);
        (config, resolver, style_directives, src)
    };

    let _occurrence_root = tempfile::tempdir().expect("should create temp dir");
    let occurrence_root = _occurrence_root.path().to_path_buf();
    let (occurrence_config, occurrence_resolver, occurrence_directives, occurrence_src) =
        write_entry_only_child_fixture(&occurrence_root, "value = @child hidden\n#[:entry]\n");
    let occurrence_failure = discover_modules_for_test(
        &occurrence_config,
        &occurrence_resolver,
        &occurrence_directives,
    );
    let occurrence_diagnostic = first_error_diagnostic(&occurrence_failure);
    assert_primary_span_text(
        &occurrence_failure,
        &occurrence_src.join("@page.moth"),
        "@child",
    );

    let _clause_root = tempfile::tempdir().expect("should create temp dir");
    let clause_root = _clause_root.path().to_path_buf();
    let (clause_config, clause_resolver, clause_directives, clause_src) =
        write_entry_only_child_fixture(&clause_root, "@child hidden\n#[:entry]\n");
    let clause_failure =
        discover_modules_for_test(&clause_config, &clause_resolver, &clause_directives);
    let clause_diagnostic = first_error_diagnostic(&clause_failure);
    assert_primary_span_text(&clause_failure, &clause_src.join("@page.moth"), "@child");

    assert_eq!(
        occurrence_diagnostic.descriptor().code,
        clause_diagnostic.descriptor().code,
        "an entry-only child has no importable surface for either authored form"
    );
}

#[test]
fn duplicate_qualified_occurrences_invoke_a_registered_provider_once() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, _src) = write_qualified_reference_probe_project(
        &root,
        "first = @app.js one\nsecond = @app.js two\n#[:entry]\n",
        &[("app.js", "export const one = 1;\n")],
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = ExternalImportProviderRegistry::empty();
    providers.register(Arc::new(CountingExternalImportProvider::new(Arc::clone(
        &calls,
    ))));

    let _modules = successful_compilation_value(discover_modules_for_test_with_providers(
        &config,
        &resolver,
        &style_directives,
        &providers,
    ));
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "two occurrences selecting one provider path must invoke the provider once"
    );
}

#[test]
fn check_only_qualified_occurrence_reaches_the_same_provider_binding_as_a_clause() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let first = src.join("first");
    let child = first.join("child");
    fs::create_dir_all(&child).expect("should create module directories");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        first.join("@first.moth"),
        "owner || -> I32:\n    return 1\n;\n",
    )
    .expect("should write owner root");
    // Both sources are owned but unreached, so check-only preparation handles them.
    fs::write(
        first.join("a_clause.moth"),
        "@child value\nprobe || -> I32:\n    return value\n;\n",
    )
    .expect("should write clause orphan");
    fs::write(
        first.join("b_occurrence.moth"),
        "probe || -> I32:\n    result = @child value\n    return result\n;\n",
    )
    .expect("should write occurrence orphan");
    fs::write(child.join("@mod.moth"), "export:\n    value #= 7\n;\n")
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

    let (schedule, graph, source_tree_index, _source_files, _string_table) =
        discover_modules_and_graph_for_test_with_check_only(
            &config,
            &resolver,
            &style_directives,
            true,
        );
    let (_waves, _provider_bindings, _source_package_dependencies, check_only_jobs) =
        schedule.into_parts();
    assert_eq!(
        check_only_jobs.len(),
        2,
        "both unreached owned sources become check-only units"
    );

    let canonical_child = fs::canonicalize(&child).expect("child should canonicalize");
    let child_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&canonical_child)
        .expect("child module should be a graph node");
    let canonical_first = fs::canonicalize(&first).expect("first should canonicalize");
    let first_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&canonical_first)
        .expect("first module should be a graph node");

    let mut check_only_sources = Vec::new();
    for job in &check_only_jobs {
        assert_eq!(
            job.provider_bindings.len(),
            1,
            "each check-only unit publishes one cross-module provider binding"
        );
        let binding = &job.provider_bindings[0];
        assert_eq!(binding.provider_module_id, child_id);
        assert_eq!(binding.provider_surface, ModuleRootRole::Normal);
        assert_eq!(
            binding.dependency_shell_id.ordinal, 0,
            "each source's first authored shell is ordinal zero"
        );
        let names: Vec<String> = job
            .prepared
            .semantic
            .prepared_header_syntax
            .module_symbols
            .module_file_paths
            .iter()
            .map(|path| {
                job.prepared.semantic.path_fork.render_portable(
                    *path,
                    &job.prepared.semantic.string_table,
                    &mut Vec::new(),
                )
            })
            .collect();
        check_only_sources.extend(names);
    }
    check_only_sources.sort();
    assert_eq!(
        check_only_sources,
        vec![
            "src/first/a_clause.moth".to_owned(),
            "src/first/b_occurrence.moth".to_owned(),
        ],
        "the clause form and the qualified-occurrence form both reach check-only binding"
    );
    assert!(
        graph
            .dependency_providers(first_id)
            .expect("frozen adjacency is readable")
            .is_empty(),
        "check-only units must not insert canonical graph edges"
    );
}
