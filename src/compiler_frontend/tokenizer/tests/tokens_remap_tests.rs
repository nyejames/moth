//! Tokenizer string-ID remapping tests.
//!
//! WHAT: verifies that token streams produced from local string tables can be remapped into a
//! merged module/global table without losing source locations or path-token metadata.
//! WHY: per-file frontend preparation depends on token outputs being safe to merge before
//! module-wide header parsing and dependency sorting consume them.

use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, PathTokenItem, Token, TokenKind};

fn make_location(scope: InternedPath) -> SourceLocation {
    SourceLocation::new(scope, CharPosition::default(), CharPosition::default())
}

fn make_token(kind: TokenKind, scope: InternedPath) -> Token {
    Token::new(kind, make_location(scope))
}

fn make_path_token_item(
    path_components: &[&str],
    alias: Option<&str>,
    string_table: &mut StringTable,
) -> PathTokenItem {
    let components: Vec<StringId> = path_components
        .iter()
        .map(|c| string_table.intern(c))
        .collect();
    let path = InternedPath::from_components(components);
    let alias = alias.map(|a| string_table.intern(a));
    let path_scope = InternedPath::from_single_str("test.moth", string_table);

    PathTokenItem {
        path,
        alias,
        path_location: make_location(path_scope.clone()),
        alias_location: alias.map(|_| make_location(path_scope)),
        from_grouped: false,
    }
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
fn path_token_item_remaps_all_fields() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let item = make_path_token_item(&["components", "Button"], Some("Btn"), &mut local_table);

    let _alpha_global = global_table.intern("alpha");

    let remap = global_table.merge_from(&local_table);

    let mut remapped_item = item.clone();
    remapped_item.remap_string_ids(&remap);

    let path_strings: Vec<&str> = remapped_item
        .path
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(path_strings, vec!["components", "Button"]);

    let alias = remapped_item
        .alias
        .expect("alias should be present after remap");
    assert_eq!(global_table.resolve(alias), "Btn");

    let path_scope = remapped_item.path_location.scope.clone();
    let path_scope_strings: Vec<&str> = path_scope
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(path_scope_strings, vec!["test.moth"]);

    let alias_location = remapped_item
        .alias_location
        .expect("alias location should be present after remap");
    let alias_scope_strings: Vec<&str> = alias_location
        .scope
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(alias_scope_strings, vec!["test.moth"]);

    assert_eq!(remapped_item.from_grouped, item.from_grouped);
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
fn file_tokens_with_path_tokens_remaps_nested_items() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let src_path_local = InternedPath::from_single_str("module.moth", &mut local_table);
    let token_scope_local = InternedPath::from_single_str("module.moth", &mut local_table);

    let path_items = vec![
        make_path_token_item(&["ui", "Button"], Some("Btn"), &mut local_table),
        make_path_token_item(&["utils", "helper"], None, &mut local_table),
    ];

    let tokens = vec![make_token(TokenKind::Path(path_items), token_scope_local)];

    let mut file_tokens = FileTokens::new(src_path_local, tokens);

    let remap = global_table.merge_from(&local_table);

    file_tokens.remap_string_ids(&remap);

    let path_token = file_tokens.tokens.first().expect("path token should exist");

    let items = match &path_token.kind {
        TokenKind::Path(items) => items,
        _ => panic!("expected Path token kind"),
    };

    assert_eq!(items.len(), 2);

    let first = &items[0];
    let first_path: Vec<&str> = first
        .path
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(first_path, vec!["ui", "Button"]);
    assert_eq!(
        global_table.resolve(first.alias.expect("alias should exist")),
        "Btn"
    );

    let second = &items[1];
    let second_path: Vec<&str> = second
        .path
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(second_path, vec!["utils", "helper"]);
    assert!(second.alias.is_none());
}

#[test]
fn rebind_source_identity_updates_scopes_without_changing_spans_or_paths() {
    let mut table = StringTable::new();

    let original_scope = InternedPath::from_single_str("stage0_absolute.moth", &mut table);
    let logical_scope = InternedPath::from_single_str("module/logical.moth", &mut table);

    let path_item = make_path_token_item(&["helper", "util"], Some("u"), &mut table);
    let tokens = vec![
        make_token(
            TokenKind::Symbol(table.intern("alpha")),
            original_scope.clone(),
        ),
        make_token(TokenKind::Path(vec![path_item]), original_scope.clone()),
    ];

    let canonical = std::path::PathBuf::from("/canonical/logical.moth");
    let mut file_tokens = FileTokens::new_with_identity(original_scope.clone(), None, None, tokens);

    let file_id = crate::compiler_frontend::symbols::identity::FileId(7);
    file_tokens.rebind_source_identity(
        logical_scope.clone(),
        Some(file_id),
        Some(canonical.clone()),
    );

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

    // Path token item locations are rebound but the import path payload is unchanged.
    let path_token = &file_tokens.tokens[1];
    let items = match &path_token.kind {
        TokenKind::Path(items) => items,
        _ => panic!("expected Path token"),
    };
    let item = &items[0];
    assert_eq!(item.path_location.scope, logical_scope);
    assert!(item.alias_location.is_some());
    assert_eq!(item.alias_location.as_ref().unwrap().scope, logical_scope);
    let path_strings: Vec<&str> = item
        .path
        .as_components()
        .iter()
        .map(|id| table.resolve(*id))
        .collect();
    assert_eq!(path_strings, vec!["helper", "util"]);
}

#[test]
fn token_kind_path_payload_remaps_in_place_and_keeps_vector_allocation() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let items = vec![
        make_path_token_item(&["ui", "Button"], Some("Btn"), &mut local_table),
        make_path_token_item(&["utils", "helper"], None, &mut local_table),
    ];
    let mut kind = TokenKind::Path(items);
    let items_ptr = match &kind {
        TokenKind::Path(items) => items.as_ptr(),
        _ => unreachable!("path kind constructed above"),
    };

    let remap = global_table.merge_from(&local_table);
    kind.remap_string_ids(&remap);

    let items = match &kind {
        TokenKind::Path(items) => items,
        _ => panic!("path kind must survive remap"),
    };
    assert_eq!(
        items.as_ptr(),
        items_ptr,
        "in-place remapping must not rebuild the path item vector"
    );
    assert_eq!(items.len(), 2);
    let first_path: Vec<&str> = items[0]
        .path
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(first_path, vec!["ui", "Button"]);
}

#[test]
fn path_token_item_path_components_keep_their_allocation_under_remap() {
    let mut local_table = StringTable::new();
    let mut global_table = StringTable::new();

    let mut item = make_path_token_item(&["components", "Button"], Some("Btn"), &mut local_table);
    let components_ptr = item.path.as_components().as_ptr();

    let remap = global_table.merge_from(&local_table);
    item.remap_string_ids(&remap);

    assert_eq!(
        item.path.as_components().as_ptr(),
        components_ptr,
        "in-place remapping must keep the interned path allocation"
    );
    let path_strings: Vec<&str> = item
        .path
        .as_components()
        .iter()
        .map(|id| global_table.resolve(*id))
        .collect();
    assert_eq!(path_strings, vec!["components", "Button"]);
}
