//! Focused tests for the frozen generic body token buffer.
//!
//! WHAT: proves canonical token payloads round-trip through one frozen string pool, repeated
//! spellings share one pool entry, and the frozen buffer stays `Send` without donor-local
//! identities.
//! WHY: generated sidecars must reparse a validated generic body against a fresh string table
//! without retaining donor `StringId`/`InternedPath` handles or a mirrored token-kind enum.

use super::{
    FrozenStringPool, GenericFunctionTemplate, ModuleMaterialisationPreparation, StableBodySyntax,
    check_materialisation_row_identity,
};
use crate::compiler_frontend::ast::generic_bounds::generated_evidence_pair_is_selected;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::ids::GenericParameterListId;
use crate::compiler_frontend::datatypes::{builtin_type_ids, environment::TypeEnvironment};
use crate::compiler_frontend::numeric_text::token::{
    NumericExponentSign, NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    CharPosition, FileTokens, SourceLocation, Token, TokenKind,
};
use crate::compiler_frontend::traits::ids::TraitId;
use rustc_hash::{FxHashMap, FxHashSet};

fn location(scope: &str, string_table: &mut StringTable) -> SourceLocation {
    SourceLocation::new(
        InternedPath::from_single_str(scope, string_table),
        CharPosition::default(),
        CharPosition::default(),
    )
}

fn sample_tokens(string_table: &mut StringTable) -> (Vec<Token>, PathSyntaxTable, PathSyntaxId) {
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
    let mut path_syntax = PathSyntaxTable::new();
    let path_id = path_syntax.push(
        InternedPath::from_components(vec![
            string_table.intern("provider"),
            string_table.intern("CONST"),
        ]),
        location("src/@mod.moth", string_table),
    );
    let tokens = vec![
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
            TokenKind::Path(path_id),
            location("src/@mod.moth", string_table),
        ),
        Token::new(
            TokenKind::Symbol(string_table.intern("import")),
            location("src/@mod.moth", string_table),
        ),
        Token::new(
            TokenKind::ChannelReceive,
            location("src/@mod.moth", string_table),
        ),
    ];
    (tokens, path_syntax, path_id)
}

fn resolved_token_text(
    token: &Token,
    path_syntax: &PathSyntaxTable,
    string_table: &StringTable,
) -> String {
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
        TokenKind::Path(id) => format!(
            "Path({})",
            path_syntax
                .try_path(*id)
                .expect("valid path handle")
                .root
                .to_portable_string(string_table)
        ),
        other => format!("{other:?}"),
    };
    format!("{kind_text}@{scope}")
}

#[test]
fn every_token_payload_round_trips_through_the_frozen_buffer() {
    let mut source_table = StringTable::new();
    let (tokens, path_syntax, path_id) = sample_tokens(&mut source_table);
    let original = FileTokens::new_with_identity(
        InternedPath::from_single_str("src/@mod.moth", &mut source_table),
        None,
        None,
        tokens.clone(),
        path_syntax.clone(),
    );

    let source_file = original.src_path.clone();
    let frozen = StableBodySyntax::capture(&original, &source_file, &source_table)
        .expect("referenced path rows should freeze into the generic artefact");
    let mut generated_table = StringTable::new();
    let generated_source_file =
        InternedPath::from_single_str("src/@mod.moth", &mut generated_table);
    let materialised = frozen
        .materialise(&generated_source_file, &mut generated_table)
        .expect("frozen body should materialise");

    let original_text = tokens
        .iter()
        .map(|token| resolved_token_text(token, &path_syntax, &source_table))
        .collect::<Vec<_>>();
    let materialised_text = materialised
        .tokens
        .iter()
        .map(|token| resolved_token_text(token, &materialised.path_syntax, &generated_table))
        .collect::<Vec<_>>();
    assert_eq!(
        materialised_text, original_text,
        "every frozen token payload must round-trip through the pool"
    );
    assert_eq!(
        materialised.src_path.to_portable_string(&generated_table),
        "src/@mod.moth"
    );
    assert_eq!(
        materialised
            .path_syntax
            .try_path(path_id)
            .expect("valid path handle")
            .root
            .to_portable_string(&generated_table),
        "provider/CONST",
        "frozen path syntax rows must round-trip through the pool"
    );
}

