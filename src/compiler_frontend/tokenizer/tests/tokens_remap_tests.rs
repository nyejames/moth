//! Tokenizer string-ID remapping tests.
//!
//! WHAT: verifies that canonical token stores produced from local string tables can be resolved
//! through a merged module/global table without losing source locations or path-token metadata.
//! WHY: per-file frontend preparation depends on token outputs being safe to merge before
//! module-wide header parsing and dependency sorting consume them.

use crate::compiler_frontend::compiler_messages::DiagnosticToken;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::test_support::TestSourceContext;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TestSourceTokensBuilder, TokenIndex, TokenRef, TokenTag, TokenizerEntryMode,
};
use std::sync::Arc;

fn path_strings(fork: &PathInternerFork, path: PathId, strings: &StringTable) -> Vec<String> {
    let mut scratch = Vec::new();
    fork.resolve_components(path, &mut scratch)
        .iter()
        .map(|id| strings.resolve(*id).to_owned())
        .collect()
}

fn token_at(tokens: &SourceTokens, index: usize) -> TokenRef<'_> {
    let index = TokenIndex::try_from_index(index).expect("source token index should fit");
    tokens
        .token(index)
        .expect("source token index should resolve")
}

fn finish_tokens(builder: TestSourceTokensBuilder) -> Arc<SourceTokens> {
    builder
        .finish()
        .expect("canonical source token fixture should finish")
}

fn remapped_string(
    token: TokenRef<'_>,
    remap: &StringIdRemap,
    global_table: &StringTable,
) -> String {
    let mut projected = DiagnosticToken::from_token_ref(token);
    projected.remap_string_ids(remap);
    global_table.resolve(projected.string_id()).to_owned()
}

fn remapped_numeric(token: TokenRef<'_>, remap: &StringIdRemap) -> NumericLiteralToken {
    let mut literal = token
        .numeric_literal()
        .expect("numeric token payload must resolve")
        .expect("numeric token must carry a literal")
        .clone();
    literal.remap_string_ids(remap);
    literal
}

#[test]
fn flat_token_payloads_remap_correctly() {
    let source = SourceId::COMPILATION_ROOT;
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let alpha_local = local_table.intern("alpha");
    let beta_local = local_table.intern("beta");
    let numeric_local = NumericLiteralToken::test_new("42", &mut local_table);

    global_table.intern("preexisting");
    let remap = global_table.merge_from(&local_table);

    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_symbol(TokenTag::SYMBOL, alpha_local, LocalSpan::source_start())
        .expect("symbol fixture token should build");
    builder
        .push_symbol(
            TokenTag::STYLE_DIRECTIVE,
            beta_local,
            LocalSpan::source_start(),
        )
        .expect("style directive fixture token should build");
    builder
        .push_symbol(
            TokenTag::STRING_SLICE_LITERAL,
            alpha_local,
            LocalSpan::source_start(),
        )
        .expect("string literal fixture token should build");
    builder
        .push_symbol(
            TokenTag::RAW_STRING_LITERAL,
            beta_local,
            LocalSpan::source_start(),
        )
        .expect("raw string fixture token should build");
    builder
        .push_numeric(numeric_local, LocalSpan::source_start())
        .expect("numeric fixture token should build");
    let owner = finish_tokens(builder);

    let symbol = token_at(owner.as_ref(), 0);
    assert_eq!(symbol.tag(), TokenTag::SYMBOL);
    assert_eq!(remapped_string(symbol, &remap, &global_table), "alpha");

    let style = token_at(owner.as_ref(), 1);
    assert_eq!(style.tag(), TokenTag::STYLE_DIRECTIVE);
    assert_eq!(remapped_string(style, &remap, &global_table), "beta");

    let string_lit = token_at(owner.as_ref(), 2);
    assert_eq!(string_lit.tag(), TokenTag::STRING_SLICE_LITERAL);
    assert_eq!(remapped_string(string_lit, &remap, &global_table), "alpha");

    let raw_lit = token_at(owner.as_ref(), 3);
    assert_eq!(raw_lit.tag(), TokenTag::RAW_STRING_LITERAL);
    assert_eq!(remapped_string(raw_lit, &remap, &global_table), "beta");

    let numeric = token_at(owner.as_ref(), 4);
    assert_eq!(numeric.tag(), TokenTag::NUMERIC_LITERAL);
    let remapped_numeric = remapped_numeric(numeric, &remap);
    assert_eq!(
        global_table.resolve(remapped_numeric.source_text),
        "42",
        "numeric source text should resolve in the merged table"
    );
    assert_eq!(
        global_table.resolve(remapped_numeric.normalized_text),
        "42",
        "numeric normalized text should resolve in the merged table"
    );
}

