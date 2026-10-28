//! Struct parsing regression tests.
//!
//! WHAT: validates struct definitions, defaults, constructors, and field access.
//! WHY: struct parsing feeds both type resolution and HIR place lowering.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{DeclarationValue, NodeKind};
use crate::compiler_frontend::ast::const_values::{ConstValueId, ConstValuePayload};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::compiler_messages::GenericInferenceSubject;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    GenericInstantiationShape, InvalidGenericInstantiationView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_label_spans, ast_error_primary_span, parse_single_file_ast,
    parse_single_file_ast_build_result, parse_single_file_ast_diagnostic,
};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// One exact `I32` scalar value for struct-default fixtures.
///
/// WHY: an unconstrained whole literal defaults to `I32`, so completed defaults carry the exact
///      `I32` identity.
fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32")
}

/// Resolves a parsed declaration initializer to its expression view.
fn initializer_view<'a>(ast: &'a Ast, declaration_value: &DeclarationValue) -> ExpressionView<'a> {
    let id = declaration_value
        .initializer()
        .expect("fixture initializer");
    ast.expressions.view(id).expect("fixture expression view")
}

/// Returns the folded constant behind a struct field default.
///
/// WHY: struct field defaults are stored as folded constants, not as authored expressions.
fn default_constant(declaration_value: &DeclarationValue) -> ConstValueId {
    declaration_value
        .default()
        .expect("fixture field default constant")
}

/// Returns the `OptionSome` payload id behind a present optional default.
///
/// WHY: a present optional default folds to `OptionSome(inner)` carrying the declared optional
///      type on the outer value.
fn optional_payload_inner(ast: &Ast, id: ConstValueId, optional_type: TypeId) -> ConstValueId {
    assert_eq!(
        ast.const_values
            .metadata(id)
            .expect("optional default metadata")
            .type_id,
        optional_type,
        "a present optional default must keep its declared optional type"
    );
    let Some(ConstValuePayload::OptionSome(inner)) = ast.const_values.payload(id) else {
        panic!("a present optional default should fold to OptionSome");
    };
    *inner
}

/// Returns the constant payload behind a constant-backed expression view.
fn constant_payload_of<'a>(ast: &'a Ast, view: ExpressionView<'a>) -> &'a ConstValuePayload {
    let ExpressionOperationView::Constant(id) = view.operation() else {
        panic!("expected a constant-backed expression");
    };
    ast.const_values
        .payload(id)
        .expect("constant id must resolve to a payload")
}

