//! Focused tests for frozen generic body and resource-default materialisation.
//!
//! WHAT: proves canonical token payloads round-trip through one source-owned token range and
//! its bounded parser adapter, repeated spellings share one string entry, frozen resource
//! defaults cross the generated sidecar boundary through stable origins, and retained bodies
//! stay `Send` without owning a second token store.
//! WHY: the integration case `generic_parameter_default_file_value_success` owns authored
//! `@assets/logo.svg` syntax through Stage 0 resolution. These tests own the freeze-to-sidecar
//! seam from that resolved AST representation while keeping donor source identity explicit.

use super::super::{GenericFunctionBody, GenericFunctionTemplate};
use super::ModuleMaterialisationInput;
use super::artefact_emit::{ModuleMaterialisationContext, check_materialisation_row_identity};
use super::frozen_file_references::StableResolvedFileReferenceOutcome;
use super::frozen_syntax::StableBodySyntax;
use super::preparation_freeze::ModuleMaterialisationPreparation;
use super::stable_types::GeneratedFoldedValueMaterialiser;
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
use crate::compiler_frontend::source::{
    FrozenIdentityHandle, LocalSpan, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathInternerBuilder, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result;
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, SourceTokens, Token, TokenIndex, TokenKind, TokenRange,
};
use crate::compiler_frontend::traits::ids::TraitId;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

fn token_span() -> LocalSpan {
    LocalSpan::source_start()
}

fn sample_tokens(
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> (Vec<Token>, PathSyntaxTable, PathSyntaxId) {
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
        path_fork
            .try_intern_components(&[
                string_table.intern("provider"),
                string_table.intern("CONST"),
            ])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, token_span()),
    );
    let tokens = vec![
        Token::new(
            TokenKind::Symbol(string_table.intern("hello")),
            token_span(),
        ),
        Token::new(
            TokenKind::StyleDirective(string_table.intern("md")),
            token_span(),
        ),
        Token::new(
            TokenKind::StringSliceLiteral(string_table.intern("slice text")),
            token_span(),
        ),
        Token::new(
            TokenKind::RawStringLiteral(string_table.intern("raw text")),
            token_span(),
        ),
        Token::new(TokenKind::CharLiteral('x'), token_span()),
        Token::new(TokenKind::BoolLiteral(true), token_span()),
        Token::new(TokenKind::NumericLiteral(numeric), token_span()),
        Token::new(TokenKind::Path(path_id), token_span()),
        Token::new(
            TokenKind::Symbol(string_table.intern("import")),
            token_span(),
        ),
        Token::new(TokenKind::ChannelReceive, token_span()),
    ];
    (tokens, path_syntax, path_id)
}

