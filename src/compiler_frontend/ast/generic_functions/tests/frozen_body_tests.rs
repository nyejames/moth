//! Focused tests for frozen generic body and resource-default materialisation.
//!
//! WHAT: proves canonical token payloads round-trip through one frozen string pool, repeated
//! spellings share one pool entry, frozen resource defaults cross the generated sidecar boundary
//! through stable origins, and the frozen buffer stays `Send` without donor-local identities.
//! WHY: the integration case `generic_parameter_default_file_value_success` owns authored
//! `@assets/logo.svg` syntax through Stage 0 resolution. These tests own the freeze-to-sidecar
//! seam from that resolved AST representation without retaining donor `StringId`/`InternedPath`/
//! `ResourceId` handles or a mirrored token-kind enum.

use super::{
    FrozenStringPool, GenericFunctionBody, GenericFunctionTemplate, ModuleMaterialisationContext,
    ModuleMaterialisationInput, ModuleMaterialisationPreparation, StableBodySyntax,
    StableResolvedFileReferenceOutcome, check_materialisation_row_identity,
};
use crate::compiler_frontend::ast::Stage0ResolutionFacts;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::generic_bounds::generated_evidence_pair_is_selected;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::materialize_owned_folded_string;
use crate::compiler_frontend::ast::module_ast::scope_context::{
    FrozenResolvedFileReference, FrozenResolvedFileReferenceOutcome,
    Stage0ResolvedFileReferenceOutcome,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::GenericParameterListId;
use crate::compiler_frontend::datatypes::{builtin_type_ids, environment::TypeEnvironment};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicFoldedValue,
};
use crate::compiler_frontend::numeric_text::token::{
    NumericExponentSign, NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::source::{SourceDatabase, SourceId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result;
use crate::compiler_frontend::tokenizer::tokens::{
    CharPosition, FileTokens, SourceLocation, Token, TokenKind,
};
use crate::compiler_frontend::traits::ids::TraitId;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

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

fn capture_test_body(
    original: &FileTokens,
    source_file: &InternedPath,
    source_table: &StringTable,
) -> StableBodySyntax {
    let mut tokens = original.clone();
    tokens.file_id = Some(SourceId::from_index(0));

    let mut resolved_references = ResolvedFileReferenceTable::new();
    for (path_syntax, _) in tokens.path_syntax.iter() {
        resolved_references
            .push(ResolvedFileReference {
                source_file: SourceId::from_index(0),
                path_syntax,
                class: PreparedFileReferenceClass::ResourceFile,
                outcome: ResolvedFileReferenceOutcome::Target(
                    ResolvedFileReferenceTarget::ResourceSource {
                        source: crate::compiler_frontend::paths::file_references::ResourceSourceId::from_index(0),
                        owner_relative_path: PortableResourcePath::from_relative_logical_path(
                            Path::new("assets/test.svg"),
                        )
                        .expect("test resource path is portable"),
                    },
                ),
            })
            .expect("test path rows should be unique");
    }
    let facts = Stage0ResolutionFacts::ordinary(resolved_references, SourceDatabase::empty());
    let no_content_value = |_path: &InternedPath| -> Result<PublicFoldedValue, CompilerError> {
        Err(CompilerError::compiler_error(
            "test body has no content value resolver",
        ))
    };
    StableBodySyntax::capture(
        &tokens,
        source_file,
        source_table,
        Some(&facts),
        &no_content_value,
    )
    .expect("test body path rows should be resolved before capture")
}

fn direct_content_body_fixture() -> (
    FileTokens,
    InternedPath,
    StringTable,
    Stage0ResolutionFacts,
    PathSyntaxId,
    StableResourceOriginId,
) {
    let mut string_table = StringTable::new();
    let source_file = InternedPath::from_single_str("@mod.moth", &mut string_table);
    let path_location = location("@mod.moth", &mut string_table);
    let mut path_syntax = PathSyntaxTable::new();
    let path_id = path_syntax.push(
        InternedPath::from_single_str("@private.mtf", &mut string_table),
        path_location.clone(),
    );
    let tokens = vec![Token::new(TokenKind::Path(path_id), path_location)];
    let body = FileTokens::new_with_identity(
        source_file.clone(),
        Some(SourceId::from_index(0)),
        None,
        tokens,
        path_syntax,
    );

    let source_files = SourceDatabase::build(
        [PathBuf::from("@mod.moth"), PathBuf::from("@private.mtf")],
        Path::new("@mod.moth"),
        None,
        &mut string_table,
    )
    .expect("content fixture source identities should build");
    let mut resolved_references = ResolvedFileReferenceTable::new();
    resolved_references
        .push(ResolvedFileReference {
            source_file: SourceId::from_index(0),
            path_syntax: path_id,
            class: PreparedFileReferenceClass::ContentSource,
            outcome: ResolvedFileReferenceOutcome::Target(
                ResolvedFileReferenceTarget::ContentSource {
                    source: SourceId::from_index(1),
                },
            ),
        })
        .expect("content fixture path rows should be unique");
    let facts = Stage0ResolutionFacts::ordinary(resolved_references, source_files);
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("frozen-content-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    let resource_origin = StableResourceOriginId::module_owned(
        module_origin,
        PortableResourcePath::from_relative_logical_path(Path::new("assets/private.svg"))
            .expect("content fixture resource path should be portable"),
    );
    (
        body,
        source_file,
        string_table,
        facts,
        path_id,
        resource_origin,
    )
}

#[test]
fn frozen_content_value_captures_and_reinterns_resource_pieces() {
    let (body, source_file, source_table, facts, path_id, resource_origin) =
        direct_content_body_fixture();
    let frozen =
        StableBodySyntax::capture(&body, &source_file, &source_table, Some(&facts), &|_| {
            Ok(PublicFoldedValue::String(OwnedFoldedString::Pieces(vec![
                OwnedFoldedStringPiece::Text("private content: ".to_owned()),
                OwnedFoldedStringPiece::Resource(resource_origin.clone()),
            ])))
        })
        .expect("ordinary content value should be captured before freezing");
    let StableResolvedFileReferenceOutcome::Content { value } =
        &frozen.resolved_file_references[0].outcome
    else {
        panic!("content fixture should retain a folded content value");
    };
    assert!(matches!(
        value,
        OwnedFoldedString::Pieces(pieces)
            if matches!(pieces.as_slice(), [
                OwnedFoldedStringPiece::Text(_),
                OwnedFoldedStringPiece::Resource(origin),
            ] if origin == &resource_origin)
    ));

    let mut generated_table = StringTable::new();
    let generated_source_file = InternedPath::from_single_str("@mod.moth", &mut generated_table);
    let materialised = frozen
        .materialise(&generated_source_file, &mut generated_table)
        .expect("frozen content body should materialise");
    let Stage0ResolvedFileReferenceOutcome::Content {
        logical_path: None,
        value: Some(value),
    } = materialised
        .resolution_facts
        .lookup(None, path_id)
        .expect("materialised content row should be readable")
        .expect("materialised content row should be retained")
        .outcome
    else {
        panic!("materialised content row should carry its folded value");
    };
    let module_resources = Rc::new(RefCell::new(ModuleResourceTable::new()));

    let expression_kind = materialize_owned_folded_string(value, &mut generated_table, |origin| {
        Ok(module_resources
            .borrow_mut()
            .intern_origin(origin.clone(), SourceLocation::default()))
    })
    .expect("frozen content value should lower to a structural string");
    let ExpressionKind::StructuralString { pieces } = expression_kind else {
        panic!("resource-bearing content should remain a structural string");
    };
    let [
        ConstStringPiece::Text(prefix),
        ConstStringPiece::Resource(resource),
    ] = pieces.as_slice()
    else {
        panic!("content should retain text and resource pieces, got {pieces:?}");
    };
    assert_eq!(generated_table.resolve(*prefix), "private content: ");
    let sidecar_resources = module_resources.borrow();
    assert_eq!(sidecar_resources.origins().len(), 1);
    assert_eq!(
        sidecar_resources
            .try_origin(*resource)
            .expect("sidecar resource handle should resolve")
            .origin,
        resource_origin,
    );
}

#[test]
fn missing_content_fold_fails_loudly_during_capture() {
    let (body, source_file, source_table, facts, _, _) = direct_content_body_fixture();
    let error =
        match StableBodySyntax::capture(&body, &source_file, &source_table, Some(&facts), &|_| {
            Err(CompilerError::compiler_error(
                "synthetic content constant was not folded before capture",
            ))
        }) {
            Ok(_) => panic!("capture must reject content without a folded value"),
            Err(error) => error,
        };
    assert!(
        error
            .msg
            .contains("synthetic content constant was not folded before capture"),
        "unexpected content capture error: {error:?}"
    );
}

#[test]
fn non_string_content_fold_fails_loudly_during_capture() {
    let (body, source_file, source_table, facts, _, _) = direct_content_body_fixture();
    let error =
        match StableBodySyntax::capture(&body, &source_file, &source_table, Some(&facts), &|_| {
            Ok(PublicFoldedValue::Int(7))
        }) {
            Ok(_) => panic!("capture must reject a non-string content value"),
            Err(error) => error,
        };
    assert!(
        error
            .msg
            .contains("synthetic content constant did not fold to a String value"),
        "unexpected content capture error: {error:?}"
    );
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
    let frozen = capture_test_body(&original, &source_file, &source_table);
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
        .file_tokens
        .tokens
        .iter()
        .map(|token| {
            resolved_token_text(
                token,
                &materialised.file_tokens.path_syntax,
                &generated_table,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        materialised_text, original_text,
        "every frozen token payload must round-trip through the pool"
    );
    assert_eq!(
        materialised
            .file_tokens
            .src_path
            .to_portable_string(&generated_table),
        "src/@mod.moth"
    );
    assert_eq!(
        materialised
            .file_tokens
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

    let frozen = capture_test_body(&original, &source_file, &source_table);
    let mut generated_table = StringTable::new();
    let generated_source_file =
        InternedPath::from_single_str("src/@mod.moth", &mut generated_table);
    let materialised = frozen
        .materialise(&generated_source_file, &mut generated_table)
        .expect("the frozen body should retain its distinct declaration and file identities");

    assert_eq!(
        materialised
            .file_tokens
            .src_path
            .to_portable_string(&generated_table),
        "src/@mod.moth/generic_fn"
    );
    assert!(
        materialised
            .file_tokens
            .tokens
            .iter()
            .all(|token| token.location.scope == generated_source_file)
    );
    materialised
        .file_tokens
        .path_syntax
        .validate_file_owned_locations(&generated_source_file)
        .expect("canonical path rows stay owned by the source file, not the declaration path");
}

#[test]
fn frozen_body_preserves_multiple_referenced_canonical_path_expressions() {
    let mut source_table = StringTable::new();
    let mut path_syntax = PathSyntaxTable::new();
    let base_location = location("src/@mod.moth", &mut source_table);

    // The body references donor row 2 before donor row 0. Donor row 1 is deliberately
    // unreferenced, so the compact table must assign new handles rather than preserving either
    // referenced donor handle.
    let first_donor_path = path_syntax.push(
        InternedPath::from_components(vec![
            source_table.intern("provider"),
            source_table.intern("first"),
        ]),
        base_location.clone(),
    );
    let _unreferenced_donor_path = path_syntax.push(
        InternedPath::from_single_str("provider/unused", &mut source_table),
        base_location.clone(),
    );
    let second_donor_path = path_syntax.push(
        InternedPath::from_components(vec![
            source_table.intern("provider"),
            source_table.intern("second"),
        ]),
        base_location.clone(),
    );
    let source_file = InternedPath::from_single_str("src/@mod.moth", &mut source_table);
    let original = FileTokens::new_with_identity(
        source_file.clone(),
        Some(SourceId::from_index(0)),
        None,
        vec![
            Token::new(TokenKind::Path(second_donor_path), base_location.clone()),
            Token::new(TokenKind::Path(first_donor_path), base_location),
        ],
        path_syntax.clone(),
    );

    let mut resolved_references = ResolvedFileReferenceTable::new();
    for (path_syntax, owner_relative_path) in [
        (first_donor_path, "assets/first.svg"),
        (_unreferenced_donor_path, "assets/unused.svg"),
        (second_donor_path, "assets/second.svg"),
    ] {
        resolved_references
            .push(ResolvedFileReference {
                source_file: SourceId::from_index(0),
                path_syntax,
                class: PreparedFileReferenceClass::ResourceFile,
                outcome: ResolvedFileReferenceOutcome::Target(
                    ResolvedFileReferenceTarget::ResourceSource {
                        source: crate::compiler_frontend::paths::file_references::ResourceSourceId::from_index(0),
                        owner_relative_path: PortableResourcePath::from_relative_logical_path(
                            Path::new(owner_relative_path),
                        )
                        .expect("remapping fixture resource path should be portable"),
                    },
                ),
            })
            .expect("remapping fixture path rows should be unique");
    }
    let facts = Stage0ResolutionFacts::ordinary(resolved_references, SourceDatabase::empty());
    let frozen = StableBodySyntax::capture(
        &original,
        &source_file,
        &source_table,
        Some(&facts),
        &|_| {
            Err::<PublicFoldedValue, CompilerError>(CompilerError::compiler_error(
                "remapping fixture has no content values",
            ))
        },
    )
    .expect("remapping fixture body should freeze");

    let compact_ids = frozen
        .path_syntax
        .iter()
        .map(|(path_id, _)| path_id)
        .collect::<Vec<_>>();
    let captured_token_ids = frozen
        .tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::Path(path_id) => Some(path_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        captured_token_ids, compact_ids,
        "captured path tokens must use the compact table handles in token order"
    );
    let captured_reference_ids = frozen
        .resolved_file_references
        .iter()
        .map(|reference| reference.path_syntax)
        .collect::<Vec<_>>();
    assert_eq!(
        captured_reference_ids, compact_ids,
        "captured resolved-reference rows must use the same compact handles as the tokens"
    );
    assert_ne!(
        second_donor_path, compact_ids[0],
        "the first referenced donor handle must be remapped"
    );
    assert_ne!(
        first_donor_path, compact_ids[1],
        "the second referenced donor handle must be remapped"
    );

    let mut generated_table = StringTable::new();
    let generated_source_file =
        InternedPath::from_single_str("src/@mod.moth", &mut generated_table);
    let materialised = frozen
        .materialise(&generated_source_file, &mut generated_table)
        .expect("remapping fixture body should materialise");
    let resolve_resource = |path_id| {
        let reference = materialised
            .resolution_facts
            .lookup(None, path_id)
            .expect("materialised facts should accept a compact handle")
            .expect("materialised facts should retain each compact row");
        let Stage0ResolvedFileReferenceOutcome::Resource {
            owner_relative_path,
            ..
        } = reference.outcome
        else {
            panic!("remapping fixture should retain resource outcomes");
        };
        owner_relative_path.as_str().to_owned()
    };
    assert_eq!(
        resolve_resource(compact_ids[0]),
        "assets/second.svg",
        "the first compact handle must resolve the row referenced by donor handle 2"
    );
    assert_eq!(
        resolve_resource(compact_ids[1]),
        "assets/first.svg",
        "the second compact handle must resolve the row referenced by donor handle 0"
    );
    assert!(
        materialised
            .resolution_facts
            .lookup(None, second_donor_path)
            .expect("materialised facts should accept a donor handle lookup")
            .is_none(),
        "a donor handle that differs from its compact handle must not select a retained row"
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
    let frozen = capture_test_body(&original, &source_file, &source_table);
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

    capture_test_body(&original, &original.src_path, &source_table);

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
        body_tokens: has_body
            .then(|| GenericFunctionBody::source(FileTokens::new(path, Vec::new()))),
        declaration_location: SourceLocation::default(),
    }
}

struct ResourceDefaultMaterialisationFixture {
    preparation: ModuleMaterialisationPreparation,
    context: ModuleMaterialisationContext,
    identity: GeneratedFunctionIdentity,
    declaring_resources: ModuleResourceTable,
    declaring_resource: ResourceId,
    resource_origin: StableResourceOriginId,
}

fn resource_default_materialisation_fixture() -> ResourceDefaultMaterialisationFixture {
    let source =
        "draw type T |name T, suffix String = \"fallback\"| -> String:\n    return \"ok\"\n;\n";
    let (mut build_result, string_table) =
        parse_single_file_ast_build_result(source).expect("generic source should build");

    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("resource-default-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    let resource_origin = StableResourceOriginId::module_owned(
        module_origin.clone(),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
            .expect("resource path should be portable"),
    );
    let decoy_origin = StableResourceOriginId::module_owned(
        module_origin.clone(),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/decoy.svg"))
            .expect("decoy resource path should be portable"),
    );
    let mut declaring_resources = ModuleResourceTable::new();
    declaring_resources.intern_origin(decoy_origin, SourceLocation::default());
    let declaring_resource =
        declaring_resources.intern_origin(resource_origin.clone(), SourceLocation::default());

    let template = build_result
        .materialisation_context
        .generic_function_templates_mut()
        .values_mut()
        .next()
        .expect("the source should retain one generic template");
    let declaration_identity =
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            module_origin.clone(),
            "@page.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            "draw".to_owned(),
            None,
        ));
    template.declaration_identity = Some(declaration_identity.clone());
    let parameter = template
        .signature
        .parameters
        .iter_mut()
        .find(|parameter| parameter.id.name_str(&string_table) == Some("suffix"))
        .expect("the generic should retain its suffix parameter");
    // A resolved resource file value reaches the AST as a structural resource piece. Keep its
    // declaring-table handle here so freezing must project the stable origin before materialising.
    parameter.value.kind = ExpressionKind::StructuralString {
        pieces: vec![ConstStringPiece::Resource(declaring_resource)],
    };

    let mut preparation = build_result
        .materialisation_context
        .finish_preparation()
        .expect("generic template identity index should build");
    preparation.module_origin = Some(module_origin.clone());
    let public_interface = PublicSemanticInterface {
        module_origin,
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let context = preparation
        .clone()
        .freeze(&public_interface, &declaring_resources)
        .expect("generic resource default should freeze")
        .expect("the retained generic should produce a materialisation context");
    let identity = GeneratedFunctionIdentity::new(
        declaration_identity,
        Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]),
        Box::new([]),
    );

    ResourceDefaultMaterialisationFixture {
        preparation,
        context,
        identity,
        declaring_resources,
        declaring_resource,
        resource_origin,
    }
}

struct ResourceBodyMaterialisationFixture {
    preparation: ModuleMaterialisationPreparation,
    context: ModuleMaterialisationContext,
    identity: GeneratedFunctionIdentity,
    resource_origin: StableResourceOriginId,
}

fn resource_body_materialisation_fixture() -> ResourceBodyMaterialisationFixture {
    let source = "draw type T |name T| -> String:\n    return \"placeholder\"\n;\n";
    let (mut build_result, string_table) =
        parse_single_file_ast_build_result(source).expect("generic source should build");

    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("resource-body-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    let resource_origin = StableResourceOriginId::module_owned(
        module_origin.clone(),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
            .expect("resource path should be portable"),
    );
    let declaration_identity =
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            module_origin.clone(),
            "@page.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            "draw".to_owned(),
            None,
        ));
    let assets_component = build_result
        .materialisation_context
        .context
        .string_table
        .intern("assets");
    let logo_component = build_result
        .materialisation_context
        .context
        .string_table
        .intern("logo.svg");

    let (body_source_file, body_path_syntax) = {
        let template = build_result
            .materialisation_context
            .generic_function_templates_mut()
            .values_mut()
            .next()
            .expect("the source should retain one generic template");
        template.declaration_identity = Some(declaration_identity.clone());
        let body = template
            .body_tokens
            .as_ref()
            .expect("the generic should retain its body tokens")
            .tokens();
        let placeholder_location = body
            .tokens
            .iter()
            .find_map(|token| match token.kind {
                TokenKind::StringSliceLiteral(id) if string_table.resolve(id) == "placeholder" => {
                    Some(token.location.clone())
                }
                _ => None,
            })
            .expect("the placeholder body literal should be present");
        let mut path_syntax = PathSyntaxTable::new();
        let path_id = path_syntax.push(
            InternedPath::from_components(vec![assets_component, logo_component]),
            placeholder_location,
        );
        let mut tokens = body.tokens.clone();
        let mut replaced = false;
        for token in &mut tokens {
            if let TokenKind::StringSliceLiteral(id) = token.kind
                && string_table.resolve(id) == "placeholder"
            {
                token.kind = TokenKind::Path(path_id);
                replaced = true;
                break;
            }
        }
        assert!(replaced, "the placeholder body literal should be present");
        let body = FileTokens::new_with_identity(
            body.src_path.clone(),
            body.file_id,
            body.canonical_os_path.clone(),
            tokens,
            path_syntax,
        );
        let body_source_file = body
            .file_id
            .expect("generic body should retain its source file");
        template.body_tokens = Some(GenericFunctionBody::source(body));
        (body_source_file, path_id)
    };
    let mut resolved_references = ResolvedFileReferenceTable::new();
    resolved_references
        .push(ResolvedFileReference {
            source_file: body_source_file,
            path_syntax: body_path_syntax,
            class: PreparedFileReferenceClass::ResourceFile,
            outcome: ResolvedFileReferenceOutcome::Target(
                ResolvedFileReferenceTarget::ResourceSource {
                    source: crate::compiler_frontend::paths::file_references::ResourceSourceId::from_index(0),
                    owner_relative_path: PortableResourcePath::from_relative_logical_path(
                        Path::new("assets/logo.svg"),
                    )
                    .expect("resource path should be portable"),
                },
            ),
        })
        .expect("test body path rows should be unique");
    build_result
        .materialisation_context
        .context
        .stage0_resolution_facts = Some(Arc::new(Stage0ResolutionFacts::ordinary(
        resolved_references,
        SourceDatabase::empty(),
    )));

    let mut preparation = build_result
        .materialisation_context
        .finish_preparation()
        .expect("generic template identity index should build");
    preparation.module_origin = Some(module_origin.clone());
    let public_interface = PublicSemanticInterface {
        module_origin,
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let context = preparation
        .clone()
        .freeze(&public_interface, &ModuleResourceTable::new())
        .expect("generic resource body should freeze")
        .expect("the retained generic should produce a materialisation context");
    let identity = GeneratedFunctionIdentity::new(
        declaration_identity,
        Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]),
        Box::new([]),
    );

    ResourceBodyMaterialisationFixture {
        preparation,
        context,
        identity,
        resource_origin,
    }
}

