use super::*;

#[test]
fn legacy_import_clause_reports_dedicated_migration_with_flat_replacements() {
    let cases = [
        ("import @core/math\n", "@core/math", "import @core/math"),
        (
            "import @core/math as maths\n",
            "@core/math as maths",
            "import @core/math as maths",
        ),
        (
            "import @core/math { sin, cos }\n",
            "@core/math sin, cos",
            "import @core/math { sin, cos }",
        ),
        (
            "export:\n    import @core/math { sin }\n;\n",
            "@core/math sin",
            "import @core/math { sin }",
        ),
    ];

    for (source, expected_replacement, expected_clause) in cases {
        let result =
            parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
        let errors = expect_header_error(result, "legacy import syntax should be diagnosed");
        let diagnostic = &errors.diagnostics[0];
        assert_eq!(
            diagnostic.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::LegacyDependencyClause)
        );
        let DiagnosticPayload::LegacyDependencyClause {
            replacement: Some(replacement),
            ..
        } = diagnostic.payload
        else {
            panic!("expected a semantics-preserving migration replacement");
        };
        assert_eq!(
            errors.string_table.resolve(replacement),
            expected_replacement
        );
        assert_eq!(
            diagnostic.primary_span,
            Some(source_span_for(source, expected_clause, 0)),
            "legacy dependency diagnostic should cover the authored clause",
        );
    }
}

#[test]
fn legacy_filtered_or_nested_import_has_no_automatic_replacement() {
    for source in [
        "import @core/math as maths { sin }\n",
        "import @html { tables { row } }\n",
    ] {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth"),
            "ambiguous legacy import syntax should be diagnosed",
        );
        assert!(matches!(
            errors.diagnostics[0].payload,
            DiagnosticPayload::LegacyDependencyClause {
                replacement: None,
                ..
            }
        ));
    }
}

#[test]
fn legacy_quoted_path_and_config_import_have_no_automatic_replacement() {
    for (source, file_path) in [
        ("import @docs/\"my file.md\"\n", "src/@page.moth"),
        ("import @\"@tools\"\n", "src/@page.moth"),
        ("import @\"semi;colon\"\n", "src/@page.moth"),
        ("import @/\n", "src/@page.moth"),
        ("import @core/math\n", "config.moth"),
    ] {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, file_path, file_path),
            "legacy syntax without a safe current clause should still be diagnosed",
        );
        assert!(matches!(
            errors.diagnostics[0].payload,
            DiagnosticPayload::LegacyDependencyClause {
                replacement: None,
                ..
            }
        ));
    }
}

#[test]
fn legacy_multiline_import_clause_reports_migration_with_flat_replacement() {
    let cases = [
        ("import\n\n\n    @core/math { sin }\n", "@core/math sin"),
        (
            "export:\n    import\n        @core/math { sin }\n;\n",
            "@core/math sin",
        ),
        ("import\n    @core/math as maths\n", "@core/math as maths"),
        (
            "import\n    @core/math { sin as sine }\n",
            "@core/math sin as sine",
        ),
    ];

    for (source, expected_replacement) in cases {
        let result =
            parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
        let errors =
            expect_header_error(result, "legacy multiline import syntax should be diagnosed");
        let diagnostic = &errors.diagnostics[0];
        assert_eq!(
            diagnostic.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::LegacyDependencyClause)
        );
        let DiagnosticPayload::LegacyDependencyClause {
            replacement: Some(replacement),
            ..
        } = diagnostic.payload
        else {
            panic!("expected a semantics-preserving migration replacement for: {source}");
        };
        assert_eq!(
            errors.string_table.resolve(replacement),
            expected_replacement
        );
    }
}

#[test]
fn legacy_dependency_comment_between_keyword_and_path_reports_migration() {
    let source = "import -- keep the old clause visible\n    @core/math { sin }\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth"),
        "a comment between import and the path must still be a legacy clause",
    );
    let DiagnosticPayload::LegacyDependencyClause {
        replacement: Some(replacement),
        ..
    } = errors.diagnostics[0].payload
    else {
        panic!("expected a replacement after comment trivia");
    };
    assert_eq!(errors.string_table.resolve(replacement), "@core/math sin");
    assert_eq!(
        errors.diagnostics[0].primary_span,
        Some(source_span_for(source, source.trim_end(), 0)),
        "comment-separated legacy clause should cover the authored clause",
    );
}

#[test]
fn legacy_dependency_span_covers_import_through_closing_brace() {
    let source = "import @core/math { sin }\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth"),
        "legacy import syntax should be diagnosed",
    );
    let close_brace = source
        .find('}')
        .expect("the fixture must include a closing brace");
    let mut span_builder = ExtendedSpanBuilder::new();
    let expected_span = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(0, (close_brace + 1) as u32, &mut span_builder)
            .expect("legacy clause span should fit"),
    );
    assert_eq!(
        errors.diagnostics[0].primary_span,
        Some(expected_span),
        "the primary span must end at the closing brace"
    );
}

#[test]
fn import_followed_by_unrelated_newline_statement_is_not_legacy_clause() {
    let headers = parse_single_file_headers("import\nvalue = 1\n");
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::StartFunction)),
        "import followed by an ordinary statement must not be treated as a legacy clause"
    );
}

#[test]
fn import_is_an_ordinary_identifier() {
    let headers = parse_single_file_headers("import = 1\n");
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::StartFunction))
    );
}

#[test]
fn import_is_a_valid_struct_identifier() {
    let headers = parse_single_file_headers("Import = | source String |\n");
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Struct { .. })),
        "Import remains available as an ordinary struct identifier"
    );
}

#[test]
fn legacy_joined_clause_span_keeps_full_multibyte_extended_range() {
    let comment = "é".repeat(700);
    let source = format!("-- 🦋\nimport -- {comment}\n    @core/math {{ sin }}\n");
    let mut strings = StringTable::new();
    let mut spans = ExtendedSpanBuilder::new();
    let options = HeaderParseOptions::default();
    let styles = StyleDirectiveRegistry::built_ins();
    let path = Path::new("src/@page.moth");
    let context = HeaderTestPrepareContext {
        source_id: SourceId::COMPILATION_ROOT,
        entry_file_path: path,
        options: &options,
        style_directives: &styles,
    };
    let failure = match prepare_test_source_file(
        &source,
        path,
        &HeaderTestPrepareContext {
            source_id: SourceId::from_index(3),
            ..context
        },
        &mut strings,
        0,
        0,
        &mut spans,
    ) {
        Ok(_) => panic!("legacy joined clause should be diagnosed"),
        Err(failure) => failure,
    };
    let FileFrontendPrepareFailure::Diagnosed(error) = failure else {
        panic!("expected source diagnostic");
    };
    let span = error
        .diagnostic
        .primary_span
        .expect("per-file producer captures joined clause");
    let range = span.resolve_with(spans.resolver_for(span.source()));
    assert_eq!(span.source(), SourceId::from_index(3));
    assert_eq!(range.start() as usize, source.find("import").unwrap());
    assert_eq!(range.end() as usize, source.find('}').unwrap() + 1);
    assert!(range.end() - range.start() > 1022);
}
