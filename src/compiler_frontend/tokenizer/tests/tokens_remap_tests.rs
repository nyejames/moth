//! Tokenizer string-ID remapping tests.
//!
//! WHAT: verifies that token streams produced from local string tables can be remapped into a
//! merged module/global table without losing source locations or path-token metadata.
//! WHY: per-file frontend preparation depends on token outputs being safe to merge before
//! module-wide header parsing and dependency sorting consume them.

use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};

fn make_location(scope: InternedPath) -> SourceLocation {
    SourceLocation::new(scope, CharPosition::default(), CharPosition::default())
}

fn make_token(kind: TokenKind, scope: InternedPath) -> Token {
    Token::new(kind, make_location(scope))
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
    let mut global_table = StringTable::new();

    let scope = InternedPath::from_single_str("test.moth", &mut local_table);
    let mut path_syntax = PathSyntaxTable::new();
    let id = path_syntax.push(
        InternedPath::from_components(vec![
            local_table.intern("components"),
            local_table.intern("Button"),
        ]),
        make_location(scope),
    );

    let _alpha_global = global_table.intern("alpha");

    let remap = global_table.merge_from(&local_table);

    path_syntax.remap_string_ids(&remap);

    let path = path_syntax.try_path(id).expect("valid path handle");
    let path_strings: Vec<&str> = path
        .root
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(path_strings, vec!["components", "Button"]);

    let path_scope_strings: Vec<&str> = path
        .location
        .scope
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(path_scope_strings, vec!["test.moth"]);
}

#[test]
fn file_tokens_remaps_src_path_and_tokens_preserves_canonical_os_path() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let src_path_local = InternedPath::from_single_str("local.moth", &mut local_table);
    let token_scope_local = InternedPath::from_single_str("local.moth", &mut local_table);

    let symbol_local = local_table.intern("my_symbol");

    let tokens = vec![
        make_token(TokenKind::Symbol(symbol_local), token_scope_local.clone()),
        make_token(
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("7", &mut local_table)),
            token_scope_local.clone(),
        ),
    ];

    let canonical_path = std::path::PathBuf::from("/absolute/local.moth");
    let mut file_tokens = FileTokens::new_with_identity(
        src_path_local.clone(),
        None,
        Some(canonical_path.clone()),
        tokens,
        PathSyntaxTable::new(),
    );

    global_table.intern("preexisting");

    let remap = global_table.merge_from(&local_table);

    file_tokens.remap_string_ids(&remap);

    let src_path_strings: Vec<&str> = file_tokens
        .src_path
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(src_path_strings, vec!["local.moth"]);

    assert_eq!(
        file_tokens.canonical_os_path,
        Some(canonical_path),
        "canonical_os_path should be preserved by remap"
    );

    let first_token = file_tokens
        .tokens
        .first()
        .expect("first token should exist");
    assert!(
        matches!(first_token.kind, TokenKind::Symbol(id) if global_table.resolve(id) == "my_symbol"),
        "token symbol should resolve correctly after remap"
    );

    let first_location_strings: Vec<&str> = first_token
        .location
        .scope
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(first_location_strings, vec!["local.moth"]);

    let second_token = file_tokens
        .tokens
        .get(1)
        .expect("second token should exist");
    assert!(
        matches!(second_token.kind, TokenKind::NumericLiteral(_)),
        "numeric token should remain numeric after remap"
    );
}

#[test]
fn file_tokens_with_path_tokens_leave_table_remapping_to_the_prepared_file_owner() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let src_path_local = InternedPath::from_single_str("module.moth", &mut local_table);
    let token_scope_local = InternedPath::from_single_str("module.moth", &mut local_table);

    let mut path_syntax = PathSyntaxTable::new();
    let ui_button = path_syntax.push(
        InternedPath::from_components(vec![local_table.intern("ui"), local_table.intern("Button")]),
        make_location(token_scope_local.clone()),
    );
    let utils_helper = path_syntax.push(
        InternedPath::from_components(vec![
            local_table.intern("utils"),
            local_table.intern("helper"),
        ]),
        make_location(token_scope_local.clone()),
    );

    let tokens = vec![
        make_token(TokenKind::Path(ui_button), token_scope_local.clone()),
        make_token(TokenKind::Path(utils_helper), token_scope_local),
    ];

    let mut file_tokens =
        FileTokens::new_with_identity(src_path_local, None, None, tokens, path_syntax);

    let remap = global_table.merge_from(&local_table);

    file_tokens.remap_string_ids(&remap);

    assert!(
        matches!(file_tokens.tokens[0].kind, TokenKind::Path(id) if id == ui_button),
        "path handles are dense table indexes and must survive remap unchanged"
    );

    let first = file_tokens
        .path_syntax
        .try_path(ui_button)
        .expect("valid path handle");
    let first_path: Vec<&str> = first
        .root
        .as_components()
        .iter()
        .map(|id| local_table.resolve(*id))
        .collect();
    assert_eq!(first_path, vec!["ui", "Button"]);

    let second = file_tokens
        .path_syntax
        .try_path(utils_helper)
        .expect("valid path handle");
    let second_path: Vec<&str> = second
        .root
        .as_components()
        .iter()
        .map(|id| local_table.resolve(*id))
        .collect();
    assert_eq!(second_path, vec!["utils", "helper"]);
}