#[test]
fn frozen_body_keeps_declaration_path_distinct_from_owning_source_file() {
    let mut source_table = StringTable::new();
    let (tokens, path_syntax, _) = sample_tokens(&mut source_table);
    let source_file = InternedPath::from_single_str("src/@mod.moth", &mut source_table);
    let declaration_path = source_file.join_str("generic_fn", &mut source_table);
    let original = FileTokens::new_with_identity(declaration_path, None, None, tokens, path_syntax);

    let frozen = StableBodySyntax::capture(&original, &source_file, &source_table)
        .expect("a declaration-qualified stream with file-owned locations should freeze");
    let mut generated_table = StringTable::new();
    let generated_source_file =
        InternedPath::from_single_str("src/@mod.moth", &mut generated_table);
    let materialised = frozen
        .materialise(&generated_source_file, &mut generated_table)
        .expect("the frozen body should retain its distinct declaration and file identities");

    assert_eq!(
        materialised.src_path.to_portable_string(&generated_table),
        "src/@mod.moth/generic_fn"
    );
    assert!(
        materialised
            .tokens
            .iter()
            .all(|token| token.location.scope == generated_source_file)
    );
    materialised
        .path_syntax
        .validate_file_owned_locations(&generated_source_file)
        .expect("canonical path rows stay owned by the source file, not the declaration path");
}

#[test]
fn frozen_body_preserves_multiple_referenced_canonical_path_expressions() {
    let mut source_table = StringTable::new();
    let mut path_syntax = PathSyntaxTable::new();
    let base_location = location("src/@mod.moth", &mut source_table);

    let nested_path = path_syntax.push(
        InternedPath::from_components(vec![
            source_table.intern("provider"),
            source_table.intern("nested"),
            source_table.intern("leaf"),
        ]),
        base_location.clone(),
    );

    let component_path = path_syntax.push(
        InternedPath::from_components(vec![
            source_table.intern("provider"),
            source_table.intern("chain"),
            source_table.intern("part"),
        ]),
        base_location,
    );
    path_syntax.push(
        InternedPath::from_single_str("unused", &mut source_table),
        location("src/@mod.moth", &mut source_table),
    );

    let original = FileTokens::new_with_identity(
        InternedPath::from_single_str("src/@mod.moth", &mut source_table),
        None,
        None,
        vec![
            Token::new(
                TokenKind::Path(nested_path),
                location("src/@mod.moth", &mut source_table),
            ),
            Token::new(
                TokenKind::Path(component_path),
                location("src/@mod.moth", &mut source_table),
            ),
        ],
        path_syntax.clone(),
    );
    let source_file = original.src_path.clone();
    let frozen = StableBodySyntax::capture(&original, &source_file, &source_table)
        .expect("referenced path rows should freeze into the generic artefact");
    let mut generated_table = StringTable::new();
    let generated_source_file =
        InternedPath::from_single_str("src/@mod.moth", &mut generated_table);
    let materialised = frozen
        .materialise(&generated_source_file, &mut generated_table)
        .expect("frozen path selection kinds should materialise");

    assert_eq!(
        materialised.path_syntax.paths().len(),
        2,
        "persistent generic syntax must retain only its two referenced canonical path rows"
    );

    assert_eq!(
        materialised
            .path_syntax
            .try_path(nested_path)
            .expect("valid path handle")
            .root
            .len(),
        3
    );
    assert_eq!(
        materialised
            .path_syntax
            .try_path(component_path)
            .expect("valid path handle")
            .root
            .len(),
        3
    );
}

