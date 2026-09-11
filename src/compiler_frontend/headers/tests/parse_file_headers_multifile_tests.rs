use super::*;

#[test]
fn multi_file_parsing_aggregates_headers_const_fragments_and_runtime_count() {
    let sources = vec![
        (
            "[runtime1]\n#[const1]\n[runtime2]\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
        (
            "helper_func || -> Int:\n    return 1\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let headers = parse_multi_file_headers(&sources, "src/@page.moth");

    // Entry file: 2 runtime templates + 1 const template + 1 start function = 2 headers
    // (const template + start function; runtime templates are inside start function)
    // Non-entry file: 1 function header
    assert!(
        headers.headers.len() >= 2,
        "expected headers from both files to be aggregated"
    );

    // Verify const fragment from entry file is preserved.
    assert_eq!(
        headers.top_level_const_fragments.len(),
        1,
        "expected one const fragment from entry file"
    );
    assert_eq!(
        headers.top_level_const_fragments[0].runtime_insertion_index, 1,
        "const fragment should be inserted after 1 runtime fragment (the one before it)"
    );

    // Verify runtime fragment count is correct for entry file.
    assert_eq!(
        headers.entry_runtime_fragment_count, 2,
        "expected 2 runtime fragments from entry file"
    );
}

#[test]
fn multi_file_parsing_aggregates_warnings_from_all_files() {
    let sources = vec![
        (
            "Status_type :: bad_variant;\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
        (
            "Helper_type :: other_variant;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let (result, warnings, _string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");

    assert!(result.is_ok(), "expected successful header parsing");
    assert_eq!(
        warnings.len(),
        4,
        "expected four naming-convention warnings (two from each file)"
    );
    assert!(
        warnings.iter().all(|warning| matches!(
            warning.kind,
            DiagnosticKind::Rule(
                crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::IdentifierNamingConvention
            )
        )),
        "all warnings should be naming convention warnings"
    );
}

#[test]
fn multi_file_parsing_preserves_warnings_before_later_parse_error() {
    // The helper file emits naming warnings, then fails on a later duplicate declaration.
    // Those file-local warnings must still be merged even though the file contributes no output.
    let sources = vec![
        (
            "io.line([: [\"hello\"]])\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
        (
            "Status_type :: bad_variant;\ndup ||:\n;\ndup ||:\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let (result, warnings, _string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");

    assert!(
        result.is_err(),
        "expected header parsing to fail due to duplicate declaration"
    );

    assert_eq!(
        warnings.len(),
        2,
        "expected two naming-convention warnings from the failing helper file to be preserved"
    );
    assert!(
        warnings.iter().all(|warning| matches!(
            warning.kind,
            DiagnosticKind::Rule(
                crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::IdentifierNamingConvention
            )
        )),
        "all warnings should be naming convention warnings"
    );
}

#[test]
fn per_file_fork_merge_produces_correct_headers_and_warnings_for_multiple_files() {
    let sources = [
        (
            "FooA #= \"a\"\nBarA #= \"b\"\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
        (
            "FooB #= \"c\"\nBarB #= \"d\"\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let (result, warnings, string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");

    let headers = result.expect("headers should parse");

    // 4 constant headers + 1 start header = 5 headers
    assert_eq!(headers.headers.len(), 5, "expected 4 constants + 1 start");

    let constant_names: Vec<String> = headers
        .headers
        .iter()
        .filter_map(|header| match &header.kind {
            HeaderKind::Constant { .. } => header
                .tokens
                .src_path
                .name()
                .map(|n| string_table.resolve(n).to_owned()),
            _ => None,
        })
        .collect();

    assert!(constant_names.contains(&"FooA".to_owned()));
    assert!(constant_names.contains(&"BarA".to_owned()));
    assert!(constant_names.contains(&"FooB".to_owned()));
    assert!(constant_names.contains(&"BarB".to_owned()));

    // PascalCase top-level constant names should produce naming warnings.
    assert_eq!(
        warnings.len(),
        4,
        "expected four naming convention warnings for PascalCase constants"
    );
    assert!(
        warnings.iter().all(|warning| matches!(
            warning.kind,
            DiagnosticKind::Rule(
                crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::IdentifierNamingConvention
            )
        )),
        "all warnings should be naming convention warnings"
    );
}

#[test]
fn per_file_fork_merge_remaps_non_identity_strings_across_multiple_files() {
    // Both files intern generated deferred-feature strings into their local suffixes.
    // Because the fork source is shared and frozen before the loop, the second merge must remap
    // that local ID past the first file's generated string in the module table.
    let sources = [
        (
            "Foo #= \"a\"\n#[public_surface_fragment]\n".to_owned(),
            "src/helper_a.moth".to_owned(),
        ),
        (
            "Bar #= \"b\"\n#[const_fragment]\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let (result, warnings, string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");

    assert!(
        result.is_err(),
        "expected header parsing to fail due to deferred header features"
    );

    // PascalCase constants produce naming warnings before the errors.
    assert_eq!(
        warnings.len(),
        2,
        "expected two naming convention warnings before errors"
    );

    let errors = result.err().expect("expected errors").into_diagnostics();
    assert_eq!(errors.len(), 2, "expected two deferred feature errors");

    let mut feature_names = Vec::new();
    for error in &errors {
        let DiagnosticPayload::DeferredFeature { reason } = &error.payload else {
            panic!("expected DeferredFeature payload, got {:?}", error.payload);
        };
        match reason {
            DeferredFeatureReason::NamedFeature { feature } => {
                feature_names.push(string_table.resolve(*feature).to_owned());
            }
            other => panic!("expected NamedFeature reason, got {:?}", other),
        }
    }

    assert!(
        feature_names
            .iter()
            .all(|feature| { feature == "top-level const templates in ordinary source files" })
    );
}

#[test]
fn dependency_only_file_contributes_file_dependency_clauses_and_module_file_paths() {
    use crate::compiler_frontend::headers::symbol_collection::build_module_symbols;

    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let (helper_output, _) = prepare_single_file(
        "@core/math\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    let options = HeaderParseOptions::default();
    let styles = StyleDirectiveRegistry::built_ins();
    let context = HeaderTestPrepareContext {
        source_id: SourceId::from_index(1),
        entry_file_path: &entry_file_path,
        options: &options,
        style_directives: &styles,
    };
    let mut page_spans = ExtendedSpanBuilder::new();
    let page_output = prepare_test_source_file(
        "value #= 1\n",
        &entry_file_path,
        &context,
        &mut string_table,
        0,
        0,
        &mut page_spans,
    )
    .expect("entry source should prepare");

    let mut prepared_files = vec![helper_output, page_output];
    let module_symbols = build_module_symbols(
        &mut prepared_files,
        &mut string_table,
        &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
    )
    .expect("module symbols should build");

    let helper_path = InternedPath::try_from_filesystem_path(
        &PathBuf::from("src/helper.moth"),
        &mut string_table,
    )
    .expect("test path should be UTF-8");
    let page_path =
        InternedPath::try_from_filesystem_path(&PathBuf::from("src/@page.moth"), &mut string_table)
            .expect("test path should be UTF-8");

    assert!(
        module_symbols.module_file_paths.contains(&helper_path),
        "dependency-only files must contribute to module_file_paths"
    );
    assert!(
        module_symbols.module_file_paths.contains(&page_path),
        "entry files must contribute to module_file_paths"
    );

    let helper_dependencies = module_symbols
        .file_dependency_clauses_by_source
        .get(&helper_path)
        .expect("dependency-only file clauses must be registered");

    assert_eq!(helper_dependencies.len(), 1);
    assert_eq!(
        helper_dependencies[0]
            .dependency
            .path
            .to_portable_string(&string_table),
        "core/math"
    );
    assert_eq!(
        helper_dependencies[0].export_mode,
        crate::compiler_frontend::headers::types::HeaderExportMode::Private
    );
}

#[test]
fn per_file_prepare_output_preserves_file_role_and_dependencies_on_output() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "@core/math\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::Normal);
    assert_eq!(output.file_dependency_clauses.len(), 1);
    assert_eq!(
        output.file_dependency_clauses[0]
            .dependency
            .path
            .to_portable_string(&string_table),
        "core/math"
    );
}

#[test]
fn retained_js_provider_path_records_external_target() {
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        "@drawing.js as drawing\n",
        &PathBuf::from("src/@page.moth"),
        &PathBuf::from("src/@page.moth"),
        &mut string_table,
    );
    assert_eq!(output.file_dependency_clauses.len(), 1);
    match &output.file_dependency_clauses[0].dependency.target {
        crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::ExternalProvider {
            prefix_component_count,
            extension,
        } => {
            assert_eq!(*prefix_component_count, 1);
            assert_eq!(string_table.resolve(*extension), "js");
        }
        other => panic!("expected an external provider target, got {other:?}"),
    }
}

#[test]
fn explicit_extension_provider_requires_alias_or_selection() {
    let result =
        parse_single_file_headers_with_entry("@drawing.js\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "bare provider clauses should be rejected");
    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidDependencyClause {
                reason: InvalidDependencyClauseReason::ProviderRequiresBinding,
                ..
            }
        )
    }));
}

#[test]
fn dependency_clause_is_rejected_in_config_source() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("config.moth");
    let options = HeaderParseOptions {
        entry_file_id: None,
        project_path_resolver: None,
        entry_file_role: None,
        active_root_role: ModuleRootRole::Normal,
    };
    let style_directives = StyleDirectiveRegistry::built_ins();
    let context = HeaderTestPrepareContext {
        source_id: SourceId::COMPILATION_ROOT,
        entry_file_path: &file_path,
        options: &options,
        style_directives: &style_directives,
    };
    let mut span_builder = ExtendedSpanBuilder::new();
    let error = match prepare_test_source_file(
        "@core/math sin\n",
        &file_path,
        &context,
        &mut string_table,
        0,
        0,
        &mut span_builder,
    ) {
        Ok(_) => panic!("config dependency clause should be rejected"),
        Err(error) => error,
    };
    let FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError { diagnostic, .. }) = error
    else {
        panic!("config dependency rejection must use a source diagnostic");
    };
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::InvalidDependencyClause {
            reason: InvalidDependencyClauseReason::DependencyClauseNotAllowed,
            ..
        }
    ));
}

#[test]
fn retained_dependency_shells_get_deterministic_ordinals_per_authored_clause() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "@one a\n@two\nexport:\n    @one a\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    assert_eq!(
        output.file_dependency_clauses.len(),
        3,
        "every authored clause must keep its own retained shell even when it repeats a path"
    );

    let direct_selection = &output.file_dependency_clauses[0];
    let direct_selections = direct_selection
        .selections(&output.dependency_selections)
        .expect("direct-selection clause range should be valid");
    assert_eq!(direct_selection.dependency.dependency_shell_id.ordinal, 0);
    assert_eq!(direct_selections.len(), 1);
    assert_eq!(direct_selection.export_mode, HeaderExportMode::Private);

    let bare = &output.file_dependency_clauses[1];
    assert_eq!(bare.dependency.dependency_shell_id.ordinal, 1);
    assert!(
        bare.selections(&output.dependency_selections)
            .expect("bare clause range should be valid")
            .is_empty()
    );
    assert_eq!(bare.export_mode, HeaderExportMode::Private);

    let public = &output.file_dependency_clauses[2];
    let public_selections = public
        .selections(&output.dependency_selections)
        .expect("public clause range should be valid");
    assert_eq!(public.dependency.dependency_shell_id.ordinal, 2);
    assert_eq!(public_selections.len(), 1);
    assert_eq!(
        public.export_mode,
        HeaderExportMode::Public,
        "the repeated public re-export keeps its own retained shell and visibility"
    );
}

