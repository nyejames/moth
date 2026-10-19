//! Type-syntax parsing and resolution regression tests.
//!
//! WHAT: validates type annotation parsing and type resolution in composite types.
//! WHY: type syntax is the source of truth for frontend type identity; parser drift here
//!      affects every downstream type check.

use crate::compiler_frontend::ast::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::ast_nodes::DeclarationShell;
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;

use crate::compiler_frontend::ast::type_resolution::{
    ResolvedTypeAlias, TypeResolutionContext, resolve_parsed_type_annotation,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, DiagnosticToken, GenericApplicationErrorReason,
    InvalidCollectionTypeReason, InvalidGenericInstantiationReason, InvalidMapTypeReason,
    InvalidTypeAnnotationReason, NameNamespace,
};
use crate::compiler_frontend::datatypes::definitions::{StructTypeDefinition, TypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::TypeParameterId;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::datatypes::{TypeId, builtin_type_ids};
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    ParsedNamedTypeReference, TypeAnnotationContext, builtin_scalar_type_name_for_tag,
    parse_type_annotation_cursor,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TestSourceTokensBuilder, TokenTag,
};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;
use std::rc::Rc;
use std::sync::Arc;

enum FixturePart {
    Static(TokenTag),
    Symbol(StringId),
    Numeric(NumericLiteralToken),
}

fn numeric_token(value: &str, string_table: &mut StringTable) -> FixturePart {
    FixturePart::Numeric(NumericLiteralToken::test_new(value, string_table))
}

fn stream_from_tokens(
    tokens: Vec<FixturePart>,
    _string_table: &mut StringTable,
) -> Arc<SourceTokens> {
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    for token in tokens {
        match token {
            FixturePart::Static(tag) => builder
                .push_static(tag, LocalSpan::source_start())
                .expect("static type fixture token should build"),
            FixturePart::Symbol(value) => builder
                .push_symbol(TokenTag::SYMBOL, value, LocalSpan::source_start())
                .expect("symbol type fixture token should build"),
            FixturePart::Numeric(value) => builder
                .push_numeric(value, LocalSpan::source_start())
                .expect("numeric type fixture token should build"),
        }
    }
    builder
        .finish()
        .expect("canonical type fixture tokens should build")
}

/// Parse a type annotation through the canonical source owner and declaration cursor.
fn parse_type_annotation_test(
    stream: &Arc<SourceTokens>,
    context: TypeAnnotationContext,
    string_table: &mut StringTable,
) -> Result<ParsedTypeRef, HeaderParseFailure> {
    let range = stream
        .full_range()
        .expect("test canonical source owner must expose a checked full range");
    let cursor = stream
        .cursor(range)
        .expect("test canonical source owner must accept its checked full range");
    let mut declaration_cursor =
        DeclarationCursor::new(cursor).expect("test canonical cursor must build");
    parse_type_annotation_cursor(&mut declaration_cursor, context, string_table)
}

fn token(tag: TokenTag) -> FixturePart {
    FixturePart::Static(tag)
}
fn symbol_token(value: StringId) -> FixturePart {
    FixturePart::Symbol(value)
}

fn assert_diagnostic_payload(
    diagnostic: impl std::borrow::Borrow<CompilerDiagnostic>,
    expected: impl FnOnce(&DiagnosticPayload) -> bool,
    expected_description: &str,
) {
    let diagnostic = diagnostic.borrow();

    assert!(
        expected(&diagnostic.payload),
        "expected {expected_description}, got {:?}",
        diagnostic.payload
    );
}

/// Unwrap a type-syntax parse failure into its authored-source diagnostic.
///
/// WHAT: matches the typed `HeaderParseFailure` lane so tests pin the user diagnostic
///       instead of conflating infrastructure failure with a source diagnostic.
/// WHY: infrastructure failures must abort the test explicitly rather than being mistaken
///      for a source diagnostic.
fn unwrap_type_parse_diagnostic(failure: HeaderParseFailure) -> CompilerDiagnostic {
    match failure {
        HeaderParseFailure::Diagnostic(diagnostic) => diagnostic,
        HeaderParseFailure::Infrastructure(error) => {
            panic!("type syntax infrastructure failure is not a source diagnostic: {error:?}")
        }
    }
}

fn register_single_parameter_struct(
    type_environment: &mut TypeEnvironment,
    path: &PathId,
    parameter_name: StringId,
) -> TypeId {
    let parameters = type_environment.register_generic_parameter_list(
        [(TypeParameterId(0), parameter_name)].into_iter(),
        &FxHashMap::default(),
    );
    let (_, type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: *path,
        fields: Box::new([]),
        generic_parameters: Some(parameters.list_id),
        const_record: false,
    });
    type_id
}

fn resolve_type_annotation_error(
    parsed: ParsedTypeRef,
    string_table: &mut StringTable,
    expected_failure: &str,
) -> CompilerDiagnostic {
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );

    resolve_parsed_type_annotation(&parsed, None, &mut resolution_context, string_table, None)
        .expect_err(expected_failure)
}

#[test]
fn declaration_context_allows_inferred_annotations() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![token(TokenTag::ASSIGN), token(TokenTag::EOF)],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("declaration type annotation should parse");

    assert_eq!(parsed, ParsedTypeRef::Inferred);
}

