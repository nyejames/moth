//! Struct parsing regression tests.
//!
//! WHAT: validates struct definitions, defaults, constructors, and field access.
//! WHY: struct parsing feeds both type resolution and HIR place lowering.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::{Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateIrStore, TemplateTirPhase, TemplateTirReference, TemplateViewContext,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticLabelMessage, DiagnosticLabelStyle, DiagnosticPayload, GenericInferenceSubject,
    InvalidFieldAccessReason, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::declaration_syntax::r#struct::validate_struct_default_values;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_build_result, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn body_local_struct_default_preserves_missing_template_authority() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let fields = [Declaration {
        id: PathId::ROOT,
        value: Expression::template(
            Template {
                tir_reference: TemplateTirReference {
                    root: TemplateIrId::new(99),
                    phase: TemplateTirPhase::Composed,
                    context: TemplateViewContext::default(),
                },
                span: None,
            },
            ValueMode::ImmutableOwned,
        ),
        binding_span: None,
        config_qualifier: None,
    }];

    let error = validate_struct_default_values(&fields, &store)
        .expect_err("missing struct-default TIR authority must fail");

    assert!(matches!(error, TemplateError::Infrastructure(_)));
}

#[test]
fn authored_runtime_struct_default_remains_a_source_diagnostic() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let fields = [Declaration {
        id: PathId::ROOT,
        value: Expression::reference_with_type_id(
            PathId::ROOT,
            DataType::Bool,
            builtin_type_ids::BOOL,
            Option::<SourceSpan>::default(),
            ValueMode::ImmutableReference,
            ConstRecordState::RuntimeValue,
        ),
        binding_span: None,
        config_qualifier: None,
    }];

    let error = validate_struct_default_values(&fields, &store)
        .expect_err("runtime struct default must be rejected");
    let TemplateError::Diagnostic(diagnostic) = error else {
        panic!("authored runtime default should remain a source diagnostic");
    };

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStructDefaultValue
    ));
}

#[test]
fn parses_struct_definitions_with_field_defaults() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("Point = |\n    x Int,\n    y Int = 2,\n|\n");

    let struct_node = ast
        .nodes
        .iter()
        .find(|node| {
            matches!(
                &node.kind,
                NodeKind::StructDefinition(path, ..)
                    if path_fork
                        .component(*path)
                        .map(|id| string_table.resolve(id))
                        == Some("Point")
            )
        })
        .expect("expected struct definition");

    let NodeKind::StructDefinition(path, fields) = &struct_node.kind else {
        panic!("expected struct definition node");
    };

    assert_eq!(
        path_fork
            .component(*path)
            .map(|id| string_table.resolve(id)),
        Some("Point")
    );
    assert_eq!(fields.len(), 2);
    assert!(matches!(fields[0].value.kind, ExpressionKind::NoValue));
    assert!(matches!(fields[1].value.kind, ExpressionKind::Int(2)));
}