fn resolved_token_text(
    token: &Token,
    path_syntax: &PathSyntaxTable,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> String {
    match &token.kind {
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
        TokenKind::Path(id) => {
            let path = path_syntax.try_path(*id).expect("valid path handle");
            format!(
                "Path({})",
                path_fork.render_portable(path.root, string_table, &mut Vec::new())
            )
        }
        other => format!("{other:?}"),
    }
}
fn body_view(original: &FileTokens, declaration_path: PathId) -> GenericFunctionBody {
    // Test fixtures are often still in the mutable preparing state. Model the production
    // post-publication handoff explicitly: retain the canonical `SourceTokens` allocation
    // directly (as `function_signatures.rs` does via `canonical_source_tokens_arc`) rather
    // than a `FileTokens` shell.
    let frozen = FileTokens::new_frozen(
        original.src_path,
        original.file_id,
        original.canonical_os_path.clone(),
        original.tokens.clone(),
        original
            .path_syntax_table()
            .expect("test fixture should expose its path table")
            .clone(),
    );
    let owner = frozen
        .canonical_source_tokens_arc()
        .expect("test fixture must use a canonical source owner");
    let source_tokens: &SourceTokens = owner.as_ref();
    let end = TokenIndex::try_from_index(frozen.tokens.len()).expect("test token range fits");
    let range = TokenRange::try_new_for(
        source_tokens,
        TokenIndex::try_from_index(0).expect("test token range fits"),
        end,
    )
    .expect("test token range must be in bounds");
    GenericFunctionBody::source(
        owner,
        range,
        None,
        declaration_path,
        frozen.canonical_os_path.clone(),
    )
    .expect("test generic body should retain its checked source range")
}
fn capture_test_body(
    original: &FileTokens,
    source_file: &PathId,
    path_fork: &PathInternerFork,
    source_table: &StringTable,
) -> StableBodySyntax {
    let source_file_id = original.file_id;

    let mut resolved_references = ResolvedFileReferenceTable::new();
    for (path_syntax, _) in original.path_syntax.iter() {
        resolved_references.push(ResolvedFileReference {
                source_file: source_file_id,
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
    let facts =
        Stage0ResolutionFacts::ordinary(resolved_references, SourceDatabase::empty().into());
    let no_content_value = |_path: &PathId| -> Result<PublicFoldedValue, CompilerError> {
        Err(CompilerError::compiler_error(
            "test body has no content value resolver",
        ))
    };
    let body = body_view(original, original.src_path);
    let mut capture_table = source_table.clone();
    StableBodySyntax::capture(
        &body,
        *source_file,
        path_fork,
        &mut capture_table,
        Some(&facts),
        FrozenIdentityHandle::new(),
        &no_content_value,
    )
    .expect("test body path rows should be resolved before capture")
}
/// Build a materialisation fork that retains the complete source path domain.
///
/// Frozen bodies keep build-wide `PathId`s directly, so an independent generated fork must inherit
/// every source node rather than relying on equal numeric IDs. Its string table is cloned for the
/// same reason: inherited path components retain their source-table `StringId`s.
fn generated_materialisation_domain(
    source_fork: &PathInternerFork,
    source_table: &StringTable,
) -> (PathInternerFork, StringTable) {
    (
        source_fork.fork_source().fork_for_module(),
        source_table.clone(),
    )
}

fn direct_content_body_fixture() -> (
    FileTokens,
    PathId,
    PathInternerFork,
    StringTable,
    Stage0ResolutionFacts,
    PathSyntaxId,
    StableResourceOriginId,
) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_file = path_fork
        .try_intern_portable_path("@mod.moth", &mut string_table)
        .expect("test path fits");
    let source_files = SourceDatabase::build(
        [PathBuf::from("@mod.moth"), PathBuf::from("@private.mtf")],
        Path::new("@mod.moth"),
        None,
        &mut string_table,
    )
    .expect("content fixture source identities should build");
    let body_file_id = source_files
        .get_by_canonical_path(Path::new("@mod.moth"))
        .expect("content fixture body source identity should be registered")
        .id;
    let path_span = LocalSpan::source_start();
    let mut path_syntax = PathSyntaxTable::new();
    let path_id = path_syntax.push(
        path_fork
            .try_intern_portable_path("@private.mtf", &mut string_table)
            .expect("test path fits"),
        SourceSpan::new(body_file_id, path_span),
    );
    let tokens = vec![Token::new(TokenKind::Path(path_id), path_span)];
    let body = FileTokens::new_with_identity(source_file, body_file_id, None, tokens, path_syntax);

    let mut resolved_references = ResolvedFileReferenceTable::new();
    resolved_references
        .push(ResolvedFileReference {
            source_file: body_file_id,
            path_syntax: path_id,
            class: PreparedFileReferenceClass::ContentSource,
            outcome: ResolvedFileReferenceOutcome::Target(
                ResolvedFileReferenceTarget::ContentSource {
                    source: source_files
                        .get_by_canonical_path(Path::new("@private.mtf"))
                        .expect("content fixture target identity should be registered")
                        .id,
                },
            ),
        })
        .expect("content fixture path rows should be unique");
    let facts = Stage0ResolutionFacts::ordinary(resolved_references, source_files.into());
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
        path_fork,
        string_table,
        facts,
        path_id,
        resource_origin,
    )
}

#[test]
fn frozen_content_value_captures_and_reinterns_resource_pieces() {
    let (body, source_file, path_fork, source_table, facts, _, resource_origin) =
        direct_content_body_fixture();
    let body = body_view(&body, source_file);
    let mut capture_table = source_table.clone();
    let frozen = StableBodySyntax::capture(
        &body,
        source_file,
        &path_fork,
        &mut capture_table,
        Some(&facts),
        FrozenIdentityHandle::new(),
        &|_| {
            Ok(PublicFoldedValue::String(OwnedFoldedString::Pieces(vec![
                OwnedFoldedStringPiece::Text("private content: ".to_owned()),
                OwnedFoldedStringPiece::Resource(resource_origin.clone()),
            ])))
        },
    )
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

    let (mut path_fork, mut generated_table) =
        generated_materialisation_domain(&path_fork, &source_table);
    let generated_source_file = path_fork
        .try_intern_portable_path("@mod.moth", &mut generated_table)
        .expect("test path fits");
    let path_id = frozen.resolved_file_references[0].path_syntax;
    let materialised = frozen
        .materialise(
            generated_source_file,
            &mut path_fork,
            &mut generated_table,
            None,
        )
        .expect("frozen content body should materialise");
    let Stage0ResolvedFileReferenceOutcome::Content {
        logical_path: None,
        value: Some(value),
    } = materialised
        .resolution_facts
        .lookup(frozen.canonical_donor_file_id(), path_id)
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
            .intern_origin(origin.clone(), None))
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
    let (body, source_file, path_fork, source_table, facts, _, _) = direct_content_body_fixture();
    let body = body_view(&body, source_file);
    let mut capture_table = source_table.clone();
    let error = match StableBodySyntax::capture(
        &body,
        source_file,
        &path_fork,
        &mut capture_table,
        Some(&facts),
        FrozenIdentityHandle::new(),
        &|_| {
            Err(CompilerError::compiler_error(
                "synthetic content constant was not folded before capture",
            ))
        },
    ) {
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
    let (body, source_file, path_fork, source_table, facts, _, _) = direct_content_body_fixture();
    let body = body_view(&body, source_file);
    let mut capture_table = source_table.clone();
    let error = match StableBodySyntax::capture(
        &body,
        source_file,
        &path_fork,
        &mut capture_table,
        Some(&facts),
        FrozenIdentityHandle::new(),
        &|_| Ok(PublicFoldedValue::Int(7)),
    ) {
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
    let mut path_fork = PathInternerFork::empty();
    let (tokens, path_syntax, _) = sample_tokens(&mut source_table, &mut path_fork);
    let original = FileTokens::new_with_identity(
        path_fork
            .try_intern_portable_path("src/@mod.moth", &mut source_table)
            .expect("test path fits"),
        SourceId::COMPILATION_ROOT,
        None,
        tokens.clone(),
        path_syntax.clone(),
    );

    let source_file = original.src_path;
    let frozen = capture_test_body(&original, &source_file, &path_fork, &source_table);
    let (mut generated_path_fork, mut generated_table) =
        generated_materialisation_domain(&path_fork, &source_table);
    let generated_source_file = generated_path_fork
        .try_intern_portable_path("src/@mod.moth", &mut generated_table)
        .expect("test path fits");
    let path_id = frozen.resolved_file_references[0].path_syntax;
    let materialised = frozen
        .materialise(
            generated_source_file,
            &mut generated_path_fork,
            &mut generated_table,
            None,
        )
        .expect("frozen body should materialise");
    assert_eq!(
        materialised.source_owner.file_id,
        SourceId::COMPILATION_ROOT,
        "materialised generic syntax must retain its concrete donor identity",
    );
    let materialised_body = materialised
        .into_generic_body()
        .expect("materialised body should retain its checked range");
    let materialised_stream = materialised_body
        .parser_stream(&mut generated_table, &mut generated_path_fork)
        .expect("materialised body should derive a bounded parser adapter");
    let canonical_len = materialised_body
        .materialised_owner()
        .expect("materialised body should retain a donor shell")
        .canonical_source_tokens()
        .expect("materialised donor shell should share a canonical owner")
        .len();
    assert!(
        canonical_len >= materialised_stream.tokens.len(),
        "the compatibility adapter must remain bounded by its canonical owner",
    );
    assert!(
        materialised_body
            .resolution_facts()
            .expect("materialised body should retain Stage 0 facts")
            .lookup(SourceId::COMPILATION_ROOT, path_id)
            .is_ok(),
        "materialised facts must accept the same concrete donor identity",
    );

    let original_text = tokens
        .iter()
        .map(|token| resolved_token_text(token, &path_syntax, &path_fork, &source_table))
        .collect::<Vec<_>>();
    let materialised_path_syntax = materialised_stream
        .path_syntax_table()
        .expect("materialised adapter should retain path rows");
    let materialised_text = materialised_stream
        .tokens
        .iter()
        .map(|token| {
            resolved_token_text(
                token,
                materialised_path_syntax,
                &generated_path_fork,
                &generated_table,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        materialised_text, original_text,
        "every retained token payload must round-trip through a bounded adapter",
    );
    assert_eq!(
        generated_path_fork.render_portable(
            materialised_stream.src_path,
            &generated_table,
            &mut Vec::new(),
        ),
        "src/@mod.moth",
    );
    let path = materialised_path_syntax
        .try_path(path_id)
        .expect("valid path handle")
        .root;
    assert_eq!(
        generated_path_fork.render_portable(path, &generated_table, &mut Vec::new()),
        "provider/CONST",
        "retained path syntax rows must round-trip through the adapter",
    );
}

#[test]
fn frozen_body_keeps_declaration_path_distinct_from_owning_source_file() {
    let mut source_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (tokens, path_syntax, _) = sample_tokens(&mut source_table, &mut path_fork);
    let source_file = path_fork
        .try_intern_portable_path("src/@mod.moth", &mut source_table)
        .expect("test path fits");
    let declaration_path = path_fork
        .try_intern_child(source_file, source_table.intern("generic_fn"))
        .expect("test declaration path fits");
    let original = FileTokens::new_with_identity(
        declaration_path,
        SourceId::COMPILATION_ROOT,
        None,
        tokens,
        path_syntax,
    );

    let frozen = capture_test_body(&original, &source_file, &path_fork, &source_table);
    let (mut path_fork, mut generated_table) =
        generated_materialisation_domain(&path_fork, &source_table);
    let generated_source_file = path_fork
        .try_intern_portable_path("src/@mod.moth", &mut generated_table)
        .expect("test path fits");
    let materialised = frozen
        .materialise(
            generated_source_file,
            &mut path_fork,
            &mut generated_table,
            None,
        )
        .expect("the frozen body should retain its distinct declaration and file identities");

    assert_eq!(
        path_fork.render_portable(
            materialised.declaration_path,
            &generated_table,
            &mut Vec::new(),
        ),
        "src/@mod.moth/generic_fn",
    );
    assert_eq!(
        materialised.source_owner.file_id,
        SourceId::COMPILATION_ROOT,
        "the canonical owner identity must remain distinct from declaration paths",
    );
    let materialised_body = materialised
        .into_generic_body()
        .expect("materialised body should retain its checked range");
    let materialised_stream = materialised_body
        .parser_stream(&mut generated_table, &mut path_fork)
        .expect("materialised body should derive its bounded parser adapter");
    materialised_stream
        .path_syntax_table()
        .expect("adapter should retain donor path rows")
        .validate_file_owned_locations(SourceId::COMPILATION_ROOT)
        .expect("canonical path rows stay owned by the donor source file");
}

#[test]
fn frozen_body_preserves_multiple_referenced_canonical_path_expressions() {
    let mut source_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let base_span = LocalSpan::source_start();

    // The body references donor row 2 before donor row 0. Donor row 1 is deliberately
    // unreferenced, so retained facts must preserve the original donor handles and omit the
    // unreferenced row rather than inventing a second body-local handle domain.
    let first_donor_path = path_syntax.push(
        path_fork
            .try_intern_components(&[
                source_table.intern("provider"),
                source_table.intern("first"),
            ])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, base_span),
    );
    let _unreferenced_donor_path = path_syntax.push(
        path_fork
            .try_intern_portable_path("provider/unused", &mut source_table)
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, base_span),
    );
    let second_donor_path = path_syntax.push(
        path_fork
            .try_intern_components(&[
                source_table.intern("provider"),
                source_table.intern("second"),
            ])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, base_span),
    );
    let source_file = path_fork
        .try_intern_portable_path("src/@mod.moth", &mut source_table)
        .expect("test path fits");
    let original = FileTokens::new_with_identity(
        source_file,
        SourceId::COMPILATION_ROOT,
        None,
        vec![
            Token::new(TokenKind::Path(second_donor_path), base_span),
            Token::new(TokenKind::Path(first_donor_path), base_span),
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
    let facts =
        Stage0ResolutionFacts::ordinary(resolved_references, SourceDatabase::empty().into());
    let generic_body = body_view(&original, source_file);
    let mut capture_table = source_table.clone();
    let frozen = StableBodySyntax::capture(
        &generic_body,
        source_file,
        &path_fork,
        &mut capture_table,
        Some(&facts),
        FrozenIdentityHandle::new(),
        &|_| {
            Err::<PublicFoldedValue, CompilerError>(CompilerError::compiler_error(
                "remapping fixture has no content values",
            ))
        },
    )
    .expect("remapping fixture body should freeze");

    let retained_ids = frozen
        .resolved_file_references
        .iter()
        .map(|reference| reference.path_syntax)
        .collect::<Vec<_>>();
    let captured_token_ids = original
        .tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::Path(path_id) => Some(path_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        captured_token_ids, retained_ids,
        "captured path facts must preserve donor handles in first-reference order",
    );

    let (mut path_fork, mut generated_table) =
        generated_materialisation_domain(&path_fork, &source_table);
    let generated_source_file = path_fork
        .try_intern_portable_path("src/@mod.moth", &mut generated_table)
        .expect("test path fits");
    let materialised = frozen
        .materialise(
            generated_source_file,
            &mut path_fork,
            &mut generated_table,
            None,
        )
        .expect("remapping fixture body should materialise");
    let materialised_owner = materialised.source_owner.file_id;
    let materialised_body = materialised
        .into_generic_body()
        .expect("materialised body should retain its checked source range");
    let parser_stream = materialised_body
        .parser_stream(&mut generated_table, &mut path_fork)
        .expect("same-boundary materialised body should derive a bounded adapter");
    let parser_token_ids = parser_stream
        .tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::Path(path_id) => Some(path_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        parser_token_ids, captured_token_ids,
        "same-boundary parser adapters must preserve the donor PathSyntaxIds used by facts",
    );
    for path_id in &retained_ids {
        assert!(
            materialised_body
                .resolution_facts()
                .expect("materialised body should retain Stage 0 facts")
                .lookup(materialised_owner, *path_id)
                .expect("materialised facts should accept the donor handle")
                .is_some(),
            "every parser path handle must resolve in the retained donor facts",
        );
    }
    assert!(
        materialised_body
            .resolution_facts()
            .expect("materialised body should retain Stage 0 facts")
            .lookup(materialised_owner, _unreferenced_donor_path)
            .expect("materialised facts should accept the donor owner")
            .is_none(),
        "unreferenced donor rows must not be retained in generic facts",
    );
    let resolve_resource = |path_id| {
        let reference = materialised_body
            .resolution_facts()
            .expect("materialised body should retain Stage 0 facts")
            .lookup(materialised_owner, path_id)
            .expect("materialised facts should accept a donor handle")
            .expect("materialised facts should retain each referenced row");
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
        resolve_resource(retained_ids[0]),
        "assets/second.svg",
        "the first retained handle must resolve the row referenced by donor handle 2",
    );
    assert_eq!(
        resolve_resource(retained_ids[1]),
        "assets/first.svg",
        "the second retained handle must resolve the row referenced by donor handle 0",
    );
}

#[test]
fn repeated_spellings_share_one_frozen_string_entry() {
    let mut source_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let symbol_id = source_table.intern("hello");
    let mut path_syntax = PathSyntaxTable::new();
    let path_span = token_span();
    let path_id = path_syntax.push(
        path_fork
            .try_intern_components(&[symbol_id])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, path_span),
    );
    let tokens = vec![
        Token::new(TokenKind::Symbol(symbol_id), path_span),
        Token::new(TokenKind::Symbol(symbol_id), path_span),
        Token::new(TokenKind::Path(path_id), path_span),
    ];
    let original = FileTokens::new_with_identity(
        path_fork
            .try_intern_portable_path("src/@mod.moth", &mut source_table)
            .expect("test path fits"),
        SourceId::COMPILATION_ROOT,
        None,
        tokens,
        path_syntax,
    );

    let source_file = original.src_path;
    let frozen = capture_test_body(&original, &source_file, &path_fork, &source_table);
    assert_eq!(
        frozen.resolved_file_references.len(),
        1,
        "repeated path spellings should retain one compact donor reference row",
    );
    assert_eq!(
        frozen.canonical_donor_file_id(),
        SourceId::COMPILATION_ROOT,
        "stable syntax should retain the canonical donor owner",
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
    let mut path_fork = PathInternerFork::empty();
    let (tokens, path_syntax, _) = sample_tokens(&mut source_table, &mut path_fork);
    let source_path = path_fork
        .try_intern_portable_path("src/@mod.moth", &mut source_table)
        .expect("test path fits");
    let original = FileTokens::new_with_identity(
        source_path,
        SourceId::COMPILATION_ROOT,
        None,
        tokens,
        path_syntax,
    );

    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    capture_test_body(&original, &original.src_path, &path_fork, &source_table);

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
fn generated_materialisation_keeps_live_path_string_pair_aligned() {
    let fixture = resource_body_materialisation_fixture();
    let mut boundary_strings = fixture.preparation.string_table.clone();
    let mut boundary_paths = fixture.owner_path_fork.fork_source().fork_for_module();
    let provider_path = boundary_paths
        .try_intern_portable_path("provider-only/generated", &mut boundary_strings)
        .expect("the provider path should fit");

    let (generated_strings, requester_remap, boundary_base_len) = fixture
        .preparation
        .fork_materialisation_string_table(&boundary_strings)
        .expect("the requester table should remain a live-table prefix");
    assert!(requester_remap.is_identity());
    assert_eq!(boundary_base_len, boundary_strings.len());

    let generated_paths = boundary_paths.fork_source().fork_for_module();
    assert_eq!(
        generated_paths.render_portable(provider_path, &generated_strings, &mut Vec::new()),
        "provider-only/generated",
        "generated paths must resolve through the current boundary string table"
    );
}

#[test]
fn generated_materialisation_rejects_an_incompatible_string_prefix() {
    let fixture = resource_body_materialisation_fixture();
    let incompatible_strings = StringTable::new();
    let error = fixture
        .preparation
        .fork_materialisation_string_table(&incompatible_strings)
        .expect_err("an incompatible requester prefix must be rejected");

    assert!(
        error
            .msg
            .contains("not an exact prefix of the live boundary string table"),
        "the invariant failure should identify the incompatible string domain: {}",
        error.msg
    );
}

fn retained_template(
    path: PathId,
    declaration_identity: GeneratedDeclarationIdentity,
    has_body: bool,
) -> GenericFunctionTemplate {
    GenericFunctionTemplate {
        function_path: path,
        source_file: path,
        declaration_identity: Some(declaration_identity),
        generic_parameter_owner: None,
        generic_parameter_list_id: GenericParameterListId(0),
        signature: FunctionSignature::default(),
        body_tokens: has_body.then(|| {
            let file = FileTokens::new(path, SourceId::COMPILATION_ROOT, Vec::new());
            body_view(&file, path)
        }),
        declaration_span: None,
    }
}

struct ResourceDefaultMaterialisationFixture {
    preparation: ModuleMaterialisationPreparation,
    owner_path_fork: PathInternerFork,
    context: ModuleMaterialisationContext,
    identity: GeneratedFunctionIdentity,
    declaring_resources: ModuleResourceTable,
    declaring_resource: ResourceId,
    resource_origin: StableResourceOriginId,
}

fn resource_default_materialisation_fixture() -> ResourceDefaultMaterialisationFixture {
    let source =
        "draw type T |name T, suffix String = \"fallback\"| -> String:\n    return \"ok\"\n;\n";
    let (mut build_result, path_fork, string_table) =
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
    declaring_resources.intern_origin(decoy_origin, None);
    let declaring_resource = declaring_resources.intern_origin(resource_origin.clone(), None);

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
        .find(|parameter| {
            path_fork
                .component(parameter.id)
                .map(|id| string_table.resolve(id))
                == Some("suffix")
        })
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
        .freeze(&public_interface, &declaring_resources, &path_fork)
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
        owner_path_fork: path_fork,
        identity,
        declaring_resources,
        declaring_resource,
        resource_origin,
    }
}

struct ResourceBodyMaterialisationFixture {
    preparation: ModuleMaterialisationPreparation,
    context: ModuleMaterialisationContext,
    owner_path_fork: PathInternerFork,
    identity: GeneratedFunctionIdentity,
    resource_origin: StableResourceOriginId,
}

fn resource_body_materialisation_fixture() -> ResourceBodyMaterialisationFixture {
    let source = "draw type T |name T| -> String:\n    return \"placeholder\"\n;\n";
    let (mut build_result, mut path_fork, string_table) =
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
        let declaration_path = template.function_path;
        let body = template
            .body_tokens
            .as_ref()
            .expect("the generic should retain its body tokens");
        let mut body_fork = path_fork.fork_source().fork_for_module();
        let mut body_table = string_table.clone();
        let body = body
            .parser_stream(&mut body_table, &mut body_fork)
            .expect("the generic should derive a bounded body adapter");
        let placeholder_span = body
            .tokens
            .iter()
            .find_map(|token| match token.kind {
                TokenKind::StringSliceLiteral(id) if body_table.resolve(id) == "placeholder" => {
                    Some(token.span)
                }
                _ => None,
            })
            .expect("the placeholder body literal should be present");
        let mut path_syntax = PathSyntaxTable::new();
        let path_id = path_syntax.push(
            path_fork
                .try_intern_components(&[assets_component, logo_component])
                .expect("test path fits"),
            SourceSpan::new(body.file_id, placeholder_span),
        );
        let mut tokens = body.tokens.clone();
        let mut replaced = false;
        for token in &mut tokens {
            if let TokenKind::StringSliceLiteral(id) = token.kind
                && body_table.resolve(id) == "placeholder"
            {
                token.kind = TokenKind::Path(path_id);
                replaced = true;
                break;
            }
        }
        assert!(replaced, "the placeholder body literal should be present");
        let body_file_id = body.file_id;
        let body = FileTokens::new_with_identity(
            body.src_path,
            body_file_id,
            body.canonical_os_path.clone(),
            tokens,
            path_syntax,
        );
        let body_source_file = body_file_id;
        template.body_tokens = Some(body_view(&body, declaration_path));
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
        SourceDatabase::empty().into(),
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
        .freeze(&public_interface, &ModuleResourceTable::new(), &path_fork)
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
        owner_path_fork: path_fork,
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

    let mut path_fork = fixture.owner_path_fork.fork_source().fork_for_module();
    let materialised = fixture
        .context
        .materialise_ast_at(
            0,
            ModuleMaterialisationInput {
                path_fork: &mut path_fork,
                identity: &fixture.identity,
                requester_context: &fixture.preparation,
                requester_call_span: None,
                boundary_string_table: &fixture.preparation.string_table,
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
        .find(|parameter| {
            matches!(
                parameter.value.kind,
                ExpressionKind::StructuralString { .. }
            )
        })
        .expect("the generated signature should retain its suffix resource parameter");
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
    assert!(
        sidecar_origin.first_authored_span.is_some(),
        "resource provenance should retain an authored source span",
    );
}

#[test]
fn materialised_generic_bodies_keep_colliding_path_facts_separate() {
    let capture = |relative_path: &str| {
        let mut source_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let source_file = path_fork
            .try_intern_portable_path("@body.moth", &mut source_table)
            .expect("test path fits");
        let path_span = token_span();
        let mut path_syntax = PathSyntaxTable::new();
        let path_id = path_syntax.push(
            path_fork
                .try_intern_portable_path("@resource.bin", &mut source_table)
                .expect("test path fits"),
            SourceSpan::new(SourceId::COMPILATION_ROOT, path_span),
        );
        let body = FileTokens::new_with_identity(
            source_file,
            SourceId::COMPILATION_ROOT,
            None,
            vec![Token::new(TokenKind::Path(path_id), path_span)],
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
        let facts =
            Stage0ResolutionFacts::ordinary(resolved_references, SourceDatabase::empty().into());
        let generic_body = body_view(&body, source_file);
        let mut capture_table = source_table.clone();
        let frozen = StableBodySyntax::capture(
            &generic_body,
            source_file,
            &path_fork,
            &mut capture_table,
            Some(&facts),
            FrozenIdentityHandle::new(),
            &|_| {
                Err::<PublicFoldedValue, CompilerError>(CompilerError::compiler_error(
                    "collision fixture has no content values",
                ))
            },
        )
        .expect("collision fixture body should freeze");
        let path_syntax_id = frozen
            .resolved_file_references
            .first()
            .expect("collision fixture should retain one path row")
            .path_syntax;
        let owner_path_fork = path_fork;
        let owner_string_table = source_table;
        (frozen, path_syntax_id, owner_path_fork, owner_string_table)
    };

    let (first_frozen, first_path_id, first_path_fork, first_string_table) =
        capture("assets/first.svg");
    let (second_frozen, second_path_id, second_path_fork, second_string_table) =
        capture("assets/second.svg");
    assert_eq!(
        first_path_id, second_path_id,
        "independent body captures should preserve the same source-owned handle when their row order matches"
    );

    let materialise = |(frozen, owner_path_fork, owner_string_table): (
        StableBodySyntax,
        PathInternerFork,
        StringTable,
    )| {
        let (mut path_fork, mut generated_table) =
            generated_materialisation_domain(&owner_path_fork, &owner_string_table);
        let generated_source_file = path_fork
            .try_intern_portable_path("@body.moth", &mut generated_table)
            .expect("test path fits");
        let materialised = frozen
            .materialise(
                generated_source_file,
                &mut path_fork,
                &mut generated_table,
                None,
            )
            .expect("collision fixture body should materialise");
        materialised
            .into_generic_body()
            .expect("collision fixture should retain a checked generic body")
    };
    let first_body = materialise((first_frozen, first_path_fork, first_string_table));
    let second_body = materialise((second_frozen, second_path_fork, second_string_table));

    let target_path = |body: &GenericFunctionBody, path_id: PathSyntaxId| {
        let reference = body
            .resolution_facts()
            .expect("materialised body should carry its Stage 0 facts")
            .lookup(body.donor_source_id(), path_id)
            .expect("materialised body facts should accept its donor handle")
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
    let mut path_fork = fixture.owner_path_fork.fork_source().fork_for_module();
    let materialised = fixture
        .context
        .materialise_ast_at(
            0,
            ModuleMaterialisationInput {
                path_fork: &mut path_fork,
                identity: &fixture.identity,
                requester_context: &fixture.preparation,
                requester_call_span: None,
                boundary_string_table: &fixture.preparation.string_table,
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
        artefact.body.resolved_file_references.len(),
        1,
        "freezing should retain the body's referenced path row before materialisation",
    );
    assert_eq!(
        artefact.body.canonical_donor_file_id(),
        SourceId::COMPILATION_ROOT,
        "the frozen body should retain its canonical donor source owner",
    );
    let [reference] = artefact.body.resolved_file_references.as_ref() else {
        panic!("the frozen body should retain one resolved-reference row");
    };
    assert_eq!(
        reference.path_syntax,
        PathSyntaxId::try_from_index(0).expect("compact path handle fits"),
        "the captured resolved row must use the compact path handle",
    );
    assert_eq!(reference.class, PreparedFileReferenceClass::ResourceFile);
    let StableResolvedFileReferenceOutcome::Resource {
        owner_relative_path,
    } = &reference.outcome
    else {
        panic!("the frozen body should retain a resource outcome");
    };
    assert_eq!(
        owner_relative_path, "assets/logo.svg",
        "the frozen body must retain the resolved resource path without materialising",
    );
}

#[test]
fn repeated_frozen_resource_body_materialisations_preserve_stable_origin() {
    let fixture = resource_body_materialisation_fixture();
    let materialise = || {
        let mut path_fork = fixture.owner_path_fork.fork_source().fork_for_module();
        fixture
            .context
            .materialise_ast_at(
                0,
                ModuleMaterialisationInput {
                    path_fork: &mut path_fork,
                    identity: &fixture.identity,
                    requester_context: &fixture.preparation,
                    requester_call_span: None,
                    boundary_string_table: &fixture.preparation.string_table,
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
    let mut path_fork = fixture.owner_path_fork.fork_source().fork_for_module();
    let mut materialiser = GeneratedFoldedValueMaterialiser {
        type_environment: &mut type_environment,
        external_registry: &external_registry,
        nominal_source: &fixture.preparation,
        template_ir_store,
        module_resources: Rc::clone(&module_resources),
        path_fork: &mut path_fork,
    };
    let mut string_table = StringTable::new();
    let first = super::materialize_public_folded_value(
        &mut materialiser,
        folded_default,
        string_type_id,
        &mut string_table,
        None,
    )
    .expect("the first frozen default projection should succeed");
    let second = super::materialize_public_folded_value(
        &mut materialiser,
        folded_default,
        string_type_id,
        &mut string_table,
        None,
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

    let materialise_once = || {
        let mut path_fork = fixture.owner_path_fork.fork_source().fork_for_module();
        fixture
            .context
            .materialise_ast_at(
                0,
                ModuleMaterialisationInput {
                    path_fork: &mut path_fork,
                    identity: &fixture.identity,
                    requester_context: &fixture.preparation,
                    requester_call_span: None,
                    boundary_string_table: &fixture.preparation.string_table,
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
            .find(|parameter| {
                matches!(
                    parameter.value.kind,
                    ExpressionKind::StructuralString { .. }
                )
            })
            .expect("the generated signature should retain its suffix resource parameter");
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
    let mut path_fork = PathInternerFork::empty();
    let identity = generated_identity("indexed").declaration().clone();
    let body_path = path_fork
        .try_intern_portable_path("src/indexed.moth", &mut string_table)
        .expect("test path fits");
    let imported_path = path_fork
        .try_intern_portable_path("src/imported.moth", &mut string_table)
        .expect("test path fits");
    let mut templates = FxHashMap::default();
    templates.insert(
        body_path,
        retained_template(body_path, identity.clone(), true),
    );
    templates.insert(
        imported_path,
        retained_template(imported_path, identity.clone(), false),
    );

    let index = ModuleMaterialisationPreparation::generic_template_identity_index(&templates)
        .expect("bodyless imported templates must not conflict with retained bodies");
    assert_eq!(index.get(&identity), Some(&body_path));

    let duplicate_path = path_fork
        .try_intern_portable_path("src/duplicate.moth", &mut string_table)
        .expect("test path fits");
    templates.insert(
        duplicate_path,
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
    let error = match Stage0ResolutionFacts::frozen_generic(
        SourceId::COMPILATION_ROOT,
        vec![frozen_resource_reference(
            PathSyntaxId::NONE,
            "assets/missing.svg",
        )],
    ) {
        Ok(_) => panic!("frozen generic facts must reject an absent path handle"),
        Err(error) => error,
    };
    assert_eq!(
        error.msg,
        "frozen generic resolved-reference row has an absent PathSyntaxId marker"
    );
}

#[test]
fn ordinary_stage0_lookup_returns_empty_for_unknown_row() {
    let facts = Stage0ResolutionFacts::ordinary(
        ResolvedFileReferenceTable::new(),
        SourceDatabase::empty().into(),
    );
    assert!(
        facts
            .lookup(SourceId::COMPILATION_ROOT, PathSyntaxId::NONE)
            .expect("ordinary Stage 0 lookup with its declaring SourceId should succeed")
            .is_none(),
        "ordinary Stage 0 lookup for an unknown row should return no view",
    );
}

#[test]
fn frozen_generic_lookup_requires_retained_owner() {
    let owner = SourceId::COMPILATION_ROOT;
    let facts = Stage0ResolutionFacts::frozen_generic(owner, Vec::new())
        .expect("empty frozen facts should build");
    assert!(
        facts
            .lookup(owner, PathSyntaxId::NONE)
            .expect("frozen lookup with its owner should succeed")
            .is_none(),
        "frozen lookup with the retained owner should remain valid",
    );
    assert!(
        facts
            .lookup(SourceId::from_index(7), PathSyntaxId::NONE)
            .is_err(),
        "frozen lookup must reject a non-owning SourceId so donor and requester stay distinct",
    );
}

#[test]
fn frozen_generic_rejects_duplicate_path_handle() {
    let mut path_syntax = PathSyntaxTable::new();
    let path_id = path_syntax.push(
        PathId::ROOT,
        SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    );
    let error = match Stage0ResolutionFacts::frozen_generic(
        SourceId::COMPILATION_ROOT,
        vec![
            frozen_resource_reference(path_id, "assets/first.svg"),
            frozen_resource_reference(path_id, "assets/second.svg"),
        ],
    ) {
        Ok(_) => panic!("frozen generic facts must reject duplicate path handles"),
        Err(error) => error,
    };
    assert_eq!(
        error.msg,
        "frozen generic resolved-reference table contains duplicate path handles"
    );
}

#[test]
fn generic_body_rejects_foreign_source_range() {
    let shell = FileTokens::new(PathId::ROOT, SourceId::COMPILATION_ROOT, Vec::new());
    let owner = shell
        .canonical_source_tokens_arc()
        .expect("test fixture must use a canonical source owner");
    let foreign_range =
        TokenRange::from_raw(SourceId::from_index(7), 0, 0).expect("range ordering is valid");
    let error = GenericFunctionBody::source(owner, foreign_range, None, PathId::ROOT, None)
        .expect_err("a generic body must reject a range owned by another source");
    assert!(
        error.msg.contains("foreign source identity"),
        "unexpected foreign-range error: {error:?}",
    );
}
#[test]
fn stale_in_range_template_row_fails_declaration_identity_validation() {
    let expected = generated_identity("expected");
    let stale = generated_identity("stale");
    let context = ModuleMaterialisationContext::from_identities_for_test(vec![
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

#[test]
fn provider_context_rebases_same_index_foreign_paths_by_spelling_for_the_requester() {
    // A published provider context retains the path table and source string table that issued
    // its retained `PathId`s. A requester from an independent boundary can hold a different
    // spelling at the same numeric index, so numeric reuse would render the wrong module.
    // The rebase must re-intern every retained path by spelling into the requester's fork and
    // rewrite the retained artefact paths through that remap.
    let mut provider_strings = StringTable::new();
    let mut provider_builder = PathInternerBuilder::new();
    let provider_source_file = provider_builder
        .try_intern_portable_path("provider/@mod.moth", &mut provider_strings)
        .expect("provider paths fit the checked table");
    let provider_function_path = provider_builder
        .try_intern_portable_path("provider/@mod.moth/generate", &mut provider_strings)
        .expect("provider paths fit the checked table");
    let provider_path_table = Arc::new(provider_builder.freeze());
    let source_string_table = Arc::new(provider_strings.freeze());

    let mut context = ModuleMaterialisationContext::from_identities_for_test(vec![
        generated_identity("published").declaration().clone(),
    ]);
    context.artefacts[0].source_file = provider_source_file;
    context.artefacts[0].function_path = provider_function_path;
    context.install_identity_tables(Arc::clone(&provider_path_table), source_string_table);

    // The requester's fork already interns "shared/@mod.moth" at indexes the provider table
    // also occupies, while naming different components behind them: same numeric index,
    // different spelling domains.
    let mut requester_strings = StringTable::new();
    requester_strings.intern("shared");
    requester_strings.intern("@mod.moth");
    let mut requester_fork = PathInternerFork::empty();
    let requester_shared = requester_fork
        .try_intern_portable_path("shared/@mod.moth", &mut requester_strings)
        .expect("requester paths fit the checked table");

    let rebased = context
        .rebased_for_requester(&mut requester_fork, &mut requester_strings)
        .expect("a well-formed provider context rebases into the requester domain");

    let artefact = &rebased.artefacts[0];
    assert_ne!(
        artefact.source_file, provider_source_file,
        "the requester domain must re-issue the retained source path"
    );
    assert_ne!(
        artefact.function_path, provider_function_path,
        "the requester domain must re-issue the retained function path"
    );

    // Both spellings must render correctly in their own domains, and the requester's
    // remapped paths must resolve through the requester's live fork table. The returned
    // context does not retain another complete requester snapshot.
    let requester_table = requester_fork.snapshot_table();
    let requester_frozen_strings = Arc::new(requester_strings.clone().freeze());
    let mut scratch = Vec::new();
    assert_eq!(
        provider_path_table.render_portable_frozen(
            provider_source_file,
            context
                .source_string_table
                .as_ref()
                .expect("source strings"),
            &mut scratch
        ),
        "provider/@mod.moth"
    );
    assert_eq!(
        requester_table.render_portable_frozen(
            artefact.source_file,
            &requester_frozen_strings,
            &mut scratch
        ),
        "provider/@mod.moth",
        "the requester spelling must follow the provider's, not its own colliding node"
    );
    let requester_rendered_function = requester_table.render_portable_frozen(
        artefact.function_path,
        &requester_frozen_strings,
        &mut scratch,
    );
    assert_eq!(
        requester_rendered_function, "provider/@mod.moth/generate",
        "the remapped function path must spell the provider's components"
    );
    assert_ne!(
        requester_table.render_portable_frozen(
            artefact.source_file,
            &requester_frozen_strings,
            &mut scratch
        ),
        requester_table.render_portable_frozen(
            requester_shared,
            &requester_frozen_strings,
            &mut scratch
        ),
        "the same numeric prefix must not collapse distinct domains"
    );
}

#[test]
fn incomplete_or_missing_retained_identity_pair_rejects_rebase() {
    let mut incomplete = ModuleMaterialisationContext::from_identities_for_test(vec![
        generated_identity("incomplete").declaration().clone(),
    ]);
    incomplete.path_table = Some(Arc::new(PathInternerFork::empty().snapshot_table()));
    let mut requester_fork = PathInternerFork::empty();
    let mut requester_strings = StringTable::new();
    let error = incomplete
        .rebased_for_requester(&mut requester_fork, &mut requester_strings)
        .err()
        .expect("an incomplete retained identity pair must reject rebasing");
    assert!(
        error.msg.contains("incomplete identity table pair"),
        "unexpected incomplete-pair error: {error:?}"
    );

    let missing = ModuleMaterialisationContext::from_identities_for_test(vec![
        generated_identity("missing").declaration().clone(),
    ]);
    let error = missing
        .rebased_for_requester(&mut requester_fork, &mut requester_strings)
        .err()
        .expect("a rebase without an issuing identity pair must reject");
    assert!(
        error
            .msg
            .contains("cannot rebase without an issuing identity table pair"),
        "unexpected missing-pair error: {error:?}"
    );
}
