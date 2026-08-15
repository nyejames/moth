//! Type-syntax parsing and resolution regression tests.
//!
//! WHAT: validates type annotation parsing and type resolution in composite types.
//! WHY: type syntax is the source of truth for frontend type identity; parser drift here
//!      affects every downstream type check.

use crate::compiler_frontend::ast::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::Expression;

use crate::compiler_frontend::ast::type_resolution::{
    ResolvedTypeAnnotation, TypeResolutionContext, resolve_diagnostic_type_to_type_id,
    resolve_diagnostic_type_to_type_id_checked, resolve_diagnostic_type_to_type_id_opt,
    resolve_parsed_type_annotation, resolve_type,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, GenericApplicationErrorReason,
    InvalidCollectionTypeReason, InvalidGenericInstantiationReason, InvalidMapTypeReason,
    InvalidTypeAnnotationReason, NameNamespace,
};
use crate::compiler_frontend::datatypes::definitions::{StructTypeDefinition, TypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_identity_bridge::GenericBaseType;
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::datatypes::{DataType, TypeId, builtin_type_ids};
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    TypeAnnotationContext, parse_type_annotation,
};
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationKind, GenericDeclarationMetadata,
};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};

fn numeric_token(value: &str, string_table: &mut StringTable) -> Token {
    Token::new(
        TokenKind::NumericLiteral(NumericLiteralToken::test_new(value, string_table)),
        SourceLocation::default(),
    )
}

use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;
use std::rc::Rc;

fn stream_from_tokens(tokens: Vec<Token>, string_table: &mut StringTable) -> FileTokens {
    FileTokens::new(
        InternedPath::from_single_str("type_syntax_tests", string_table),
        tokens,
    )
}

fn token(kind: TokenKind) -> Token {
    Token::new(kind, SourceLocation::default())
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

fn single_parameter_metadata(
    parameter_name: crate::compiler_frontend::symbols::string_interning::StringId,
) -> GenericDeclarationMetadata {
    GenericDeclarationMetadata {
        kind: GenericDeclarationKind::Struct,
        parameters: GenericParameterList {
            parameters: vec![GenericParameter {
                id: TypeParameterId(0),
                name: parameter_name,
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            }],
        },
        declaration_location: SourceLocation::default(),
    }
}

fn resolve_type_annotation_error(
    parsed: ParsedTypeRef,
    string_table: &mut StringTable,
    expected_failure: &str,
) -> CompilerDiagnostic {
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut type_environment = TypeEnvironment::new();
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    resolve_parsed_type_annotation(
        parsed,
        &SourceLocation::default(),
        &mut resolution_context,
        string_table,
        None,
    )
    .expect_err(expected_failure)
    .as_ref()
    .to_owned()
}

#[test]
fn declaration_context_allows_inferred_annotations() {
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![token(TokenKind::Assign), token(TokenKind::Eof)],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect("declaration type annotation should parse");

    assert_eq!(parsed, ParsedTypeRef::Inferred);
}

#[test]
fn resolved_type_annotation_carries_canonical_type_id() {
    let mut string_table = StringTable::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut type_environment = TypeEnvironment::new();
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let location = SourceLocation::default();
    let resolved = resolve_parsed_type_annotation(
        ParsedTypeRef::BuiltinInt {
            location: location.to_owned(),
        },
        &location,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("builtin annotation resolution should succeed");

    assert_eq!(resolved.diagnostic_type, DataType::Int);
    assert_eq!(resolved.type_id, Some(builtin_type_ids::INT));
}

#[test]
fn resolved_inferred_annotation_has_no_type_id() {
    let mut string_table = StringTable::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut type_environment = TypeEnvironment::new();
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let resolved = resolve_parsed_type_annotation(
        ParsedTypeRef::Inferred,
        &SourceLocation::default(),
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("inferred annotation resolution should succeed");

    assert_eq!(resolved.diagnostic_type, DataType::Inferred);
    assert_eq!(resolved.type_id, None);
}

#[test]
fn declaration_context_parses_named_optional_type() {
    let mut string_table = StringTable::new();
    let point = string_table.intern("Point");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::Symbol(point)),
            token(TokenKind::QuestionMark),
            token(TokenKind::Assign),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect("named optional declaration type annotation should parse");

    assert_eq!(
        parsed,
        ParsedTypeRef::Optional {
            inner: Box::new(ParsedTypeRef::Named {
                name: point,
                location: SourceLocation::default()
            }),
            location: SourceLocation::default(),
        }
    );
}

#[test]
fn signature_parameter_rejects_none_type() {
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![token(TokenKind::DatatypeNone), token(TokenKind::Eof)],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("none parameter type should fail");

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
    let mut stream = stream_from_tokens(
        vec![token(TokenKind::TraitThis), token(TokenKind::Eof)],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("reserved trait keyword type should fail");

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
    let mut stream = stream_from_tokens(
        vec![token(TokenKind::Type), token(TokenKind::Eof)],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect_err("type keyword should be reserved");

    assert!(matches!(
        error.payload,
        DiagnosticPayload::InvalidTypeAnnotation {
            context: TypeAnnotationContext::DeclarationTarget,
            reason: InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
                found: TokenKind::Type,
            },
        }
    ));
}

#[test]
fn signature_return_rejects_bare_of_keyword_with_structured_syntax_error() {
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![token(TokenKind::Of), token(TokenKind::Eof)],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureReturn,
        &string_table,
    )
    .expect_err("of keyword should fail in type position");

    assert!(matches!(
        error.payload,
        DiagnosticPayload::UnexpectedToken {
            found: TokenKind::Of
        }
    ));
}

#[test]
fn parses_generic_type_application() {
    let mut string_table = StringTable::new();
    let box_name = string_table.intern("Box");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::Symbol(box_name)),
            token(TokenKind::Of),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect("generic type application should parse");

    assert_eq!(
        parsed,
        ParsedTypeRef::Applied {
            base: Box::new(ParsedTypeRef::Named {
                name: box_name,
                location: SourceLocation::default()
            }),
            arguments: vec![ParsedTypeRef::BuiltinString {
                location: SourceLocation::default()
            }],
            location: SourceLocation::default(),
        }
    );
}

#[test]
fn public_option_type_syntax_is_deferred() {
    let mut string_table = StringTable::new();
    let option_name = string_table.intern("Option");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::Symbol(option_name)),
            token(TokenKind::Of),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
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
    let result_name = string_table.intern("Result");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::Symbol(result_name)),
            token(TokenKind::Of),
            token(TokenKind::DatatypeInt),
            token(TokenKind::Comma),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
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
    let box_name = string_table.intern("Box");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::Symbol(box_name)),
            token(TokenKind::Of),
            token(TokenKind::DatatypeString),
            token(TokenKind::CloseCurly),
            token(TokenKind::Assign),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect("collection element generic type application should parse");

    assert_eq!(
        parsed,
        ParsedTypeRef::Collection {
            element: Box::new(ParsedTypeRef::Applied {
                base: Box::new(ParsedTypeRef::Named {
                    name: box_name,
                    location: SourceLocation::default()
                }),
                arguments: vec![ParsedTypeRef::BuiltinString {
                    location: SourceLocation::default()
                }],
                location: SourceLocation::default(),
            }),
            location: SourceLocation::default(),
            fixed_capacity: None,
        }
    );
}

