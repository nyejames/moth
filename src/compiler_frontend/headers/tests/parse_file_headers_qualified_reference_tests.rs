//! Expression-qualified reference occurrence regression tests.
//!
//! WHAT: validates that ordinary `.moth` header preparation retains one occurrence per authored
//!       `PATH SYMBOL` pair, excludes its path row from file values and leaves top-level
//!       dependency clauses, explicit-extension file values and newline-separated symbols alone.
//! WHY: Stage 0 provider discovery, later semantic binding and AST lowering all consume this one
//!      fact. A missed occurrence loses a provider dependency; a false occurrence changes the
//!      meaning of existing file values.

use super::*;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::ExplicitMothExtensionView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::CommonSyntaxMistakeView;
use crate::compiler_frontend::headers::qualified_references::QualifiedReferenceOccurrence;
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass;

fn occurrences(output: &FileFrontendPrepareOutput) -> &[QualifiedReferenceOccurrence] {
    &output.qualified_references
}

fn constant_initializer_reference_names(
    output: &FileFrontendPrepareOutput,
    string_table: &StringTable,
) -> Vec<String> {
    output
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Constant { declaration } => Some(&declaration.initializer_references),
            _ => None,
        })
        .expect("fixture should prepare one constant header")
        .iter()
        .map(|reference| string_table.resolve(reference.name).to_owned())
        .collect()
}

#[test]
fn constant_initializer_records_one_occurrence_with_exact_spans() {
    let source = "value #= @math square(12)\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        source,
        &PathBuf::from("src/helper.moth"),
        &PathBuf::from("src/@entry.moth"),
        &mut string_table,
    );

    let occurrences = occurrences(&output);
    assert_eq!(occurrences.len(), 1, "one authored pair is one occurrence");
    let occurrence = &occurrences[0];
    assert_eq!(
        string_table.resolve(occurrence.selected),
        "square",
        "the selected symbol keeps its authored spelling"
    );
    assert_eq!(
        occurrence.dependency.span,
        source_span_for(source, "@math", 0)
    );
    assert_eq!(
        occurrence.selected_span,
        source_span_for(source, "square", 0)
    );
    assert_eq!(
        occurrence.dependency.dependency_shell_id.source,
        output.file_id
    );
    assert_eq!(
        occurrence.dependency.dependency_shell_id.ordinal, 0,
        "a file without dependency clauses starts its occurrence shells at ordinal zero"
    );
    assert!(
        output.file_dependency_clauses.is_empty(),
        "an expression occurrence is not a dependency clause"
    );
    assert!(
        output.structural_file_references.references().is_empty(),
        "the occurrence path must not also be filed as an extensionless file value"
    );
}

#[test]
fn quoted_path_component_and_tabs_record_one_occurrence() {
    let source = "value #= @vendor/\"my helper\"\t value\n";
    let mut string_table = StringTable::new();
    let (output, span_builder) = prepare_single_file(
        source,
        &PathBuf::from("src/helper.moth"),
        &PathBuf::from("src/@entry.moth"),
        &mut string_table,
    );

    let occurrences = occurrences(&output);
    assert_eq!(occurrences.len(), 1);
    assert_eq!(
        string_table.resolve(occurrences[0].selected),
        "value",
        "tab separation is the same-line whitespace separator"
    );
    let path_range = occurrences[0]
        .dependency
        .span
        .resolve_with(span_builder.resolver_for(output.file_id));
    assert_eq!(
        source.get(path_range.start() as usize..path_range.end() as usize),
        Some("@vendor/\"my helper\""),
        "the quoted path component stays inside the authored path token"
    );
}

#[test]
fn newline_between_path_and_symbol_records_no_occurrence() {
    let source = "f || -> I32:\n    @math\n    square\n;\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        source,
        &PathBuf::from("src/helper.moth"),
        &PathBuf::from("src/@entry.moth"),
        &mut string_table,
    );

    assert!(
        occurrences(&output).is_empty(),
        "a newline ends the selection: the next symbol is a separate token"
    );
}

#[test]
fn top_level_statement_path_stays_a_dependency_clause() {
    let source = "@math square\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        source,
        &PathBuf::from("src/@entry.moth"),
        &PathBuf::from("src/@entry.moth"),
        &mut string_table,
    );

    assert!(
        occurrences(&output).is_empty(),
        "statement-position paths keep their dependency-clause role"
    );
    assert_eq!(output.file_dependency_clauses.len(), 1);
    let selections = output.file_dependency_clauses[0]
        .selections(&output.dependency_selections)
        .expect("clause selection range should be valid");
    assert_eq!(selections.len(), 1);
    assert_eq!(string_table.resolve(selections[0].source_name), "square");
    assert_eq!(
        output.file_dependency_clauses[0]
            .dependency
            .dependency_shell_id
            .ordinal,
        0
    );
}

#[test]
fn content_source_file_value_without_a_selected_symbol_is_unchanged() {
    let source = "logo #= @docs/intro.mtf\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        source,
        &PathBuf::from("src/helper.moth"),
        &PathBuf::from("src/@entry.moth"),
        &mut string_table,
    );

    assert!(occurrences(&output).is_empty());
    let references = output.structural_file_references.references();
    assert_eq!(
        references.len(),
        1,
        "a bare content path stays one prepared file value"
    );
    assert_eq!(
        references[0].class,
        PreparedFileReferenceClass::ContentSource
    );
}