#[test]
fn resolved_type_annotation_carries_canonical_type_id() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );

    let resolved = resolve_parsed_type_annotation(
        &ParsedTypeRef::BuiltinFixedScalar {
            scalar: FixedScalar::I32,
            span: None,
        },
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("builtin annotation resolution should succeed");

    assert_eq!(
        resolved.type_id,
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
}

#[test]
fn resolved_inferred_annotation_has_no_type_id() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );

    let resolved = resolve_parsed_type_annotation(
        &ParsedTypeRef::Inferred,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("inferred annotation resolution should succeed");

    assert_eq!(resolved.type_id, None);
}

#[test]
fn declaration_context_parses_named_optional_type() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let point = string_table.intern("Point");
    let stream = stream_from_tokens(
        vec![
            symbol_token(point),
            token(TokenTag::QUESTION_MARK),
            token(TokenTag::ASSIGN),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("named optional declaration type annotation should parse");

    assert!(matches!(
        parsed,
        ParsedTypeRef::Optional {
            inner,
            span: Some(_),
        } if matches!(
            *inner,
            ParsedTypeRef::Named {
                name,
                span: Some(_),
            } if name == point
        )
    ));
}

#[test]
fn dec_family_spellings_parse_as_builtin_number_types() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();

    for (spelling, expected_scale) in [
        ("Dec", 0u16),
        ("Dec0", 0),
        ("Dec1", 1),
        ("Dec2", 2),
        ("Dec256", 256),
    ] {
        let name = string_table.intern(spelling);
        let stream = stream_from_tokens(
            vec![symbol_token(name), token(TokenTag::EOF)],
            &mut string_table,
        );

        let parsed = parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .unwrap_or_else(|failure| {
            panic!("{spelling:?} should parse as a builtin scaled type: {failure:?}")
        });

        let expected_scale =
            NumberScale::new(expected_scale).expect("test scale is within capacity");
        assert!(
            matches!(
                parsed,
                ParsedTypeRef::BuiltinNumber { scale, span: Some(_) } if scale == expected_scale
            ),
            "{spelling:?} should be the builtin scaled type at scale {expected_scale}, got {parsed:?}"
        );
    }
}

#[test]
fn builtin_scalar_spelling_mapper_covers_every_scalar_keyword() {
    // The mapper is the single spelling owner behind scalar-constructor diagnostics and
    // builtin-target header classification, so every scalar keyword must keep its authored
    // name here instead of falling through to a generic diagnostic.
    for (tag, spelling) in [
        (TokenTag::DATATYPE_BOOL, "Bool"),
        (TokenTag::DATATYPE_STRING, "String"),
        (TokenTag::DATATYPE_CHAR, "Char"),
        (TokenTag::DATATYPE_I8, "I8"),
        (TokenTag::DATATYPE_I16, "I16"),
        (TokenTag::DATATYPE_I32, "I32"),
        (TokenTag::DATATYPE_I64, "I64"),
        (TokenTag::DATATYPE_U8, "U8"),
        (TokenTag::DATATYPE_U16, "U16"),
        (TokenTag::DATATYPE_U32, "U32"),
        (TokenTag::DATATYPE_U64, "U64"),
        (TokenTag::DATATYPE_F16, "F16"),
        (TokenTag::DATATYPE_F32, "F32"),
        (TokenTag::DATATYPE_F64, "F64"),
        (TokenTag::DATATYPE_BYTE, "Byte"),
    ] {
        assert_eq!(
            builtin_scalar_type_name_for_tag(tag),
            Some(spelling),
            "{tag:?} should keep its authored spelling"
        );
    }
}

#[test]
fn invalid_and_retired_scaled_spellings_stay_named_types() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();

    // Invalid scale spellings (`Dec01`, `Dec257`) stay named so resolution reports the
    // familiar unknown-type diagnostic, and the retired `Number` family spellings are
    // ordinary named types now.
    for spelling in ["Dec01", "Dec257", "DecBox", "Number", "Number0", "Number2"] {
        let name = string_table.intern(spelling);
        let stream = stream_from_tokens(
            vec![symbol_token(name), token(TokenTag::EOF)],
            &mut string_table,
        );

        let parsed = parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .unwrap_or_else(|failure| panic!("{spelling:?} should parse as a named type: {failure:?}"));

        assert!(
            matches!(
                parsed,
                ParsedTypeRef::Named { name: parsed_name, span: Some(_) } if parsed_name == name
            ),
            "{spelling:?} must stay on the named-type path, got {parsed:?}"
        );
    }
}

#[test]
fn signature_parameter_rejects_none_type() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![token(TokenTag::DATATYPE_NONE), token(TokenTag::EOF)],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("none parameter type should fail"),
    );

    assert!(matches!(
        error.payload,
        DiagnosticPayload::InvalidTypeAnnotation {
            context: TypeAnnotationContext::SignatureParameter,
            reason: InvalidTypeAnnotationReason::NoneNotAllowed,
        }
    ));
}

#[test]
fn signature_parameter_rejects_reserved_trait_this_type() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![token(TokenTag::TRAIT_THIS), token(TokenTag::EOF)],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("reserved trait keyword type should fail"),
    );

    assert!(matches!(
        error.payload,
        DiagnosticPayload::InvalidTypeAnnotation {
            context: TypeAnnotationContext::SignatureParameter,
            reason: InvalidTypeAnnotationReason::ReservedTraitKeyword,
        }
    ));
}