#[test]
fn rejects_nested_generic_type_application() {
    let mut string_table = StringTable::new();
    let box_name = string_table.intern("Box");
    let map_name = string_table.intern("Map");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::Symbol(box_name)),
            token(TokenKind::Of),
            token(TokenKind::Symbol(map_name)),
            token(TokenKind::Of),
            token(TokenKind::DatatypeString),
            token(TokenKind::Comma),
            token(TokenKind::DatatypeInt),
            token(TokenKind::Assign),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect_err("nested generic type application should fail");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::DatatypeString),
            token(TokenKind::QuestionMark),
            token(TokenKind::QuestionMark),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureReturn,
        &string_table,
    )
    .expect_err("duplicate optional marker should fail");

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
    let maybe_name = string_table.intern("MaybeString");
    let maybe_path = InternedPath::from_single_str("MaybeString", &mut string_table);

    let unresolved = DataType::Option(Box::new(DataType::NamedType(maybe_name)));
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut type_environment = TypeEnvironment::new();
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
        ResolvedTypeAnnotation {
            source_ref: ParsedTypeRef::Inferred,
            diagnostic_type: DataType::Option(Box::new(DataType::StringSlice)),
            type_id: None,
        },
    );

    let mut resolution_context = TypeResolutionContext {
        declaration_table: &declaration_table,
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

    let error = resolve_type(
        &unresolved,
        &SourceLocation::default(),
        &mut resolution_context,
        &string_table,
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
    let point_name = string_table.intern("Point");

    let unresolved =
        DataType::collection(DataType::Option(Box::new(DataType::NamedType(point_name))));

    let point_path = InternedPath::from_single_str("Point", &mut string_table);
    let declarations = vec![Declaration {
        id: point_path,
        value: Expression::no_value(
            SourceLocation::default(),
            DataType::Int,
            ValueMode::ImmutableOwned,
        ),
    }];

    let declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations));
    let mut type_environment = TypeEnvironment::new();
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let location = SourceLocation::default();
    let resolved = resolve_type(
        &unresolved,
        &location,
        &mut resolution_context,
        &string_table,
    )
    .expect("named type resolution should succeed");

    assert_eq!(
        resolved,
        DataType::collection(DataType::Option(Box::new(DataType::Int)))
    );
}

