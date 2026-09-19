//! Header-owned dependency-clause scanner tests.

use super::*;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidDependencyClauseReason, PathKind,
};
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, SourceTokens, Token, TokenKind, TokenTag, TokenizerEntryMode,
};

fn tokenize_source(source: &str) -> (FileTokens, StringTable, ExtendedSpanBuilder) {
    tokenize_named_source_with_id(source, "test.moth", SourceId::COMPILATION_ROOT)
}

fn tokenize_named_source_with_id(
    source: &str,
    file_name: &str,
    source_id: SourceId,
) -> (FileTokens, StringTable, ExtendedSpanBuilder) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path(file_name, &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        source_id,
        &mut span_builder,
    )
    .expect("source should tokenize");
    (tokens, string_table, span_builder)
}

fn path_token_index(tokens: &FileTokens) -> usize {
    tokens
        .source_tokens()
        .expect("lexer output owns canonical source tokens")
        .shapes()
        .iter()
        .position(|shape| shape.tag() == TokenTag::PATH)
        .expect("expected dependency path")
}

fn canonical_from_tokens(
    source: SourceId,
    tokens: Vec<Token>,
    path_syntax: PathSyntaxTable,
) -> SourceTokens {
    let mut numeric_literals = NumericLiteralStore::with_source(source);
    for token in &tokens {
        if let TokenKind::NumericLiteral(literal) = &token.kind {
            numeric_literals.push(literal.clone());
        }
    }
    SourceTokens::try_from_tokens(source, tokens, numeric_literals, path_syntax)
        .expect("canonical dependency fixture should satisfy source-token invariants")
}

fn expect_infrastructure_error(error: DependencyClauseParseError, case: &str) {
    match error {
        DependencyClauseParseError::Infrastructure(_) => {}
        DependencyClauseParseError::Diagnostic(diagnostic) => panic!(
            "{case}: malformed path lookup must not fabricate a user diagnostic: {:?}",
            diagnostic.payload
        ),
    }
}

fn parse_clause(source: &str) -> (ScannedDependencyClause, StringTable) {
    let (tokens, string_table, _span_builder) = tokenize_source(source);
    let path_index = path_token_index(&tokens);
    let canonical = tokens
        .source_tokens()
        .expect("lexer output owns canonical source tokens");
    let (clause, _) =
        parse_dependency_clause_at_source(canonical, path_index, &tokens.path_syntax, tokens.file_id)
            .expect("clause should parse");
    (clause, string_table)
}

fn clause_diagnostic(source: &str) -> CompilerDiagnostic {
    let (tokens, _, _span_builder) = tokenize_source(source);
    let path_index = path_token_index(&tokens);
    let canonical = tokens
        .source_tokens()
        .expect("lexer output owns canonical source tokens");
    match parse_dependency_clause_at_source(
        canonical,
        path_index,
        &tokens.path_syntax,
        tokens.file_id,
    )
    .expect_err("clause should fail")
    {
        DependencyClauseParseError::Diagnostic(diagnostic) => diagnostic,
        DependencyClauseParseError::Infrastructure(_) => {
            panic!("expected a user-facing diagnostic, not an infrastructure error")
        }
    }
}

fn clause_error(source: &str) -> InvalidDependencyClauseReason {
    let error = clause_diagnostic(source);
    let DiagnosticPayload::InvalidDependencyClause { reason, .. } = error.payload else {
        panic!(
            "expected dependency-clause diagnostic, got {:?}",
            error.payload
        );
    };
    reason
}

#[test]
fn recognises_likely_unquoted_filename_component() {
    let error = clause_diagnostic("@docs/my file.md\n");
    assert!(matches!(
        error.payload,
        DiagnosticPayload::InvalidPath {
            path_kind: PathKind::WhitespaceMustBeQuoted
        }
    ));
}

#[test]
fn parses_namespace_and_alias() {
    let (bare, _) = parse_clause("@core/math\n");
    assert!(matches!(
        bare.binding,
        ScannedDependencyBinding::Namespace { alias: None }
    ));

    let (aliased, strings) = parse_clause("@core/math as maths\n");
    let ScannedDependencyBinding::Namespace { alias: Some(alias) } = aliased.binding else {
        panic!("expected namespace alias");
    };
    assert_eq!(strings.resolve(alias.name), "maths");
}