#[test]
fn direct_selection_and_namespace_clauses_keep_provider_root_and_selection_shape() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "@one a\n@one/a\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    assert_eq!(
        output.file_dependency_clauses.len(),
        2,
        "direct-selection and namespace clauses that flatten to the same path must both be retained"
    );

    let direct_selection = &output.file_dependency_clauses[0];
    let direct_selections = direct_selection
        .selections(&output.dependency_selections)
        .expect("direct-selection clause range should be valid");
    assert_eq!(direct_selection.dependency.dependency_shell_id.ordinal, 0);
    assert_eq!(
        direct_selection
            .dependency
            .path
            .to_portable_string(&string_table),
        "one"
    );
    assert_eq!(string_table.resolve(direct_selections[0].source_name), "a");

    let bare = &output.file_dependency_clauses[1];
    assert!(
        bare.selections(&output.dependency_selections)
            .expect("bare clause range should be valid")
            .is_empty(),
        "the bare clause is a namespace binding"
    );
    assert_eq!(bare.dependency.dependency_shell_id.ordinal, 1);
    assert_eq!(
        bare.dependency.path.to_portable_string(&string_table),
        "one/a"
    );
    assert_ne!(
        direct_selection.dependency.span, bare.dependency.span,
        "each authored occurrence keeps its own source span"
    );
}