#[test]
fn resolves_generic_instance_base_to_canonical_nominal_path() {
    let mut string_table = StringTable::new();
    let box_name = string_table.intern("Box");
    let t_name = string_table.intern("T");
    let unresolved = DataType::GenericInstance {
        base: GenericBaseType::Named(box_name),
        arguments: vec![DataType::StringSlice],
    };

    let box_path = InternedPath::from_single_str("Box", &mut string_table);
    let declarations = vec![Declaration {
        id: box_path.to_owned(),
        value: Expression::no_value(
            SourceLocation::default(),
            DataType::runtime_struct(box_path.to_owned(), builtin_type_ids::NONE),
            ValueMode::ImmutableOwned,
        ),
    }];
    let mut generic_declarations = FxHashMap::default();
    generic_declarations.insert(box_path.to_owned(), single_parameter_metadata(t_name));

    let declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations));
    let mut type_environment = TypeEnvironment::new();
    let mut resolution_context = TypeResolutionContext {
        declaration_table: &declaration_table,
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

    let location = SourceLocation::default();
    let resolved = resolve_type(
        &unresolved,
        &location,
        &mut resolution_context,
        &string_table,
    )
    .expect("generic base should resolve");

    assert_eq!(
        resolved,
        DataType::GenericInstance {
            base: GenericBaseType::ResolvedNominal(box_path),
            arguments: vec![DataType::StringSlice],
        }
    );
}

