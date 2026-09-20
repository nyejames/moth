//! Focused tests for frozen generic body and resource-default materialisation.
//!
//! WHAT: proves canonical token payloads reach a requester through one source-owned token
//! window, repeated spellings share one string entry, frozen resource defaults cross the
//! generated sidecar boundary through stable origins, and retained bodies stay `Send` without
//! owning a second token store.
//! WHY: the integration case `generic_parameter_default_file_value_success` owns authored
//! `@assets/logo.svg` syntax through Stage 0 resolution. These tests own the freeze-to-sidecar
//! seam from that resolved AST representation while keeping donor source identity explicit.

use super::super::{GenericFunctionBody, GenericFunctionTemplate, MaterialisedDonorContext};
use super::ModuleMaterialisationInput;
use super::artefact_emit::{ModuleMaterialisationContext, check_materialisation_row_identity};
use super::frozen_file_references::StableResolvedFileReferenceOutcome;
use super::frozen_syntax::{SharedDonorIdentity, StableBodySyntax};
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
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, render_payload,
};
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
    ExtendedSpanBuilder, FrozenIdentityHandle, LocalSpan, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathInternerBuilder, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TestSourceTokensBuilder, TokenIndex, TokenRange, TokenTag, TokenViewError,
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
) -> (Arc<SourceTokens>, PathSyntaxId) {
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
    let mut path_syntax = PathSyntaxTable::with_source(SourceId::COMPILATION_ROOT);
    let path_id = path_syntax.push(
        path_fork
            .try_intern_components(&[
                string_table.intern("provider"),
                string_table.intern("CONST"),
            ])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, token_span()),
    );
    let mut builder =
        TestSourceTokensBuilder::with_path_syntax(SourceId::COMPILATION_ROOT, path_syntax);
    builder
        .push_symbol(TokenTag::SYMBOL, string_table.intern("hello"), token_span())
        .expect("symbol fixture token should be valid");
    builder
        .push_symbol(
            TokenTag::STYLE_DIRECTIVE,
            string_table.intern("md"),
            token_span(),
        )
        .expect("style directive fixture token should be valid");
    builder
        .push_symbol(
            TokenTag::STRING_SLICE_LITERAL,
            string_table.intern("slice text"),
            token_span(),
        )
        .expect("string fixture token should be valid");
    builder
        .push_symbol(
            TokenTag::RAW_STRING_LITERAL,
            string_table.intern("raw text"),
            token_span(),
        )
        .expect("raw string fixture token should be valid");
    builder
        .push_char(TokenTag::CHAR_LITERAL, 'x', token_span())
        .expect("char fixture token should be valid");
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, token_span())
        .expect("boolean fixture token should be valid");
    builder
        .push_numeric(numeric, token_span())
        .expect("numeric fixture token should be valid");
    builder
        .push_path(TokenTag::PATH, path_id, token_span())
        .expect("path fixture token should be valid");
    builder
        .push_symbol(
            TokenTag::SYMBOL,
            string_table.intern("import"),
            token_span(),
        )
        .expect("import fixture token should be valid");
    builder
        .push_static(TokenTag::CHANNEL_RECEIVE, token_span())
        .expect("channel fixture token should be valid");
    (
        builder
            .finish()
            .expect("canonical token fixture should finish"),
        path_id,
    )
}

