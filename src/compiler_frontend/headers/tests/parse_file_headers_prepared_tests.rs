use super::*;

/// Extended token spans keep their source-owned table through aggregation. Later span producers
/// may append to that same builder without invalidating the retained header's earlier handles.
#[test]
fn prepared_output_keeps_the_span_table_its_retained_tokens_index() {
    let quoted = "x".repeat(1500);
    let source = format!("value = \"{quoted}\"\n");
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let mut span_builder = ExtendedSpanBuilder::new();
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let file_tokens = tokenize(
        &source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    let mut outputs = [prepare_file_from_tokens(
        file_tokens,
        &file_path,
        &options,
        &mut string_table,
        0,
        0,
        &mut span_builder,
    )
    .expect("preparation should succeed")];
    let prepared = prepare_header_syntax(
        &mut outputs,
        &mut string_table,
        &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
    )
    .expect("header syntax should aggregate");
    let whole_source = LocalSpan::exact(0, source.len() as u32, &mut span_builder)
        .expect("a later source span should fit");
    let resolver = span_builder.resolver();
    let literal = prepared
        .headers
        .iter()
        .flat_map(|header| header.tokens.tokens.iter())
        .find(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))
        .expect("the retained header must keep its long string literal");
    let resolved = literal.span.resolve_with(resolver);

    assert_eq!(
        source.get(resolved.start() as usize..resolved.end() as usize),
        Some(format!("\"{quoted}\"").as_str()),
        "the retained token's span must resolve through the prepared output's own table"
    );
    let whole_range = whole_source.resolve_with(resolver);
    assert_eq!(whole_range.start(), 0);
    assert_eq!(whole_range.end(), source.len() as u32);
}

#[test]
fn diagnosed_aggregation_preserves_the_source_span_builder() {
    let quoted = "é".repeat(2 * 1024 * 1024);
    let source =
        format!("helper ||:\n    value = \"{quoted}\"\n    setting #Config of Int = 1\n;\n");
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let mut span_builder = ExtendedSpanBuilder::new();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let file_tokens = tokenize(
        &source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    let mut outputs = [prepare_file_from_tokens(
        file_tokens,
        &file_path,
        &HeaderParseOptions::default(),
        &mut string_table,
        0,
        0,
        &mut span_builder,
    )
    .expect("preparation should succeed")];
    let literal_span = outputs[0]
        .headers
        .iter()
        .flat_map(|header| header.tokens.tokens.iter())
        .find(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))
        .expect("the function body should retain its long literal")
        .span;

    let diagnostics = match prepare_header_syntax(
        &mut outputs,
        &mut string_table,
        &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
    ) {
        Err(failure) => expect_aggregation_diagnostics(failure),
        Ok(_) => panic!("a nested config qualifier should fail aggregation"),
    };
    assert!(diagnostics.errors().any(|diagnostic| matches!(
        &diagnostic.payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::ConfigQualifierInvalidPlacement,
            ..
        }
    )));
    let primary_span = diagnostics
        .errors()
        .next()
        .expect("config placement diagnostic")
        .primary_span
        .expect("aggregation captures its primary span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let marker_range =
        primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert!(marker_range.start() >= 4 * 1024 * 1024);
    assert_eq!(
        &source[marker_range.start() as usize..marker_range.end() as usize],
        "#"
    );
    let whole_source = LocalSpan::exact(0, source.len() as u32, &mut span_builder)
        .expect("the diagnosed source builder should remain live");
    let resolver = span_builder.resolver();
    let literal_range = literal_span.resolve_with(resolver);
    assert_eq!(
        source.get(literal_range.start() as usize..literal_range.end() as usize),
        Some(format!("\"{quoted}\"").as_str())
    );
    let whole_range = whole_source.resolve_with(resolver);
    assert_eq!(whole_range.start(), 0);
    assert_eq!(whole_range.end(), source.len() as u32);
}

#[test]
fn aggregation_diagnostics_keep_distinct_source_ids_in_authored_order() {
    let sources = [
        "helper ||:\n setting #Config of Int = 1\n;\n",
        "other ||:\n setting #Config of Int = 2\n;\n",
    ];
    let mut string_table = StringTable::new();
    let options = HeaderParseOptions::default();
    let styles = StyleDirectiveRegistry::built_ins();
    let entry = Path::new("src/@page.moth");
    let context = HeaderTestPrepareContext {
        source_id: SourceId::COMPILATION_ROOT,
        entry_file_path: entry,
        options: &options,
        style_directives: &styles,
    };
    let mut builders = [ExtendedSpanBuilder::new(), ExtendedSpanBuilder::new()];
    let mut outputs = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        outputs.push(
            prepare_test_source_file(
                source,
                Path::new(if index == 0 {
                    "src/@page.moth"
                } else {
                    "src/helper.moth"
                }),
                &HeaderTestPrepareContext {
                    source_id: SourceId::from_index(index + 1),
                    ..context
                },
                &mut string_table,
                0,
                0,
                &mut builders[index],
            )
            .expect("declaration shells prepare"),
        );
    }
    let failure = prepare_header_syntax(
        &mut outputs,
        &mut string_table,
        &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
    )
    .err()
    .expect("nested qualifiers fail aggregation");
    let diagnostics = expect_aggregation_diagnostics(failure).into_diagnostics();
    assert_eq!(diagnostics.len(), 2);
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        let span = diagnostic
            .primary_span
            .expect("aggregation retains primary source span");
        assert_eq!(span.source(), SourceId::from_index(index + 1));
        let range = span.resolve_with(builders[index].resolver_for(span.source()));
        assert_eq!(
            &sources[index][range.start() as usize..range.end() as usize],
            "#"
        );
    }
}

