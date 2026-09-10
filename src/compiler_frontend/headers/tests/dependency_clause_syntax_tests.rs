//! Header-owned dependency-clause scanner tests.

use super::*;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidDependencyClauseReason, PathKind,
};
use crate::compiler_frontend::headers::dependency_clause_syntax::DependencyClauseParseError;
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, Token, TokenKind, TokenizerEntryMode,
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
    let source_path = InternedPath::from_single_str(file_name, &mut string_table);
    let mut span_builder = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        source_id,
        &mut span_builder,
    )
    .expect("source should tokenize");
    (tokens, string_table, span_builder)
}

fn path_token_index(tokens: &FileTokens) -> usize {
    tokens
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("expected dependency path")
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
    let path_index = tokens
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("expected dependency path");
    let (clause, _) = parse_dependency_clause(
        &tokens.tokens,
        path_index,
        &tokens.path_syntax,
        tokens.file_id,
    )
    .expect("clause should parse");
    (clause, string_table)
}

fn clause_diagnostic(source: &str) -> CompilerDiagnostic {
    let (tokens, _, _span_builder) = tokenize_source(source);
    let path_index = tokens
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("expected dependency path");
    match parse_dependency_clause(
        &tokens.tokens,
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
    let (tokens, strings, _) = tokenize_source(source);
    let tan_span = tokens
        .tokens
        .iter()
        .find(|token| {
            matches!(
                &token.kind,
                TokenKind::Symbol(id) if strings.resolve(*id) == "tan"
            )
        })
        .expect("expected the unexpected adjacent selection")
        .span;
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
    let (two_path_tokens, _, _span_builder) = tokenize_source("@core/math\n@other/path\n");
    let second_path_index = two_path_tokens
        .tokens
        .iter()
        .rposition(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("expected a second path token");
    let (other_file, _, _span_builder) =
        tokenize_named_source_with_id("@other/path sin\n", "other.moth", SourceId::from_index(1));
    let mut span_mismatch_tokens = tokens.tokens.clone();
    let mut mismatch_builder = ExtendedSpanBuilder::new();
    span_mismatch_tokens[path_index].span =
        LocalSpan::exact(1, 1, &mut mismatch_builder).expect("mismatch span should fit inline");
    let mut none_tokens = tokens.tokens.clone();
    if let TokenKind::Path(id) = &mut none_tokens[path_index].kind {
        *id = PathSyntaxId::NONE;
    }
    let mut one_row_table = PathSyntaxTable::new();
    one_row_table.push(
        InternedPath::from_single_str("only", &mut StringTable::new()),
        SourceSpan::new(tokens.file_id, tokens.tokens[path_index].span),
    );
    let empty_table = PathSyntaxTable::new();

    let cases: [(&str, &[Token], &PathSyntaxTable, usize); 5] = [
        (
            "none_handle",
            none_tokens.as_slice(),
            &tokens.path_syntax,
            path_index,
        ),
        (
            "out_of_range_non_none",
            &two_path_tokens.tokens,
            &one_row_table,
            second_path_index,
        ),
        (
            "empty_wrong_table",
            tokens.tokens.as_slice(),
            &empty_table,
            path_index,
        ),
        (
            "different_non_empty_table",
            tokens.tokens.as_slice(),
            &other_file.path_syntax,
            path_index,
        ),
        (
            "span_mismatch",
            span_mismatch_tokens.as_slice(),
            &tokens.path_syntax,
            path_index,
        ),
    ];

    for (case, clause_tokens, table, index) in cases {
        let error = match parse_dependency_clause(
            clause_tokens,
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

    let (tokens, strings, _) = tokenize_source(source);
    let name_span = tokens
        .tokens
        .iter()
        .find(|token| {
            matches!(
                &token.kind,
                TokenKind::Symbol(id) if strings.resolve(*id) == name
            )
        })
        .expect("fixture must contain the selected name")
        .span;
    let comma_span = tokens
        .tokens
        .iter()
        .find(|token| matches!(token.kind, TokenKind::Comma))
        .expect("fixture must contain the continuation comma")
        .span;

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