fn generated_body_resource_handle(materialised: &super::MaterialisedGenericAst) -> ResourceId {
    let function = materialised
        .build_result
        .ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, _, body) if path == &materialised.instance_path => Some(body),
            _ => None,
        })
        .expect("materialisation should emit the generated function");
    let [
        AstNode {
            kind: NodeKind::Return(values),
            ..
        },
    ] = function.as_slice()
    else {
        panic!("generated function should contain one return node, got {function:?}");
    };
    let [value] = values.as_slice() else {
        panic!("generated function should return one value, got {values:?}");
    };
    structural_resource_handle(value)
}

fn structural_resource_handle(expression: &Expression) -> ResourceId {
    let ExpressionKind::StructuralString { pieces } = &expression.kind else {
        panic!("expected a structural string, got {:?}", expression.kind);
    };
    let [ConstStringPiece::Resource(resource)] = pieces.as_slice() else {
        panic!("expected one resource piece, got {pieces:?}");
    };
    *resource
}

#[test]
fn frozen_resource_parameter_default_materialises_into_a_sidecar_local_table() {
    let fixture = resource_default_materialisation_fixture();
    let artefact = fixture
        .context
        .artefacts
        .first()
        .expect("the frozen context should retain one generic artefact");
    let parameter = artefact
        .signature
        .parameters
        .iter()
        .find(|parameter| parameter.name == "suffix")
        .expect("the frozen signature should retain its suffix parameter");
    let Some(PublicFoldedValue::String(OwnedFoldedString::Pieces(pieces))) =
        parameter.folded_default.as_ref()
    else {
        panic!("the parameter default should freeze as a structural public string");
    };
    assert_eq!(
        pieces.as_slice(),
        &[OwnedFoldedStringPiece::Resource(
            fixture.resource_origin.clone()
        )],
        "freezing must carry the stable resource origin, not a donor ResourceId"
    );

    let requester_call_location = SourceLocation::default();
    let materialised = fixture
        .context
        .materialise_ast_at(
            0,
            ModuleMaterialisationInput {
                identity: &fixture.identity,
                requester_context: &fixture.preparation,
                requester_call_location: &requester_call_location,
                external_package_registry: fixture.preparation.external_package_registry.as_ref(),
                style_directives: &fixture.preparation.style_directives,
                build_profile: fixture.preparation.build_profile,
                template_const_loop_iteration_limit: fixture
                    .preparation
                    .template_const_loop_iteration_limit,
                #[cfg(feature = "timers")]
                timing_context: None,
            },
        )
        .expect("the frozen generic should materialise");
    let generated_signature = materialised
        .build_result
        .ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, signature, _) if path == &materialised.instance_path => {
                Some(signature)
            }
            _ => None,
        })
        .expect("materialisation should emit the generated function");
    let generated_parameter = generated_signature
        .parameters
        .iter()
        .find(|parameter| parameter.id.name_str(&materialised.string_table) == Some("suffix"))
        .expect("the generated signature should retain its suffix parameter");
    let sidecar_resource = structural_resource_handle(&generated_parameter.value);

    assert_eq!(
        fixture
            .declaring_resources
            .try_origin(fixture.declaring_resource)
            .expect("the declaring handle should resolve")
            .origin,
        fixture.resource_origin,
    );
    let sidecar_resources = materialised
        .build_result
        .module_resources
        .as_ref()
        .expect("generated AST should retain its sidecar resource table");
    let sidecar_resources = sidecar_resources.borrow();
    assert_eq!(
        sidecar_resources.origins().len(),
        1,
        "the generated default should add one sidecar-local origin"
    );
    let sidecar_origin = sidecar_resources
        .try_origin(sidecar_resource)
        .expect("the generated handle should resolve in its sidecar table");
    assert_eq!(
        sidecar_origin.origin, fixture.resource_origin,
        "the sidecar handle must round-trip to the frozen stable origin"
    );
    assert_eq!(
        sidecar_origin
            .first_authored_location
            .scope
            .to_portable_string(&materialised.string_table),
        "@page.moth",
        "resource provenance should retain the authored source file",
    );
    assert_eq!(
        sidecar_origin.first_authored_location.start_pos,
        CharPosition {
            line_number: 0,
            char_column: 38,
        },
        "resource provenance should start at the file-value path expression",
    );
    assert_eq!(
        sidecar_origin.first_authored_location.end_pos,
        CharPosition {
            line_number: 0,
            char_column: 47,
        },
        "resource provenance should end at the file-value path expression",
    );
}