#[test]
fn imported_module_root_prepare_output_has_imported_root_role() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@mod.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "Button = | label String |\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::ImportedModuleRoot);
    assert!(output.file_dependency_clauses.is_empty());
}

#[test]
fn entry_normal_module_root_file_is_assigned_active_module_root_role() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "greeting #= \"hello\"\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::ActiveModuleRoot);
}

#[test]
fn api_only_active_roots_export_declarations_without_synthesizing_start() {
    for root_role in [
        ModuleRootRole::Support,
        ModuleRootRole::ProjectPackageFacade,
    ] {
        let mut string_table = StringTable::new();
        let file_path = PathBuf::from("src/styles/+package.moth");
        let mut span_builder = ExtendedSpanBuilder::new();
        let output = prepare_active_root_with_role(
            "export:\n    theme #= \"dark\"\n;\n",
            &file_path,
            root_role,
            &mut string_table,
            &mut span_builder,
        )
        .expect("API-only roots should accept public declarations");

        assert_eq!(output.file_role, FileRole::ActiveApiOnlyModuleRoot);
        assert!(output.headers.iter().any(|header| {
            matches!(header.kind, HeaderKind::Constant { .. })
                && header.export_mode == HeaderExportMode::Public
        }));
        assert!(
            output
                .headers
                .iter()
                .all(|header| !matches!(header.kind, HeaderKind::StartFunction)),
            "API-only roots must not synthesize start"
        );
        assert_eq!(output.runtime_fragment_count, 0);
        assert_eq!(output.const_template_count, 0);
        assert!(!output.has_non_trivial_root_body);
    }
}

