//! Tokenizer string-ID remapping tests.
//!
//! WHAT: verifies that token streams produced from local string tables can be remapped into a
//! merged module/global table without losing source locations or path-token metadata.
//! WHY: per-file frontend preparation depends on token outputs being safe to merge before
//! module-wide header parsing and dependency sorting consume them.

use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
fn path_strings(fork: &PathInternerFork, path: PathId, strings: &StringTable) -> Vec<String> {
    let mut scratch = Vec::new();
    fork.resolve_components(path, &mut scratch)
        .iter()
        .map(|id| strings.resolve(*id).to_owned())
        .collect()
}

fn make_token(kind: TokenKind, _scope: PathId) -> Token {
    Token::new(kind, LocalSpan::source_start())
}

#[test]
fn flat_token_kinds_remap_correctly() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let alpha_local = local_table.intern("alpha");
    let beta_local = local_table.intern("beta");

    global_table.intern("alpha");
    let _gamma_global = global_table.intern("gamma");

    let numeric_token = NumericLiteralToken::test_new("42", &mut local_table);

    let remap = global_table.merge_from(&local_table);

    let mut symbol = TokenKind::Symbol(alpha_local);
    symbol.remap_string_ids(&remap);
    assert!(
        matches!(symbol, TokenKind::Symbol(id) if global_table.resolve(id) == "alpha"),
        "symbol should resolve to 'alpha' in global table"
    );

    let mut style = TokenKind::StyleDirective(beta_local);
    style.remap_string_ids(&remap);
    assert!(
        matches!(style, TokenKind::StyleDirective(id) if global_table.resolve(id) == "beta"),
        "style directive should resolve to 'beta' in global table"
    );

    let mut string_lit = TokenKind::StringSliceLiteral(alpha_local);
    string_lit.remap_string_ids(&remap);
    assert!(
        matches!(string_lit, TokenKind::StringSliceLiteral(id) if global_table.resolve(id) == "alpha"),
        "string slice literal should resolve to 'alpha' in global table"
    );

    let mut raw_lit = TokenKind::RawStringLiteral(beta_local);
    raw_lit.remap_string_ids(&remap);
    assert!(
        matches!(raw_lit, TokenKind::RawStringLiteral(id) if global_table.resolve(id) == "beta"),
        "raw string literal should resolve to 'beta' in global table"
    );

    let mut non_string_kind = TokenKind::NumericLiteral(numeric_token);
    non_string_kind.remap_string_ids(&remap);
    assert!(
        matches!(non_string_kind, TokenKind::NumericLiteral(ref token) if global_table.resolve(token.source_text) == "42" && global_table.resolve(token.normalized_text) == "42"),
        "non-string-bearing numeric token kind should remap both source_text and normalized_text"
    );
}

#[test]
fn path_syntax_rows_remap_all_fields() {
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();

    let mut path_syntax = PathSyntaxTable::new();
    let id = path_syntax.push(
        local_paths
            .try_intern_components(&[
                local_table.intern("components"),
                local_table.intern("Button"),
            ])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );

    let _alpha_global = global_table.intern("alpha");
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
    assert_eq!(path.span.source(), SourceId::COMPILATION_ROOT);
}

#[test]
fn file_tokens_remaps_src_path_and_tokens_preserves_canonical_os_path() {
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();

    let src_path_local = local_paths
        .try_intern_portable_path("local.moth", &mut local_table)
        .expect("test path fits");
    let token_scope_local = src_path_local;
    let symbol_local = local_table.intern("my_symbol");
    let tokens = vec![
        make_token(TokenKind::Symbol(symbol_local), token_scope_local),
        make_token(
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("7", &mut local_table)),
            token_scope_local,
        ),
    ];
    let canonical_path = std::path::PathBuf::from("/absolute/local.moth");
    let mut file_tokens = FileTokens::new_with_identity(
        src_path_local,
        SourceId::COMPILATION_ROOT,
        Some(canonical_path.clone()),
        tokens,
        PathSyntaxTable::new(),
    );
    global_table.intern("preexisting");
    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    file_tokens.remap_string_ids(&remap);
    file_tokens.remap_path_ids(&path_remap);
    assert_eq!(
        path_strings(&global_paths, file_tokens.src_path, &global_table),
        vec!["local.moth"]
    );
    assert_eq!(file_tokens.canonical_os_path, Some(canonical_path));
    let first_token = file_tokens.tokens.first().expect("first token should exist");
    assert!(matches!(
        first_token.kind,
        TokenKind::Symbol(id) if global_table.resolve(id) == "my_symbol"
    ));
    assert_eq!(first_token.span, LocalSpan::source_start());
    assert!(matches!(
        file_tokens.tokens.get(1).expect("second token should exist").kind,
        TokenKind::NumericLiteral(_)
    ));
}