#[test]
fn materialised_generic_bodies_keep_colliding_path_facts_separate() {
    let capture = |relative_path: &str| {
        let mut source_table = StringTable::new();
        let source_file = InternedPath::from_single_str("@body.moth", &mut source_table);
        let path_location = location("@body.moth", &mut source_table);
        let mut path_syntax = PathSyntaxTable::new();
        let path_id = path_syntax.push(
            InternedPath::from_single_str("@resource.bin", &mut source_table),
            path_location.clone(),
        );
        let body = FileTokens::new_with_identity(
            source_file.clone(),
            Some(SourceId::from_index(0)),
            None,
            vec![Token::new(TokenKind::Path(path_id), path_location)],
            path_syntax,
        );
        let mut resolved_references = ResolvedFileReferenceTable::new();
        resolved_references
            .push(ResolvedFileReference {
                source_file: SourceId::from_index(0),
                path_syntax: path_id,
                class: PreparedFileReferenceClass::ResourceFile,
                outcome: ResolvedFileReferenceOutcome::Target(
                    ResolvedFileReferenceTarget::ResourceSource {
                        source: crate::compiler_frontend::paths::file_references::ResourceSourceId::from_index(0),
                        owner_relative_path: PortableResourcePath::from_relative_logical_path(
                            Path::new(relative_path),
                        )
                        .expect("collision fixture resource path should be portable"),
                    },
                ),
            })
            .expect("collision fixture path rows should be unique");
        let facts = Stage0ResolutionFacts::ordinary(resolved_references, SourceDatabase::empty());
        let frozen =
            StableBodySyntax::capture(&body, &source_file, &source_table, Some(&facts), &|_| {
                Err::<PublicFoldedValue, CompilerError>(CompilerError::compiler_error(
                    "collision fixture has no content values",
                ))
            })
            .expect("collision fixture body should freeze");
        let compact_path_id = frozen
            .resolved_file_references
            .first()
            .expect("collision fixture should retain one path row")
            .path_syntax;
        (frozen, compact_path_id)
    };

    let (first_frozen, first_path_id) = capture("assets/first.svg");
    let (second_frozen, second_path_id) = capture("assets/second.svg");
    assert_eq!(
        first_path_id, second_path_id,
        "independent body captures should restart compact handles at the same value"
    );

    let materialise = |frozen: StableBodySyntax| {
        let mut generated_table = StringTable::new();
        let generated_source_file =
            InternedPath::from_single_str("@body.moth", &mut generated_table);
        let materialised = frozen
            .materialise(&generated_source_file, &mut generated_table)
            .expect("collision fixture body should materialise");
        GenericFunctionBody::materialised(materialised.file_tokens, materialised.resolution_facts)
    };
    let first_body = materialise(first_frozen);
    let second_body = materialise(second_frozen);

    let target_path = |body: &GenericFunctionBody, path_id: PathSyntaxId| {
        let reference = body
            .resolution_facts()
            .expect("materialised body should carry its Stage 0 facts")
            .lookup(None, path_id)
            .expect("materialised body facts should accept its compact handle")
            .expect("materialised body should retain its path row");
        let Stage0ResolvedFileReferenceOutcome::Resource {
            owner_relative_path,
            ..
        } = reference.outcome
        else {
            panic!("collision fixture should retain resource outcomes");
        };
        owner_relative_path.as_str().to_owned()
    };

    assert_eq!(
        target_path(&first_body, first_path_id),
        "assets/first.svg",
        "first body must resolve its own row"
    );
    assert_eq!(
        target_path(&second_body, second_path_id),
        "assets/second.svg",
        "second body must resolve its own row despite the colliding handle"
    );
}

