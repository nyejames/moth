use super::*;

#[test]
fn one_dependency_shell_and_selection_list_per_authored_clause() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "@core/math sin, cos as cosine\n@core/io\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    let clauses = &output.file_dependency_clauses;
    assert_eq!(
        clauses.len(),
        2,
        "the direct-selection clause owns both selections and the simple clause owns its namespace"
    );

    // Both selections of the direct-selection clause share one authored-clause shell.
    let selection_shell = clauses[0].dependency.dependency_shell_id;
    let direct_selections = clauses[0]
        .selections(&output.dependency_selections)
        .expect("direct-selection clause range should be valid");
    assert_eq!(direct_selections.len(), 2);
    assert_ne!(clauses[0].dependency.path, PathId::ROOT);
    assert_eq!(
        string_table.resolve(direct_selections[0].source_name),
        "sin"
    );
    assert_eq!(
        string_table.resolve(direct_selections[1].source_name),
        "cos"
    );

    // The next authored clause receives the next shell ordinal regardless of the
    // clause's selected-name count.
    let simple = &clauses[1];
    assert_eq!(
        simple.dependency.dependency_shell_id.source,
        selection_shell.source
    );
    assert_eq!(simple.dependency.dependency_shell_id.ordinal, 1);
    assert!(
        simple
            .selections(&output.dependency_selections)
            .expect("simple clause range should be valid")
            .is_empty()
    );
    assert_ne!(simple.dependency.path, PathId::ROOT);
}

#[test]
fn selected_name_duplicate_declaration_preserves_both_exact_spans() {
    let source = "@core/math sin\nsin #= 1\n";
    let result = parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "a selected name must conflict with a declaration");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let previous_span = diagnostic
        .labels
        .iter()
        .find(|label| label.message.as_ref() == Some(&DiagnosticLabelMessage::PreviousDeclaration))
        .and_then(|label| label.span)
        .expect("expected the selected name's previous span");
    assert_eq!(previous_span, source_span_for(source, "sin", 0));
    assert_eq!(
        diagnostic.primary_span,
        Some(source_span_for(source, "sin", 1))
    );
}

#[test]
fn selected_alias_duplicate_declaration_uses_the_alias_span() {
    let source = "@core/math sin as local\nlocal #= 1\n";
    let result = parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "a selected alias must conflict with a declaration");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let previous_span = diagnostic
        .labels
        .iter()
        .find(|label| label.message.as_ref() == Some(&DiagnosticLabelMessage::PreviousDeclaration))
        .and_then(|label| label.span)
        .expect("expected the selected alias's previous span");
    assert_eq!(previous_span, source_span_for(source, "local", 0));
    assert_eq!(
        diagnostic.primary_span,
        Some(source_span_for(source, "local", 1))
    );
}

#[test]
fn declaration_followed_by_selection_preserves_declaration_and_selection_spans() {
    let source = "line #= 1\n@core/io line\n";
    let result = parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "a selection must conflict with a declaration");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::ImportNameCollision { .. }
            )
        })
        .unwrap_or_else(|| {
            panic!(
                "expected dependency-name collision diagnostic: {:?}",
                errors.diagnostics
            )
        });

    let previous_span = diagnostic
        .labels
        .iter()
        .find(|label| label.message.as_ref() == Some(&DiagnosticLabelMessage::PreviousDeclaration))
        .and_then(|label| label.span)
        .expect("expected the declaration's previous span");
    assert_eq!(previous_span, source_span_for(source, "line", 0));
    assert_eq!(
        diagnostic.primary_span,
        Some(source_span_for(source, "line", 1))
    );
}

