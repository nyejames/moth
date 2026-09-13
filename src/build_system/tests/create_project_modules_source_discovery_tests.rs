use super::*;
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
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);
    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
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
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);
    crate::compiler_frontend::pipeline_test_support::reset_file_frontend_prepare_count_for_test(
        &canonical_root,
    );

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
            .any(|path| {
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
            matches!(input_file.source, PreparedSourceKind::PlainMarkdown),
            "missing-source loading should produce PlainMarkdown inputs"
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
    let mut string_table = StringTable::new();

    let messages = load_missing_source_path_for_test(
        missing_source.clone(),
        crate::builder_surface::SourceFileKind::PlainMarkdown,
        &mut string_table,
    )
    .expect_err("missing source read should fail");

    let error = messages
        .infrastructure_error()
        .expect("expected infrastructure file error");
    assert!(
        error
            .msg
            .contains("Error reading file when adding new moth files to parse"),
        "unexpected infrastructure message: {}",
        error.msg,
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
    let messages = match load_missing_source_paths_with_registered_paths_for_test(
        vec![
            first_missing.clone(),
            loaded.clone(),
            second_missing.clone(),
        ],
        SourceFileKind::PlainMarkdown,
        &mut string_table,
    ) {
        Ok(_) => panic!("a serial missing-source failure should be reported"),
        Err(messages) => messages,
    };

    let source_files = messages
        .source_database_for_diagnostic(0)
        .expect("mixed source loading should publish its finalized source context");
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

    let error = messages
        .infrastructure_error()
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

    let messages = match load_missing_source_paths_with_registered_paths_for_test(
        source_paths.clone(),
        SourceFileKind::PlainMarkdown,
        &mut string_table,
    ) {
        Ok(_) => panic!("parallel missing-source failures should be reported"),
        Err(messages) => messages,
    };

    let source_files = messages
        .source_database_for_diagnostic(0)
        .expect("parallel source loading should publish its finalized source context");
    for (index, path) in source_paths.iter().enumerate() {
        let source_id = source_files
            .get_by_canonical_path(path)
            .expect("every parallel source should retain its registration slot")
            .id;
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

    let error = messages
        .infrastructure_error()
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
    let mut path_fork =
        crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();

    super::source_discovery::collect_reachable_input_files(
        &nested_entry,
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
        let mut path_fork =
            crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();

        super::source_discovery::collect_reachable_input_files(
            &nested_entry,
            &resolver,
            &style_directives,
            &mut external_imports,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &mut ResourceInputRegistry::new(),
            &mut path_fork,
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
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);

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
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);

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
        let path_text = messages.diagnostic_render_context(0).render_path(*path);
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
        messages.infrastructure_error().is_none(),
        "missing provider target must not surface a filesystem infrastructure error"
    );
    let diagnostic = first_error_diagnostic(&messages);
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-IMPORT-0005",
        "expected missing import target diagnostic, got {:?}",
        diagnostic
    );
    assert_primary_span_text(&messages, &src.join("@page.moth"), "@missing.js");
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
    assert_primary_span_text(&messages, &src.join("@page.moth"), "@HELPER");
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

    let (modules, source_files) =
        discover_modules_and_source_files_for_test(&config, &resolver, &style_directives)
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
