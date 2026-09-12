use super::*;

// ------------------------------
//  Export block parsing tests
// ------------------------------

#[test]
fn export_alone_is_rejected() {
    let result =
        parse_single_file_headers_with_entry("export\n", "src/@mod.moth", "src/@page.moth");
    let errors = expect_header_error(result, "export without a block colon should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::ExpectedToken { expected, .. }
                if expected == DiagnosticToken::from(TokenKind::Colon)
        )
    }));
}

#[test]
fn legacy_inline_export_declaration_is_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export Button = | label String |\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "legacy inline export should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::ExpectedToken { expected, .. }
                if expected == DiagnosticToken::from(TokenKind::Colon)
        )
    }));
}

#[test]
fn export_dependency_path_parsed_as_public_surface_dependency() {
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        "export:\n    @button Button\n;\n",
        &PathBuf::from("src/@mod.moth"),
        &PathBuf::from("src/@page.moth"),
        &mut string_table,
    );

    assert_eq!(output.file_dependency_clauses.len(), 1);
    assert_eq!(
        output.file_dependency_clauses[0].export_mode,
        HeaderExportMode::Public
    );
    assert_ne!(
        output.file_dependency_clauses[0].dependency.path,
        PathId::ROOT
    );
    let selections = output.file_dependency_clauses[0]
        .selections(&output.dependency_selections)
        .expect("retained public export selection range should be valid");
    assert_eq!(selections.len(), 1);
    assert_eq!(string_table.resolve(selections[0].source_name), "Button");
}

#[test]
fn export_block_accepts_an_item_without_a_following_newline() {
    let headers = parse_single_file_headers_with_entry(
        "export: Button = | label String |\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    )
    .expect("export block should accept its first item after the colon");

    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Struct { .. })
            && header.export_mode == HeaderExportMode::Public
    }));
}

#[test]
fn legacy_export_path_syntax_is_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export @card Card, render as render_card\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "legacy export path syntax should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::ExpectedToken { expected, .. }
                if expected == DiagnosticToken::from(TokenKind::Colon)
        )
    }));
}

#[test]
fn export_bare_path_rejected_as_deferred_namespace_export() {
    let result =
        parse_single_file_headers_with_entry("export @layout\n", "src/@mod.moth", "src/@page.moth");
    let errors = expect_header_error(
        result,
        "bare namespace export should be rejected as deferred",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::ExpectedToken { expected, .. }
                if expected == DiagnosticToken::from(TokenKind::Colon)
        )
    }));
}

#[test]
fn export_before_authored_declaration_marks_header_public() {
    let source = "export:\n    Button = | label String |\n    render |button Button| -> String:\n        return button.label\n    ;\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let public_headers: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| header.export_mode == HeaderExportMode::Public)
        .collect();

    assert_eq!(
        public_headers.len(),
        2,
        "expected two public headers: struct and function"
    );
}

#[test]
fn unmarked_authored_declarations_in_module_root_remain_private() {
    let source = "Button = | label String |\nrender |button Button| -> String:\n    return button.label\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let non_start_headers: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .collect();

    assert!(
        non_start_headers
            .iter()
            .all(|header| header.export_mode == HeaderExportMode::Private),
        "unmarked declarations in a module root should remain private"
    );
}

#[test]
fn duplicate_declaration_detection_works_with_exported_declarations() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    Button = | label String |\n;\nButton = | title String |\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(
        result,
        "duplicate declaration with export should still be rejected",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::DuplicateDeclaration { .. }
    )));
}

#[test]
fn export_before_constant_marks_header_public() {
    let source = "export:\n    theme #= \"dark\"\n    threshold #Int = 42\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let public_constants: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| {
            matches!(header.kind, HeaderKind::Constant { .. })
                && header.export_mode == HeaderExportMode::Public
        })
        .collect();

    assert_eq!(
        public_constants.len(),
        2,
        "expected two public constant headers"
    );
}

#[test]
fn export_before_type_alias_marks_header_public() {
    let source = "export:\n    UserId as Int\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let alias_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::TypeAlias { .. }))
        .expect("expected type alias header");

    assert_eq!(alias_header.export_mode, HeaderExportMode::Public);
}

#[test]
fn export_before_choice_marks_header_public() {
    let source = "export:\n    Status :: Ready, Failed | message String |;\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let choice_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Choice { .. }))
        .expect("expected choice header");

    assert_eq!(choice_header.export_mode, HeaderExportMode::Public);
}

#[test]
fn export_before_trait_declaration_marks_header_public() {
    let source = "export:\n    DISPLAY_TEXT must:\n        display |This| -> String\n    ;\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let trait_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Trait { .. }))
        .expect("expected trait header");

    assert_eq!(trait_header.export_mode, HeaderExportMode::Public);
}

#[test]
fn export_before_runtime_template_is_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    [: hello ]\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "export before runtime template should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget)));
}

#[test]
fn public_dependency_and_private_dependency_keep_distinct_retained_shells() {
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        "@button Button\nexport:\n    @button Button\n;\n",
        &PathBuf::from("src/@mod.moth"),
        &PathBuf::from("src/@page.moth"),
        &mut string_table,
    );

    assert_eq!(
        output.file_dependency_clauses.len(),
        2,
        "the private dependency and the public re-export each retain their own authored clause"
    );
    assert_eq!(
        output.file_dependency_clauses[0].export_mode,
        HeaderExportMode::Private
    );
    assert_eq!(
        output.file_dependency_clauses[0]
            .dependency
            .dependency_shell_id
            .ordinal,
        0
    );
    assert_eq!(
        output.file_dependency_clauses[1].export_mode,
        HeaderExportMode::Public
    );
    assert_eq!(
        output.file_dependency_clauses[1]
            .dependency
            .dependency_shell_id
            .ordinal,
        1
    );
}

#[test]
fn capacity_references_extract_value_refs_without_treating_element_type_as_value_ref() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/test.moth");
    let source = "make |items ~{capacity MyType}| -> Int:
    return 1
;
";
    let (output, mut span_builder) =
        prepare_single_file(source, &file_path, &file_path, &mut string_table);

    let headers = prepare_and_bind_headers_result(
        vec![output],
        std::slice::from_mut(&mut span_builder),
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    )
    .expect("headers should parse");

    let make_header = headers
        .headers
        .iter()
        .find(|h| {
            matches!(h.kind, HeaderKind::Function { .. }) && h.tokens.src_path != PathId::ROOT
        })
        .expect("make header should exist");

    let capacity_names: Vec<_> = make_header
        .capacity_references
        .iter()
        .map(|r| string_table.resolve(r.name))
        .collect();

    assert!(
        capacity_names.contains(&"capacity"),
        "bare capacity syntax should reference the capacity constant"
    );
    assert!(
        !capacity_names.contains(&"MyType"),
        "element type name must not be treated as a capacity value reference"
    );
}