#[test]
fn duplicate_selected_aliases_preserve_first_and_current_alias_spans() {
    let source = "@core/io line as value\n@core/io debug as value\n";
    let result = parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "duplicate selected aliases must conflict");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::ImportNameCollision { .. }
            )
        })
        .unwrap_or_else(|| {
            panic!(
                "expected dependency-name collision diagnostic: {:?}",
                errors.diagnostics
            )
        });

    let previous_span = diagnostic
        .labels
        .iter()
        .find(|label| label.message.as_ref() == Some(&DiagnosticLabelMessage::PreviousDeclaration))
        .and_then(|label| label.span)
        .expect("expected the first selected alias's previous span");
    assert_eq!(previous_span, source_span_for(source, "value", 0));
    assert_eq!(
        diagnostic.primary_span,
        Some(source_span_for(source, "value", 1))
    );
}

#[test]
fn direct_selection_empty_range_is_rejected_in_the_internal_error_lane() {
    let clause = malformed_direct_selection_clause(DependencySelectionRange::new(0, 0));
    let error = clause
        .selections(&[])
        .expect_err("empty direct-selection ranges must not become namespace bindings");
    assert!(error.msg.contains("empty selection range"));
}

#[test]
fn direct_selection_reversed_range_is_rejected_in_the_internal_error_lane() {
    let clause = malformed_direct_selection_clause(DependencySelectionRange::new(2, 1));
    let error = clause
        .selections(&[])
        .expect_err("reversed direct-selection ranges must fail closed");
    assert!(error.msg.contains("outside a table"));
}

#[test]
fn direct_selection_out_of_bounds_range_is_rejected_in_the_internal_error_lane() {
    let clause = malformed_direct_selection_clause(DependencySelectionRange::new(0, 1));
    let error = clause
        .selections(&[])
        .expect_err("out-of-bounds direct-selection ranges must fail closed");
    assert!(error.msg.contains("outside a table"));
}

#[test]
fn namespace_alias_duplicate_declaration_uses_the_alias_span() {
    let source = "@core/io as io\nio #= 1\n";
    let result = parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "a namespace alias must conflict with a declaration");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let previous_span = diagnostic
        .labels
        .iter()
        .find(|label| label.message.as_ref() == Some(&DiagnosticLabelMessage::PreviousDeclaration))
        .and_then(|label| label.span)
        .expect("expected the namespace alias's previous span");
    assert_eq!(previous_span, source_span_for(source, "io", 1));
    assert_eq!(
        diagnostic.primary_span,
        Some(source_span_for(source, "io", 2))
    );
}

#[test]
fn inferred_namespace_duplicate_declaration_uses_the_provider_path_span() {
    let source = "@core/io\nio #= 1\n";
    let result = parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(
        result,
        "an inferred namespace name must conflict with a declaration",
    );
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let previous_span = diagnostic
        .labels
        .iter()
        .find(|label| label.message.as_ref() == Some(&DiagnosticLabelMessage::PreviousDeclaration))
        .and_then(|label| label.span)
        .expect("expected the provider path's previous span");
    assert_eq!(previous_span, source_span_for(source, "@core/io", 0));
    assert_eq!(
        diagnostic.primary_span,
        Some(source_span_for(source, "io", 1))
    );
}

#[test]
fn inferred_namespace_provider_path_span_excludes_trailing_whitespace() {
    let source = "@core/io   \nio #= 1\n";
    let result = parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(
        result,
        "trailing whitespace must not enter the inferred namespace path span",
    );
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let previous_span = diagnostic
        .labels
        .iter()
        .find(|label| label.message.as_ref() == Some(&DiagnosticLabelMessage::PreviousDeclaration))
        .and_then(|label| label.span)
        .expect("expected the inferred namespace path's previous span");
    assert_eq!(previous_span, source_span_for(source, "@core/io", 0));
}

#[test]
fn retained_clause_uses_one_shell_for_the_provider_binding_index() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "@drawing.js draw, clear\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    let clauses = &output.file_dependency_clauses;
    assert_eq!(clauses.len(), 1);
    let shell = clauses[0].dependency.dependency_shell_id;
    assert!(
        clauses
            .iter()
            .all(|clause| clause.dependency.dependency_shell_id == shell)
    );
}