#[test]
fn declaration_target_rejects_type_keyword_inside_type_annotation() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![token(TokenTag::TYPE), token(TokenTag::EOF)],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .expect_err("type keyword should be reserved"),
    );

    assert!(matches!(
        error.payload,
        DiagnosticPayload::InvalidTypeAnnotation {
            context: TypeAnnotationContext::DeclarationTarget,
            reason: InvalidTypeAnnotationReason::ExpectedTypeAnnotation { found },
        } if found == DiagnosticToken::from_static_tag(TokenTag::TYPE)
    ));
}

#[test]
fn signature_return_rejects_bare_of_keyword_with_structured_syntax_error() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![token(TokenTag::OF), token(TokenTag::EOF)],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureReturn,
            &mut string_table,
        )
        .expect_err("of keyword should fail in type position"),
    );

    assert!(matches!(
        error.payload,
        DiagnosticPayload::UnexpectedToken { found }
            if found == DiagnosticToken::from_static_tag(TokenTag::OF)
    ));
}

#[test]
fn parses_generic_type_application() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let box_name = string_table.intern("Box");
    let stream = stream_from_tokens(
        vec![
            symbol_token(box_name),
            token(TokenTag::OF),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("generic type application should parse");

    assert!(matches!(
        parsed,
        ParsedTypeRef::Applied {
            base,
            arguments,
            span: Some(_),
        } if matches!(
            *base,
            ParsedTypeRef::Named {
                name,
                span: Some(_),
            } if name == box_name
        ) && matches!(
            arguments.as_slice(),
            [ParsedTypeRef::BuiltinString { span: Some(_) }]
        )
    ));
}

#[test]
fn public_option_type_syntax_is_deferred() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let option_name = string_table.intern("Option");
    let stream = stream_from_tokens(
        vec![
            symbol_token(option_name),
            token(TokenTag::OF),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("public option syntax should parse before resolution rejects it");

    let error = resolve_type_annotation_error(
        parsed,
        &mut string_table,
        "public Option of T syntax should be rejected",
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidGenericInstantiation {
                    reason: InvalidGenericInstantiationReason::OptionTypeSyntaxNotSupported,
                    ..
                }
            )
        },
        "InvalidGenericInstantiation(OptionTypeSyntaxNotSupported)",
    );
}

#[test]
fn public_result_type_syntax_is_deferred() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let result_name = string_table.intern("Result");
    let stream = stream_from_tokens(
        vec![
            symbol_token(result_name),
            token(TokenTag::OF),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::COMMA),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("public result syntax should parse before resolution rejects it");

    let error = resolve_type_annotation_error(
        parsed,
        &mut string_table,
        "public Result of T, E syntax should be rejected",
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidGenericInstantiation {
                    reason: InvalidGenericInstantiationReason::ResultTypeSyntaxNotSupported,
                    ..
                }
            )
        },
        "InvalidGenericInstantiation(ResultTypeSyntaxNotSupported)",
    );
}

#[test]
fn parses_collection_of_generic_type_application() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let box_name = string_table.intern("Box");
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            symbol_token(box_name),
            token(TokenTag::OF),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::ASSIGN),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("collection element generic type application should parse");

    assert!(matches!(
        &parsed,
        ParsedTypeRef::Collection {
            element,
            span: Some(_),
            fixed_capacity: None,
        } if matches!(
            element.as_ref(),
            ParsedTypeRef::Applied {
                base,
                arguments,
                span: Some(_),
            } if matches!(
                base.as_ref(),
                ParsedTypeRef::Named {
                    name,
                    span: Some(_),
                } if *name == box_name
            ) && matches!(
                arguments.as_slice(),
                [ParsedTypeRef::BuiltinString { span: Some(_) }]
            )
        )
    ));
}

#[test]
fn rejects_nested_generic_type_application() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let box_name = string_table.intern("Box");
    let map_name = string_table.intern("Map");
    let stream = stream_from_tokens(
        vec![
            symbol_token(box_name),
            token(TokenTag::OF),
            symbol_token(map_name),
            token(TokenTag::OF),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::COMMA),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::ASSIGN),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .expect_err("nested generic type application should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidGenericApplication {
                    reason: GenericApplicationErrorReason::NestedApplication,
                }
            )
        },
        "InvalidGenericApplication(NestedApplication)",
    );
}

#[test]
fn duplicate_optional_marker_is_rejected() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::QUESTION_MARK),
            token(TokenTag::QUESTION_MARK),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureReturn,
            &mut string_table,
        )
        .expect_err("duplicate optional marker should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidTypeAnnotation {
                    reason: InvalidTypeAnnotationReason::DuplicateOptional,
                    ..
                }
            )
        },
        "InvalidTypeAnnotation(DuplicateOptional)",
    );
}