fn body_view(original: &Arc<SourceTokens>, declaration_path: PathId) -> GenericFunctionBody {
    let range = original
        .full_range()
        .expect("test token range must be in bounds");
    GenericFunctionBody::source(Arc::clone(original), range, None, declaration_path)
        .expect("test generic body should retain its checked source range")
}
fn capture_test_body(
    original: &Arc<SourceTokens>,
    source_file: &PathId,
    declaration_path: &PathId,
    path_fork: &PathInternerFork,
    source_table: &StringTable,
) -> StableBodySyntax {
    let source_file_id = original.source();

    let mut resolved_references = ResolvedFileReferenceTable::new();
    for (path_syntax, _) in original
        .path_syntax_table()
        .expect("test fixture should expose its path table")
        .iter()
    {
        resolved_references
            .push(ResolvedFileReference {
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
    let body = body_view(original, *declaration_path);
    let donor_identity = SharedDonorIdentity::freeze(source_table);
    StableBodySyntax::capture(
        &body,
        *source_file,
        path_fork,
        Some(&donor_identity),
        Some(&facts),
        FrozenIdentityHandle::new(),
        &no_content_value,
    )
    .expect("test body path rows should be resolved before capture")
}
fn canonical_tokens_with_path_syntax(
    source: SourceId,
    path_syntax: PathSyntaxTable,
    fill: impl FnOnce(&mut TestSourceTokensBuilder),
) -> Arc<SourceTokens> {
    let mut builder = TestSourceTokensBuilder::with_path_syntax(source, path_syntax);
    fill(&mut builder);
    builder
        .finish()
        .expect("canonical token fixture should finish")
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
    Arc<SourceTokens>,
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
    let mut path_syntax = PathSyntaxTable::with_source(body_file_id);
    let path_id = path_syntax.push(
        path_fork
            .try_intern_portable_path("@private.mtf", &mut string_table)
            .expect("test path fits"),
        SourceSpan::new(body_file_id, path_span),
    );
    let mut builder = TestSourceTokensBuilder::with_path_syntax(body_file_id, path_syntax);
    builder
        .push_path(TokenTag::PATH, path_id, path_span)
        .expect("content path fixture token should be valid");
    let body = builder
        .finish()
        .expect("canonical content fixture should finish");

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
    let donor_identity = SharedDonorIdentity::freeze(&source_table);
    let frozen = StableBodySyntax::capture(
        &body,
        source_file,
        &path_fork,
        Some(&donor_identity),
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
    let donor_identity = SharedDonorIdentity::freeze(&source_table);
    let error = match StableBodySyntax::capture(
        &body,
        source_file,
        &path_fork,
        Some(&donor_identity),
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
    let donor_identity = SharedDonorIdentity::freeze(&source_table);
    let error = match StableBodySyntax::capture(
        &body,
        source_file,
        &path_fork,
        Some(&donor_identity),
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
fn frozen_body_preserves_donor_range_and_path_spelling() {
    let mut source_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (original, _) = sample_tokens(&mut source_table, &mut path_fork);
    let source_file = path_fork
        .try_intern_portable_path("src/@mod.moth", &mut source_table)
        .expect("test path fits");
    let frozen = capture_test_body(
        &original,
        &source_file,
        &source_file,
        &path_fork,
        &source_table,
    );
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
        materialised.donor_file_id(),
        SourceId::COMPILATION_ROOT,
        "materialised generic syntax must retain its concrete donor identity",
    );
    let materialised_body = materialised
        .into_generic_body()
        .expect("materialised body should retain its checked range");
    let parse_owner = materialised_body
        .parse_owner()
        .expect("materialised body should retain a borrowed canonical parse owner");
    let owner = parse_owner.owner.as_ref();
    assert!(
        materialised_body
            .resolution_facts()
            .expect("materialised body should retain Stage 0 facts")
            .lookup(SourceId::COMPILATION_ROOT, path_id)
            .is_ok(),
        "materialised facts must accept the same concrete donor identity",
    );

    assert_eq!(
        materialised_body.token_range(),
        frozen.token_range,
        "materialisation preserves the retained donor range",
    );
    assert_eq!(
        materialised_body.token_sequence(),
        frozen.token_sequence,
        "materialisation preserves the retained donor sequence",
    );
    let materialised_path_syntax = owner
        .path_syntax_table()
        .expect("the borrowed owner should retain donor path rows");
    let path = materialised_path_syntax
        .try_path(path_id)
        .expect("valid donor path handle")
        .root;
    assert_eq!(
        path_fork.render_portable(path, &source_table, &mut Vec::new()),
        "provider/CONST",
        "the retained path row preserves donor spelling",
    );
}

#[test]
fn frozen_body_keeps_declaration_path_distinct_from_owning_source_file() {
    let mut source_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (original, _) = sample_tokens(&mut source_table, &mut path_fork);
    let source_file = path_fork
        .try_intern_portable_path("src/@mod.moth", &mut source_table)
        .expect("test path fits");
    let declaration_path = path_fork
        .try_intern_child(source_file, source_table.intern("generic_fn"))
        .expect("test declaration path fits");

    let frozen = capture_test_body(
        &original,
        &source_file,
        &declaration_path,
        &path_fork,
        &source_table,
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
        materialised.donor_file_id(),
        SourceId::COMPILATION_ROOT,
        "the canonical owner identity must remain distinct from declaration paths",
    );
    let materialised_body = materialised
        .into_generic_body()
        .expect("materialised body should retain its checked range");
    let parse_owner = materialised_body
        .parse_owner()
        .expect("materialised body should retain a borrowed canonical parse owner");
    let owner = parse_owner.owner.as_ref();
    owner
        .path_syntax_table()
        .expect("the borrowed owner should retain donor path rows")
        .validate_file_owned_locations(SourceId::COMPILATION_ROOT)
        .expect("canonical path rows stay owned by the donor source file");
}

#[test]
fn frozen_body_preserves_multiple_referenced_canonical_path_expressions() {
    let mut source_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::with_source(SourceId::COMPILATION_ROOT);
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
    let mut builder =
        TestSourceTokensBuilder::with_path_syntax(SourceId::COMPILATION_ROOT, path_syntax);
    builder
        .push_path(TokenTag::PATH, second_donor_path, base_span)
        .expect("second path fixture token should be valid");
    builder
        .push_path(TokenTag::PATH, first_donor_path, base_span)
        .expect("first path fixture token should be valid");
    let original = builder
        .finish()
        .expect("canonical path fixture should finish");

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
    let donor_identity = SharedDonorIdentity::freeze(&source_table);
    let frozen = StableBodySyntax::capture(
        &generic_body,
        source_file,
        &path_fork,
        Some(&donor_identity),
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
    let captured_token_ids = (0..original.len())
        .filter_map(|index| {
            let index = TokenIndex::try_from_index(index)?;
            original.token(index).ok()?.path_syntax_id()
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
    let materialised_owner = materialised.donor_file_id();
    let materialised_body = materialised
        .into_generic_body()
        .expect("materialised body should retain its checked source range");
    let parse_owner = materialised_body
        .parse_owner()
        .expect("same-boundary materialised body should retain its donor owner");
    let owner = parse_owner.owner.as_ref();
    let parser_token_ids = owner
        .shapes()
        .iter()
        .filter_map(|shape| shape.path_syntax_id())
        .collect::<Vec<_>>();
    assert_eq!(
        parser_token_ids, captured_token_ids,
        "a same-boundary borrowed owner must preserve the donor PathSyntaxIds used by facts",
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
    let mut path_syntax = PathSyntaxTable::with_source(SourceId::COMPILATION_ROOT);
    let path_span = token_span();
    let path_id = path_syntax.push(
        path_fork
            .try_intern_components(&[symbol_id])
            .expect("test path fits"),
        SourceSpan::new(SourceId::COMPILATION_ROOT, path_span),
    );
    let original =
        canonical_tokens_with_path_syntax(SourceId::COMPILATION_ROOT, path_syntax, |builder| {
            builder
                .push_symbol(TokenTag::SYMBOL, symbol_id, path_span)
                .expect("first symbol fixture token should be valid");
            builder
                .push_symbol(TokenTag::SYMBOL, symbol_id, path_span)
                .expect("second symbol fixture token should be valid");
            builder
                .push_path(TokenTag::PATH, path_id, path_span)
                .expect("path fixture token should be valid");
        });
    let source_file = path_fork
        .try_intern_portable_path("src/@mod.moth", &mut source_table)
        .expect("test path fits");
    let frozen = capture_test_body(
        &original,
        &source_file,
        &source_file,
        &path_fork,
        &source_table,
    );
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
#[test]
fn source_body_captures_share_donor_strings_after_construction_owners_drop() {
    let (body_file, source_file, path_fork, source_table, facts, _, _) =
        direct_content_body_fixture();
    let body = body_view(&body_file, source_file);
    let donor_identity = SharedDonorIdentity::freeze(&source_table);
    let content_value = |_path: &PathId| -> Result<PublicFoldedValue, CompilerError> {
        Ok(PublicFoldedValue::String(OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Text("retained".to_owned()),
        ])))
    };
    let capture = || {
        StableBodySyntax::capture(
            &body,
            source_file,
            &path_fork,
            Some(&donor_identity),
            Some(&facts),
            FrozenIdentityHandle::new(),
            &content_value,
        )
        .expect("source body should capture with one declaring donor")
    };
    let first = capture();
    let second = capture();
    let first_strings = first
        .source_string_table
        .as_ref()
        .expect("source capture should retain donor strings");
    let second_strings = second
        .source_string_table
        .as_ref()
        .expect("every source capture should retain donor strings");
    assert!(
        Arc::ptr_eq(first_strings, second_strings),
        "templates in one declaring domain must share one frozen donor allocation",
    );

    let (mut generated_path_fork, mut generated_table) =
        generated_materialisation_domain(&path_fork, &source_table);
    let generated_source_file = generated_path_fork
        .try_intern_portable_path("@mod.moth", &mut generated_table)
        .expect("generated source path should fit");
    drop(donor_identity);
    drop(source_table);
    drop(body_file);
    drop(body);

    let materialised = first
        .materialise(
            generated_source_file,
            &mut generated_path_fork,
            &mut generated_table,
            None,
        )
        .expect("shared donor strings should keep the body materialisable after source drop");
    let materialised_body = materialised
        .into_generic_body()
        .expect("materialised body should retain its checked source owner");
    let parse_owner = materialised_body
        .parse_owner()
        .expect("materialised body should retain its donor owner");
    let owner = parse_owner.owner.as_ref();
    assert_eq!(
        owner.len(),
        1,
        "the retained body must remain bounded after its construction owners drop",
    );
}

#[test]
fn preparation_clones_share_one_frozen_donor_allocation() {
    let source = "draw type T |name T| -> String:\n    return \"ok\"\n;\n";
    let (build_result, _path_fork, _string_table) =
        parse_single_file_ast_build_result(source).expect("generic source should build");
    let preparation = build_result
        .materialisation_context
        .finish_preparation()
        .expect("generic template identity index should build");
    // Clone before the first cache fill: a non-shared cache would freeze once per clone here.
    let cloned = preparation.clone();
    assert!(
        Arc::ptr_eq(
            preparation.donor_identity().strings(),
            cloned.donor_identity().strings(),
        ),
        "every clone of one declaring preparation must share one frozen donor allocation",
    );
}

#[test]
fn a_materialised_body_with_only_a_path_identity_is_rejected() {
    let source = SourceId::COMPILATION_ROOT;
    let source_file = PathId::ROOT;
    let path_fork = PathInternerFork::empty();
    let owner = canonical_tokens_with_path_syntax(
        source,
        PathSyntaxTable::with_source(source),
        |builder| {
            builder
                .push_static(TokenTag::EOF, token_span())
                .expect("EOF fixture token should be valid");
        },
    );
    let error = match GenericFunctionBody::materialised(
        Arc::clone(&owner),
        TokenRange::from_raw(source, 0, 1).expect("fixture range should fit"),
        None,
        source_file,
        MaterialisedDonorContext {
            resolution_facts: Arc::new(
                Stage0ResolutionFacts::frozen_generic(source, Vec::new())
                    .expect("empty frozen facts should be valid"),
            ),
            frozen_identity_handle: FrozenIdentityHandle::new(),
            source_path_table: Some(Arc::new(path_fork.snapshot_table())),
            source_string_table: None,
        },
    ) {
        Ok(_) => panic!("a path identity without its string owner must be rejected"),
        Err(error) => error,
    };
    assert!(
        error.msg.contains("incomplete source identity table pair"),
        "unexpected incomplete donor error: {error:?}",
    );
}

#[test]
fn canonical_string_only_donors_borrow_identity_without_retaining_file_tokens() {
    let source = SourceId::COMPILATION_ROOT;
    let source_file = PathId::ROOT;
    let path_fork = PathInternerFork::empty();
    let mut donor_string_table = StringTable::new();
    let donor_symbol = donor_string_table.intern("donor");
    let owner = canonical_tokens_with_path_syntax(
        source,
        PathSyntaxTable::with_source(source),
        |builder| {
            builder
                .push_symbol(TokenTag::SYMBOL, donor_symbol, token_span())
                .expect("symbol fixture token should be valid");
        },
    );
    let end = TokenIndex::try_from_index(owner.len()).expect("test token range fits");
    let range = TokenRange::try_new_for(
        owner.as_ref(),
        TokenIndex::try_from_index(0).expect("test token range fits"),
        end,
    )
    .expect("test token range must be in bounds");
    let body = GenericFunctionBody::materialised(
        owner,
        range,
        None,
        source_file,
        MaterialisedDonorContext {
            resolution_facts: Arc::new(
                Stage0ResolutionFacts::frozen_generic(source, Vec::new())
                    .expect("empty frozen facts should be valid"),
            ),
            frozen_identity_handle: FrozenIdentityHandle::new(),
            source_path_table: None,
            source_string_table: Some(Arc::new(donor_string_table.freeze())),
        },
    )
    .expect("canonical body should retain its donor strings");
    let donor_identity = SharedDonorIdentity::freeze(&StringTable::new());
    let frozen = StableBodySyntax::capture(
        &body,
        source_file,
        &path_fork,
        Some(&donor_identity),
        body.resolution_facts().map(Arc::as_ref),
        FrozenIdentityHandle::new(),
        &|_| Err(CompilerError::compiler_error("no content rows")),
    )
    .expect("a string-only canonical donor should capture");
    let mut materialised_path_fork = PathInternerFork::empty();
    let mut materialised_strings = StringTable::new();
    let requester_symbol = materialised_strings.intern("requester");
    assert_eq!(
        requester_symbol, donor_symbol,
        "the fixture must exercise a colliding StringId",
    );
    let materialised = frozen
        .materialise(
            source_file,
            &mut materialised_path_fork,
            &mut materialised_strings,
            None,
        )
        .expect("a string-only canonical donor should stay on the canonical lane");
    let materialised_body = materialised
        .into_generic_body()
        .expect("canonical string-only body should retain its checked range");
    let parse_owner = materialised_body
        .parse_owner()
        .expect("a canonical string-only donor should retain a borrowed parse owner");
    let (cursor, source_id) = parse_owner
        .cursor()
        .expect("a borrowed parse owner should borrow its canonical cursor");
    assert_eq!(source_id, source);
    let symbol = cursor
        .current_string_id_in(&mut materialised_strings)
        .expect("the donor symbol payload should translate")
        .expect("the current token should carry a symbol payload");
    assert_eq!(
        materialised_strings.resolve(symbol),
        "donor",
        "the borrowed cursor must resolve colliding donor StringIds to the donor spelling",
    );
    let projected = cursor
        .current_diagnostic_token(&mut materialised_strings)
        .expect("materialised body diagnostic projection should succeed")
        .expect("materialised body should expose its current token");
    let diagnostic = CompilerDiagnostic::unexpected_token_from_tag(projected, None);
    let rendered = render_payload(
        &diagnostic.payload,
        DiagnosticRenderContext::new(&materialised_strings),
    )
    .message;
    assert!(
        rendered.contains("donor"),
        "materialised-body diagnostics must render donor spelling, got {rendered:?}",
    );
}

#[test]
fn malformed_materialised_donor_text_handle_is_infrastructure_failure() {
    let source = SourceId::COMPILATION_ROOT;
    let source_file = PathId::ROOT;
    let invalid_handle = StringId::from_index(0);
    let owner = canonical_tokens_with_path_syntax(
        source,
        PathSyntaxTable::with_source(source),
        |builder| {
            builder
                .push_symbol(TokenTag::SYMBOL, invalid_handle, token_span())
                .expect("malformed donor symbol shape should still be packable");
        },
    );
    let end = TokenIndex::try_from_index(owner.len()).expect("test token range fits");
    let range = TokenRange::try_new_for(
        owner.as_ref(),
        TokenIndex::try_from_index(0).expect("test token range fits"),
        end,
    )
    .expect("test token range must be in bounds");
    let empty_donor_strings = StringTable::new();
    let body = GenericFunctionBody::materialised(
        owner,
        range,
        None,
        source_file,
        MaterialisedDonorContext {
            resolution_facts: Arc::new(
                Stage0ResolutionFacts::frozen_generic(source, Vec::new())
                    .expect("empty frozen facts should be valid"),
            ),
            frozen_identity_handle: FrozenIdentityHandle::new(),
            source_path_table: None,
            source_string_table: Some(Arc::new(empty_donor_strings.freeze())),
        },
    )
    .expect("malformed donor body should retain its checked owner");
    let parse_owner = body
        .parse_owner()
        .expect("materialised donor body should expose a parse owner");
    let (cursor, _) = parse_owner
        .cursor()
        .expect("materialised donor body should construct its cursor");
    let error = cursor
        .current_diagnostic_token(&mut StringTable::new())
        .expect_err("malformed donor text must fail diagnostic projection");
    assert_eq!(error, TokenViewError::MalformedStringHandle);
    let infrastructure = CompilerDiagnostic::token_view_invariant_error(
        error,
        "materialised body diagnostic projection",
    );
    assert_eq!(infrastructure.error_type, ErrorType::Compiler);
}

#[test]
fn donor_numeric_literal_text_renders_through_borrowed_origin() {
    let source = SourceId::COMPILATION_ROOT;
    let source_file = PathId::ROOT;
    let path_fork = PathInternerFork::empty();
    let mut donor_strings = StringTable::new();
    let donor_numeric = NumericLiteralToken::new(
        NumericLiteralSign::Negative,
        donor_strings.intern("-12.5"),
        donor_strings.intern("12.5"),
        NumericLiteralKind::DecimalPoint,
        2,
        1,
        0,
        NumericExponentSign::None,
    );
    let donor_authored_text = donor_numeric.source_text;
    let donor_normalized_text = donor_numeric.normalized_text;
    let owner = canonical_tokens_with_path_syntax(
        source,
        PathSyntaxTable::with_source(source),
        |builder| {
            builder
                .push_numeric(donor_numeric, token_span())
                .expect("numeric fixture token should be valid");
        },
    );
    let end = TokenIndex::try_from_index(owner.len()).expect("test token range fits");
    let range = TokenRange::try_new_for(
        owner.as_ref(),
        TokenIndex::try_from_index(0).expect("test token range fits"),
        end,
    )
    .expect("test token range must be in bounds");
    let body = GenericFunctionBody::materialised(
        owner,
        range,
        None,
        source_file,
        MaterialisedDonorContext {
            resolution_facts: Arc::new(
                Stage0ResolutionFacts::frozen_generic(source, Vec::new())
                    .expect("empty frozen facts should be valid"),
            ),
            frozen_identity_handle: FrozenIdentityHandle::new(),
            source_path_table: None,
            source_string_table: Some(Arc::new(donor_strings.freeze())),
        },
    )
    .expect("a numeric donor body should retain its donor strings");
    let donor_identity = SharedDonorIdentity::freeze(&StringTable::new());
    let stable = StableBodySyntax::capture(
        &body,
        source_file,
        &path_fork,
        Some(&donor_identity),
        body.resolution_facts().map(Arc::as_ref),
        FrozenIdentityHandle::new(),
        &|_| Err(CompilerError::compiler_error("no content rows")),
    )
    .expect("a numeric donor body should capture");
    let mut materialised_path_fork = PathInternerFork::empty();
    let mut materialised_strings = StringTable::new();
    // The requester domain names different text behind the donor's numeric literal StringIds.
    assert_eq!(
        materialised_strings.intern("requester authored"),
        donor_authored_text,
        "the fixture must collide with the donor's authored-text StringId",
    );
    assert_eq!(
        materialised_strings.intern("requester normalized"),
        donor_normalized_text,
        "the fixture must collide with the donor's normalized-text StringId",
    );
    let materialised_body = stable
        .materialise(
            source_file,
            &mut materialised_path_fork,
            &mut materialised_strings,
            None,
        )
        .expect("a numeric donor body should materialise")
        .into_generic_body()
        .expect("a numeric donor body should retain its checked range");
    let parse_owner = materialised_body
        .parse_owner()
        .expect("the donor body should retain its parse owner");
    let (cursor, source_id) = parse_owner
        .cursor()
        .expect("the donor body should borrow its canonical cursor");
    assert_eq!(source_id, source);
    let numeric = cursor
        .current_numeric_literal_in(&mut materialised_strings)
        .expect("the donor numeric payload should translate")
        .expect("the current token should carry a numeric payload");
    assert_eq!(
        materialised_strings.resolve(numeric.source_text),
        "-12.5",
        "the borrowed numeric literal must keep the donor's authored text",
    );
    assert_eq!(
        materialised_strings.resolve(numeric.normalized_text),
        "12.5",
        "the borrowed numeric literal must keep the donor's normalized text",
    );
    assert_eq!(
        (
            numeric.kind,
            numeric.digit_count,
            numeric.fractional_digit_count,
        ),
        (NumericLiteralKind::DecimalPoint, 2, 1),
        "typed payload translation must not disturb the donor's lexical facts",
    );
    let projected = cursor
        .current_diagnostic_token(&mut materialised_strings)
        .expect("donor numeric diagnostic projection should succeed")
        .expect("the current numeric token should project into a diagnostic token");
    let diagnostic = CompilerDiagnostic::unexpected_token_from_tag(projected, None);
    let rendered = render_payload(
        &diagnostic.payload,
        DiagnosticRenderContext::new(&materialised_strings),
    )
    .message;
    assert!(
        rendered.contains("-12.5"),
        "numeric donor diagnostics must render donor-authored text, got {rendered:?}",
    );
    assert!(
        !rendered.contains("requester authored"),
        "numeric donor diagnostics must not use colliding requester text, got {rendered:?}",
    );
}

/// A segmented donor body borrows its canonical owner and registered sequence.
///
/// The skipped donor tokens remain outside the parser view while donor payload spelling is
/// translated only when consumed through the borrowed cursor.
#[test]
fn a_segmented_donor_body_borrows_its_source_sequence() {
    let source = SourceId::COMPILATION_ROOT;
    let mut donor_strings = StringTable::new();
    let mut spans = ExtendedSpanBuilder::default();
    let span_at = |offset: u32, spans: &mut ExtendedSpanBuilder| {
        LocalSpan::exact(offset, 2, spans).expect("fixture spans fit inline")
    };
    let skipped_numeric = NumericLiteralToken::new(
        NumericLiteralSign::Positive,
        donor_strings.intern("7"),
        donor_strings.intern("7"),
        NumericLiteralKind::WholeNumber,
        1,
        0,
        0,
        NumericExponentSign::None,
    );
    let kept_numeric = NumericLiteralToken::new(
        NumericLiteralSign::Negative,
        donor_strings.intern("-12.5"),
        donor_strings.intern("12.5"),
        NumericLiteralKind::DecimalPoint,
        2,
        1,
        0,
        NumericExponentSign::None,
    );
    let mut builder =
        TestSourceTokensBuilder::with_path_syntax(source, PathSyntaxTable::with_source(source));
    builder
        .push_symbol(
            TokenTag::SYMBOL,
            donor_strings.intern("alpha"),
            span_at(0, &mut spans),
        )
        .expect("alpha fixture token should be valid");
    builder
        .push_numeric(skipped_numeric, span_at(8, &mut spans))
        .expect("skipped numeric token should be valid");
    builder
        .push_symbol(
            TokenTag::SYMBOL,
            donor_strings.intern("beta"),
            span_at(16, &mut spans),
        )
        .expect("beta fixture token should be valid");
    builder
        .push_numeric(kept_numeric, span_at(24, &mut spans))
        .expect("kept numeric token should be valid");
    builder
        .push_static(TokenTag::EOF, span_at(32, &mut spans))
        .expect("EOF fixture token should be valid");
    let mut donor_owner = builder
        .finish_unfrozen()
        .expect("canonical sequence fixture should finish");
    let segments = [
        TokenRange::from_raw(source, 0, 1).expect("fixture segment should be ordered"),
        TokenRange::from_raw(source, 2, 5).expect("fixture segment should be ordered"),
    ];
    let owner =
        Arc::get_mut(&mut donor_owner).expect("canonical sequence owner should be uniquely owned");
    let sequence = owner
        .try_register_token_sequence(&segments)
        .expect("ordered adjacent segments should register");
    owner.freeze_numeric_literals();
    let donor_owner = donor_owner;
    let body = GenericFunctionBody::materialised(
        Arc::clone(&donor_owner),
        TokenRange::from_raw(source, 0, 5).expect("covering range should be ordered"),
        Some(sequence),
        PathId::ROOT,
        MaterialisedDonorContext {
            resolution_facts: Arc::new(
                Stage0ResolutionFacts::frozen_generic(source, Vec::new())
                    .expect("empty frozen facts should be valid"),
            ),
            frozen_identity_handle: FrozenIdentityHandle::new(),
            source_path_table: None,
            source_string_table: Some(Arc::new(donor_strings.freeze())),
        },
    )
    .expect("a segmented donor body should retain its donor strings");

    // The requester names different text behind the donor's payload StringIds.
    let mut requester_strings = StringTable::new();
    let _requester_fork = PathInternerFork::empty();
    requester_strings.intern("requester alpha");
    requester_strings.intern("requester beta");
    let parse_owner = body
        .parse_owner()
        .expect("a segmented donor body should retain its source owner");
    let owner = parse_owner.owner.as_ref();

    let donor_indices = [0usize, 2, 3, 4];
    assert_eq!(
        parse_owner.token_sequence,
        Some(sequence),
        "the parse owner must retain the donor's segmented sequence",
    );
    assert_eq!(
        owner.source(),
        source,
        "the borrowed owner keeps the donor source identity",
    );
    let (mut cursor, source_id) = parse_owner
        .cursor()
        .expect("a segmented donor body should borrow its canonical cursor");
    assert_eq!(source_id, source);
    let tags = (0..donor_indices.len())
        .map(|offset| {
            cursor
                .token_ref_at_offset(offset)
                .expect("the segmented cursor should expose each retained token")
                .tag()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tags,
        donor_indices
            .iter()
            .map(|index| donor_owner.shapes()[*index].tag())
            .collect::<Vec<_>>(),
        "the borrowed sequence preserves donor tags in sequence order",
    );
    let spans = (0..donor_indices.len())
        .map(|offset| {
            cursor
                .token_ref_at_offset(offset)
                .expect("the segmented cursor should expose each retained span")
                .span()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        spans,
        donor_indices
            .iter()
            .map(|index| donor_owner.spans()[*index])
            .collect::<Vec<_>>(),
        "the borrowed sequence preserves donor spans for diagnostics",
    );

    let _donor_strings = parse_owner
        .payload_origin
        .expect("materialised bodies retain donor string identity")
        .strings;
    let alpha = cursor
        .current_string_id_in(&mut requester_strings)
        .expect("the first donor symbol should translate")
        .expect("the first sequence token should be a symbol");
    assert_eq!(requester_strings.resolve(alpha), "alpha");
    cursor
        .set_position(2)
        .expect("the segmented cursor should seek to its numeric token");
    let numeric = cursor
        .current_numeric_literal_in(&mut requester_strings)
        .expect("the donor numeric payload should translate")
        .expect("the retained sequence should include one numeric token");
    assert_eq!(requester_strings.resolve(numeric.source_text), "-12.5");
    assert_eq!(requester_strings.resolve(numeric.normalized_text), "12.5");
}

#[test]
fn donor_path_handles_render_through_borrowed_origin() {
    let source = SourceId::COMPILATION_ROOT;
    let mut donor_strings = StringTable::new();
    let mut donor_builder = PathInternerBuilder::new();
    let donor_root = donor_builder
        .try_intern_portable_path("provider/CONST", &mut donor_strings)
        .expect("donor paths fit the checked table");
    let donor_path_table = Arc::new(donor_builder.freeze());
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let path_handle = path_syntax.push(donor_root, SourceSpan::new(source, token_span()));
    let owner = canonical_tokens_with_path_syntax(source, path_syntax, |builder| {
        builder
            .push_path(TokenTag::PATH, path_handle, token_span())
            .expect("path fixture token should be valid");
    });
    let end = TokenIndex::try_from_index(owner.len()).expect("test token range fits");
    let range = TokenRange::try_new_for(
        owner.as_ref(),
        TokenIndex::try_from_index(0).expect("test token range fits"),
        end,
    )
    .expect("test token range must be in bounds");
    let body = GenericFunctionBody::materialised(
        owner,
        range,
        None,
        PathId::ROOT,
        MaterialisedDonorContext {
            resolution_facts: Arc::new(
                Stage0ResolutionFacts::frozen_generic(source, Vec::new())
                    .expect("empty frozen facts should be valid"),
            ),
            frozen_identity_handle: FrozenIdentityHandle::new(),
            source_path_table: Some(Arc::clone(&donor_path_table)),
            source_string_table: Some(Arc::new(donor_strings.freeze())),
        },
    )
    .expect("a path-bearing donor body should retain its donor pair");

    // The requester fork interns a colliding path, but the borrowed cursor must preserve the
    // donor spelling through its retained path/string identity.
    let mut requester_strings = StringTable::new();
    let mut requester_fork = PathInternerFork::empty();
    let requester_other = requester_fork
        .try_intern_portable_path("shared/OTHER", &mut requester_strings)
        .expect("requester paths fit the checked table");
    assert_eq!(
        requester_other, donor_root,
        "the fixture must exercise a colliding PathId",
    );

    let parse_owner = body
        .parse_owner()
        .expect("the donor body should retain its source owner");
    let (cursor, source_id) = parse_owner
        .cursor()
        .expect("the donor body should borrow its source cursor");
    assert_eq!(source_id, source);
    let donor_row = cursor
        .current_path_syntax()
        .expect("the donor path payload should validate")
        .expect("the current token should carry a path row");
    let donor_strings = parse_owner
        .payload_origin
        .expect("path-bearing bodies retain donor identity")
        .strings;
    let mut scratch = Vec::new();
    assert_eq!(
        donor_path_table.render_portable_frozen(donor_row.root, donor_strings, &mut scratch),
        "provider/CONST",
        "the borrowed path row must preserve the donor spelling",
    );
    assert_eq!(
        donor_row.span,
        token_span(),
        "the borrowed path row must preserve the donor authored span",
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
            let owner = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT)
                .finish()
                .expect("canonical empty source fixture should finish");
            body_view(&owner, path)
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
        let body_table = string_table.clone();
        let (body_owner, body_range, body_sequence) = body.canonical_view();
        assert!(
            body_sequence.is_none(),
            "the authored source fixture body is contiguous"
        );
        let body_file_id = body_owner.source();
        let placeholder_span = (body_range.start().index()..body_range.end().index())
            .find_map(|index| {
                let token_index = TokenIndex::try_from_index(index)?;
                let token = body_owner.token(token_index).ok()?;
                (token.tag() == TokenTag::STRING_SLICE_LITERAL)
                    .then(|| {
                        token
                            .string_id()
                            .filter(|id| body_table.resolve(*id) == "placeholder")
                            .map(|_| token.span())
                    })
                    .flatten()
            })
            .expect("the placeholder body literal should be present");
        let mut path_syntax = PathSyntaxTable::with_source(body_file_id);
        let path_id = path_syntax.push(
            path_fork
                .try_intern_components(&[assets_component, logo_component])
                .expect("test path fits"),
            SourceSpan::new(body_file_id, placeholder_span),
        );
        let mut builder = TestSourceTokensBuilder::with_path_syntax(body_file_id, path_syntax);
        for index in body_range.start().index()..body_range.end().index() {
            let token_index =
                TokenIndex::try_from_index(index).expect("source body token index should fit");
            let token = body_owner
                .token(token_index)
                .expect("source body token should remain readable");
            let span = token.span();
            match token.tag() {
                TokenTag::SYMBOL
                | TokenTag::STYLE_DIRECTIVE
                | TokenTag::STRING_SLICE_LITERAL
                | TokenTag::RAW_STRING_LITERAL => {
                    let string_id = token
                        .string_id()
                        .expect("string-shaped source token should carry its payload");
                    if token.tag() == TokenTag::STRING_SLICE_LITERAL
                        && body_table.resolve(string_id) == "placeholder"
                    {
                        builder
                            .push_path(TokenTag::PATH, path_id, span)
                            .expect("replacement path token should be valid");
                    } else {
                        builder
                            .push_symbol(token.tag(), string_id, span)
                            .expect("string source token should be valid");
                    }
                }
                TokenTag::NUMERIC_LITERAL => {
                    builder
                        .push_numeric(
                            token
                                .numeric_literal()
                                .expect("numeric source token should validate")
                                .expect("numeric source token should carry its payload")
                                .clone(),
                            span,
                        )
                        .expect("numeric source token should be valid");
                }
                TokenTag::CHAR_LITERAL => {
                    builder
                        .push_char(
                            TokenTag::CHAR_LITERAL,
                            token
                                .char_value()
                                .expect("char source token should validate"),
                            span,
                        )
                        .expect("char source token should be valid");
                }
                TokenTag::BOOL_LITERAL => {
                    builder
                        .push_bool(
                            TokenTag::BOOL_LITERAL,
                            token
                                .bool_value()
                                .expect("bool source token should validate"),
                            span,
                        )
                        .expect("bool source token should be valid");
                }
                TokenTag::PATH => {
                    builder
                        .push_path(
                            TokenTag::PATH,
                            token
                                .path_syntax_id()
                                .expect("path source token should carry its handle"),
                            span,
                        )
                        .expect("path source token should be valid");
                }
                tag => {
                    builder
                        .push_static(tag, span)
                        .expect("static source token should be valid");
                }
            }
        }
        let body_owner = builder
            .finish()
            .expect("canonical resource body fixture should finish");
        let body_source_file = body_file_id;
        template.body_tokens = Some(body_view(&body_owner, declaration_path));
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
        let mut path_syntax = PathSyntaxTable::with_source(SourceId::COMPILATION_ROOT);
        let path_id = path_syntax.push(
            path_fork
                .try_intern_portable_path("@resource.bin", &mut source_table)
                .expect("test path fits"),
            SourceSpan::new(SourceId::COMPILATION_ROOT, path_span),
        );
        let mut body_builder =
            TestSourceTokensBuilder::with_path_syntax(SourceId::COMPILATION_ROOT, path_syntax);
        body_builder
            .push_path(TokenTag::PATH, path_id, path_span)
            .expect("collision fixture path token should be valid");
        let body = body_builder
            .finish()
            .expect("collision fixture source tokens should finish");
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
        let donor_identity = SharedDonorIdentity::freeze(&source_table);
        let frozen = StableBodySyntax::capture(
            &generic_body,
            source_file,
            &path_fork,
            Some(&donor_identity),
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
    let owner = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT)
        .finish()
        .expect("empty canonical source owner should finish");
    let foreign_range =
        TokenRange::from_raw(SourceId::from_index(7), 0, 0).expect("range ordering is valid");
    let error = GenericFunctionBody::source(owner, foreign_range, None, PathId::ROOT)
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

#[test]
fn materialised_body_declaration_cursor_translates_colliding_symbol_payloads() {
    let source = SourceId::COMPILATION_ROOT;
    let source_file = PathId::ROOT;
    let mut donor_strings = StringTable::new();
    let donor_current = donor_strings.intern("donor current");
    let donor_token_at = donor_strings.intern("donor token-at");
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_symbol(TokenTag::SYMBOL, donor_current, token_span())
        .expect("donor current token should be valid");
    builder
        .push_symbol(TokenTag::SYMBOL, donor_token_at, token_span())
        .expect("donor token-at token should be valid");
    let owner = builder
        .finish()
        .expect("canonical symbol fixture should finish");
    let end = TokenIndex::try_from_index(owner.len()).expect("test range should fit");
    let range = TokenRange::try_new_for(
        owner.as_ref(),
        TokenIndex::try_from_index(0).expect("test range should fit"),
        end,
    )
    .expect("test range should be in bounds");
    let body = GenericFunctionBody::materialised(
        owner,
        range,
        None,
        source_file,
        MaterialisedDonorContext {
            resolution_facts: Arc::new(
                Stage0ResolutionFacts::frozen_generic(source, Vec::new())
                    .expect("empty frozen facts should be valid"),
            ),
            frozen_identity_handle: FrozenIdentityHandle::new(),
            source_path_table: None,
            source_string_table: Some(Arc::new(donor_strings.freeze())),
        },
    )
    .expect("the materialised body should retain its donor strings");

    let mut requester_strings = StringTable::new();
    let requester_current = requester_strings.intern("requester current");
    let requester_token_at = requester_strings.intern("requester token-at");
    assert_eq!(
        requester_current, donor_current,
        "the fixture must collide on the current symbol handle",
    );
    assert_eq!(
        requester_token_at, donor_token_at,
        "the fixture must collide on the token-at symbol handle",
    );

    let parse_owner = body
        .parse_owner()
        .expect("the materialised body should retain a borrowed parse owner");
    let (cursor, source_id) = parse_owner
        .cursor()
        .expect("the materialised body should borrow its canonical cursor");
    assert_eq!(source_id, source);
    let declaration = cursor
        .declaration_cursor()
        .expect("the borrowed cursor should build a declaration cursor");
    let current = declaration
        .current_string_id_in(&mut requester_strings)
        .expect("the declaration cursor should translate its current donor symbol")
        .expect("the current token should carry a symbol payload");
    assert_eq!(
        requester_strings.resolve(current),
        "donor current",
        "DeclarationCursor must resolve current symbols through the donor origin",
    );
    let token_at = declaration
        .token_string_id_at_in(1, &mut requester_strings)
        .expect("the declaration cursor should translate its token-at donor symbol")
        .expect("the token-at position should carry a symbol payload");
    assert_eq!(
        requester_strings.resolve(token_at),
        "donor token-at",
        "token-at reads must resolve through the donor origin rather than raw requester IDs",
    );
}