#[test]
fn parses_flat_selections_and_aliases() {
    let (clause, strings) = parse_clause("@core/math sin as sine, PI as pi\n");
    let ScannedDependencyBinding::DirectSelections { selections } = clause.binding else {
        panic!("expected direct selections");
    };
    assert_eq!(selections.len(), 2);
    assert_eq!(strings.resolve(selections[0].source_name), "sin");
    assert_eq!(
        strings.resolve(selections[0].local_alias.as_ref().unwrap().name),
        "sine"
    );
    assert_eq!(strings.resolve(selections[1].source_name), "PI");
}

#[test]
fn continues_only_after_comma() {
    let (clause, _) = parse_clause("@core/math sin,\n    cos\n");
    let ScannedDependencyBinding::DirectSelections { selections } = clause.binding else {
        panic!("expected direct selections");
    };
    assert_eq!(selections.len(), 2);

    let (single, _) = parse_clause("@core/math sin\ncos = 1\n");
    assert!(
        matches!(single.binding, ScannedDependencyBinding::DirectSelections { selections } if selections.len() == 1)
    );
}

#[test]
fn reports_missing_comma_at_the_unexpected_selection_after_continuation() {
    let source = "@core/math sin,\n    cos tan\n";
    let error = clause_diagnostic(source);
    assert!(matches!(
        error.payload,
        DiagnosticPayload::InvalidDependencyClause {
            reason: InvalidDependencyClauseReason::MissingCommaBetweenSelections,
            ..
        }
    ));
    let (tokens, mut strings, _) = tokenize_source(source);
    let canonical = tokens
        .source_tokens()
        .expect("lexer output owns canonical source tokens");
    let tan_id = strings.intern("tan");
    let tan_index = canonical
        .shapes()
        .iter()
        .position(|shape| {
            shape.tag() == TokenTag::SYMBOL && shape.string_id() == Some(tan_id)
        })
        .expect("expected the unexpected adjacent selection");
    let tan_span = canonical.spans()[tan_index];
    assert_eq!(
        error.primary_span,
        Some(SourceSpan::new(tokens.file_id, tan_span))
    );
}

#[test]
fn rejects_trailing_comma_missing_comma_and_braces() {
    assert_eq!(
        clause_error("@core/math sin,\n"),
        InvalidDependencyClauseReason::MissingSelectionAfterComma
    );
    assert_eq!(
        clause_error("@core/math sin cos\n"),
        InvalidDependencyClauseReason::MissingCommaBetweenSelections
    );
    assert_eq!(
        clause_error("@core/math { sin }\n"),
        InvalidDependencyClauseReason::LegacyBraceSelections
    );
}

#[test]
fn rejects_namespace_alias_followed_by_selections_and_delimiters() {
    assert_eq!(
        clause_error("@core/math as maths sin\n"),
        InvalidDependencyClauseReason::NamespaceAliasWithSelections
    );
    assert_eq!(
        clause_error("@core/math (sin)\n"),
        InvalidDependencyClauseReason::InvalidSelectionDelimiter
    );
    assert_eq!(
        clause_error("@core/math: sin\n"),
        InvalidDependencyClauseReason::InvalidSelectionDelimiter
    );
}

