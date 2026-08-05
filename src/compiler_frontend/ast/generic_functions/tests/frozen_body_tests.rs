//! Focused tests for the frozen generic body token buffer.
//!
//! WHAT: proves canonical token payloads round-trip through one frozen string pool, repeated
//! spellings share one pool entry, and the frozen buffer stays `Send` without donor-local
//! identities.
//! WHY: generated sidecars must reparse a validated generic body against a fresh string table
//! without retaining donor `StringId`/`InternedPath` handles or a mirrored token-kind enum.

use super::{FrozenStringPool, StableBodySyntax, check_materialisation_row_identity};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::numeric_text::token::{
    NumericExponentSign, NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    CharPosition, FileTokens, PathTokenItem, SourceLocation, Token, TokenKind,
};

fn location(scope: &str, string_table: &mut StringTable) -> SourceLocation {
    SourceLocation::new(
        InternedPath::from_single_str(scope, string_table),
        CharPosition::default(),
        CharPosition::default(),
    )
}

fn sample_tokens(string_table: &mut StringTable) -> Vec<Token> {
    let numeric = NumericLiteralToken::new(
        NumericLiteralSign::Negative,
        string_table.intern("-12.5"),
        string_table.intern("12.5"),
        NumericLiteralKind::DecimalPoint,
        2,
        1,
        0,
        NumericExponentSign::None,
    );
    let path = InternedPath::from_components(vec![
        string_table.intern("provider"),
        string_table.intern("CONST"),
    ]);
    vec![
        Token::new(
            TokenKind::Symbol(string_table.intern("hello")),
            location("src/@mod.moth", string_table),
        ),
        Token::new(
            TokenKind::StyleDirective(string_table.intern("md")),
            location("src/@mod.moth", string_table),
        ),
        Token::new(
            TokenKind::StringSliceLiteral(string_table.intern("slice text")),
            location("src/@mod.moth", string_table),
        ),
        Token::new(
            TokenKind::RawStringLiteral(string_table.intern("raw text")),
            location("src/@mod.moth", string_table),
        ),
        Token::new(
            TokenKind::CharLiteral('x'),
            location("src/@mod.moth", string_table),
        ),
        Token::new(
            TokenKind::BoolLiteral(true),
            location("src/@mod.moth", string_table),
        ),
        Token::new(
            TokenKind::NumericLiteral(numeric),
            location("src/@mod.moth", string_table),
        ),
        Token::new(
            TokenKind::Path(vec![PathTokenItem {
                path: path.clone(),
                alias: Some(string_table.intern("alias")),
                path_location: location("src/@mod.moth", string_table),
                alias_location: Some(location("src/@mod.moth", string_table)),
                from_grouped: true,
            }]),
            location("src/@mod.moth", string_table),
        ),
        Token::new(TokenKind::Import, location("src/@mod.moth", string_table)),
        Token::new(
            TokenKind::ChannelReceive,
            location("src/@mod.moth", string_table),
        ),
    ]
}

fn resolved_token_text(token: &Token, string_table: &StringTable) -> String {
    let scope = token
        .location
        .scope
        .as_components()
        .iter()
        .map(|component| string_table.resolve(*component))
        .collect::<Vec<_>>()
        .join("/");
    let kind_text = match &token.kind {
        TokenKind::Symbol(id) => format!("Symbol({})", string_table.resolve(*id)),
        TokenKind::StyleDirective(id) => {
            format!("StyleDirective({})", string_table.resolve(*id))
        }
        TokenKind::StringSliceLiteral(id) => {
            format!("StringSliceLiteral({})", string_table.resolve(*id))
        }
        TokenKind::RawStringLiteral(id) => {
            format!("RawStringLiteral({})", string_table.resolve(*id))
        }
        TokenKind::CharLiteral(value) => format!("CharLiteral({value})"),
        TokenKind::BoolLiteral(value) => format!("BoolLiteral({value})"),
        TokenKind::NumericLiteral(value) => format!(
            "NumericLiteral({}, {}, {:?}, {}, {}, {}, {:?})",
            string_table.resolve(value.source_text),
            string_table.resolve(value.normalized_text),
            value.kind,
            value.digit_count,
            value.fractional_digit_count,
            value.exponent_digit_count,
            value.exponent_sign,
        ),
        TokenKind::Path(items) => format!(
            "Path({})",
            items
                .iter()
                .map(|item| {
                    let path = item
                        .path
                        .as_components()
                        .iter()
                        .map(|component| string_table.resolve(*component))
                        .collect::<Vec<_>>()
                        .join("/");
                    let alias = item
                        .alias
                        .map(|alias| string_table.resolve(alias))
                        .unwrap_or_default();
                    format!("{path}:{alias}:{}", item.from_grouped)
                })
                .collect::<Vec<_>>()
                .join("|")
        ),
        other => format!("{other:?}"),
    };
    format!("{kind_text}@{scope}")
}