#[test]
fn lexer_numeric_tokens_carry_checked_side_store_handles() {
    let source = SourceId::COMPILATION_ROOT;
    let mut source_context = TestSourceContext::new("numeric.moth");
    let mut path_fork = PathInternerFork::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let lexed = source_context
        .tokenize(
            "value = 42 + 3.5\n",
            &mut path_fork,
            &style_directives,
            TokenizerEntryMode::SourceFile,
        )
        .expect("numeric source should tokenize");

    let owner = lexed.tokens.as_ref();
    assert_eq!(owner.source(), source);
    let numeric_positions = (0..owner.len())
        .filter(|index| token_at(owner, *index).tag() == TokenTag::NUMERIC_LITERAL)
        .collect::<Vec<_>>();
    assert_eq!(numeric_positions.len(), 2);
    assert_eq!(owner.numeric_literal_store().len(), 2);
    assert_eq!(owner.numeric_literal_store().owner_source(), Some(source));
    for (expected_row, token_index) in numeric_positions.iter().enumerate() {
        let token = token_at(owner, *token_index);
        let handle = token
            .shape()
            .numeric_literal_id()
            .expect("numeric token must carry a side-store handle");
        assert_eq!(handle.index(), Some(expected_row));
        let stored = owner
            .numeric_literal_store()
            .try_get(handle)
            .expect("handle must address a staged row");
        let typed = token
            .numeric_literal()
            .expect("numeric token payload must resolve")
            .expect("numeric token must carry a literal");
        assert_eq!(stored.source_text, typed.source_text);
        assert_eq!(stored.normalized_text, typed.normalized_text);
        assert_eq!(stored.kind, typed.kind);
    }
    let non_numeric = (0..owner.len())
        .find(|index| token_at(owner, *index).tag() == TokenTag::SYMBOL)
        .expect("symbol token");
    assert_eq!(
        token_at(owner, non_numeric).shape().numeric_literal_id(),
        None
    );
}

#[test]
fn path_syntax_rows_remap_all_fields() {
    let source = SourceId::COMPILATION_ROOT;
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();

    let mut path_syntax = PathSyntaxTable::with_source(source);
    let id = path_syntax
        .try_push_for_source(
            local_paths
                .try_intern_components(&[
                    local_table.intern("components"),
                    local_table.intern("Button"),
                ])
                .expect("test path fits"),
            source,
            LocalSpan::source_start(),
        )
        .expect("path syntax row should build");

    global_table.intern("alpha");
    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    path_syntax.remap_path_ids(&path_remap);

    let path = path_syntax.try_path(id).expect("valid path handle");
    assert_eq!(
        path_strings(&global_paths, path.root, &global_table),
        vec!["components", "Button"]
    );
    assert_eq!(path_syntax.owner_source(), Some(source));
}

#[test]
fn canonical_tokens_remap_src_path_and_payloads_preserving_source_spans() {
    let source = SourceId::COMPILATION_ROOT;
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();

    let src_path_local = local_paths
        .try_intern_portable_path("local.moth", &mut local_table)
        .expect("test path fits");
    let symbol_local = local_table.intern("my_symbol");
    let numeric_local = NumericLiteralToken::test_new("7", &mut local_table);

    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_symbol(TokenTag::SYMBOL, symbol_local, LocalSpan::source_start())
        .expect("symbol fixture token should build");
    builder
        .push_numeric(numeric_local, LocalSpan::source_start())
        .expect("numeric fixture token should build");
    let owner = finish_tokens(builder);

    global_table.intern("preexisting");
    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    let src_path_global = path_remap.get(src_path_local);
    assert_eq!(
        path_strings(&global_paths, src_path_global, &global_table),
        vec!["local.moth"]
    );

    let first_token = token_at(owner.as_ref(), 0);
    assert_eq!(first_token.tag(), TokenTag::SYMBOL);
    assert_eq!(
        remapped_string(first_token, &remap, &global_table),
        "my_symbol"
    );
    assert_eq!(first_token.span(), LocalSpan::source_start());

    let second_token = token_at(owner.as_ref(), 1);
    assert_eq!(second_token.tag(), TokenTag::NUMERIC_LITERAL);
    let numeric = remapped_numeric(second_token, &remap);
    assert_eq!(global_table.resolve(numeric.source_text), "7");
    assert_eq!(global_table.resolve(numeric.normalized_text), "7");
}

