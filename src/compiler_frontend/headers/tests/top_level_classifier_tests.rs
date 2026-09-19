//! Shared top-level statement-start classification tests.

use super::*;
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, Token, TokenIndex, TokenKind,
};
use crate::compiler_frontend::utilities::token_scan::TokenFactView;

fn canonical(tokens: Vec<Token>) -> SourceTokens {
    SourceTokens::try_from_tokens(
        SourceId::COMPILATION_ROOT,
        tokens,
        NumericLiteralStore::with_source(SourceId::COMPILATION_ROOT),
        PathSyntaxTable::with_source(SourceId::COMPILATION_ROOT),
    )
    .expect("canonical classifier fixture should satisfy source-token invariants")
}

fn classify_after(tokens: Vec<TokenKind>) -> SymbolStatementStart {
    let span = LocalSpan::source_start();
    let owner = canonical(
        tokens
            .into_iter()
            .map(|kind| Token::new(kind, span))
            .collect(),
    );
    classify_symbol_statement_start_at_source(&owner, 0)
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
    let ready = string_table.intern("Ready");
    let span = LocalSpan::source_start();
    let owner = canonical(vec![
        Token::new(TokenKind::DoubleColon, span),
        Token::new(TokenKind::Symbol(ready), span),
        Token::new(TokenKind::FatArrow, span),
        Token::new(TokenKind::Eof, span),
    ]);
    let classification = classify_symbol_statement_start_at_source(&owner, 0);
    assert_eq!(classification, SymbolStatementStart::Other);
    assert!(!classification.starts_header_declaration());

    let current_index =
        TokenIndex::try_from_index(0).expect("test cursor index should fit");
    assert!(
        !starts_duplicate_top_level_header_declaration_at_source(&owner, current_index),
        "qualified match arms in the start body are not choice declarations"
    );
}

#[test]
fn classifier_views_agree_without_whole_source_projection() {
    let mut string_table = StringTable::new();
    let ready = string_table.intern("Ready");
    let span = LocalSpan::source_start();
    let owner = canonical(vec![
        Token::new(TokenKind::DoubleColon, span),
        Token::new(TokenKind::Symbol(ready), span),
        Token::new(TokenKind::FatArrow, span),
        Token::new(TokenKind::Eof, span),
    ]);
    let expected = classify_symbol_statement_start_at_source(&owner, 0);
    let source_view = TokenFactView::from_source(&owner);
    assert_eq!(
        classify_symbol_statement_start_at_scanned(source_view, 0),
        expected,
        "canonical source view must preserve choice/match-arm disambiguation"
    );
    let full_range = owner.full_range().expect("test owner range should fit");
    let ranged_view =
        TokenFactView::from_source_range(&owner, full_range).expect("full source range should fit");
    assert_eq!(
        classify_symbol_statement_start_at_scanned(ranged_view, 0),
        expected,
        "canonical bounded view must preserve follower classification"
    );
    assert!(ranged_view.get(ranged_view.len()).is_none());
}