#[test]
fn optional_present_default_preserves_struct_field_type() {
    let (ast, path_fork, mut string_table) = parse_single_file_ast(
        "Label = |\n    text String? = \"fallback\",\n    required String?,\n|\n",
    );
    let struct_node = ast
        .nodes
        .iter()
        .find(|node| {
            matches!(
                &node.kind,
                NodeKind::StructDefinition(path, ..)
                    if path_fork
                        .component(*path)
                        .map(|id| string_table.resolve(id))
                        == Some("Label")
            )
        })
        .expect("expected Label struct definition");
    let NodeKind::StructDefinition(path, fields) = &struct_node.kind else {
        panic!("expected struct definition node");
    };
    let string = ast.type_environment.builtins().string;
    assert_eq!(
        ast.type_environment
            .option_inner_type(fields[0].value.type_id),
        Some(string),
        "a present default must keep the declared String? field identity"
    );
    assert_eq!(
        fields[0].value.diagnostic_type, fields[1].value.diagnostic_type,
        "declared spelling must survive a present default"
    );
    let ExpressionKind::Coerced { value, to_type } = &fields[0].value.kind else {
        panic!(
            "present String default should be an explicit String -> String? coercion, got {:?}",
            fields[0].value
        );
    };
    assert_eq!(*to_type, fields[0].value.type_id);
    assert_eq!(value.type_id, string);
    assert!(matches!(value.kind, ExpressionKind::StringSlice(_)));

    let label_type_id = ast
        .type_environment
        .nominal_id_for_path(path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .expect("Label should have a nominal TypeId");
    let published = ast
        .type_environment
        .field_for(label_type_id, string_table.intern("text"), &path_fork)
        .expect("Label.text should be a published field");
    assert_eq!(
        ast.type_environment.option_inner_type(published.type_id),
        Some(string),
        "published FieldDefinition must keep the declared optional type"
    );
}

#[test]
fn parses_struct_construction_and_field_access_in_declarations() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "Point = |\n    x Int,\n    y Int,\n|\n\npoint = Point(1, 2)\nvalue = point.x\n",
    );

    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(point_decl) = &body[0].kind else {
        panic!("expected point declaration");
    };
    assert!(matches!(
        point_decl.value.kind,
        ExpressionKind::StructInstance(..)
    ));

    let NodeKind::VariableDeclaration(value_decl) = &body[1].kind else {
        panic!("expected field-read declaration");
    };
    assert!(
        matches!(value_decl.value.kind, ExpressionKind::FieldAccess { .. }),
        "field access should be stored as an expression-owned field-access payload"
    );
}

#[test]
fn parses_builtin_error_with_default_code_field() {
    let (ast, path_fork, string_table) = parse_single_file_ast("err = Error(\"bad\")\n");

    let body = start_function_body(&ast, &path_fork, &string_table);
    let NodeKind::VariableDeclaration(error_decl) = &body[0].kind else {
        panic!("expected error declaration");
    };
    let ExpressionKind::StructInstance(fields) = &error_decl.value.kind else {
        panic!("expected Error constructor to lower as a struct instance");
    };

    assert_eq!(fields.len(), 2);
    assert_eq!(
        path_fork
            .component(fields[0].id)
            .map(|id| string_table.resolve(id)),
        Some("message")
    );
    assert!(matches!(
        fields[0].value.kind,
        ExpressionKind::StringSlice(..)
    ));
    assert_eq!(
        path_fork
            .component(fields[1].id)
            .map(|id| string_table.resolve(id)),
        Some("code")
    );
    // Consumer-visible canonical contract: the omitted `code` materialises as a
    // genuine `U32` zero fixed-scalar value, never as an `Int` fallback.
    assert_eq!(
        fields[1].value.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    let ExpressionKind::FixedScalar(default_code) = &fields[1].value.kind else {
        panic!("expected Error.code default to lower as a fixed-scalar literal");
    };
    assert_eq!(default_code.scalar(), FixedScalar::U32);
    assert_eq!(default_code.as_u64(), Some(0));
}

#[test]
fn rejects_removed_builtin_error_fields() {
    let diagnostic = parse_single_file_ast_diagnostic("err = Error(\"bad\")\nvalue = err.kind\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFieldAccess {
            reason: InvalidFieldAccessReason::UnknownMember,
            ..
        }
    ));
}

#[test]
fn generic_struct_conflict_keeps_argument_and_expected_type_evidence_labels() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "Pair type T = |\n\
             left T,\n\
             right T,\n\
         |\n\
         bad Pair of Int = Pair(\"two\", 1)\n",
    );

    let DiagnosticPayload::InvalidGenericInstantiation {
        reason: InvalidGenericInstantiationReason::ConflictingInference { subject, .. },
        ..
    } = &diagnostic.payload
    else {
        panic!(
            "expected a generic inference conflict, got {:?}",
            diagnostic.payload
        );
    };
    assert_eq!(*subject, GenericInferenceSubject::NominalType);
    assert_eq!(diagnostic.labels.len(), 1);
    let previous_evidence_span = diagnostic.labels[0].span;

    assert_eq!(diagnostic.labels[0].style, DiagnosticLabelStyle::Secondary);
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::GenericInferencePreviousEvidence)
    );
    assert_ne!(diagnostic.primary_span, previous_evidence_span);
}