#[test]
fn canonical_path_tokens_resolve_after_preparation_path_remap() {
    let source = SourceId::COMPILATION_ROOT;
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();
    let src_path_local = local_paths
        .try_intern_portable_path("module.moth", &mut local_table)
        .expect("test path fits");
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let ui_button = path_syntax
        .try_push_for_source(
            local_paths
                .try_intern_components(&[local_table.intern("ui"), local_table.intern("Button")])
                .expect("test path fits"),
            source,
            LocalSpan::source_start(),
        )
        .expect("path syntax row should build");
    let utils_helper = path_syntax
        .try_push_for_source(
            local_paths
                .try_intern_components(&[local_table.intern("utils"), local_table.intern("helper")])
                .expect("test path fits"),
            source,
            LocalSpan::source_start(),
        )
        .expect("path syntax row should build");

    global_table.intern("preexisting");
    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    path_syntax.remap_path_ids(&path_remap);
    let mut builder = TestSourceTokensBuilder::with_path_syntax(source, path_syntax);
    builder
        .push_path(TokenTag::PATH, ui_button, LocalSpan::source_start())
        .expect("first path fixture token should build");
    builder
        .push_path(TokenTag::PATH, utils_helper, LocalSpan::source_start())
        .expect("second path fixture token should build");
    let owner = finish_tokens(builder);

    assert_eq!(
        path_strings(&global_paths, path_remap.get(src_path_local), &global_table),
        vec!["module.moth"]
    );
    assert_eq!(token_at(owner.as_ref(), 0).tag(), TokenTag::PATH);
    assert_eq!(
        token_at(owner.as_ref(), 0).shape().path_syntax_id(),
        Some(ui_button)
    );
    let first = token_at(owner.as_ref(), 0)
        .path_syntax()
        .expect("first path token should resolve")
        .expect("first path token should carry a row");
    assert_eq!(
        path_strings(&global_paths, first.root, &global_table),
        vec!["ui", "Button"]
    );
    let second = token_at(owner.as_ref(), 1)
        .path_syntax()
        .expect("second path token should resolve")
        .expect("second path token should carry a row");
    assert_eq!(
        path_strings(&global_paths, second.root, &global_table),
        vec!["utils", "helper"]
    );
}

#[test]
fn canonical_preparing_path_remap_updates_owned_path_table() {
    let source = SourceId::COMPILATION_ROOT;
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();
    let source_path = local_paths
        .try_intern_portable_path("module.moth", &mut local_table)
        .expect("test path fits");
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let button = path_syntax
        .try_push_for_source(
            local_paths
                .try_intern_components(&[local_table.intern("ui"), local_table.intern("Button")])
                .expect("test path fits"),
            source,
            LocalSpan::source_start(),
        )
        .expect("path syntax row should build");

    global_table.intern("preexisting");
    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    path_syntax.remap_path_ids(&path_remap);
    let mut builder = TestSourceTokensBuilder::with_path_syntax(source, path_syntax);
    builder
        .push_path(TokenTag::PATH, button, LocalSpan::source_start())
        .expect("path fixture token should build");
    let owner = finish_tokens(builder);

    assert_eq!(
        path_strings(&global_paths, path_remap.get(source_path), &global_table),
        vec!["module.moth"]
    );
    let path = token_at(owner.as_ref(), 0)
        .path_syntax()
        .expect("path token should resolve")
        .expect("path token should carry a row");
    assert_eq!(
        path_strings(&global_paths, path.root, &global_table),
        vec!["ui", "Button"]
    );
}

#[test]
fn rebind_source_identity_updates_source_spans_without_changing_paths() {
    let source = SourceId::COMPILATION_ROOT;
    let mut table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let original_scope = path_fork
        .try_intern_portable_path("stage0_absolute.moth", &mut table)
        .expect("test path fits");
    let logical_scope = path_fork
        .try_intern_portable_path("module/logical.moth", &mut table)
        .expect("test path fits");

    let mut path_syntax = PathSyntaxTable::with_source(source);
    let helper_util = path_syntax
        .try_push_for_source(
            path_fork
                .try_intern_components(&[table.intern("helper"), table.intern("util")])
                .expect("test path fits"),
            source,
            LocalSpan::source_start(),
        )
        .expect("path syntax row should build");
    let symbol = table.intern("alpha");
    let mut builder = TestSourceTokensBuilder::with_path_syntax(source, path_syntax);
    builder
        .push_symbol(TokenTag::SYMBOL, symbol, LocalSpan::source_start())
        .expect("symbol fixture token should build");
    builder
        .push_path(TokenTag::PATH, helper_util, LocalSpan::source_start())
        .expect("path fixture token should build");
    let owner = finish_tokens(builder);
    let mut owner = Arc::try_unwrap(owner).expect("canonical owner should be unique");

    let file_id = SourceId::from_index(7);
    owner.rebind_source_identity(file_id);

    assert_eq!(owner.source(), file_id);
    for index in 0..owner.len() {
        assert_eq!(token_at(&owner, index).span(), LocalSpan::source_start());
    }
    let path = token_at(&owner, 1)
        .path_syntax()
        .expect("path token should resolve")
        .expect("path token should carry a row");
    assert_eq!(
        owner
            .path_syntax_table()
            .expect("canonical owner should retain its path table")
            .owner_source(),
        Some(file_id)
    );
    assert_eq!(
        path_strings(&path_fork, path.root, &table),
        vec!["helper", "util"]
    );
    assert_eq!(
        path_strings(&path_fork, original_scope, &table),
        vec!["stage0_absolute.moth"]
    );
    assert_eq!(
        path_strings(&path_fork, logical_scope, &table),
        vec!["module", "logical.moth"]
    );
}