#[test]
fn corrupted_path_lookup_is_infrastructure_error() {
    let (tokens, _, _span_builder) = tokenize_source("@core/math sin\n");
    let path_index = path_token_index(&tokens);
    let canonical = tokens
        .source_tokens()
        .expect("lexer output owns canonical source tokens");
    let mut none_owner = canonical.clone();
    none_owner.corrupt_payload_for_test(path_index, TokenTag::PATH);

    let (two_path_tokens, _, _span_builder) = tokenize_source("@core/math\n@other/path\n");
    let two_path_canonical = two_path_tokens
        .source_tokens()
        .expect("lexer output owns canonical source tokens");
    let second_path_index = two_path_canonical
        .shapes()
        .iter()
        .rposition(|shape| shape.tag() == TokenTag::PATH)
        .expect("expected a second path token");
    let (other_file, _, _span_builder) =
        tokenize_named_source_with_id("@other/path sin\n", "other.moth", SourceId::from_index(1));

    let path_span = canonical.spans()[path_index];
    let mut one_row_table = PathSyntaxTable::with_source(tokens.file_id);
    one_row_table
        .try_push_for_source(PathId::ROOT, tokens.file_id, path_span)
        .expect("test path row should fit");
    let empty_table = PathSyntaxTable::new();

    let mut mismatch_builder = ExtendedSpanBuilder::new();
    let mismatch_span =
        LocalSpan::exact(1, 1, &mut mismatch_builder).expect("mismatch span should fit inline");
    let path_id = canonical.shapes()[path_index]
        .path_syntax_id()
        .expect("dependency path should carry a path handle");
    let path_root = tokens
        .path_syntax
        .try_path(path_id)
        .expect("dependency path should have a path row")
        .root;
    let mut mismatch_table = PathSyntaxTable::with_source(tokens.file_id);
    mismatch_table
        .try_push_for_source(path_root, tokens.file_id, mismatch_span)
        .expect("mismatch path row should fit");
    let mut span_mismatch_tokens = tokens.tokens.clone();
    span_mismatch_tokens[path_index].span = mismatch_span;
    let mismatch_owner =
        canonical_from_tokens(tokens.file_id, span_mismatch_tokens, mismatch_table);

    let cases: [(&str, &SourceTokens, &PathSyntaxTable, usize); 5] = [
        ("none_handle", &none_owner, &tokens.path_syntax, path_index),
        (
            "out_of_range_non_none",
            two_path_canonical,
            &one_row_table,
            second_path_index,
        ),
        ("empty_wrong_table", canonical, &empty_table, path_index),
        (
            "different_non_empty_table",
            canonical,
            &other_file.path_syntax,
            path_index,
        ),
        (
            "span_mismatch",
            &mismatch_owner,
            &tokens.path_syntax,
            path_index,
        ),
    ];

    for (case, owner, table, index) in cases {
        let error = match parse_dependency_clause_at_source(
            owner,
            index,
            table,
            SourceId::COMPILATION_ROOT,
        ) {
            Err(error) => error,
            Ok(_) => panic!("{case}: corrupted path lookup must fail"),
        };
        expect_infrastructure_error(error, case);
    }
}

fn assert_continuation_entered_statement(label: &str, source: &str, name: &str) {
    let error = clause_diagnostic(source);
    match error.payload {
        DiagnosticPayload::InvalidDependencyClause {
            reason: InvalidDependencyClauseReason::ContinuationEnteredStatement,
            ..
        } => {}
        _ => panic!(
            "{label}: expected ContinuationEnteredStatement for {source:?}, got {:?}",
            error.payload
        ),
    }

    let (tokens, mut strings, _) = tokenize_source(source);
    let canonical = tokens
        .source_tokens()
        .expect("lexer output owns canonical source tokens");
    let name_id = strings.intern(name);
    let name_index = canonical
        .shapes()
        .iter()
        .position(|shape| shape.tag() == TokenTag::SYMBOL && shape.string_id() == Some(name_id))
        .expect("fixture must contain the selected name");
    let name_span = canonical.spans()[name_index];
    let comma_index = canonical
        .shapes()
        .iter()
        .position(|shape| shape.tag() == TokenTag::COMMA)
        .expect("fixture must contain the continuation comma");
    let comma_span = canonical.spans()[comma_index];

    assert_eq!(
        error.primary_span,
        Some(SourceSpan::new(tokens.file_id, name_span))
    );
    assert_eq!(
        error.labels.len(),
        1,
        "continuation diagnostics must carry the comma as a secondary label"
    );
    assert_eq!(
        error.labels[0].span,
        Some(SourceSpan::new(tokens.file_id, comma_span)),
        "secondary label should point at the continuation comma"
    );
}