#[test]
fn alias_expanded_nested_optional_type_is_rejected() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let maybe_name = string_table.intern("MaybeString");
    let maybe_path = path_fork
        .try_intern_portable_path("MaybeString", &mut string_table)
        .expect("test path fits");

    let unresolved = ParsedTypeRef::Optional {
        inner: Box::new(ParsedTypeRef::Named {
            name: maybe_name,
            span: None,
        }),
        span: None,
    };
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let mut visible_type_aliases = FxHashMap::default();
    visible_type_aliases.insert(
        maybe_name,
        crate::compiler_frontend::headers::binding_environment::SourceDeclarationTarget::Local(
            maybe_path.to_owned(),
        ),
    );

    let mut resolved_type_aliases = FxHashMap::default();
    resolved_type_aliases.insert(
        maybe_path,
        ResolvedTypeAlias {
            target_type_id: type_environment.intern_option(type_environment.builtins().string),
            declaration_span: None,
        },
    );

    let mut resolution_context = TypeResolutionContext {
        declaration_table: &declaration_table,
        const_values: &const_values,
        visible_declaration_ids: None,
        visible_external_symbols: None,
        visible_source_bindings: None,
        visible_type_aliases: Some(&visible_type_aliases),
        resolved_type_aliases: Some(&resolved_type_aliases),
        generic_declarations_by_path: None,
        generic_parameters: None,
        generic_substitutions: None,
        resolved_struct_fields_by_path: None,
        type_environment: &mut type_environment,
        visible_namespace_records: None,
        trait_environment: None,
        trait_evidence_environment: None,
        visible_trait_names: None,
    };

    let error = resolve_parsed_type_annotation(
        &unresolved,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect_err("alias-expanded nested option should fail");

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidTypeAnnotation {
                    reason: InvalidTypeAnnotationReason::NestedOptional,
                    ..
                }
            )
        },
        "InvalidTypeAnnotation(NestedOptional)",
    );
}

#[test]
fn resolves_named_types_recursively_in_composite_types() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let point_name = string_table.intern("Point");
    let point_path = path_fork
        .try_intern_portable_path("Point", &mut string_table)
        .expect("test path fits");

    let mut type_environment = TypeEnvironment::new();
    let (_, point_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: point_path.to_owned(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let declarations = vec![Declaration {
        id: point_path,
        type_id: point_type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Shell(DeclarationShell::Struct),
        binding_span: None,
        config_qualifier: None,
    }];
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations, &path_fork));
    let const_values = ConstValueStore::default();
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );
    let unresolved = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::Optional {
            inner: Box::new(ParsedTypeRef::Named {
                name: point_name,
                span: None,
            }),
            span: None,
        }),
        span: None,
        fixed_capacity: None,
    };

    let resolved = resolve_parsed_type_annotation(
        &unresolved,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("named type resolution should succeed");
    let collection_type_id = resolved.type_id.expect("collection should have a TypeId");
    let option_type_id = resolution_context
        .type_environment
        .collection_element_type(collection_type_id)
        .expect("collection should retain its option element type");

    assert_eq!(
        resolution_context
            .type_environment
            .option_inner_type(option_type_id),
        Some(point_type_id)
    );
}

#[test]
fn resolves_generic_instance_base_to_canonical_nominal_path() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let box_name = string_table.intern("Box");
    let t_name = string_table.intern("T");
    let box_path = path_fork
        .try_intern_portable_path("Box", &mut string_table)
        .expect("test path fits");
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let box_type_id = register_single_parameter_struct(&mut type_environment, &box_path, t_name);
    let declarations = vec![Declaration {
        id: box_path.to_owned(),
        type_id: box_type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Shell(DeclarationShell::Struct),
        binding_span: None,
        config_qualifier: None,
    }];
    let mut generic_declarations = FxHashMap::default();
    generic_declarations.insert(box_path.to_owned(), GenericDeclarationKind::Struct);

    let declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations, &path_fork));
    let mut resolution_context = TypeResolutionContext {
        declaration_table: &declaration_table,
        const_values: &const_values,
        visible_declaration_ids: None,
        visible_external_symbols: None,
        visible_source_bindings: None,
        visible_type_aliases: None,
        resolved_type_aliases: None,
        generic_declarations_by_path: Some(&generic_declarations),
        generic_parameters: None,
        generic_substitutions: None,
        resolved_struct_fields_by_path: None,
        type_environment: &mut type_environment,
        visible_namespace_records: None,
        trait_environment: None,
        trait_evidence_environment: None,
        visible_trait_names: None,
    };
    let unresolved = ParsedTypeRef::Applied {
        base: Box::new(ParsedTypeRef::Named {
            name: box_name,
            span: None,
        }),
        arguments: vec![ParsedTypeRef::BuiltinString { span: None }],
        span: None,
    };

    let resolved = resolve_parsed_type_annotation(
        &unresolved,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("generic base should resolve");
    let instance_type_id = resolved
        .type_id
        .expect("generic instance should have a TypeId");
    let expected_base = resolution_context
        .type_environment
        .nominal_id_for_path(&box_path)
        .expect("Box should retain its nominal identity");
    let Some(TypeDefinition::GenericInstance(instance)) =
        resolution_context.type_environment.get(instance_type_id)
    else {
        panic!("the resolved generic type should be a registered instance");
    };

    assert_eq!(instance.base, expected_base);
    assert_eq!(
        resolution_context
            .type_environment
            .nominal_path_by_id(instance.base),
        Some(&box_path)
    );
    assert_eq!(instance.arguments.as_ref(), &[builtin_type_ids::STRING]);
}