#[test]
fn rebind_source_identity_restamps_numeric_owner_and_preserves_rows() {
    let source = SourceId::COMPILATION_ROOT;
    let mut table = StringTable::new();
    let numeric = NumericLiteralToken::test_new("7", &mut table);
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_numeric(numeric, LocalSpan::source_start())
        .expect("numeric fixture token should build");
    let owner = finish_tokens(builder);
    let mut owner = Arc::try_unwrap(owner).expect("canonical owner should be unique");
    let handle = token_at(&owner, 0)
        .shape()
        .numeric_literal_id()
        .expect("numeric token must carry a handle");
    let rebound = SourceId::from_index(11);

    owner.rebind_source_identity(rebound);

    assert_eq!(owner.numeric_literal_store().owner_source(), Some(rebound));
    owner
        .numeric_literal_store()
        .try_get(handle)
        .expect("source-local numeric row must survive rebinding");
    owner
        .numeric_literal_store()
        .try_get_for_source(handle, rebound)
        .expect("rebound owner must read its numeric row");
}

#[test]
fn frozen_numeric_store_rejects_post_publication_remap() {
    let source = SourceId::COMPILATION_ROOT;
    let mut local = StringTable::new();
    let mut global = StringTable::new();
    let numeric = NumericLiteralToken::test_new("7", &mut local);
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_numeric(numeric, LocalSpan::source_start())
        .expect("numeric fixture token should build");
    let owner = finish_tokens(builder);
    let mut owner = Arc::try_unwrap(owner).expect("canonical owner should be unique");
    global.intern("preexisting");
    let remap = global.merge_from(&local);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        owner.remap_string_ids(&remap);
    }));
    assert!(
        result.is_err(),
        "post-freeze remap must not silently mutate the canonical store"
    );
}

#[test]
fn canonical_path_handle_is_a_remap_no_op_while_table_rows_remap() {
    let source = SourceId::COMPILATION_ROOT;
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let button = path_syntax
        .try_push_for_source(
            local_paths
                .try_intern_components(&[local_table.intern("ui"), local_table.intern("Button")])
                .expect("test path fits"),
            source,
            LocalSpan::source_start(),
        )
        .expect("path syntax row should build");
    let handle_before = button;

    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    path_syntax.remap_path_ids(&path_remap);
    let mut builder = TestSourceTokensBuilder::with_path_syntax(source, path_syntax);
    builder
        .push_path(TokenTag::PATH, button, LocalSpan::source_start())
        .expect("path fixture token should build");
    let owner = finish_tokens(builder);

    let token = token_at(owner.as_ref(), 0);
    assert_eq!(token.tag(), TokenTag::PATH);
    assert_eq!(token.shape().path_syntax_id(), Some(handle_before));
    let path = token
        .path_syntax()
        .expect("path token should resolve")
        .expect("path token should carry a row");
    assert_eq!(
        path_strings(&global_paths, path.root, &global_table),
        vec!["ui", "Button"]
    );
}

#[test]
fn path_table_root_components_keep_their_allocation_under_remap() {
    let source = SourceId::COMPILATION_ROOT;
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let button = path_syntax
        .try_push_for_source(
            local_paths
                .try_intern_components(&[
                    local_table.intern("components"),
                    local_table.intern("Button"),
                ])
                .expect("test path fits"),
            source,
            LocalSpan::source_start(),
        )
        .expect("path syntax row should build");
    let handle_before = button;

    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    path_syntax.remap_path_ids(&path_remap);
    let mut builder = TestSourceTokensBuilder::with_path_syntax(source, path_syntax);
    builder
        .push_path(TokenTag::PATH, button, LocalSpan::source_start())
        .expect("path fixture token should build");
    let owner = finish_tokens(builder);

    let token = token_at(owner.as_ref(), 0);
    assert_eq!(token.shape().path_syntax_id(), Some(handle_before));
    let path = token
        .path_syntax()
        .expect("path token should resolve")
        .expect("path token should carry a row");
    assert_eq!(
        path_strings(&global_paths, path.root, &global_table),
        vec!["components", "Button"]
    );
}