#[test]
fn generic_instance_resolution_rejects_wrong_arity() {
    let mut string_table = StringTable::new();
    let box_name = string_table.intern("Box");
    let t_name = string_table.intern("T");
    let unresolved = DataType::GenericInstance {
        base: GenericBaseType::Named(box_name),
        arguments: vec![DataType::StringSlice, DataType::Int],
    };

    let box_path = InternedPath::from_single_str("Box", &mut string_table);
    let declarations = vec![Declaration {
        id: box_path.to_owned(),
        value: Expression::no_value(
            SourceLocation::default(),
            DataType::runtime_struct(box_path.to_owned(), builtin_type_ids::NONE),
            ValueMode::ImmutableOwned,
        ),
    }];
    let mut generic_declarations = FxHashMap::default();
    generic_declarations.insert(box_path, single_parameter_metadata(t_name));

    let declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations));
    let mut type_environment = TypeEnvironment::new();
    let mut resolution_context = TypeResolutionContext {
        declaration_table: &declaration_table,
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

    let location = SourceLocation::default();
    let error = resolve_type(
        &unresolved,
        &location,
        &mut resolution_context,
        &string_table,
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
    let box_name = string_table.intern("Box");
    let t_name = string_table.intern("T");
    let unresolved = DataType::NamedType(box_name);

    let box_path = InternedPath::from_single_str("Box", &mut string_table);
    let declarations = vec![Declaration {
        id: box_path.to_owned(),
        value: Expression::no_value(
            SourceLocation::default(),
            DataType::runtime_struct(box_path.to_owned(), builtin_type_ids::NONE),
            ValueMode::ImmutableOwned,
        ),
    }];
    let mut generic_declarations = FxHashMap::default();
    generic_declarations.insert(box_path, single_parameter_metadata(t_name));

    let declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations));
    let mut type_environment = TypeEnvironment::new();
    let mut resolution_context = TypeResolutionContext {
        declaration_table: &declaration_table,
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

    let location = SourceLocation::default();
    let error = resolve_type(
        &unresolved,
        &location,
        &mut resolution_context,
        &string_table,
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
    let missing = string_table.intern("Missing");

    let unresolved = DataType::NamedType(missing);
    let location = SourceLocation::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(vec![]));
    let mut type_environment = TypeEnvironment::new();
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let error = resolve_type(
        &unresolved,
        &location,
        &mut resolution_context,
        &string_table,
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
fn returns_data_type_interns_multi_return_tuple_type_id() {
    let mut type_environment = TypeEnvironment::new();

    let returns_type_id = resolve_diagnostic_type_to_type_id(
        &DataType::Returns(vec![DataType::Int, DataType::Bool]),
        &mut type_environment,
    );

    assert_eq!(
        type_environment.tuple_field_ids(returns_type_id),
        Some(
            [
                type_environment.builtins().int,
                type_environment.builtins().bool
            ]
            .as_slice()
        )
    );
}

#[test]
fn optional_generic_instance_conversion_rejects_unresolved_arguments() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let box_path = InternedPath::from_single_str("Box", &mut string_table);
    type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: box_path.to_owned(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let missing_type = string_table.intern("Missing");
    let generic_instance = DataType::GenericInstance {
        base: GenericBaseType::ResolvedNominal(box_path),
        arguments: vec![DataType::StringSlice, DataType::NamedType(missing_type)],
    };

    let converted =
        resolve_diagnostic_type_to_type_id_opt(&generic_instance, &mut type_environment);

    assert_eq!(converted, None);
    let interned_generic_instances = (0..128)
        .filter(|id| {
            matches!(
                type_environment.get(TypeId(*id)),
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
fn checked_type_id_conversion_rejects_unresolved_named_type() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let missing = string_table.intern("Missing");
    let location = SourceLocation::default();

    let error = resolve_diagnostic_type_to_type_id_checked(
        &DataType::NamedType(missing),
        &mut type_environment,
        &location,
    )
    .expect_err("checked conversion should reject unresolved type names");

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
fn parses_collection_with_capacity() {
    // New pre-element syntax: {64 Int}
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            numeric_token("64", &mut string_table),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect("collection with capacity should parse");

    assert!(matches!(
        &parsed,
        ParsedTypeRef::Collection {
            fixed_capacity: Some(ParsedCollectionCapacity::Literal { value: 64, .. }),
            ..
        }
    ));
    assert!(matches!(&parsed, ParsedTypeRef::Collection { element, .. }
        if **element == ParsedTypeRef::BuiltinInt { location: SourceLocation::default() }
    ));
}

#[test]
fn parses_collection_without_capacity() {
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect("collection without capacity should parse");

    assert_eq!(
        parsed,
        ParsedTypeRef::Collection {
            element: Box::new(ParsedTypeRef::BuiltinInt {
                location: SourceLocation::default()
            }),
            location: SourceLocation::default(),
            fixed_capacity: None,
        }
    );
}

#[test]
fn rejects_old_post_element_collection_capacity_syntax() {
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeInt),
            numeric_token("64", &mut string_table),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect_err("old post-element capacity syntax should not parse");

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::ExpectedToken {
                    expected: TokenKind::CloseCurly,
                    found: Some(TokenKind::NumericLiteral(_)),
                }
            )
        },
        "ExpectedToken(CloseCurly, NumericLiteral(64))",
    );
}

#[test]
fn parses_collection_with_generic_element_and_capacity() {
    // New pre-element syntax: {16 Box of String}
    let mut string_table = StringTable::new();
    let box_name = string_table.intern("Box");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            numeric_token("16", &mut string_table),
            token(TokenKind::Symbol(box_name)),
            token(TokenKind::Of),
            token(TokenKind::DatatypeString),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
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
    let capacity_name = string_table.intern("capacity");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::Symbol(capacity_name)),
            token(TokenKind::Add),
            numeric_token("16", &mut string_table),
            token(TokenKind::DatatypeInt),
            token(TokenKind::QuestionMark),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect_err("arithmetic in capacity position should be rejected");

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
    let rows_name = string_table.intern("rows");
    let cols_name = string_table.intern("cols");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::Symbol(rows_name)),
            token(TokenKind::OpenCurly),
            token(TokenKind::Symbol(cols_name)),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
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
    assert_eq!(
        *inner_element,
        ParsedTypeRef::BuiltinInt {
            location: SourceLocation::default()
        }
    );
}

#[test]
fn parses_namespaced_type_using_member_name_case() {
    let mut string_table = StringTable::new();
    let namespace_name = string_table.intern("canvas");
    let type_name = string_table.intern("Canvas2d");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::Symbol(namespace_name)),
            token(TokenKind::Dot),
            token(TokenKind::Symbol(type_name)),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            numeric_token("64", &mut string_table),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("capacity-only shorthand should be rejected in signature context");

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
    let capacity_name = string_table.intern("capacity");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::Symbol(capacity_name)),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("lower-snake capacity-only shorthand should be rejected in signatures");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            numeric_token("64", &mut string_table),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeInt),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect_err("missing collection close delimiter should fail");

    assert_diagnostic_payload(
        error,
        |payload| {
            matches!(
                payload,
                DiagnosticPayload::ExpectedToken {
                    expected: TokenKind::CloseCurly,
                    found: Some(TokenKind::Eof),
                }
            )
        },
        "ExpectedToken(CloseCurly)",
    );
}