#[test]
fn generic_instance_resolution_rejects_wrong_arity() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let box_name = string_table.intern("Box");
    let t_name = string_table.intern("T");
    let unresolved = ParsedTypeRef::Applied {
        base: Box::new(ParsedTypeRef::Named {
            name: box_name,
            span: None,
        }),
        arguments: vec![
            ParsedTypeRef::BuiltinString { span: None },
            ParsedTypeRef::BuiltinFixedScalar {
                scalar: FixedScalar::I32,
                span: None,
            },
        ],
        span: None,
    };

    let box_path = path_fork
        .try_intern_portable_path("Box", &mut string_table)
        .expect("test path fits");
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let box_type_id = register_single_parameter_struct(&mut type_environment, &box_path, t_name);
    let declarations = vec![Declaration {
        id: box_path.to_owned(),
        type_id: box_type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Shell(DeclarationShell::Struct),
        binding_span: None,
        config_qualifier: None,
    }];
    let mut generic_declarations = FxHashMap::default();
    generic_declarations.insert(box_path, GenericDeclarationKind::Struct);

    let declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations, &path_fork));
    let mut resolution_context = TypeResolutionContext {
        declaration_table: &declaration_table,
        const_values: &const_values,
        visible_declaration_ids: None,
        visible_external_symbols: None,
        visible_source_bindings: None,
        visible_type_aliases: None,
        resolved_type_aliases: None,
        generic_declarations_by_path: Some(&generic_declarations),
        generic_parameters: None,
        generic_substitutions: None,
        resolved_struct_fields_by_path: None,
        type_environment: &mut type_environment,
        visible_namespace_records: None,
        trait_environment: None,
        trait_evidence_environment: None,
        visible_trait_names: None,
    };

    let error = resolve_parsed_type_annotation(
        &unresolved,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect_err("wrong generic arity should fail");

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidGenericInstantiation {
                    type_name,
                    reason: InvalidGenericInstantiationReason::WrongArgumentCount {
                        expected: 1,
                        found: 2,
                    },
                } if *type_name == Some(box_name)
            )
        },
        "InvalidGenericInstantiation(WrongArgumentCount)",
    );
}

#[test]
fn bare_generic_type_name_requires_type_arguments() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let box_name = string_table.intern("Box");
    let t_name = string_table.intern("T");
    let unresolved = ParsedTypeRef::Named {
        name: box_name,
        span: None,
    };

    let box_path = path_fork
        .try_intern_portable_path("Box", &mut string_table)
        .expect("test path fits");
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let box_type_id = register_single_parameter_struct(&mut type_environment, &box_path, t_name);
    let declarations = vec![Declaration {
        id: box_path.to_owned(),
        type_id: box_type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Shell(DeclarationShell::Struct),
        binding_span: None,
        config_qualifier: None,
    }];
    let mut generic_declarations = FxHashMap::default();
    generic_declarations.insert(box_path, GenericDeclarationKind::Struct);

    let declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations, &path_fork));
    let mut resolution_context = TypeResolutionContext {
        declaration_table: &declaration_table,
        const_values: &const_values,
        visible_declaration_ids: None,
        visible_external_symbols: None,
        visible_source_bindings: None,
        visible_type_aliases: None,
        resolved_type_aliases: None,
        generic_declarations_by_path: Some(&generic_declarations),
        generic_parameters: None,
        generic_substitutions: None,
        resolved_struct_fields_by_path: None,
        type_environment: &mut type_environment,
        visible_namespace_records: None,
        trait_environment: None,
        trait_evidence_environment: None,
        visible_trait_names: None,
    };

    let error = resolve_parsed_type_annotation(
        &unresolved,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect_err("bare generic type name should fail");

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidGenericInstantiation {
                    type_name,
                    reason: InvalidGenericInstantiationReason::MissingTypeArguments,
                } if *type_name == Some(box_name)
            )
        },
        "InvalidGenericInstantiation(MissingTypeArguments)",
    );
}

#[test]
fn unknown_named_type_reports_consistent_error() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let missing = string_table.intern("Missing");

    let unresolved = ParsedTypeRef::Named {
        name: missing,
        span: None,
    };
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        vec![],
        &PathInternerFork::empty(),
    ));
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );

    let error = resolve_parsed_type_annotation(
        &unresolved,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect_err("unknown type should fail");

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::UnknownName {
                    name,
                    namespace: NameNamespace::Type,
                } if *name == missing
            )
        },
        "UnknownName(type)",
    );
}

#[test]
fn unresolved_generic_argument_does_not_intern_instance() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let box_name = string_table.intern("Box");
    let first_parameter = string_table.intern("T");
    let second_parameter = string_table.intern("U");
    let box_path = path_fork
        .try_intern_portable_path("Box", &mut string_table)
        .expect("test path fits");
    let mut type_environment = TypeEnvironment::new();
    let generic_parameters = type_environment.register_generic_parameter_list(
        [
            (TypeParameterId(0), first_parameter),
            (TypeParameterId(1), second_parameter),
        ]
        .into_iter(),
        &FxHashMap::default(),
    );
    let (_, box_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: box_path.to_owned(),
        fields: Box::new([]),
        generic_parameters: Some(generic_parameters.list_id),
        const_record: false,
    });
    let declarations = vec![Declaration {
        id: box_path.to_owned(),
        type_id: box_type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Shell(DeclarationShell::Struct),
        binding_span: None,
        config_qualifier: None,
    }];
    let mut generic_declarations = FxHashMap::default();
    generic_declarations.insert(box_path, GenericDeclarationKind::Struct);
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations, &path_fork));
    let const_values = ConstValueStore::default();
    let mut resolution_context = TypeResolutionContext {
        declaration_table: &declaration_table,
        const_values: &const_values,
        visible_declaration_ids: None,
        visible_external_symbols: None,
        visible_source_bindings: None,
        visible_type_aliases: None,
        resolved_type_aliases: None,
        generic_declarations_by_path: Some(&generic_declarations),
        generic_parameters: None,
        generic_substitutions: None,
        resolved_struct_fields_by_path: None,
        type_environment: &mut type_environment,
        visible_namespace_records: None,
        trait_environment: None,
        trait_evidence_environment: None,
        visible_trait_names: None,
    };
    let missing_type = string_table.intern("Missing");
    let unresolved = ParsedTypeRef::Applied {
        base: Box::new(ParsedTypeRef::Named {
            name: box_name,
            span: None,
        }),
        arguments: vec![
            ParsedTypeRef::BuiltinString { span: None },
            ParsedTypeRef::Named {
                name: missing_type,
                span: None,
            },
        ],
        span: None,
    };

    let error = resolve_parsed_type_annotation(
        &unresolved,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect_err("an unresolved generic argument must fail");
    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::UnknownName {
                    name,
                    namespace: NameNamespace::Type,
                } if *name == missing_type
            )
        },
        "UnknownName(type)",
    );
    let interned_generic_instances = (0..128)
        .filter(|id| {
            matches!(
                resolution_context.type_environment.get(TypeId(*id)),
                Some(TypeDefinition::GenericInstance(_))
            )
        })
        .count();
    assert_eq!(
        interned_generic_instances, 0,
        "unresolved generic arguments must not intern a truncated instance"
    );
}