#[test]
fn frozen_resource_body_materialises_into_a_sidecar_local_table() {
    let fixture = resource_body_materialisation_fixture();
    let requester_call_location = SourceLocation::default();
    let materialised = fixture
        .context
        .materialise_ast_at(
            0,
            ModuleMaterialisationInput {
                identity: &fixture.identity,
                requester_context: &fixture.preparation,
                requester_call_location: &requester_call_location,
                external_package_registry: fixture.preparation.external_package_registry.as_ref(),
                style_directives: &fixture.preparation.style_directives,
                build_profile: fixture.preparation.build_profile,
                template_const_loop_iteration_limit: fixture
                    .preparation
                    .template_const_loop_iteration_limit,
                #[cfg(feature = "timers")]
                timing_context: None,
            },
        )
        .expect("the frozen generic body should materialise");
    let generated_resource = generated_body_resource_handle(&materialised);
    let sidecar_resources = materialised
        .build_result
        .module_resources
        .as_ref()
        .expect("generated AST should retain its sidecar resource table");
    let sidecar_resources = sidecar_resources.borrow();
    assert_eq!(
        sidecar_resources.origins().len(),
        1,
        "the generated body should add one sidecar-local origin"
    );
    assert_eq!(
        sidecar_resources
            .try_origin(generated_resource)
            .expect("the generated body handle should resolve in its sidecar table")
            .origin,
        fixture.resource_origin,
        "the generated body handle must round-trip to the frozen stable origin"
    );
}

