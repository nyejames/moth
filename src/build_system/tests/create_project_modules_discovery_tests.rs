use super::*;
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

    let (modules, source_files) =
        discover_modules_and_source_files_for_test(&config, &resolver, &style_directives)
            .expect("module discovery should pass");
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(modules.len(), 1, "expected exactly one entry module");

    let source_theme = fs::canonicalize(src.join("helpers/theme.moth")).expect("canonical source");
    let package_theme =
        fs::canonicalize(lib.join("helpers/theme.moth")).expect("canonical package file");
    let discovered_paths = module_source_paths(modules[0], &source_files);
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
        .map(|input| {
            collected
                .source_files
                .sources()
                .get(input.source_id())
                .and_then(|identity| {
                    identity
                        .canonical_os_path
                        .clone()
                        .map(|canonical| canonical.into_path_buf())
                })
                .expect("discovered source should have a canonical path")
        })
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

    let (modules, _resource_inputs, source_files) =
        discover_modules_for_test_with_resource_inputs(&config, &resolver, &style_directives)
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
    let ResolvedFileReferenceOutcome::Diagnostic(diagnostic) = &references[0].outcome else {
        panic!("missing resource should retain a typed diagnostic outcome");
    };
    let source_id = source_files
        .get_by_canonical_path(
            &fs::canonicalize(src.join("@page.moth"))
                .expect("entry source path should canonicalize"),
        )
        .expect("entry source should remain in the source database")
        .id;
    let span = diagnostic
        .primary_span
        .expect("Stage 0 file-reference diagnostics retain their authored span");
    assert_eq!(span.source(), source_id);
    let range = span.byte_range(&source_files);
    let source_text = source_files
        .retained_text(source_id)
        .expect("entry source snapshot should remain available");
    let expected_start = source_text
        .find("@assets/missing.svg")
        .expect("fixture should contain the missing resource path") as u32;
    assert_eq!(range.start(), expected_start);
    assert_eq!(
        range.end(),
        expected_start + "@assets/missing.svg".len() as u32
    );
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
    let (modules, resource_inputs, _source_files) =
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
    let (modules, resource_inputs, _source_files) =
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
            collected
                .source_files
                .sources()
                .get(input.source_id())
                .and_then(|identity| identity.canonical_os_path.as_deref())
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<HashSet<_>>();
    assert!(input_paths.contains("main.moth"));
    assert!(input_paths.contains("intro.mtf"));
    assert!(input_paths.contains("second.mtf"));
    assert!(input_paths.contains("notes.md"));
    assert!(!input_paths.contains("helper.moth"));

    // The Markdown target is never tokenized, so it reaches discovery's cache-miss branch. Its
    // snapshot must arrive in the final slot: nothing reads this source again from disk.
    let markdown_source_id = collected
        .input_files
        .iter()
        .map(PreparedSourceInput::source_id)
        .find(|source_id| {
            collected
                .source_files
                .sources()
                .get(*source_id)
                .and_then(|identity| identity.canonical_os_path.as_deref())
                .and_then(Path::file_name)
                == Some(OsStr::new("notes.md"))
        })
        .expect("the Markdown content target should be a prepared input");
    assert_eq!(
        collected
            .source_files
            .sources()
            .retained_text(markdown_source_id),
        Some("# notes\n")
    );

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
        PreparedSourcePackageRoots::default(),
        &source_file_kinds,
        ModuleRootTable::empty(),
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
    let mut project_source_files = None;
    let frontend = super::compilation::compile_single_file_frontend_with_inputs(
        &config,
        crate::compiler_frontend::FrontendBuildProfile::Dev,
        &test_style_directives(),
        &mut builder_surface,
        entry.extension().expect("entry should have an extension"),
        &mut string_table,
        &mut project_source_files,
        &crate::compiler_frontend::build_config::BuildConfigInputSet::new(),
        crate::build_system::create_project_modules::FrontendCompilationMode::Canonical,
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