#[test]
fn parses_collection_with_capacity() {
    // New pre-element syntax: {64 I32}
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            numeric_token("64", &mut string_table),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("collection with capacity should parse");

    assert!(matches!(
        &parsed,
        ParsedTypeRef::Collection {
            fixed_capacity: Some(ParsedCollectionCapacity::Literal { value: 64, .. }),
            ..
        }
    ));
    assert!(matches!(
        &parsed,
        ParsedTypeRef::Collection { element, .. }
            if matches!(
                **element,
                ParsedTypeRef::BuiltinFixedScalar {
                    scalar: FixedScalar::I32,
                    span: Some(_),
                }
            )
    ));
}

#[test]
fn parses_collection_without_capacity() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("collection without capacity should parse");

    assert!(matches!(
        parsed,
        ParsedTypeRef::Collection {
            element,
            span: Some(_),
            fixed_capacity: None,
        } if matches!(
            *element,
            ParsedTypeRef::BuiltinFixedScalar {
                scalar: FixedScalar::I32,
                span: Some(_),
            }
        )
    ));
}

#[test]
fn rejects_old_post_element_collection_capacity_syntax() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_I32),
            numeric_token("64", &mut string_table),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .expect_err("old post-element capacity syntax should not parse"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::ExpectedToken {
                    expected,
                    found: Some(found),
                } if *expected == DiagnosticToken::from_static_tag(TokenTag::CLOSE_CURLY)
                    && found.tag() == TokenTag::NUMERIC_LITERAL
            )
        },
        "ExpectedToken(CloseCurly, NumericLiteral(64))",
    );
}

#[test]
fn parses_collection_with_generic_element_and_capacity() {
    // New pre-element syntax: {16 Box of String}
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let box_name = string_table.intern("Box");
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            numeric_token("16", &mut string_table),
            symbol_token(box_name),
            token(TokenTag::OF),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("collection with generic element and capacity should parse");

    assert!(matches!(
        &parsed,
        ParsedTypeRef::Collection {
            fixed_capacity: Some(ParsedCollectionCapacity::Literal { value: 16, .. }),
            ..
        }
    ));
    assert!(matches!(&parsed, ParsedTypeRef::Collection { element, .. }
        if matches!(**element, ParsedTypeRef::Applied { .. })
    ));
}

#[test]
fn rejects_collection_capacity_arithmetic_before_optional_element() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let capacity_name = string_table.intern("capacity");
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            symbol_token(capacity_name),
            token(TokenTag::ADD),
            numeric_token("16", &mut string_table),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::QUESTION_MARK),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .expect_err("arithmetic in capacity position should be rejected"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidCollectionType {
                    reason: InvalidCollectionTypeReason::CapacityNotConstant,
                    ..
                }
            )
        },
        "InvalidCollectionType(CapacityNotConstant)",
    );
}

#[test]
fn parses_nested_fixed_collection_bare_capacity_constants() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let rows_name = string_table.intern("rows");
    let cols_name = string_table.intern("cols");
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            symbol_token(rows_name),
            token(TokenTag::OPEN_CURLY),
            symbol_token(cols_name),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("nested fixed collection should parse");

    let ParsedTypeRef::Collection {
        fixed_capacity: Some(rows_capacity),
        element,
        ..
    } = parsed
    else {
        panic!("expected outer fixed collection");
    };
    assert!(
        matches!(rows_capacity, ParsedCollectionCapacity::BareConstant { name, .. } if name == rows_name)
    );

    let ParsedTypeRef::Collection {
        fixed_capacity: Some(cols_capacity),
        element: inner_element,
        ..
    } = *element
    else {
        panic!("expected inner fixed collection element");
    };
    assert!(
        matches!(cols_capacity, ParsedCollectionCapacity::BareConstant { name, .. } if name == cols_name)
    );
    assert!(matches!(
        *inner_element,
        ParsedTypeRef::BuiltinFixedScalar {
            scalar: FixedScalar::I32,
            span: Some(_),
        }
    ));
}

