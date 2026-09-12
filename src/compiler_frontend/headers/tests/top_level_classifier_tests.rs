//! Shared top-level statement-start classification tests.

use super::*;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{Token, TokenKind};

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

    let mut token_stream = FileTokens::new(
        path_fork.try_intern_portable_path("src/@page.moth", &mut string_table).expect("test path fits"),
        SourceId::COMPILATION_ROOT,
        tokens,
    );
    token_stream.index = 0;
    assert!(!starts_duplicate_top_level_header_declaration(
        &token_stream
    ));
}