#[test]
fn frozen_resource_body_captures_resolved_subset_before_materialisation() {
    let fixture = resource_body_materialisation_fixture();
    let artefact = fixture
        .context
        .artefacts
        .first()
        .expect("the frozen context should retain one generic artefact");
    assert_eq!(
        artefact.body.path_syntax.paths().len(),
        1,
        "freezing should retain the body's referenced path row before materialisation"
    );
    let token_path = artefact
        .body
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(path_id) => Some(path_id),
            _ => None,
        })
        .expect("the frozen body should retain its path token");
    let [reference] = artefact.body.resolved_file_references.as_ref() else {
        panic!("the frozen body should retain one resolved-reference row");
    };
    assert_eq!(
        reference.path_syntax, token_path,
        "the captured resolved row must use the frozen token's compact handle"
    );
    assert_eq!(reference.class, PreparedFileReferenceClass::ResourceFile);
    let StableResolvedFileReferenceOutcome::Resource {
        owner_relative_path,
    } = &reference.outcome
    else {
        panic!("the frozen body should retain a resource outcome");
    };
    assert_eq!(
        artefact.body.pool[owner_relative_path.index() as usize],
        "assets/logo.svg",
        "the frozen body must retain the resolved resource path without materialising"
    );
}

#[test]
fn repeated_frozen_resource_body_materialisations_preserve_stable_origin() {
    let fixture = resource_body_materialisation_fixture();
    let requester_call_location = SourceLocation::default();
    let materialise = || {
        fixture
            .context
            .materialise_ast_at(
                0,
                ModuleMaterialisationInput {
                    identity: &fixture.identity,
                    requester_context: &fixture.preparation,
                    requester_call_location: &requester_call_location,
                    external_package_registry: fixture
                        .preparation
                        .external_package_registry
                        .as_ref(),
                    style_directives: &fixture.preparation.style_directives,
                    build_profile: fixture.preparation.build_profile,
                    template_const_loop_iteration_limit: fixture
                        .preparation
                        .template_const_loop_iteration_limit,
                    #[cfg(feature = "timers")]
                    timing_context: None,
                },
            )
            .expect("the frozen generic body should materialise")
    };
    let first = materialise();
    let second = materialise();
    let body_origin = |materialised: &super::MaterialisedGenericAst| {
        let resource = generated_body_resource_handle(materialised);
        let sidecar_resources = materialised
            .build_result
            .module_resources
            .as_ref()
            .expect("generated AST should retain its sidecar resource table");
        let sidecar_resources = sidecar_resources.borrow();
        assert_eq!(
            sidecar_resources.origins().len(),
            1,
            "each body materialisation should retain one sidecar origin row"
        );
        sidecar_resources
            .try_origin(resource)
            .expect("the body handle should resolve in its own sidecar table")
            .origin
            .clone()
    };
    let first_origin = body_origin(&first);
    let second_origin = body_origin(&second);
    assert_eq!(
        first_origin, fixture.resource_origin,
        "the first body materialisation must preserve its frozen stable origin"
    );
    assert_eq!(
        second_origin, fixture.resource_origin,
        "the second body materialisation must preserve its frozen stable origin"
    );
    assert_eq!(
        first_origin, second_origin,
        "independent body materialisations must preserve one stable resource origin"
    );
}