#[test]
fn explicit_extension_path_with_a_following_symbol_is_an_occurrence() {
    // A resource path is not a provider. Phase 1 records the authored shape; provider
    // validation diagnoses `@docs/intro.mtf content` later rather than silently reinterpreting
    // the file value as a namespace.
    let source = "logo #= @docs/intro.mtf content\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        source,
        &PathBuf::from("src/helper.moth"),
        &PathBuf::from("src/@entry.moth"),
        &mut string_table,
    );

    let occurrences = occurrences(&output);
    assert_eq!(occurrences.len(), 1);
    assert_eq!(string_table.resolve(occurrences[0].selected), "content");
    assert!(
        output.structural_file_references.references().is_empty(),
        "the occurrence owns its path row, so it is not also a content file value"
    );
}

#[test]
fn occurrence_shells_follow_clause_shells_without_reusing_an_ordinal() {
    let source = "@core/math sin\nvalue #= @math cos\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        source,
        &PathBuf::from("src/helper.moth"),
        &PathBuf::from("src/@entry.moth"),
        &mut string_table,
    );

    assert_eq!(output.file_dependency_clauses.len(), 1);
    assert_eq!(
        output.file_dependency_clauses[0]
            .dependency
            .dependency_shell_id
            .ordinal,
        0
    );
    let occurrence = occurrences(&output)
        .first()
        .expect("the initializer contributes one occurrence");
    assert_eq!(
        occurrence.dependency.dependency_shell_id.ordinal, 1,
        "occurrences allocate after every authored clause in the same file"
    );
    assert_eq!(
        occurrence.dependency.dependency_shell_id.source,
        output.file_dependency_clauses[0]
            .dependency
            .dependency_shell_id
            .source
    );
}

#[test]
fn selected_symbol_is_not_reported_as_an_initializer_reference() {
    let source = "value #= @math square(12) + other\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        source,
        &PathBuf::from("src/helper.moth"),
        &PathBuf::from("src/@entry.moth"),
        &mut string_table,
    );

    let reference_names = constant_initializer_reference_names(&output, &string_table);
    assert!(
        !reference_names.iter().any(|name| name == "square"),
        "the selected name belongs to the occurrence, not to local reference scanning: {reference_names:?}"
    );
    assert!(
        reference_names.iter().any(|name| name == "other"),
        "ordinary bare references in the same initializer stay visible: {reference_names:?}"
    );
}

#[test]
fn alias_after_a_selected_symbol_is_reported_at_the_alias() {
    let source = "value #= @math square as squared\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/helper.moth", "src/@entry.moth");
    let errors = expect_header_error(outcome, "an expression alias must be rejected");
    let diagnostic = find_diagnostic_family::<CommonSyntaxMistakeView>(&errors.diagnostics)
        .expect("expected the common-syntax-mistake diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0031");
    assert_eq!(
        diagnostic.reason_key(),
        Some("common_syntax_mistake.invalid_as_operator")
    );
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "as", 0)),
        "the illegal alias is labeled at its own token"
    );
}

#[test]
fn occurrence_paths_reuse_dependency_clause_path_diagnostics() {
    for (source, reason_key, path_text) in [
        (
            "value #= @/ name\n",
            "invalid_import_path.public_root",
            "@/",
        ),
        (
            "value #= @./x name\n",
            "invalid_import_path.current_directory_segment",
            "@./x",
        ),
        (
            "value #= @../x name\n",
            "invalid_import_path.parent_directory_segment",
            "@../x",
        ),
    ] {
        let outcome =
            parse_single_file_headers_with_entry(source, "src/helper.moth", "src/@entry.moth");
        let errors = expect_header_error(
            outcome,
            "an invalid qualified-reference path must be rejected like a clause path",
        );
        let diagnostic = errors
            .diagnostics
            .views()
            .next()
            .expect("expected one path diagnostic");
        assert_eq!(diagnostic.reason_key(), Some(reason_key), "{source}");
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, path_text, 0)),
            "the path diagnostic must label the authored path token"
        );
    }
}

#[test]
fn occurrence_with_an_authored_moth_extension_keeps_the_source_extension_diagnostic() {
    let source = "value #= @helper.moth name\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/helper.moth", "src/@entry.moth");
    let errors = expect_header_error(
        outcome,
        "an explicit .moth path must be rejected in expression position",
    );
    let diagnostic = find_diagnostic_family::<ExplicitMothExtensionView>(&errors.diagnostics)
        .expect("expected the explicit .moth extension diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-IMPORT-0020");
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "@helper.moth", 0))
    );
}

#[test]
fn call_argument_commas_separate_two_occurrences() {
    let source = "value #= f(@math square, @math cosine)\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        source,
        &PathBuf::from("src/helper.moth"),
        &PathBuf::from("src/@entry.moth"),
        &mut string_table,
    );

    let occurrences = occurrences(&output);
    let selections: Vec<&str> = occurrences
        .iter()
        .map(|occurrence| string_table.resolve(occurrence.selected))
        .collect();
    assert_eq!(
        selections,
        vec!["square", "cosine"],
        "commas inside call arguments separate occurrences instead of aliasing them"
    );
    let ordinals: Vec<u32> = occurrences
        .iter()
        .map(|occurrence| occurrence.dependency.dependency_shell_id.ordinal)
        .collect();
    assert_eq!(
        ordinals,
        vec![0, 1],
        "one shell per occurrence, in source order"
    );
    assert_eq!(
        occurrences[0].dependency.span,
        source_span_for(source, "@math", 0)
    );
    assert_eq!(
        occurrences[0].selected_span,
        source_span_for(source, "square", 0)
    );
    assert_eq!(
        occurrences[1].dependency.span,
        source_span_for(source, "@math", 1)
    );
    assert_eq!(
        occurrences[1].selected_span,
        source_span_for(source, "cosine", 0)
    );
}