#[test]
fn api_only_active_roots_reject_every_root_activity_form() {
    for root_role in [
        ModuleRootRole::Support,
        ModuleRootRole::ProjectPackageFacade,
    ] {
        for source in ["value = 1\n", "[3]\n", "#[3]\n"] {
            let mut string_table = StringTable::new();
            let file_path = PathBuf::from("src/styles/+package.moth");
            let mut span_builder = ExtendedSpanBuilder::new();
            let diagnostic = match prepare_active_root_with_role(
                source,
                &file_path,
                root_role,
                &mut string_table,
                &mut span_builder,
            ) {
                Ok(_) => panic!("API-only root activity should be rejected"),
                Err(FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
                    diagnostic,
                    ..
                })) => diagnostic,
                Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                    panic!(
                        "API-only root source rejection became infrastructure failure: {error:?}"
                    )
                }
            };

            assert_eq!(
                diagnostic.kind,
                DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTopLevelRuntimeStatement)
            );
        }
    }
}

#[test]
fn support_package_root_file_is_assigned_imported_module_root_role() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/styles/+package.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "theme #= \"dark\"\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::ImportedModuleRoot);
}

#[test]
fn ordinary_source_file_is_assigned_normal_role() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "value #= 1\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::Normal);
}

#[test]
fn support_package_root_file_accepts_an_export_block() {
    let headers = parse_single_file_headers_with_entry(
        "export:\n    Button = | label String |\n;\n",
        "src/styles/+package.moth",
        "src/@page.moth",
    )
    .expect("a `+*.moth` support-package root should be export-capable");

    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Struct { .. })
            && header.export_mode == HeaderExportMode::Public
    }));
}