#[test]
fn every_token_payload_round_trips_through_the_frozen_buffer() {
    let mut source_table = StringTable::new();
    let tokens = sample_tokens(&mut source_table);
    let original = FileTokens::new(
        InternedPath::from_single_str("src/@mod.moth", &mut source_table),
        tokens.clone(),
    );

    let frozen = StableBodySyntax::capture(&original, &source_table);
    let mut generated_table = StringTable::new();
    let materialised = frozen
        .materialise(&mut generated_table)
        .expect("frozen body should materialise");

    let original_text = tokens
        .iter()
        .map(|token| resolved_token_text(token, &source_table))
        .collect::<Vec<_>>();
    let materialised_text = materialised
        .tokens
        .iter()
        .map(|token| resolved_token_text(token, &generated_table))
        .collect::<Vec<_>>();
    assert_eq!(
        materialised_text, original_text,
        "every frozen token payload must round-trip through the pool"
    );
    assert_eq!(
        materialised.src_path.to_portable_string(&generated_table),
        "src/@mod.moth"
    );
}

#[test]
fn repeated_spellings_share_one_frozen_string_entry() {
    let mut source_table = StringTable::new();
    let symbol_id = source_table.intern("hello");
    let tokens = vec![
        Token::new(
            TokenKind::Symbol(symbol_id),
            location("src/@mod.moth", &mut source_table),
        ),
        Token::new(
            TokenKind::Symbol(symbol_id),
            location("src/@mod.moth", &mut source_table),
        ),
        Token::new(
            TokenKind::Path(vec![PathTokenItem {
                path: InternedPath::from_components(vec![symbol_id]),
                alias: None,
                path_location: location("src/@mod.moth", &mut source_table),
                alias_location: None,
                from_grouped: false,
            }]),
            location("src/@mod.moth", &mut source_table),
        ),
    ];
    let original = FileTokens::new(
        InternedPath::from_single_str("src/@mod.moth", &mut source_table),
        tokens,
    );

    let frozen = StableBodySyntax::capture(&original, &source_table);
    assert_eq!(
        frozen
            .pool
            .iter()
            .filter(|text| text.as_str() == "hello")
            .count(),
        1,
        "repeated spellings must occupy one frozen string entry"
    );
}

#[test]
fn frozen_body_syntax_is_send_without_donor_identity() {
    fn assert_send<T: Send>() {}

    assert_send::<StableBodySyntax>();
    assert_send::<FrozenStringPool>();
}

fn generated_identity(name: &str) -> GeneratedFunctionIdentity {
    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("frozen-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            origin,
            "@main.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            name.to_owned(),
            None,
        )),
        Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]),
        Box::new([]),
    )
}

#[test]
fn invalid_frozen_token_index_returns_compiler_error() {
    let mut string_table = StringTable::new();
    let frozen = StableBodySyntax {
        source_path: Box::new([]),
        pool: Box::new([]),
        tokens: Box::new([Token::new(
            TokenKind::Symbol(StringId::from_index(0)),
            SourceLocation::default(),
        )]),
    };

    let error = frozen
        .materialise(&mut string_table)
        .expect_err("a frozen payload outside the pool is corrupt");
    assert!(
        error.msg.contains("out-of-range pool entry 0"),
        "unexpected frozen token error: {error:?}"
    );
}

#[test]
fn invalid_frozen_location_index_returns_compiler_error() {
    let mut string_table = StringTable::new();
    let frozen = StableBodySyntax {
        source_path: Box::new([]),
        pool: Box::new([]),
        tokens: Box::new([Token::new(
            TokenKind::Import,
            SourceLocation::new(
                InternedPath::from_components(vec![StringId::from_index(3)]),
                CharPosition::default(),
                CharPosition::default(),
            ),
        )]),
    };

    let error = frozen
        .materialise(&mut string_table)
        .expect_err("a frozen scope outside the pool is corrupt");
    assert!(
        error.msg.contains("out-of-range pool entry 3"),
        "unexpected frozen location error: {error:?}"
    );
}

#[test]
fn stale_in_range_template_row_fails_declaration_identity_validation() {
    let expected = generated_identity("expected");
    let stale = generated_identity("stale");
    let context = super::ModuleMaterialisationContext::from_identities_for_test(vec![
        expected.declaration().clone(),
    ]);
    let artefact = &context.artefacts[0];

    check_materialisation_row_identity(artefact, &expected)
        .expect("the indexed row matches the request identity");
    let error = check_materialisation_row_identity(artefact, &stale)
        .expect_err("a stale but in-range row must never materialise");
    assert!(
        error.msg.contains("declaration identity"),
        "unexpected row identity error: {error:?}"
    );
}