#[test]
fn optional_present_default_preserves_constant_struct_field_type() {
    let source = "FALLBACK_TEXT #= \"fallback\"\n\
Label = |\n\
    text String? = FALLBACK_TEXT,\n\
    required String?,\n\
|\n";
    let (ast, path_fork, mut string_table) = parse_single_file_ast(source);
    let struct_node = ast
        .nodes
        .iter()
        .find(|node| {
            matches!(
                &node.kind,
                NodeKind::StructDefinition(path, ..)
                    if path_fork
                        .component(*path)
                        .map(|id| string_table.resolve(id))
                        == Some("Label")
            )
        })
        .expect("expected Label struct definition");
    let NodeKind::StructDefinition(path, fields) = &struct_node.kind else {
        panic!("expected struct definition node");
    };
    let defaulted = &fields[0].value;
    let required = &fields[1].value;
    let string = ast.type_environment.builtins().string;

    assert_eq!(
        ast.type_environment.option_inner_type(defaulted.type_id),
        Some(string),
        "a constant-backed present default must keep the declared String? field identity"
    );
    assert_eq!(
        defaulted.diagnostic_type, required.diagnostic_type,
        "the declared String? spelling must survive a constant-backed field default"
    );
    let ExpressionKind::Coerced { value, to_type } = &defaulted.kind else {
        panic!(
            "present String default should retain its explicit String -> String? coercion, got {defaulted:?}"
        );
    };
    assert_eq!(*to_type, defaulted.type_id);
    assert_eq!(value.type_id, string);
    assert!(
        matches!(value.kind, ExpressionKind::StringSlice(_)),
        "the constant should inline as a String payload, got {:?}",
        value.kind
    );

    let label_type_id = ast
        .type_environment
        .nominal_id_for_path(path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .expect("Label should have a nominal TypeId");
    let published = ast
        .type_environment
        .field_for(label_type_id, string_table.intern("text"), &path_fork)
        .expect("Label.text should be a published field");
    assert_eq!(
        ast.type_environment.option_inner_type(published.type_id),
        Some(string),
        "published FieldDefinition must keep the declared optional type"
    );
}

#[test]
fn optional_present_default_survives_nested_nominal_field_publication() {
    let source = "INNER_LABEL #= \"nested\"\n\
Inner = |\n\
    label String? = INNER_LABEL,\n\
|\n\
Outer = |\n\
    child Inner = Inner(),\n\
|\n\
outer = Outer()\n";
    let (ast, path_fork, mut string_table) = parse_single_file_ast(source);
    let inner_node = ast
        .nodes
        .iter()
        .find(|node| {
            matches!(
                &node.kind,
                NodeKind::StructDefinition(path, ..)
                    if path_fork
                        .component(*path)
                        .map(|id| string_table.resolve(id))
                        == Some("Inner")
            )
        })
        .expect("expected Inner struct definition");
    let NodeKind::StructDefinition(inner_path, inner_fields) = &inner_node.kind else {
        panic!("expected Inner struct definition node");
    };
    let outer_node = ast
        .nodes
        .iter()
        .find(|node| {
            matches!(
                &node.kind,
                NodeKind::StructDefinition(path, ..)
                    if path_fork
                        .component(*path)
                        .map(|id| string_table.resolve(id))
                        == Some("Outer")
            )
        })
        .expect("expected Outer struct definition");
    let NodeKind::StructDefinition(outer_path, outer_fields) = &outer_node.kind else {
        panic!("expected Outer struct definition node");
    };
    let string = ast.type_environment.builtins().string;
    let inner_type_id = ast
        .type_environment
        .nominal_id_for_path(inner_path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .expect("Inner should have a nominal TypeId");
    let outer_type_id = ast
        .type_environment
        .nominal_id_for_path(outer_path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .expect("Outer should have a nominal TypeId");
    assert_eq!(
        outer_fields[0].value.type_id, inner_type_id,
        "Outer.child's constructor default should resolve to the nested Inner nominal"
    );
    let ExpressionKind::StructInstance(nested_fields) = &outer_fields[0].value.kind else {
        panic!(
            "Outer.child should retain its explicit nested nominal default, got {:?}",
            outer_fields[0].value.kind
        );
    };
    let nested_label = nested_fields
        .iter()
        .find(|field| {
            path_fork
                .component(field.id)
                .map(|id| string_table.resolve(id))
                == Some("label")
        })
        .expect("the inlined Inner default should retain its label field");
    assert_eq!(
        ast.type_environment
            .option_inner_type(nested_label.value.type_id),
        Some(string),
        "inlining the nested constructor default must keep Inner.label as String?"
    );
    let ExpressionKind::Coerced {
        value: nested_value,
        to_type: nested_to_type,
    } = &nested_label.value.kind
    else {
        panic!(
            "the inlined nested label should retain its String -> String? coercion, got {:?}",
            nested_label.value.kind
        );
    };
    assert_eq!(*nested_to_type, nested_label.value.type_id);
    assert_eq!(nested_value.type_id, string);
    assert!(
        matches!(nested_value.kind, ExpressionKind::StringSlice(_)),
        "the inlined nested label should keep its String payload, got {:?}",
        nested_value.kind
    );
    let ExpressionKind::Coerced { value, to_type } = &inner_fields[0].value.kind else {
        panic!(
            "Inner.label should retain its present String -> String? default, got {:?}",
            inner_fields[0].value.kind
        );
    };
    assert_eq!(*to_type, inner_fields[0].value.type_id);
    assert_eq!(value.type_id, string);
    assert!(
        matches!(value.kind, ExpressionKind::StringSlice(_)),
        "Inner.label default should retain a String payload, got {:?}",
        value.kind
    );
    assert_eq!(
        ast.type_environment
            .option_inner_type(inner_fields[0].value.type_id),
        Some(string)
    );

    let published_inner_label = ast
        .type_environment
        .field_for(inner_type_id, string_table.intern("label"), &path_fork)
        .expect("Inner.label should be published");
    assert_eq!(
        ast.type_environment
            .option_inner_type(published_inner_label.type_id),
        Some(string),
        "the inner nominal's published FieldDefinition must remain String?"
    );
    let published_outer_child = ast
        .type_environment
        .field_for(outer_type_id, string_table.intern("child"), &path_fork)
        .expect("Outer.child should be published");
    assert_eq!(
        published_outer_child.type_id, inner_type_id,
        "the nested nominal field should retain the Inner identity"
    );
}

#[test]
fn optional_present_default_does_not_supply_generic_nominal_type_argument() {
    let source = "FALLBACK_TEXT #= \"generic\"\n\
Box type T = |\n\
    value T,\n\
    label String? = FALLBACK_TEXT,\n\
|\n\
boxed = Box(value = 7)\n";
    let (build_result, path_fork, mut string_table) =
        parse_single_file_ast_build_result(source).expect("generic nominal default should resolve");
    let ast = &build_result.ast;
    let boxed = start_function_body(ast, &path_fork, &string_table)
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::VariableDeclaration(declaration)
                if path_fork
                    .component(declaration.id)
                    .map(|id| string_table.resolve(id))
                    == Some("boxed") =>
            {
                Some(declaration)
            }
            _ => None,
        })
        .expect("boxed declaration should be present");
    let int = ast.type_environment.builtins().int;
    let string = ast.type_environment.builtins().string;

    let TypeDefinition::GenericInstance(instance) = ast
        .type_environment
        .get(boxed.value.type_id)
        .expect("boxed value should have a resolved type")
    else {
        panic!("Box(value = 7) should resolve to a concrete generic instance");
    };
    assert_eq!(
        instance.arguments.as_ref(),
        &[int],
        "the ordinary value field should establish T as Int"
    );
    let published_label = ast
        .type_environment
        .field_for(
            boxed.value.type_id,
            string_table.intern("label"),
            &path_fork,
        )
        .expect("the concrete Box should publish its label field");
    assert_eq!(
        ast.type_environment
            .option_inner_type(published_label.type_id),
        Some(string),
        "the independent String? default must not be substituted as T"
    );
}

#[test]
fn optional_present_default_numeric_payloads_keep_declared_types() {
    let source = "NumericDefaults = |\n\
    octet U8? = 255,\n\
    amount Dec2? = 1.20,\n\
|\n";
    let (ast, path_fork, mut string_table) = parse_single_file_ast(source);
    let struct_node = ast
        .nodes
        .iter()
        .find(|node| {
            matches!(
                &node.kind,
                NodeKind::StructDefinition(path, ..)
                    if path_fork
                        .component(*path)
                        .map(|id| string_table.resolve(id))
                        == Some("NumericDefaults")
            )
        })
        .expect("expected NumericDefaults struct definition");
    let NodeKind::StructDefinition(path, fields) = &struct_node.kind else {
        panic!("expected NumericDefaults struct definition node");
    };
    let u8_type = builtin_type_ids::fixed_scalar(FixedScalar::U8);
    let u8_default = &fields[0].value;
    assert_eq!(
        ast.type_environment.option_inner_type(u8_default.type_id),
        Some(u8_type),
        "U8? default should keep its declared optional identity"
    );
    assert!(matches!(
        &u8_default.diagnostic_type,
        DataType::Option(inner) if matches!(inner.as_ref(), DataType::FixedScalar(FixedScalar::U8))
    ));
    let ExpressionKind::Coerced { value, to_type } = &u8_default.kind else {
        panic!("U8? default should retain its receiving coercion, got {u8_default:?}");
    };
    assert_eq!(*to_type, u8_default.type_id);
    assert_eq!(value.type_id, u8_type);
    let ExpressionKind::FixedScalar(value) = &value.kind else {
        panic!(
            "U8? default should preserve its fixed-scalar payload, got {:?}",
            value.kind
        );
    };
    assert_eq!(value.scalar(), FixedScalar::U8);
    assert_eq!(value.as_u64(), Some(255));

    let dec_default = &fields[1].value;
    let dec_type = ast
        .type_environment
        .option_inner_type(dec_default.type_id)
        .expect("Dec2? default should remain optional");
    assert_eq!(
        ast.type_environment
            .number_scale(dec_type)
            .map(|scale| scale.get()),
        Some(2),
        "Dec2? default should preserve its declared scale"
    );
    assert!(matches!(
        &dec_default.diagnostic_type,
        DataType::Option(inner) if matches!(inner.as_ref(), DataType::Number(scale) if scale.get() == 2)
    ));
    let ExpressionKind::Coerced { value, to_type } = &dec_default.kind else {
        panic!("Dec2? default should retain its receiving coercion, got {dec_default:?}");
    };
    assert_eq!(*to_type, dec_default.type_id);
    assert_eq!(value.type_id, dec_type);
    let ExpressionKind::Number(number) = &value.kind else {
        panic!(
            "Dec2? default should preserve its exact Number payload, got {:?}",
            value.kind
        );
    };
    assert_eq!(number.coefficient().to_string(), "120");
    assert_eq!(number.scale().get(), 2);

    let nominal_type_id = ast
        .type_environment
        .nominal_id_for_path(path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .expect("NumericDefaults should have a nominal TypeId");
    let published_octet = ast
        .type_environment
        .field_for(nominal_type_id, string_table.intern("octet"), &path_fork)
        .expect("NumericDefaults.octet should be published");
    let published_amount = ast
        .type_environment
        .field_for(nominal_type_id, string_table.intern("amount"), &path_fork)
        .expect("NumericDefaults.amount should be published");
    assert_eq!(
        ast.type_environment
            .option_inner_type(published_octet.type_id),
        Some(u8_type)
    );
    assert_eq!(
        ast.type_environment
            .option_inner_type(published_amount.type_id),
        Some(dec_type)
    );
}
