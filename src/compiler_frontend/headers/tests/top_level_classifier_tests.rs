//! Shared top-level statement-start classification tests.

use super::*;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TestSourceTokensBuilder, TokenIndex, TokenTag,
};
use crate::compiler_frontend::utilities::token_scan::TokenFactView;
use std::sync::Arc;

fn canonical_static(tags: Vec<TokenTag>) -> Arc<SourceTokens> {
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    for tag in tags {
        builder
            .push_static(tag, LocalSpan::source_start())
            .expect("static classifier fixture should build");
    }
    builder
        .finish()
        .expect("canonical classifier fixture should satisfy source-token invariants")
}

fn classify_after(tags: Vec<TokenTag>) -> SymbolStatementStart {
    let owner = canonical_static(tags);
    classify_symbol_statement_start_at_source(&owner, 0)
}

fn qualified_match_arm_owner(ready: StringId) -> Arc<SourceTokens> {
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    let span = LocalSpan::source_start();
    builder
        .push_static(TokenTag::DOUBLE_COLON, span)
        .expect("static classifier fixture should build");
    builder
        .push_symbol(TokenTag::SYMBOL, ready, span)
        .expect("symbol classifier fixture should build");
    builder
        .push_static(TokenTag::FAT_ARROW, span)
        .expect("static classifier fixture should build");
    builder
        .push_static(TokenTag::EOF, span)
        .expect("static classifier fixture should build");
    builder
        .finish()
        .expect("canonical classifier fixture should satisfy source-token invariants")
}

#[test]
fn runtime_binding_is_a_statement_but_not_a_header_declaration() {
    let classification = classify_after(vec![TokenTag::ASSIGN]);
    assert!(classification.starts_statement_after_dependency_selection());
    assert!(!classification.starts_header_declaration());
}

#[test]
fn function_and_compile_time_bindings_are_header_declarations() {
    assert!(classify_after(vec![TokenTag::TYPE_PARAMETER_BRACKET]).starts_header_declaration());
    assert!(classify_after(vec![TokenTag::HASH]).starts_header_declaration());
}

#[test]
fn qualified_match_arm_is_not_a_choice_declaration() {
    let mut string_table = StringTable::new();
    let ready = string_table.intern("Ready");
    let owner = qualified_match_arm_owner(ready);
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
    let owner = qualified_match_arm_owner(ready);
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