#[test]
fn file_tokens_preparing_remap_updates_owned_path_table() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let source_path = InternedPath::from_single_str("module.moth", &mut local_table);
    let mut path_syntax = PathSyntaxTable::new();
    let button = path_syntax.push(
        InternedPath::from_components(vec![local_table.intern("ui"), local_table.intern("Button")]),
        make_location(source_path.clone()),
    );
    let tokens = vec![make_token(TokenKind::Path(button), source_path.clone())];
    let mut file_tokens =
        FileTokens::new_with_identity(source_path, None, None, tokens, path_syntax);

    global_table.intern("preexisting");
    let remap = global_table.merge_from(&local_table);

    file_tokens
        .remap_preparing_string_ids(&remap)
        .expect("the preparing token stream should own a mutable path table");

    let path = file_tokens
        .path_syntax
        .try_path(button)
        .expect("valid path handle");
    let path_strings: Vec<&str> = path
        .root
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(path_strings, vec!["ui", "Button"]);
}

#[test]
fn rebind_source_identity_updates_scopes_without_changing_spans_or_paths() {
    let mut table = StringTable::new();

    let original_scope = InternedPath::from_single_str("stage0_absolute.moth", &mut table);
    let logical_scope = InternedPath::from_single_str("module/logical.moth", &mut table);

    let mut path_syntax = PathSyntaxTable::new();
    let helper_util = path_syntax.push(
        InternedPath::from_components(vec![table.intern("helper"), table.intern("util")]),
        make_location(original_scope.clone()),
    );
    let tokens = vec![
        make_token(
            TokenKind::Symbol(table.intern("alpha")),
            original_scope.clone(),
        ),
        make_token(TokenKind::Path(helper_util), original_scope.clone()),
    ];

    let canonical = std::path::PathBuf::from("/canonical/logical.moth");
    let mut file_tokens =
        FileTokens::new_with_identity(original_scope.clone(), None, None, tokens, path_syntax);

    let file_id = crate::compiler_frontend::source::SourceId::from_index(7);
    file_tokens
        .rebind_source_identity(
            logical_scope.clone(),
            Some(file_id),
            Some(canonical.clone()),
        )
        .expect("the sole mutable source table should accept final identity rebinding");

    // Top-level identity fields are rebound.
    assert_eq!(file_tokens.src_path, logical_scope);
    assert_eq!(file_tokens.file_id, Some(file_id));
    assert_eq!(file_tokens.canonical_os_path, Some(canonical));

    // Every token location scope is rebound, spans are untouched.
    for token in &file_tokens.tokens {
        assert_eq!(token.location.scope, logical_scope);
        assert_eq!(token.location.start_pos, CharPosition::default());
        assert_eq!(token.location.end_pos, CharPosition::default());
    }

    // Path table locations are rebound but the root payload is unchanged.
    let path = file_tokens
        .path_syntax
        .try_path(helper_util)
        .expect("valid path handle");
    assert_eq!(path.location.scope, logical_scope);
    let path_strings: Vec<&str> = path
        .root
        .as_components()
        .iter()
        .map(|id| table.resolve(*id))
        .collect();
    assert_eq!(path_strings, vec!["helper", "util"]);
}

#[test]
fn token_kind_path_handle_is_a_remap_no_op_while_table_rows_remap() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let mut path_syntax = PathSyntaxTable::new();
    let button = path_syntax.push(
        InternedPath::from_components(vec![local_table.intern("ui"), local_table.intern("Button")]),
        make_location(InternedPath::from_single_str("test.moth", &mut local_table)),
    );
    let mut kind = TokenKind::Path(button);
    let handle_before = match &kind {
        TokenKind::Path(id) => *id,
        _ => unreachable!("path kind constructed above"),
    };

    let remap = global_table.merge_from(&local_table);
    kind.remap_string_ids(&remap);
    path_syntax.remap_string_ids(&remap);

    assert!(
        matches!(kind, TokenKind::Path(id) if id == handle_before),
        "path handles are dense table indexes and must not be rewritten by string remap"
    );
    let path_strings: Vec<&str> = path_syntax
        .try_path(button)
        .expect("valid path handle")
        .root
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(path_strings, vec!["ui", "Button"]);
}

#[test]
fn path_table_root_components_keep_their_allocation_under_remap() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let mut path_syntax = PathSyntaxTable::new();
    let button = path_syntax.push(
        InternedPath::from_components(vec![
            local_table.intern("components"),
            local_table.intern("Button"),
        ]),
        make_location(InternedPath::from_single_str("test.moth", &mut local_table)),
    );
    let components_ptr = path_syntax
        .try_path(button)
        .expect("valid path handle")
        .root
        .as_components()
        .as_ptr();

    let remap = global_table.merge_from(&local_table);
    path_syntax.remap_string_ids(&remap);

    assert_eq!(
        path_syntax
            .try_path(button)
            .expect("valid path handle")
            .root
            .as_components()
            .as_ptr(),
        components_ptr,
        "in-place remapping must keep the interned path allocation"
    );
    let path_strings: Vec<&str> = path_syntax
        .try_path(button)
        .expect("valid path handle")
        .root
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(path_strings, vec!["components", "Button"]);
}