#[test]
fn repeated_spellings_share_one_frozen_string_entry() {
    let mut source_table = StringTable::new();
    let symbol_id = source_table.intern("hello");
    let mut path_syntax = PathSyntaxTable::new();
    let path_id = path_syntax.push(
        InternedPath::from_components(vec![symbol_id]),
        location("src/@mod.moth", &mut source_table),
    );
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
            TokenKind::Path(path_id),
            location("src/@mod.moth", &mut source_table),
        ),
    ];
    let original = FileTokens::new_with_identity(
        InternedPath::from_single_str("src/@mod.moth", &mut source_table),
        None,
        None,
        tokens,
        path_syntax,
    );

    let source_file = original.src_path.clone();
    let frozen = StableBodySyntax::capture(&original, &source_file, &source_table)
        .expect("referenced path rows should freeze into the generic artefact");
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

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn persistent_generic_subset_counts_stay_separate_from_authored_path_rows() {
    use crate::compiler_frontend::instrumentation::{
        capture_frontend_counters_for_test, log_frontend_counters, reset_frontend_counters,
    };
    use crate::timing::start_benchmark_collection;

    let mut source_table = StringTable::new();
    let (tokens, path_syntax, _) = sample_tokens(&mut source_table);
    let source_path = InternedPath::from_single_str("src/@mod.moth", &mut source_table);
    let original = FileTokens::new_with_identity(source_path, None, None, tokens, path_syntax);

    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    StableBodySyntax::capture(&original, &original.src_path, &source_table)
        .expect("the sample generic body should capture its canonical path subset");

    log_frontend_counters();
    let observations = timing_session.finish();
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };

    assert_eq!(counter_value("path_syntax_row_count"), 0.0);
    assert_eq!(
        counter_value("persistent_generic_path_syntax_subset_copy_count"),
        1.0
    );
    assert_eq!(
        counter_value("persistent_generic_path_syntax_row_copy_count"),
        1.0
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

fn retained_template(
    path: InternedPath,
    declaration_identity: GeneratedDeclarationIdentity,
    has_body: bool,
) -> GenericFunctionTemplate {
    GenericFunctionTemplate {
        function_path: path.clone(),
        source_file: path.clone(),
        declaration_identity: Some(declaration_identity),
        generic_parameter_owner: None,
        generic_parameter_list_id: GenericParameterListId(0),
        signature: FunctionSignature::default(),
        body_tokens: has_body.then(|| FileTokens::new(path, Vec::new())),
        declaration_location: SourceLocation::default(),
    }
}

#[test]
fn requester_template_identity_index_is_exact_and_rejects_duplicate_bodies() {
    let mut string_table = StringTable::new();
    let identity = generated_identity("indexed").declaration().clone();
    let body_path = InternedPath::from_single_str("src/indexed.moth", &mut string_table);
    let imported_path = InternedPath::from_single_str("src/imported.moth", &mut string_table);
    let mut templates = FxHashMap::default();
    templates.insert(
        body_path.clone(),
        retained_template(body_path.clone(), identity.clone(), true),
    );
    templates.insert(
        imported_path.clone(),
        retained_template(imported_path, identity.clone(), false),
    );

    let index = ModuleMaterialisationPreparation::generic_template_identity_index(&templates)
        .expect("bodyless imported templates must not conflict with retained bodies");
    assert_eq!(index.get(&identity), Some(&body_path));

    let duplicate_path = InternedPath::from_single_str("src/duplicate.moth", &mut string_table);
    templates.insert(
        duplicate_path.clone(),
        retained_template(duplicate_path, identity, true),
    );
    assert!(
        ModuleMaterialisationPreparation::generic_template_identity_index(&templates).is_err(),
        "two retained bodies must not publish one declaration identity"
    );
}

#[test]
fn invalid_frozen_token_index_returns_compiler_error() {
    let mut string_table = StringTable::new();
    let frozen = StableBodySyntax {
        declaration_path: Box::new([]),
        pool: Box::new([]),
        tokens: Box::new([Token::new(
            TokenKind::Symbol(StringId::from_index(0)),
            SourceLocation::default(),
        )]),
        path_syntax: Default::default(),
    };

    let error = frozen
        .materialise(&InternedPath::default(), &mut string_table)
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
        declaration_path: Box::new([]),
        pool: Box::new([]),
        tokens: Box::new([Token::new(
            TokenKind::Eof,
            SourceLocation::new(
                InternedPath::from_components(vec![StringId::from_index(3)]),
                CharPosition::default(),
                CharPosition::default(),
            ),
        )]),
        path_syntax: Default::default(),
    };

    let error = frozen
        .materialise(&InternedPath::default(), &mut string_table)
        .expect_err("a frozen scope outside the pool is corrupt");
    assert!(
        error.msg.contains("out-of-range pool entry 3"),
        "unexpected frozen location error: {error:?}"
    );
}