#[test]
fn parses_namespaced_type_using_member_name_case() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let namespace_name = string_table.intern("canvas");
    let type_name = string_table.intern("Canvas2d");
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            symbol_token(namespace_name),
            token(TokenTag::DOT),
            symbol_token(type_name),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("namespaced collection element should parse");

    assert!(matches!(parsed, ParsedTypeRef::Collection {
        fixed_capacity: None,
        element,
        ..
    } if matches!(element.as_ref(), ParsedTypeRef::Qualified { path, .. }
        if path == &vec![namespace_name, type_name]
    )));
}

#[test]
fn rejects_capacity_only_shorthand_in_signature_context() {
    // Capacity-only shorthand {64} is not allowed in signature/alias/field/return contexts.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            numeric_token("64", &mut string_table),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("capacity-only shorthand should be rejected in signature context"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidCollectionType {
                    reason: InvalidCollectionTypeReason::ShorthandCapacityNotAllowed,
                }
            )
        },
        "InvalidCollectionType(ShorthandCapacityNotAllowed)",
    );
}

#[test]
fn rejects_lower_snake_capacity_only_shorthand_in_signature_context() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let capacity_name = string_table.intern("capacity");
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            symbol_token(capacity_name),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("lower-snake capacity-only shorthand should be rejected in signatures"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidCollectionType {
                    reason: InvalidCollectionTypeReason::ShorthandCapacityNotAllowed,
                }
            )
        },
        "InvalidCollectionType(ShorthandCapacityNotAllowed)",
    );
}

#[test]
fn parses_capacity_only_shorthand_in_declaration_target() {
    // Capacity-only shorthand {64} in declaration target — element type is inferred.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            numeric_token("64", &mut string_table),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("capacity-only shorthand should parse in declaration target");

    assert!(matches!(
        &parsed,
        ParsedTypeRef::Collection {
            fixed_capacity: Some(ParsedCollectionCapacity::Literal { value: 64, .. }),
            ..
        }
    ));
    assert!(matches!(&parsed, ParsedTypeRef::Collection { element, .. }
        if **element == ParsedTypeRef::Inferred
    ));
}

#[test]
fn rejects_collection_type_missing_close_curly_with_expected_token() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .expect_err("missing collection close delimiter should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::ExpectedToken {
                    expected,
                    found: Some(found),
                } if *expected == DiagnosticToken::from_static_tag(TokenTag::CLOSE_CURLY)
                    && *found == DiagnosticToken::from_static_tag(TokenTag::EOF)
            )
        },
        "ExpectedToken(CloseCurly)",
    );
}

// -------------------------------------------------------------
//  Map type syntax parsing tests
// -------------------------------------------------------------

#[test]
fn parses_simple_map_type() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("simple map type should parse");

    assert!(matches!(parsed, ParsedTypeRef::Map {
        key,
        value,
        ..
    } if matches!(*key, ParsedTypeRef::BuiltinString { .. })
        && matches!(*value, ParsedTypeRef::BuiltinFixedScalar { .. })
    ));
}

#[test]
fn parses_map_type_with_collection_value() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::SignatureParameter,
        &mut string_table,
    )
    .expect("map with collection value should parse");

    assert!(matches!(parsed, ParsedTypeRef::Map {
        ref key,
        ref value,
        ..
    } if matches!(**key, ParsedTypeRef::BuiltinString { .. })
        && matches!(**value, ParsedTypeRef::Collection { ref element, .. }
            if matches!(**element, ParsedTypeRef::BuiltinFixedScalar { .. })
        )
    ));
}

#[test]
fn parses_map_type_with_nested_map_value() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::SignatureReturn,
        &mut string_table,
    )
    .expect("map with nested map value should parse");

    assert!(matches!(parsed, ParsedTypeRef::Map {
        key,
        value,
        ..
    } if matches!(*key, ParsedTypeRef::BuiltinString { .. })
        && matches!(*value, ParsedTypeRef::Map { .. })
    ));
}

#[test]
fn parses_map_type_in_parameter_context() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_BOOL),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::SignatureParameter,
        &mut string_table,
    )
    .expect("map type in parameter context should parse");

    assert!(matches!(parsed, ParsedTypeRef::Map { .. }));
}

#[test]
fn rejects_map_type_with_empty_key() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .expect_err("empty map key should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidMapType {
                    reason: InvalidMapTypeReason::EmptyMapKeyType,
                }
            )
        },
        "InvalidMapType(EmptyMapKeyType)",
    );
}

#[test]
fn rejects_map_type_with_empty_value() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .expect_err("empty map value should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidMapType {
                    reason: InvalidMapTypeReason::EmptyMapValueType,
                }
            )
        },
        "InvalidMapType(EmptyMapValueType)",
    );
}

#[test]
fn rejects_map_type_with_multiple_separators() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_BOOL),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::DeclarationTarget,
            &mut string_table,
        )
        .expect_err("multiple map separators should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidMapType {
                    reason: InvalidMapTypeReason::MultipleMapSeparators,
                }
            )
        },
        "InvalidMapType(MultipleMapSeparators)",
    );
}

#[test]
fn rejects_fixed_capacity_map_syntax_on_key_side() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            numeric_token("4", &mut string_table),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("fixed capacity on key side should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidMapType {
                    reason: InvalidMapTypeReason::FixedCapacityNotAllowed,
                }
            )
        },
        "InvalidMapType(FixedCapacityNotAllowed)",
    );
}

#[test]
fn rejects_fixed_capacity_map_syntax_on_value_side() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            numeric_token("4", &mut string_table),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("fixed capacity on value side should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidMapType {
                    reason: InvalidMapTypeReason::FixedCapacityNotAllowed,
                }
            )
        },
        "InvalidMapType(FixedCapacityNotAllowed)",
    );
}

