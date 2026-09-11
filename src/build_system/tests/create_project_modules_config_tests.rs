use super::*;
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
fn config_span_tables_finalize_for_success_and_diagnosed_results() {
    let cases = [
        (
            "successful",
            "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
            false,
        ),
        (
            "diagnosed",
            "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    release_output = \"src/out\",\n|\n",
            true,
        ),
    ];

    for (label, source, diagnosed) in cases {
        let _temp = tempfile::tempdir().expect("should create temporary project");
        let root = _temp.path().to_path_buf();
        fs::create_dir(root.join("src")).expect("should create the source directory");
        let config_path = root.join(settings::CONFIG_FILE_NAME);
        fs::write(&config_path, source).expect("should write config");

        let mut config = Config::new(root);
        let style_directives = test_style_directives();
        let frontend_surface =
            crate::projects::html_project::html_project_builder::HtmlProjectBuilder::new()
                .frontend_surface();
        let build_config_inputs =
            crate::compiler_frontend::build_config::BuildConfigInputSet::new();
        let services = ProjectConfigParseServices {
            style_directives: &style_directives,
            frontend_surface: &frontend_surface,
            build_config_inputs: &build_config_inputs,
        };
        let mut string_table = StringTable::new();
        let mut source_files = SourceDatabase::empty();

        let result = compile_project_config_file(
            &mut config,
            &config_path,
            &services,
            &mut source_files,
            &mut string_table,
        );
        if diagnosed {
            let messages = result.expect_err("invalid output settings should diagnose");
            assert!(
                messages.error_diagnostics().any(|diagnostic| {
                    matches!(
                        &diagnostic.payload,
                        DiagnosticPayload::InvalidConfig {
                            reason: InvalidConfigReason::InvalidOutputFolder {
                                reason: InvalidOutputFolderReason::InsideOrEqualToEntryRoot,
                                ..
                            },
                            ..
                        }
                    )
                }),
                "{label} config should retain its typed output diagnostic",
            );
        } else {
            result.expect("valid config should compile and apply");
        }

        let canonical_config_path =
            fs::canonicalize(&config_path).expect("config path should canonicalize");
        let config_file_id = source_files
            .get_by_canonical_path(&canonical_config_path)
            .expect("compiled config should have a registered source slot")
            .id;
        assert!(
            source_files.retained_text(config_file_id).is_some(),
            "{label} config should retain the compiled snapshot",
        );

        let installation_error = source_files
            .install_extended_spans(config_file_id, ExtendedSpanBuilder::default().freeze())
            .expect_err("the config span table should already be installed");
        assert_eq!(
            installation_error.error_type,
            ErrorType::Compiler,
            "{label} config should reject a second span-table installation",
        );
    }
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
    assert!(
        diagnostic.primary_span.is_some(),
        "the output-folder diagnostic should retain its authored span"
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
    assert!(
        diagnostic.primary_span.is_some(),
        "the malformed dependency diagnostic should retain its authored span"
    );
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
    assert!(
        diagnostic.primary_span.is_some(),
        "the config dependency diagnostic should retain its authored span"
    );
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

    assert!(
        diagnostic.primary_span.is_some(),
        "the duplicate-key diagnostic should retain its authored span"
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