#[test]
fn parses_struct_definitions_with_field_defaults() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("Point = |\n    x I32,\n    y I32 = 2,\n|\n");

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
    assert!(matches!(fields[0].value, DeclarationValue::Absent));
    assert!(matches!(
        ast.const_values.payload(default_constant(&fields[1].value)),
        Some(ConstValuePayload::FixedScalar(value)) if value == &i32_value(2)
    ));
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
        ast.type_environment.option_inner_type(fields[0].type_id),
        Some(string),
        "a present default must keep the declared String? field identity"
    );
    assert_eq!(
        fields[0].type_id, fields[1].type_id,
        "the declared String? semantic type must survive a present default"
    );
    let text_inner =
        optional_payload_inner(&ast, default_constant(&fields[0].value), fields[0].type_id);
    assert_eq!(
        ast.const_values
            .metadata(text_inner)
            .expect("text default payload metadata")
            .type_id,
        string
    );
    assert!(matches!(
        ast.const_values.payload(text_inner),
        Some(ConstValuePayload::String(_))
    ));

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
        "Point = |\n    x I32,\n    y I32,\n|\n\npoint = Point(1, 2)\nvalue = point.x\n",
    );

    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(point_decl) = &body[0].kind else {
        panic!("expected point declaration");
    };
    assert!(matches!(
        initializer_view(&ast, &point_decl.value).operation(),
        ExpressionOperationView::Struct { .. }
    ));

    let NodeKind::VariableDeclaration(value_decl) = &body[1].kind else {
        panic!("expected field-read declaration");
    };
    assert!(
        matches!(
            initializer_view(&ast, &value_decl.value).operation(),
            ExpressionOperationView::FieldAccess { .. }
        ),
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
    let ExpressionOperationView::Struct { fields } =
        initializer_view(&ast, &error_decl.value).operation()
    else {
        panic!("expected Error constructor to lower as a struct instance");
    };

    let field_views: Vec<_> = fields.iter().collect();
    assert_eq!(field_views.len(), 2);
    assert_eq!(
        path_fork
            .component(field_views[0].name())
            .map(|id| string_table.resolve(id)),
        Some("message")
    );
    assert!(matches!(
        constant_payload_of(&ast, field_views[0].value()),
        ConstValuePayload::String(..)
    ));
    assert_eq!(
        path_fork
            .component(field_views[1].name())
            .map(|id| string_table.resolve(id)),
        Some("code")
    );
    // Consumer-visible canonical contract: the omitted `code` materialises as a
    // genuine `U32` zero fixed-scalar value, never a removed identity fallback.
    assert_eq!(
        field_views[1]
            .value()
            .value_type_id()
            .expect("Error.code default type"),
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    let ConstValuePayload::FixedScalar(default_code) =
        constant_payload_of(&ast, field_views[1].value())
    else {
        panic!("expected Error.code default to lower as a fixed-scalar literal");
    };
    assert_eq!(default_code.scalar(), FixedScalar::U32);
    assert_eq!(default_code.as_u64(), Some(0));
}

#[test]
fn rejects_removed_builtin_error_fields() {
    let failure = parse_single_file_ast_diagnostic("err = Error(\"bad\")\nvalue = err.kind\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0048",
        Some("invalid_field_access.unknown_member"),
    );
}

#[test]
fn generic_struct_conflict_keeps_argument_and_expected_type_evidence_labels() {
    let failure = parse_single_file_ast_diagnostic(
        "Pair type T = |\n\
             left T,\n\
             right T,\n\
         |\n\
         bad Pair of I32 = Pair(\"two\", 1)\n",
    );
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0057",
        Some("invalid_generic_instantiation.conflicting_inference"),
    );
    let facts = diagnostic
        .family_view::<InvalidGenericInstantiationView>()
        .expect("generic conflict should retain its typed inference facts");
    let GenericInstantiationShape::ConflictingInference { subject, .. } = facts.shape else {
        panic!("expected conflicting generic inference facts");
    };
    assert_eq!(subject, GenericInferenceSubject::NominalType);

    // Durable labels are secondary labels; their message retains the previous-evidence role.
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();
    assert_eq!(labels.len(), 1);
    assert!(matches!(
        labels[0].message,
        DurableLabelMessage::GenericInferencePreviousEvidence
    ));
    let label_spans = ast_error_label_spans(diagnostic);
    assert_eq!(label_spans.len(), 1);
    assert!(label_spans[0].is_some());
    assert_ne!(ast_error_primary_span(diagnostic), label_spans[0]);
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
    let defaulted = &fields[0];
    let required = &fields[1];
    assert!(matches!(required.value, DeclarationValue::Absent));
    let string = ast.type_environment.builtins().string;

    assert_eq!(
        ast.type_environment.option_inner_type(defaulted.type_id),
        Some(string),
        "a constant-backed present default must keep the declared String? field identity"
    );
    assert_eq!(
        defaulted.type_id, required.type_id,
        "the declared String? semantic type must survive a constant-backed field default"
    );
    let default_inner =
        optional_payload_inner(&ast, default_constant(&defaulted.value), defaulted.type_id);
    assert_eq!(
        ast.const_values
            .metadata(default_inner)
            .expect("default payload metadata")
            .type_id,
        string
    );
    assert!(
        matches!(
            ast.const_values.payload(default_inner),
            Some(ConstValuePayload::String(_))
        ),
        "the constant should inline as a String payload"
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
        outer_fields[0].type_id, inner_type_id,
        "Outer.child's constructor default should resolve to the nested Inner nominal"
    );
    let outer_child = default_constant(&outer_fields[0].value);
    assert_eq!(
        ast.const_values
            .metadata(outer_child)
            .expect("Outer.child default metadata")
            .type_id,
        inner_type_id,
        "Outer.child's constructor default should fold to an Inner record"
    );
    let Some(ConstValuePayload::Record(nested_fields)) = ast.const_values.payload(outer_child)
    else {
        panic!("Outer.child should retain its explicit nested nominal default");
    };
    let nested_label = nested_fields
        .iter()
        .find(|field| {
            path_fork
                .component(field.name)
                .map(|id| string_table.resolve(id))
                == Some("label")
        })
        .expect("the inlined Inner default should retain its label field");
    let nested_label_inner =
        optional_payload_inner(&ast, nested_label.value, inner_fields[0].type_id);
    assert_eq!(
        ast.const_values
            .metadata(nested_label_inner)
            .expect("nested label payload metadata")
            .type_id,
        string,
        "inlining the nested constructor default must keep Inner.label as String?"
    );
    assert!(
        matches!(
            ast.const_values.payload(nested_label_inner),
            Some(ConstValuePayload::String(_))
        ),
        "the inlined nested label should keep its String payload"
    );
    let inner_label = optional_payload_inner(
        &ast,
        default_constant(&inner_fields[0].value),
        inner_fields[0].type_id,
    );
    assert_eq!(
        ast.const_values
            .metadata(inner_label)
            .expect("Inner.label payload metadata")
            .type_id,
        string,
        "Inner.label default payload should be String"
    );
    assert!(
        matches!(
            ast.const_values.payload(inner_label),
            Some(ConstValuePayload::String(_))
        ),
        "Inner.label default should retain a String payload"
    );
    assert_eq!(
        ast.type_environment
            .option_inner_type(inner_fields[0].type_id),
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
    let int = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let string = ast.type_environment.builtins().string;

    let TypeDefinition::GenericInstance(instance) = ast
        .type_environment
        .get(boxed.type_id)
        .expect("boxed value should have a resolved type")
    else {
        panic!("Box(value = 7) should resolve to a concrete generic instance");
    };
    assert_eq!(
        instance.arguments.as_ref(),
        &[int],
        "the ordinary value field should establish T as I32"
    );
    let published_label = ast
        .type_environment
        .field_for(boxed.type_id, string_table.intern("label"), &path_fork)
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
    assert_eq!(
        ast.type_environment.option_inner_type(fields[0].type_id),
        Some(u8_type),
        "U8? default should keep its declared optional identity"
    );
    let u8_default =
        optional_payload_inner(&ast, default_constant(&fields[0].value), fields[0].type_id);
    assert_eq!(
        ast.const_values
            .metadata(u8_default)
            .expect("U8 payload metadata")
            .type_id,
        u8_type
    );
    let ConstValuePayload::FixedScalar(value) =
        ast.const_values.payload(u8_default).expect("U8 payload")
    else {
        panic!("U8? default should preserve its fixed-scalar payload");
    };
    assert_eq!(value.scalar(), FixedScalar::U8);
    assert_eq!(value.as_u64(), Some(255));

    let dec_type = ast
        .type_environment
        .option_inner_type(fields[1].type_id)
        .expect("Dec2? default should remain optional");
    assert_eq!(
        ast.type_environment
            .number_scale(dec_type)
            .map(|scale| scale.get()),
        Some(2),
        "Dec2? default should preserve its declared scale"
    );
    let dec_default =
        optional_payload_inner(&ast, default_constant(&fields[1].value), fields[1].type_id);
    assert_eq!(
        ast.const_values
            .metadata(dec_default)
            .expect("Dec2 payload metadata")
            .type_id,
        dec_type
    );
    let ConstValuePayload::Number(number) =
        ast.const_values.payload(dec_default).expect("Dec2 payload")
    else {
        panic!("Dec2? default should preserve its exact Number payload");
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
