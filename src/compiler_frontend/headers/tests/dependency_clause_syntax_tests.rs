//! Header-owned dependency-clause scanner tests.

use super::*;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidDependencyClauseReason, PathKind,
};
use crate::compiler_frontend::headers::dependency_clause_syntax::DependencyClauseParseError;
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, Token, TokenKind, TokenizerEntryMode,
};

fn tokenize_source(source: &str) -> (FileTokens, StringTable) {
    tokenize_named_source(source, "test.moth")
}

fn tokenize_named_source(source: &str, file_name: &str) -> (FileTokens, StringTable) {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str(file_name, &mut string_table);
    let tokens = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        None,
    )
    .expect("source should tokenize");
    (tokens, string_table)
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
    let (tokens, string_table) = tokenize_source(source);
    let path_index = tokens
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("expected dependency path");
    let (clause, _) = parse_dependency_clause(&tokens.tokens, path_index, &tokens.path_syntax)
        .expect("clause should parse");
    (clause, string_table)
}

fn clause_diagnostic(source: &str) -> CompilerDiagnostic {
    let (tokens, _) = tokenize_source(source);
    let path_index = tokens
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("expected dependency path");
    match parse_dependency_clause(&tokens.tokens, path_index, &tokens.path_syntax)
        .expect_err("clause should fail")
    {
        DependencyClauseParseError::Diagnostic(diagnostic) => *diagnostic,
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
    let error = clause_diagnostic("@core/math sin,\n    cos tan\n");
    assert!(matches!(
        error.payload,
        DiagnosticPayload::InvalidDependencyClause {
            reason: InvalidDependencyClauseReason::MissingCommaBetweenSelections,
            ..
        }
    ));
    assert_eq!(error.primary_location.start_pos.line_number, 1);
    assert_eq!(error.primary_location.start_pos.char_column, 9);
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
    let (tokens, _) = tokenize_source("@core/math sin\n");
    let path_index = path_token_index(&tokens);
    let (two_path_tokens, _) = tokenize_source("@core/math\n@other/path\n");
    let second_path_index = two_path_tokens
        .tokens
        .iter()
        .rposition(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("expected a second path token");
    let (other_file, _) = tokenize_named_source("@other/path sin\n", "other.moth");
    let mut location_mismatch_tokens = tokens.tokens.clone();
    location_mismatch_tokens[path_index]
        .location
        .start_pos
        .line_number += 10;
    let mut none_tokens = tokens.tokens.clone();
    if let TokenKind::Path(ref mut id) = none_tokens[path_index].kind {
        *id = PathSyntaxId::NONE;
    }
    let mut one_row_table = PathSyntaxTable::new();
    one_row_table.push(
        InternedPath::from_single_str("only", &mut StringTable::new()),
        tokens.tokens[path_index].location.clone(),
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
            "location_mismatch",
            location_mismatch_tokens.as_slice(),
            &tokens.path_syntax,
            path_index,
        ),
    ];

    for (case, clause_tokens, table, index) in cases {
        let error = match parse_dependency_clause(clause_tokens, index, table) {
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

    let name_line = source
        .lines()
        .position(|line| line.contains(name))
        .expect("fixture must contain the selected name");
    let name_column = source
        .lines()
        .nth(name_line)
        .and_then(|line| line.find(name))
        .expect("selected name column")
        + 1;
    let comma_column = source
        .lines()
        .next()
        .and_then(|line| line.rfind(','))
        .expect("continuation comma")
        + 1;

    assert_eq!(
        error.primary_location.start_pos.line_number,
        name_line as i32
    );
    assert_eq!(
        error.primary_location.start_pos.char_column,
        name_column as i32
    );
    assert!(
        error.labels.len() >= 2,
        "continuation diagnostics must carry the comma as a secondary span"
    );
    let comma_label = error
        .labels
        .iter()
        .find(|label| label.location.start_pos.char_column == comma_column as i32)
        .expect("secondary label should point at the continuation comma");
    assert_eq!(comma_label.location.start_pos.line_number, 0);
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