#[test]
fn file_tokens_with_path_tokens_leave_table_remapping_to_the_prepared_file_owner() {
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();
    let src_path_local = local_paths
        .try_intern_portable_path("module.moth", &mut local_table)
        .expect("test path fits");
    let token_scope_local = src_path_local;
    let mut path_syntax = PathSyntaxTable::new();
    let ui_button = path_syntax.push(
        local_paths
            .try_intern_components(&[local_table.intern("ui"), local_table.intern("Button")])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let utils_helper = path_syntax.push(
        local_paths
            .try_intern_components(&[
                local_table.intern("utils"),
                local_table.intern("helper"),
            ])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let tokens = vec![
        make_token(TokenKind::Path(ui_button), token_scope_local),
        make_token(TokenKind::Path(utils_helper), token_scope_local),
    ];
    let mut file_tokens = FileTokens::new_with_identity(
        src_path_local,
        SourceId::COMPILATION_ROOT,
        None,
        tokens,
        path_syntax,
    );
    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    file_tokens.remap_string_ids(&remap);
    file_tokens
        .remap_preparing_path_ids(&path_remap)
        .expect("path table should be preparing");
    assert!(matches!(
        file_tokens.tokens[0].kind,
        TokenKind::Path(id) if id == ui_button
    ));
    let first = file_tokens.path_syntax.try_path(ui_button).expect("valid path handle");
    assert_eq!(
        path_strings(&global_paths, first.root, &global_table),
        vec!["ui", "Button"]
    );
    let second = file_tokens.path_syntax.try_path(utils_helper).expect("valid path handle");
    assert_eq!(
        path_strings(&global_paths, second.root, &global_table),
        vec!["utils", "helper"]
    );
}

#[test]
fn file_tokens_preparing_remap_updates_owned_path_table() {
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();
    let source_path = local_paths
        .try_intern_portable_path("module.moth", &mut local_table)
        .expect("test path fits");
    let mut path_syntax = PathSyntaxTable::new();
    let button = path_syntax.push(
        local_paths
            .try_intern_components(&[local_table.intern("ui"), local_table.intern("Button")])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let tokens = vec![make_token(TokenKind::Path(button), source_path)];
    let mut file_tokens = FileTokens::new_with_identity(
        source_path,
        SourceId::COMPILATION_ROOT,
        None,
        tokens,
        path_syntax,
    );
    global_table.intern("preexisting");
    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    file_tokens
        .remap_preparing_string_ids(&remap)
        .expect("the preparing token stream should own a mutable path table");
    file_tokens
        .remap_preparing_path_ids(&path_remap)
        .expect("the preparing path table should remap");
    let path = file_tokens.path_syntax.try_path(button).expect("valid path handle");
    assert_eq!(
        path_strings(&global_paths, path.root, &global_table),
        vec!["ui", "Button"]
    );
}

#[test]
fn rebind_source_identity_updates_source_spans_without_changing_paths() {
    let mut table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();

    let original_scope = path_fork.try_intern_portable_path("stage0_absolute.moth", &mut table).expect("test path fits");
    let logical_scope = path_fork.try_intern_portable_path("module/logical.moth", &mut table).expect("test path fits");

    let mut path_syntax = PathSyntaxTable::new();
    let helper_util = path_syntax.push(
        path_fork.try_intern_components(&[table.intern("helper"), table.intern("util")]).expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let tokens = vec![
        make_token(
            TokenKind::Symbol(table.intern("alpha")),
            original_scope.clone(),
        ),
        make_token(TokenKind::Path(helper_util), original_scope.clone()),
    ];

    let canonical = std::path::PathBuf::from("/canonical/logical.moth");
    let mut file_tokens = FileTokens::new_with_identity(
        original_scope.clone(),
        SourceId::COMPILATION_ROOT,
        None,
        tokens,
        path_syntax,
    );

    let file_id = SourceId::from_index(7);
    file_tokens
        .rebind_source_identity(logical_scope.clone(), file_id, Some(canonical.clone()))
        .expect("the sole mutable source table should accept final identity rebinding");

    // Top-level identity fields are rebound.
    assert_eq!(file_tokens.src_path, logical_scope);
    assert_eq!(file_tokens.file_id, file_id);
    assert_eq!(file_tokens.canonical_os_path, Some(canonical));

    // Every token keeps its local span while the enclosing stream supplies the final source ID.
    for token in &file_tokens.tokens {
        assert_eq!(token.span, LocalSpan::source_start());
    }

    // Path table spans are rebound but the root payload is unchanged.
    let path = file_tokens
        .path_syntax
        .try_path(helper_util)
        .expect("valid path handle");
    assert_eq!(path.span.source(), file_id);
    assert_eq!(
        path_strings(&path_fork, path.root, &table),
        vec!["helper", "util"]
    );
}

#[test]
fn token_kind_path_handle_is_a_remap_no_op_while_table_rows_remap() {
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let button = path_syntax.push(
        local_paths
            .try_intern_components(&[local_table.intern("ui"), local_table.intern("Button")])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let mut kind = TokenKind::Path(button);
    let handle_before = match &kind {
        TokenKind::Path(id) => *id,
        _ => unreachable!("path kind constructed above"),
    };
    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    kind.remap_string_ids(&remap);
    path_syntax.remap_path_ids(&path_remap);
    assert!(matches!(kind, TokenKind::Path(id) if id == handle_before));
    let path = path_syntax.try_path(button).expect("valid path handle");
    assert_eq!(
        path_strings(&global_paths, path.root, &global_table),
        vec!["ui", "Button"]
    );
}

#[test]
fn path_table_root_components_keep_their_allocation_under_remap() {
    let mut local_table = StringTable::new();
    let mut local_paths = PathInternerFork::empty();
    let mut global_table = StringTable::new();
    let mut global_paths = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let button = path_syntax.push(
        local_paths
            .try_intern_components(&[
                local_table.intern("components"),
                local_table.intern("Button"),
            ])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let remap = global_table.merge_from(&local_table);
    let path_remap = global_paths
        .merge_delta_from(&local_paths, &remap)
        .expect("path delta should merge");
    path_syntax.remap_path_ids(&path_remap);
    let path = path_syntax.try_path(button).expect("valid path handle");
    assert_eq!(
        path_strings(&global_paths, path.root, &global_table),
        vec!["components", "Button"]
    );
}