#[test]
fn rejects_named_capacity_map_syntax() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let capacity_name = string_table.intern("capacity");
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            symbol_token(capacity_name),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("named capacity on map key side should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidMapType {
                    reason: InvalidMapTypeReason::FixedCapacityNotAllowed,
                }
            )
        },
        "InvalidMapType(FixedCapacityNotAllowed)",
    );
}

#[test]
fn rejects_postfix_capacity_map_syntax_with_colon() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::COLON),
            numeric_token("5", &mut string_table),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("postfix capacity with colon on map value side should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidMapType {
                    reason: InvalidMapTypeReason::FixedCapacityNotAllowed,
                }
            )
        },
        "InvalidMapType(FixedCapacityNotAllowed)",
    );
}

#[test]
fn rejects_postfix_capacity_map_syntax_with_number() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            numeric_token("5", &mut string_table),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("postfix capacity with number on map value side should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidMapType {
                    reason: InvalidMapTypeReason::FixedCapacityNotAllowed,
                }
            )
        },
        "InvalidMapType(FixedCapacityNotAllowed)",
    );
}

#[test]
fn rejects_postfix_capacity_map_syntax_on_key_side() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::COLON),
            numeric_token("5", &mut string_table),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("postfix capacity with colon on map key side should fail"),
    );

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::InvalidMapType {
                    reason: InvalidMapTypeReason::FixedCapacityNotAllowed,
                }
            )
        },
        "InvalidMapType(FixedCapacityNotAllowed)",
    );
}

#[test]
fn map_separator_inside_nested_braces_is_ignored() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let stream = stream_from_tokens(
        vec![
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::OPEN_CURLY),
            token(TokenTag::DATATYPE_STRING),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_I32),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::ASSIGN),
            token(TokenTag::DATATYPE_BOOL),
            token(TokenTag::CLOSE_CURLY),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::DeclarationTarget,
        &mut string_table,
    )
    .expect("map with inner map key should parse");

    assert!(matches!(parsed, ParsedTypeRef::Map {
        key,
        value,
        ..
    } if matches!(*key, ParsedTypeRef::Map { .. })
        && matches!(*value, ParsedTypeRef::BuiltinBool { .. })
    ));
}

#[test]
fn map_type_walker_visits_named_types_in_key_and_value() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let key_name = string_table.intern("MyKey");
    let value_name = string_table.intern("MyValue");

    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::Named {
            name: key_name,
            span: None,
        }),
        value: Box::new(ParsedTypeRef::Named {
            name: value_name,
            span: None,
        }),
        span: None,
    };

    let mut found = Vec::new();
    crate::compiler_frontend::declaration_syntax::type_syntax::for_each_named_type_in_parsed_ref(
        &parsed,
        &mut |reference| found.push(reference),
    );

    assert_eq!(
        found,
        vec![
            ParsedNamedTypeReference::Bare(key_name),
            ParsedNamedTypeReference::Bare(value_name),
        ]
    );
}

#[test]
fn named_type_walker_preserves_qualified_path() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let root = string_table.intern("models");
    let member = string_table.intern("Names");
    let parsed = ParsedTypeRef::Qualified {
        path: vec![root, member],
        span: None,
    };

    let mut found = Vec::new();
    crate::compiler_frontend::declaration_syntax::type_syntax::for_each_named_type_in_parsed_ref(
        &parsed,
        &mut |reference| found.push(reference),
    );

    assert_eq!(
        found,
        vec![ParsedNamedTypeReference::Qualified(&[root, member])]
    );
}

#[test]
fn parses_qualified_type_with_three_segments() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let root_name = string_table.intern("io");
    let child_name = string_table.intern("input");
    let type_name = string_table.intern("Input");
    let stream = stream_from_tokens(
        vec![
            symbol_token(root_name),
            token(TokenTag::DOT),
            symbol_token(child_name),
            token(TokenTag::DOT),
            symbol_token(type_name),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation_test(
        &stream,
        TypeAnnotationContext::SignatureParameter,
        &mut string_table,
    )
    .expect("qualified type should parse");

    assert!(matches!(parsed, ParsedTypeRef::Qualified { path, .. }
        if path == vec![root_name, child_name, type_name]
    ));
}

#[test]
fn rejects_generic_application_on_qualified_base() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let root_name = string_table.intern("io");
    let child_name = string_table.intern("input");
    let type_name = string_table.intern("Input");
    let stream = stream_from_tokens(
        vec![
            symbol_token(root_name),
            token(TokenTag::DOT),
            symbol_token(child_name),
            token(TokenTag::DOT),
            symbol_token(type_name),
            token(TokenTag::OF),
            symbol_token(string_table.intern("T")),
            token(TokenTag::EOF),
        ],
        &mut string_table,
    );

    let error = unwrap_type_parse_diagnostic(
        parse_type_annotation_test(
            &stream,
            TypeAnnotationContext::SignatureParameter,
            &mut string_table,
        )
        .expect_err("generic application on qualified base should fail"),
    );

    assert!(
        matches!(
            error.payload,
            DiagnosticPayload::InvalidGenericApplication {
                reason: GenericApplicationErrorReason::OnNonNamedType,
                ..
            }
        ),
        "expected OnNonNamedType error, got {:?}",
        error.payload
    );
}