/// WHAT: owns idempotence within one shared sidecar table: repeated projections of one frozen
///       default reuse one local handle and one origin row.
/// WHY: `intern_origin` must collapse repeated stable-origin projections without making a
///      `ResourceId` valid outside the table that issued it.

#[test]
fn repeated_frozen_resource_default_projection_reuses_one_sidecar_handle() {
    let fixture = resource_default_materialisation_fixture();
    let folded_default = fixture
        .context
        .artefacts
        .first()
        .expect("the frozen context should retain one generic artefact")
        .signature
        .parameters
        .iter()
        .find(|parameter| parameter.name == "suffix")
        .and_then(|parameter| parameter.folded_default.as_ref())
        .expect("the frozen suffix parameter should have a default");

    // `materialise_ast_at` creates a fresh table for each call. This repeat proof therefore keeps
    // the one table shared by all folded-value projections inside one generated materialisation.
    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;
    let external_registry = ExternalPackageRegistry::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let module_resources = Rc::new(RefCell::new(ModuleResourceTable::new()));
    let mut materialiser = super::GeneratedFoldedValueMaterialiser {
        type_environment: &mut type_environment,
        external_registry: &external_registry,
        nominal_source: &fixture.preparation,
        template_ir_store,
        module_resources: Rc::clone(&module_resources),
    };
    let mut string_table = StringTable::new();
    let first = super::materialize_public_folded_value(
        &mut materialiser,
        folded_default,
        string_type_id,
        &mut string_table,
        &SourceLocation::default(),
    )
    .expect("the first frozen default projection should succeed");
    let second = super::materialize_public_folded_value(
        &mut materialiser,
        folded_default,
        string_type_id,
        &mut string_table,
        &SourceLocation::default(),
    )
    .expect("the repeated frozen default projection should succeed");
    let first_resource = structural_resource_handle(&first);
    let second_resource = structural_resource_handle(&second);

    assert_eq!(
        first_resource, second_resource,
        "one sidecar table must reuse the handle for a repeated stable origin"
    );
    let module_resources = module_resources.borrow();
    assert_eq!(
        module_resources.origins().len(),
        1,
        "repeated projection must retain one sidecar table row"
    );
    assert_eq!(
        module_resources
            .try_origin(first_resource)
            .expect("the reused handle should resolve in the shared sidecar table")
            .origin,
        fixture.resource_origin,
    );
}