// -------------------------------------------------------------
//  Map type syntax parsing tests (Phase 3)
// -------------------------------------------------------------

#[test]
fn parses_simple_map_type() {
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect("simple map type should parse");

    assert!(matches!(parsed, ParsedTypeRef::Map {
        key,
        value,
        ..
    } if matches!(*key, ParsedTypeRef::BuiltinString { .. })
        && matches!(*value, ParsedTypeRef::BuiltinInt { .. })
    ));
}

#[test]
fn parses_map_type_with_collection_value() {
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect("map with collection value should parse");

    assert!(matches!(parsed, ParsedTypeRef::Map {
        ref key,
        ref value,
        ..
    } if matches!(**key, ParsedTypeRef::BuiltinString { .. })
        && matches!(**value, ParsedTypeRef::Collection { ref element, .. }
            if matches!(**element, ParsedTypeRef::BuiltinInt { .. })
        )
    ));
}

#[test]
fn parses_map_type_with_nested_map_value() {
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureReturn,
        &string_table,
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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeBool),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeString),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect("map type in parameter context should parse");

    assert!(matches!(parsed, ParsedTypeRef::Map { .. }));
}

#[test]
fn rejects_map_type_with_empty_key() {
    let mut string_table = StringTable::new();
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect_err("empty map key should fail");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect_err("empty map value should fail");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeBool),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
    )
    .expect_err("multiple map separators should fail");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            numeric_token("4", &mut string_table),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("fixed capacity on key side should fail");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            numeric_token("4", &mut string_table),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("fixed capacity on value side should fail");

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
    let capacity_name = string_table.intern("capacity");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::Symbol(capacity_name)),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("named capacity on map key side should fail");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            token(TokenKind::Colon),
            numeric_token("5", &mut string_table),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("postfix capacity with colon on map value side should fail");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            numeric_token("5", &mut string_table),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("postfix capacity with number on map value side should fail");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Colon),
            numeric_token("5", &mut string_table),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("postfix capacity with colon on map key side should fail");

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
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::OpenCurly),
            token(TokenKind::OpenCurly),
            token(TokenKind::DatatypeString),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeInt),
            token(TokenKind::CloseCurly),
            token(TokenKind::Assign),
            token(TokenKind::DatatypeBool),
            token(TokenKind::CloseCurly),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::DeclarationTarget,
        &string_table,
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
    let key_name = string_table.intern("MyKey");
    let value_name = string_table.intern("MyValue");

    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::Named {
            name: key_name,
            location: SourceLocation::default(),
        }),
        value: Box::new(ParsedTypeRef::Named {
            name: value_name,
            location: SourceLocation::default(),
        }),
        location: SourceLocation::default(),
    };

    let mut found = Vec::new();
    crate::compiler_frontend::declaration_syntax::type_syntax::for_each_named_type_in_parsed_ref(
        &parsed,
        &mut |name| found.push(name),
    );

    assert_eq!(found, vec![key_name, value_name]);
}

#[test]
fn parses_qualified_type_with_three_segments() {
    let mut string_table = StringTable::new();
    let root_name = string_table.intern("io");
    let child_name = string_table.intern("input");
    let type_name = string_table.intern("Input");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::Symbol(root_name)),
            token(TokenKind::Dot),
            token(TokenKind::Symbol(child_name)),
            token(TokenKind::Dot),
            token(TokenKind::Symbol(type_name)),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let parsed = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect("qualified type should parse");

    assert!(matches!(parsed, ParsedTypeRef::Qualified { path, .. }
        if path == vec![root_name, child_name, type_name]
    ));
}

#[test]
fn rejects_generic_application_on_qualified_base() {
    let mut string_table = StringTable::new();
    let root_name = string_table.intern("io");
    let child_name = string_table.intern("input");
    let type_name = string_table.intern("Input");
    let mut stream = stream_from_tokens(
        vec![
            token(TokenKind::Symbol(root_name)),
            token(TokenKind::Dot),
            token(TokenKind::Symbol(child_name)),
            token(TokenKind::Dot),
            token(TokenKind::Symbol(type_name)),
            token(TokenKind::Of),
            token(TokenKind::Symbol(string_table.intern("T"))),
            token(TokenKind::Eof),
        ],
        &mut string_table,
    );

    let error = parse_type_annotation(
        &mut stream,
        TypeAnnotationContext::SignatureParameter,
        &string_table,
    )
    .expect_err("generic application on qualified base should fail");

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