#[test]
fn frozen_body_rejects_token_scope_outside_the_materialised_source_identity() {
    let mut string_table = StringTable::new();
    let frozen = StableBodySyntax {
        declaration_path: Box::new(["src/@mod.moth".to_owned()]),
        pool: Box::new(["other.moth".to_owned()]),
        tokens: Box::new([Token::new(
            TokenKind::Eof,
            SourceLocation::new(
                InternedPath::from_components(vec![StringId::from_index(0)]),
                CharPosition::default(),
                CharPosition::default(),
            ),
        )]),
        path_syntax: Default::default(),
    };

    let source_file = InternedPath::from_single_str("src/@mod.moth", &mut string_table);
    let error = frozen
        .materialise(&source_file, &mut string_table)
        .expect_err("a frozen token must retain the same source identity as its body");
    assert!(
        error.msg.contains(
            "frozen generic body location does not use the prepared file's source identity"
        ),
        "unexpected frozen source-scope error: {error:?}"
    );
}

#[test]
fn invalid_frozen_path_handle_returns_compiler_error() {
    let mut source_table = StringTable::new();
    let (tokens, path_syntax, _) = sample_tokens(&mut source_table);
    let source_path = InternedPath::from_single_str("src/@mod.moth", &mut source_table);
    let original = FileTokens::new_with_identity(source_path, None, None, tokens, path_syntax);
    let source_file = original.src_path.clone();
    let mut frozen = StableBodySyntax::capture(&original, &source_file, &source_table)
        .expect("referenced path rows should freeze into the generic artefact");
    let TokenKind::Path(path_id) = &mut frozen
        .tokens
        .iter_mut()
        .find(|token| matches!(&token.kind, TokenKind::Path(_)))
        .expect("sample body should contain one path token")
        .kind
    else {
        panic!("sample body should retain a path token");
    };
    *path_id = PathSyntaxId::NONE;

    let mut generated_table = StringTable::new();
    let generated_source_file =
        InternedPath::from_single_str("src/@mod.moth", &mut generated_table);
    let error = frozen
        .materialise(&generated_source_file, &mut generated_table)
        .expect_err("a stale frozen path handle must fail before downstream parsing");
    assert!(
        error.msg.contains("absent PathSyntaxId marker"),
        "unexpected frozen path-handle error: {error:?}"
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
        "unexpected declaration identity error: {error:?}"
    );
}

#[test]
fn generated_evidence_authorization_requires_the_selected_trait_pair() {
    let type_environment = TypeEnvironment::new();
    let selected = FxHashSet::from_iter([(builtin_type_ids::INT, TraitId(7))]);

    assert!(generated_evidence_pair_is_selected(
        builtin_type_ids::INT,
        TraitId(7),
        &type_environment,
        &selected,
    ));
    assert!(!generated_evidence_pair_is_selected(
        builtin_type_ids::INT,
        TraitId(8),
        &type_environment,
        &selected,
    ));
}