/// WHAT: owns stable-origin identity across independent materialisations: each fresh sidecar
///       resolves its own handle to the origin frozen by the declaring module.
/// WHY: production materialisation must re-intern the frozen origin for every generated AST
///      rather than minting a distinct origin for each call.
#[test]
fn repeated_frozen_resource_default_materialisations_preserve_stable_origin_across_sidecars() {
    let fixture = resource_default_materialisation_fixture();
    let folded_default = fixture
        .context
        .artefacts
        .first()
        .expect("the frozen context should retain one generic artefact")
        .signature
        .parameters
        .iter()
        .find(|parameter| parameter.name == "suffix")
        .and_then(|parameter| parameter.folded_default.as_ref())
        .expect("the frozen suffix parameter should have a default");
    let PublicFoldedValue::String(OwnedFoldedString::Pieces(pieces)) = folded_default else {
        panic!("the parameter default should freeze as a structural public string");
    };
    let [OwnedFoldedStringPiece::Resource(frozen_origin)] = pieces.as_slice() else {
        panic!("the frozen default should retain one stable resource origin");
    };
    assert_eq!(
        frozen_origin, &fixture.resource_origin,
        "the public folded default must retain the declaring module's origin"
    );

    let requester_call_location = SourceLocation::default();
    let materialise_once = || {
        fixture
            .context
            .materialise_ast_at(
                0,
                ModuleMaterialisationInput {
                    identity: &fixture.identity,
                    requester_context: &fixture.preparation,
                    requester_call_location: &requester_call_location,
                    external_package_registry: fixture
                        .preparation
                        .external_package_registry
                        .as_ref(),
                    style_directives: &fixture.preparation.style_directives,
                    build_profile: fixture.preparation.build_profile,
                    template_const_loop_iteration_limit: fixture
                        .preparation
                        .template_const_loop_iteration_limit,
                    #[cfg(feature = "timers")]
                    timing_context: None,
                },
            )
            .expect("the frozen generic should materialise")
    };
    let first_materialised = materialise_once();
    let second_materialised = materialise_once();

    let generated_resource = |materialised: &super::MaterialisedGenericAst| {
        let generated_signature = materialised
            .build_result
            .ast
            .nodes
            .iter()
            .find_map(|node| match &node.kind {
                NodeKind::Function(path, signature, _) if path == &materialised.instance_path => {
                    Some(signature)
                }
                _ => None,
            })
            .expect("materialisation should emit the generated function");
        let generated_parameter = generated_signature
            .parameters
            .iter()
            .find(|parameter| parameter.id.name_str(&materialised.string_table) == Some("suffix"))
            .expect("the generated signature should retain its suffix parameter");
        structural_resource_handle(&generated_parameter.value)
    };
    let first_resource = generated_resource(&first_materialised);
    let second_resource = generated_resource(&second_materialised);

    let first_origin = {
        let sidecar_resources = first_materialised
            .build_result
            .module_resources
            .as_ref()
            .expect("first generated AST should retain its sidecar resource table");
        let sidecar_resources = sidecar_resources.borrow();
        assert_eq!(
            sidecar_resources.origins().len(),
            1,
            "the first materialisation should retain one sidecar origin row"
        );
        sidecar_resources
            .try_origin(first_resource)
            .expect("the first handle should resolve in its own sidecar table")
            .origin
            .clone()
    };
    let second_origin = {
        let sidecar_resources = second_materialised
            .build_result
            .module_resources
            .as_ref()
            .expect("second generated AST should retain its sidecar resource table");
        let sidecar_resources = sidecar_resources.borrow();
        assert_eq!(
            sidecar_resources.origins().len(),
            1,
            "the second materialisation should retain one sidecar origin row"
        );
        sidecar_resources
            .try_origin(second_resource)
            .expect("the second handle should resolve in its own sidecar table")
            .origin
            .clone()
    };

    assert_eq!(
        first_origin, *frozen_origin,
        "the first sidecar must resolve to the origin frozen in the public default"
    );
    assert_eq!(
        second_origin, *frozen_origin,
        "the second sidecar must resolve to the origin frozen in the public default"
    );
    assert_eq!(
        first_origin, second_origin,
        "independent materialisations must preserve one stable resource origin"
    );
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

fn frozen_resource_reference(
    path_syntax: PathSyntaxId,
    owner_relative_path: &str,
) -> FrozenResolvedFileReference {
    FrozenResolvedFileReference {
        path_syntax,
        class: PreparedFileReferenceClass::ResourceFile,
        outcome: FrozenResolvedFileReferenceOutcome::Resource {
            owner_relative_path: PortableResourcePath::from_relative_logical_path(Path::new(
                owner_relative_path,
            ))
            .expect("frozen refusal fixture resource path should be portable"),
        },
    }
}

#[test]
fn frozen_generic_rejects_absent_path_handle() {
    let error = match Stage0ResolutionFacts::frozen_generic(vec![frozen_resource_reference(
        PathSyntaxId::NONE,
        "assets/missing.svg",
    )]) {
        Ok(_) => panic!("frozen generic facts must reject an absent path handle"),
        Err(error) => error,
    };
    assert_eq!(
        error.msg,
        "frozen generic resolved-reference row has an absent PathSyntaxId marker"
    );
}

#[test]
fn frozen_generic_rejects_duplicate_compact_path_handle() {
    let mut path_syntax = PathSyntaxTable::new();
    let path_id = path_syntax.push(InternedPath::default(), SourceLocation::default());
    let error = match Stage0ResolutionFacts::frozen_generic(vec![
        frozen_resource_reference(path_id, "assets/first.svg"),
        frozen_resource_reference(path_id, "assets/second.svg"),
    ]) {
        Ok(_) => panic!("frozen generic facts must reject duplicate compact path handles"),
        Err(error) => error,
    };
    assert_eq!(
        error.msg,
        "frozen generic resolved-reference table contains duplicate path handles"
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
        resolved_file_references: Box::new([]),
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
        resolved_file_references: Box::new([]),
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
        resolved_file_references: Box::new([]),
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
    let mut frozen = capture_test_body(&original, &source_file, &source_table);
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
