use super::*;
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
        |resolution, string_table, provider_paths| {
            let provider = provider_root(&["project"], provider_paths);
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
                resolution.path_fork(),
                &mut external_imports,
                *resolution,
                string_table,
            ) else {
                panic!("source-package @project dependency must be rejected");
            };
            let failure = error.into_failure(
                string_table,
                std::sync::Arc::new(resolution.path_fork().snapshot_table()),
            );
            let messages = failure.into_messages(string_table);
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
    fs::write(
        src.join("@page.moth"),
        "-- é🦋\n@drawing.js as drawing\n#[:ok]\n",
    )
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
    assert_primary_span_text(&messages, &src.join("@page.moth"), "@drawing.js");

    let mut source_string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(&config)).expect("entry root should resolve");
    let empty_providers = ExternalImportProviderRegistry::empty();
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        &empty_providers,
        &mut source_string_table,
    )
    .expect("source tree index should rebuild for span assertions");
    let mut source_files =
        source_database_for_test(&source_tree_index, &resolver, &mut source_string_table);
    let entry_path =
        fs::canonicalize(src.join("@page.moth")).expect("entry source path should canonicalize");
    let mut selected_source_texts = super::source_loading::SelectedSourceTextMap::default();
    selected_source_texts
        .load(&entry_path)
        .expect("entry source snapshot should load");
    selected_source_texts
        .retain_into(&mut source_files)
        .expect("entry source snapshot should retain");
    let source_id = source_files
        .get_by_canonical_path(&entry_path)
        .expect("entry source should remain in the source database")
        .id;
    let span = diagnostic
        .primary_span
        .expect("provider diagnostic should retain its authored path span");
    assert_eq!(span.source(), source_id);
    let range = span.byte_range(&source_files);
    let source_text = source_files
        .retained_text(source_id)
        .expect("entry source snapshot should remain available");
    let expected_start = source_text
        .find("@drawing.js")
        .expect("fixture should contain the provider path") as u32;
    assert_eq!(range.start(), expected_start);
    assert_eq!(
        range.end(),
        expected_start + "@drawing.js".len() as u32,
        "provider span should cover the full UTF-8 path token"
    );
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
            messages.diagnostic_render_context(0).render_path(*path),
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
            messages.diagnostic_render_context(0).render_path(*path),
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
    assert_primary_span_text(&messages, &src.join("@page.moth"), "@intro");
}