#[test]
fn comma_continued_into_declaration_reports_continuation_entered_statement() {
    let cases = [
        (
            "value binding",
            "@html/tables data,\nrow = build_row()\n",
            "row",
        ),
        (
            "compile-time binding",
            "@html/tables data,\nrow #= build_row()\n",
            "row",
        ),
        (
            "function declaration",
            "@html/tables data,\nrow |value Int| -> String:\n    return value\n;\n",
            "row",
        ),
        (
            "choice declaration",
            "@html/tables data,\nStatus ::\n    Ready,\n;\n",
            "Status",
        ),
        (
            "struct declaration",
            "@html/tables data,\nRow = |value Int|\n",
            "Row",
        ),
        (
            "trait declaration",
            "@html/tables data,\nSHOW must:\n;\n",
            "SHOW",
        ),
        (
            "specialised conformance",
            "@html/tables data,\nBox of T must SHOW\n",
            "Box",
        ),
    ];

    for (label, source, name) in cases {
        assert_continuation_entered_statement(label, source, name);
    }
}

#[test]
fn comma_continued_selection_alias_is_not_a_type_alias_declaration() {
    let (clause, string_table) = parse_clause("@html/tables data,\nRow as row_alias\n");
    let ScannedDependencyBinding::DirectSelections { selections } = clause.binding else {
        panic!("`as` after a continued selection is a selected alias, not a type-alias header");
    };
    assert_eq!(selections.len(), 2);
    assert_eq!(string_table.resolve(selections[1].source_name), "Row");
    assert_eq!(
        selections[1]
            .local_alias
            .as_ref()
            .map(|alias| string_table.resolve(alias.name)),
        Some("row_alias")
    );
}

#[test]
fn clause_terminated_without_comma_is_valid_before_declaration() {
    let (clause, _) = parse_clause("@html/tables data\nrow = build_row()\n");
    let ScannedDependencyBinding::DirectSelections { selections } = clause.binding else {
        panic!("expected direct selections");
    };
    assert_eq!(selections.len(), 1);
}

#[test]
fn canonical_view_agrees_with_bounded_range_scan() {
    let (tokens, _, _) = tokenize_source("@core/math sin, cos\n");
    let path_index = path_token_index(&tokens);
    let table = &tokens.path_syntax;
    let source = tokens.file_id;
    let canonical = tokens
        .source_tokens()
        .expect("lexer output owns canonical source tokens");
    let full_range = canonical.full_range().expect("test source range should fit");
    let bounded = crate::compiler_frontend::utilities::token_scan::TokenFactView::from_source_range(
        canonical,
        full_range,
    )
    .expect("bounded canonical source range should fit");
    let from_range = parse_dependency_clause_scanned(bounded, path_index, table, source)
        .expect("bounded range scan should parse");
    let from_source = parse_dependency_clause_at_source(canonical, path_index, table, source)
        .expect("canonical scan should parse");

    assert_eq!(from_range.1, from_source.1);
    assert_eq!(
        from_range.0.provider.path, from_source.0.provider.path,
        "canonical scan must preserve the provider root"
    );
    match (&from_range.0.binding, &from_source.0.binding) {
        (
            ScannedDependencyBinding::DirectSelections {
                selections: expected,
            },
            ScannedDependencyBinding::DirectSelections { selections: actual },
        ) => assert_eq!(
            expected.len(),
            actual.len(),
            "canonical scan must preserve selections"
        ),
        _ => panic!("expected direct selections in both scans"),
    }
}

#[test]
fn canonical_dependency_scan_rejects_foreign_caller_source() {
    let (tokens, _, _) = tokenize_source("@core/math sin\n");
    let path_index = path_token_index(&tokens);
    let canonical = tokens
        .source_tokens()
        .expect("lexer output owns canonical source tokens");
    let path_span = canonical.spans()[path_index];
    let mut unowned_table = PathSyntaxTable::new();
    unowned_table
        .try_push_local(PathId::ROOT, path_span)
        .expect("test path row should fit");

    let error = parse_dependency_clause_at_source(
        canonical,
        path_index,
        &unowned_table,
        SourceId::from_index(1),
    )
    .expect_err("canonical scan must reject a mismatched caller source");
    expect_infrastructure_error(error, "foreign caller source");
}