#[test]
fn preparation_related_labels_keep_the_continuation_comma_and_name() {
    let source = "-- é🦋\n@core/math sin,\nvalue = 1\n";
    let mut strings = StringTable::new();
    let mut spans = ExtendedSpanBuilder::new();
    let options = HeaderParseOptions::default();
    let styles = StyleDirectiveRegistry::built_ins();
    let path = Path::new("src/@page.moth");
    let context = HeaderTestPrepareContext {
        source_id: SourceId::from_index(4),
        entry_file_path: path,
        options: &options,
        style_directives: &styles,
    };
    let failure = prepare_test_source_file(source, path, &context, &mut strings, 0, 0, &mut spans)
        .err()
        .expect("continued clause must diagnose statement");
    let FileFrontendPrepareFailure::Diagnosed(error) = failure else {
        panic!("expected source diagnostic");
    };
    let primary_span = error
        .diagnostic
        .primary_span
        .expect("per-file preparation captures the primary name span");
    assert_eq!(primary_span.source(), context.source_id);
    let primary_range = primary_span.resolve_with(spans.resolver_for(primary_span.source()));
    assert_eq!(
        &source[primary_range.start() as usize..primary_range.end() as usize],
        "value"
    );
    assert_eq!(error.diagnostic.labels.len(), 1);
    let comma_label = &error.diagnostic.labels[0];
    let span = comma_label
        .span
        .expect("per-file preparation captures related labels");
    assert_eq!(span.source(), context.source_id);
    let range = span.resolve_with(spans.resolver_for(span.source()));
    assert_eq!(&source[range.start() as usize..range.end() as usize], ",");
}

#[test]
fn diagnosed_header_failure_keeps_source_identity_and_extended_span_owner() {
    let quoted = "x".repeat(1500);
    let source = format!("value = \"{quoted}\"\nexport:\n;\n");
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let source_id = SourceId::from_index(7);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut file_tokens = tokenize(
        &source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        source_id,
        &mut span_builder,
    )
    .expect("source should tokenize");
    let long_span = file_tokens
        .tokens
        .iter()
        .find(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))
        .expect("tokenized source should retain the long string literal")
        .span;
    let expected_range = long_span.resolve_with(span_builder.resolver_for(source_id));

    let error = match parse_file_headers_with_table(
        &mut file_tokens,
        &file_path,
        &HeaderParseOptions::default(),
        &mut string_table,
        0,
        0,
        &mut span_builder,
    ) {
        Err(FileFrontendPrepareFailure::Diagnosed(error)) => error,
        Ok(_) => panic!("an empty export block should diagnose during header preparation"),
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
            panic!("header syntax should diagnose, not fail infrastructure: {error:?}")
        }
    };

    assert!(
        error.warnings.is_empty(),
        "empty export block should not emit preparation warnings"
    );
    let diagnostic_span = error
        .diagnostic
        .primary_span
        .expect("empty export block should retain its source span");
    assert_eq!(diagnostic_span.source(), source_id);
    let diagnostic_range =
        diagnostic_span.resolve_with(span_builder.resolver_for(diagnostic_span.source()));
    assert_eq!(
        source.get(diagnostic_range.start() as usize..diagnostic_range.end() as usize),
        Some("export"),
        "the empty export diagnostic should cover the authored export keyword"
    );
    let retained_range = long_span.resolve_with(span_builder.resolver_for(source_id));
    assert_eq!(retained_range, expected_range);
    assert_eq!(
        source.get(retained_range.start() as usize..retained_range.end() as usize),
        Some(format!("\"{quoted}\"").as_str()),
        "the diagnosed header must retain the builder that owns its extended token span"
    );
}

#[test]
fn dependency_shell_with_compilation_root_identity_prepares() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = tokenize(
        "@core/math\n",
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");

    let output = prepare_file_from_tokens(
        file_tokens,
        &file_path,
        &HeaderParseOptions::default(),
        &mut string_table,
        0,
        0,
        &mut span_builder,
    )
    .expect("a dependency shell with a compilation-root identity should prepare");

    assert_eq!(output.file_id, SourceId::COMPILATION_ROOT);
    assert_eq!(
        output.file_dependency_clauses[0]
            .dependency
            .dependency_shell_id
            .source,
        SourceId::COMPILATION_ROOT
    );
}
