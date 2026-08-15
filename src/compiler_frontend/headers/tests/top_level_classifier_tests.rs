//! Shared top-level statement-start classification tests.

use super::*;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{SourceLocation, TokenKind};

fn classify_after(tokens: Vec<TokenKind>) -> SymbolStatementStart {
    let location = SourceLocation::default();
    let tokens = tokens
        .into_iter()
        .map(|kind| Token::new(kind, location.clone()))
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
    let ready = string_table.intern("Ready");
    let location = SourceLocation::default();
    let tokens = vec![
        Token::new(TokenKind::DoubleColon, location.clone()),
        Token::new(TokenKind::Symbol(ready), location.clone()),
        Token::new(TokenKind::FatArrow, location.clone()),
        Token::new(TokenKind::Eof, location),
    ];
    let classification = classify_symbol_statement_start_at(&tokens, 0);
    assert_eq!(classification, SymbolStatementStart::Other);
    assert!(!classification.starts_header_declaration());

    let mut token_stream = FileTokens::new(
        InternedPath::from_single_str("src/@page.moth", &mut string_table),
        tokens,
    );
    token_stream.index = 0;
    assert!(!starts_duplicate_top_level_header_declaration(
        &token_stream
    ));
}
