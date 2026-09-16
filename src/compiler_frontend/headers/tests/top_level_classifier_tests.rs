//! Shared top-level statement-start classification tests.

use super::*;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};

fn classify_after(tokens: Vec<TokenKind>) -> SymbolStatementStart {
    let span = LocalSpan::source_start();
    let tokens = tokens
        .into_iter()
        .map(|kind| Token::new(kind, span))
        .collect::<Vec<_>>();
    classify_symbol_statement_start_at(&tokens, 0)
}

#[test]
fn runtime_binding_is_a_statement_but_not_a_header_declaration() {
    let classification = classify_after(vec![TokenKind::Assign]);
    assert!(classification.starts_statement_after_dependency_selection());
    assert!(!classification.starts_header_declaration());
}

#[test]
fn function_and_compile_time_bindings_are_header_declarations() {
    assert!(classify_after(vec![TokenKind::TypeParameterBracket]).starts_header_declaration());
    assert!(classify_after(vec![TokenKind::Hash]).starts_header_declaration());
}

#[test]
fn qualified_match_arm_is_not_a_choice_declaration() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let ready = string_table.intern("Ready");
    let span = LocalSpan::source_start();
    let tokens = vec![
        Token::new(TokenKind::DoubleColon, span),
        Token::new(TokenKind::Symbol(ready), span),
        Token::new(TokenKind::FatArrow, span),
        Token::new(TokenKind::Eof, span),
    ];
    let classification = classify_symbol_statement_start_at(&tokens, 0);
    assert_eq!(classification, SymbolStatementStart::Other);
    assert!(!classification.starts_header_declaration());

    let token_stream = FileTokens::new(
        path_fork
            .try_intern_portable_path("src/@page.moth", &mut string_table)
            .expect("test path fits"),
        SourceId::COMPILATION_ROOT,
        tokens,
    );
    let current_index =
        TokenIndex::try_from_index(token_stream.index).expect("test cursor index should fit");
    let canonical = token_stream
        .source_tokens()
        .expect("test stream owns canonical source tokens");
    assert!(
        !starts_duplicate_top_level_header_declaration_at_source(canonical, current_index),
        "qualified match arms in the start body are not choice declarations"
    );
}

#[test]
fn classifier_views_agree_without_whole_source_projection() {
    use crate::compiler_frontend::utilities::token_scan::TokenFactView;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let ready = string_table.intern("Ready");
    let span = LocalSpan::source_start();
    let tokens = vec![
        Token::new(TokenKind::DoubleColon, span),
        Token::new(TokenKind::Symbol(ready), span),
        Token::new(TokenKind::FatArrow, span),
        Token::new(TokenKind::Eof, span),
    ];
    let expected = classify_symbol_statement_start_at(&tokens, 0);
    let from_stream_tokens = FileTokens::new(
        path_fork
            .try_intern_portable_path("src/@page.moth", &mut string_table)
            .expect("test path fits"),
        SourceId::COMPILATION_ROOT,
        tokens,
    );
    let streamed = TokenFactView::from_stream(&from_stream_tokens);
    assert_eq!(
        classify_symbol_statement_start_at_scanned(streamed, 0),
        expected,
        "bounded stream view must preserve choice/match-arm disambiguation"
    );
    let canonical = from_stream_tokens
        .source_tokens()
        .expect("test stream owns canonical source tokens");
    assert_eq!(
        classify_symbol_statement_start_at_source(canonical, 0),
        expected,
        "canonical view must preserve follower classification"
    );
    assert!(streamed.get(streamed.len()).is_none());
}
