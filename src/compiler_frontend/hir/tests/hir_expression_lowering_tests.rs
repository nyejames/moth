//! HIR expression lowering regression tests.
//!
//! WHAT: covers how typed AST expressions become HIR values, preludes, and places.
//! WHY: expression lowering is broad and subtle enough that behavior changes need focused regression tests.

use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, LoopBindings, NodeKind, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode,
};
use crate::compiler_frontend::ast::expressions::expression::{
    ConstRecordState, Expression, ExpressionKind,
    FallibleCarrierVariant as AstFallibleCarrierVariant, FallibleExpressionHandling,
    FallibleHandling, Operator, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::expressions::expression_kind::MapLiteralEntry;
use crate::compiler_frontend::ast::statements::fallible_handling::wrap_catch_expression;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::ProducedValues;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotKey, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopControlKind, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateBranch, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::definitions::{
    BuiltinTypeDefinition, ChoiceTypeDefinition, ConstructedTypeDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, BuiltinTypeKey, NominalTypeId, TypeConstructor, TypeId,
};
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirMapOp, HirVariantCarrier, OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::{
    expressions_to_owned_render_node, register_local, runtime_template_expression, setup_builder,
};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{
    BlockId, ChoiceId, FieldId, FunctionId, LocalId, StructId,
};
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::reactivity::{
    HirReactiveSource, HirReactiveSourceKind, ReactiveSourceId,
};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
use crate::compiler_frontend::tests::hir_fixture_support::raw_template_expression_for_hir_invariant;
use crate::compiler_frontend::tests::type_id_fixture_support::{
    choice_construct_expr, const_record_reference_expr, field_access_node, handled_result_expr,
    inferred_type_reference_expr, option_none_expr, result_carrier_type_id, runtime_expr,
    runtime_operand_item, runtime_operator_item,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn field_symbol(
    parent: &InternedPath,
    field_name: &str,
    string_table: &mut StringTable,
) -> InternedPath {
    parent.append(string_table.intern(field_name))
}

/// Builds the aggregate-wrapper node for a loop's owning head wrapper.
///
/// WHAT: a `Sequence` of prefix `Text`, `AggregateOutput`, suffix `Text`.
/// WHY: keeping this boundary shape local makes the aggregate splice explicit.
fn text_aggregate_wrapper_node(
    prefix: StringId,
    suffix: StringId,
    _string_table: &StringTable,
    location: &SourceLocation,
) -> OwnedRuntimeTemplateNode {
    OwnedRuntimeTemplateNode::Sequence {
        children: vec![
            OwnedRuntimeTemplateNode::Text {
                text: prefix,
                reactive_subscription: None,
                location: location.to_owned(),
            },
            OwnedRuntimeTemplateNode::AggregateOutput,
            OwnedRuntimeTemplateNode::Text {
                text: suffix,
                reactive_subscription: None,
                location: location.to_owned(),
            },
        ],
    }
}

fn runtime_template_bool_if_expression(
    condition: Expression,
    then_content: Vec<Expression>,
    else_content: Option<Vec<Expression>>,
    location: SourceLocation,
    string_table: &StringTable,
) -> Expression {
    let then_body = expressions_to_owned_render_node(&then_content, string_table);
    let fallback = else_content
        .map(|content| Box::new(expressions_to_owned_render_node(&content, string_table)));

    let body = OwnedRuntimeTemplateNode::BranchChain {
        branches: vec![OwnedRuntimeTemplateBranch {
            selector: TemplateBranchSelector::Bool(condition),
            body: then_body,
            location: location.clone(),
        }],
        fallback,
        location: location.clone(),
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(body),
        location: location.clone(),
    };

    Expression::runtime_template_handoff(handoff, ValueMode::ImmutableOwned)
}

#[allow(
    clippy::too_many_arguments,
    reason = "test fixture builder mirrors template option-capture payloads"
)]
fn runtime_template_option_capture_expression(
    scrutinee: Expression,
    capture_name: crate::compiler_frontend::symbols::string_interning::StringId,
    capture_path: InternedPath,
    inner_type_id: TypeId,
    then_content: Vec<Expression>,
    else_content: Option<Vec<Expression>>,
    location: SourceLocation,
    string_table: &StringTable,
) -> Expression {
    let then_body = expressions_to_owned_render_node(&then_content, string_table);
    let fallback = else_content
        .map(|content| Box::new(expressions_to_owned_render_node(&content, string_table)));

    let body = OwnedRuntimeTemplateNode::BranchChain {
        branches: vec![OwnedRuntimeTemplateBranch {
            selector: TemplateBranchSelector::OptionPresentCapture {
                scrutinee,
                pattern: Box::new(MatchPattern::OptionPresentCapture {
                    name: capture_name,
                    binding_path: capture_path,
                    inner_type_id,
                    location: location.clone(),
                    binding_location: location.clone(),
                }),
            },
            body: then_body,
            location: location.clone(),
        }],
        fallback,
        location: location.clone(),
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(body),
        location: location.clone(),
    };

    Expression::runtime_template_handoff(handoff, ValueMode::ImmutableOwned)
}

fn runtime_template_range_loop_expression(
    bindings: LoopBindings,
    range: RangeLoopSpec,
    body_content: Vec<Expression>,
    aggregate_prefix: StringId,
    aggregate_suffix: StringId,
    location: SourceLocation,
    string_table: &StringTable,
) -> Expression {
    let body = expressions_to_owned_render_node(&body_content, string_table);
    let aggregate_wrapper =
        text_aggregate_wrapper_node(aggregate_prefix, aggregate_suffix, string_table, &location);

    let node = OwnedRuntimeTemplateNode::Loop {
        header: TemplateLoopHeader::Range {
            bindings: Box::new(bindings),
            range: Box::new(range),
        },
        body: Box::new(body),
        aggregate_wrapper: Some(Box::new(aggregate_wrapper)),
        location: location.clone(),
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(node),
        location: location.clone(),
    };

    Expression::runtime_template_handoff(handoff, ValueMode::ImmutableOwned)
}

fn runtime_template_collection_loop_expression(
    bindings: LoopBindings,
    iterable: Expression,
    body_content: Vec<Expression>,
    aggregate_prefix: StringId,
    aggregate_suffix: StringId,
    location: SourceLocation,
    string_table: &StringTable,
) -> Expression {
    let body = expressions_to_owned_render_node(&body_content, string_table);
    let aggregate_wrapper =
        text_aggregate_wrapper_node(aggregate_prefix, aggregate_suffix, string_table, &location);

    let node = OwnedRuntimeTemplateNode::Loop {
        header: TemplateLoopHeader::Collection {
            bindings: Box::new(bindings),
            iterable: Box::new(iterable),
        },
        body: Box::new(body),
        aggregate_wrapper: Some(Box::new(aggregate_wrapper)),
        location: location.clone(),
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(node),
        location: location.clone(),
    };

    Expression::runtime_template_handoff(handoff, ValueMode::ImmutableOwned)
}

fn runtime_template_conditional_loop_expression(
    condition: Expression,
    body_content: Vec<Expression>,
    aggregate_prefix: StringId,
    aggregate_suffix: StringId,
    location: SourceLocation,
    string_table: &StringTable,
) -> Expression {
    let body = expressions_to_owned_render_node(&body_content, string_table);
    let aggregate_wrapper =
        text_aggregate_wrapper_node(aggregate_prefix, aggregate_suffix, string_table, &location);

    let node = OwnedRuntimeTemplateNode::Loop {
        header: TemplateLoopHeader::Conditional {
            condition: Box::new(condition),
        },
        body: Box::new(body),
        aggregate_wrapper: Some(Box::new(aggregate_wrapper)),
        location: location.clone(),
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(node),
        location: location.clone(),
    };

    Expression::runtime_template_handoff(handoff, ValueMode::ImmutableOwned)
}

fn loop_binding(name: &str, type_id: TypeId, string_table: &mut StringTable) -> Declaration {
    Declaration {
        id: InternedPath::from_single_str(name, string_table),
        value: Expression::new(
            ExpressionKind::NoValue,
            SourceLocation::default(),
            type_id,
            crate::compiler_frontend::datatypes::DataType::Inferred,
            ValueMode::ImmutableOwned,
        ),
    }
}

#[test]
fn runtime_template_slot_placeholder_materializes_as_no_output_owned_node() {
    let mut string_table = StringTable::new();
    let before = string_table.intern("before ");
    let after = string_table.intern("after");
    let location = test_source_location(1);

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
            children: vec![
                OwnedRuntimeTemplateNode::Text {
                    text: before,
                    reactive_subscription: None,
                    location: location.clone(),
                },
                OwnedRuntimeTemplateNode::Slot {
                    location: location.clone(),
                },
                OwnedRuntimeTemplateNode::Text {
                    text: after,
                    reactive_subscription: None,
                    location: location.clone(),
                },
            ],
        }),
        location,
    };
    let body = match &handoff.body {
        crate::compiler_frontend::ast::templates::OwnedRuntimeTemplateBody::Render(node) => node,
        _ => panic!("slot-shaped template should have a render handoff body"),
    };
    assert!(
        matches!(body, OwnedRuntimeTemplateNode::Sequence { children, .. } if children.iter().any(|child| matches!(child, OwnedRuntimeTemplateNode::Slot { .. }))),
        "slot-shaped template handoff should contain a no-output Slot node"
    );

    let mut builder = setup_builder(&mut string_table);
    let lowered = builder
        .lower_expression(&Expression::runtime_template_handoff(
            handoff,
            ValueMode::ImmutableOwned,
        ))
        .expect("slot-shaped runtime templates should lower in HIR");

    assert!(lowered.prelude.is_empty());
    assert!(builder.module.functions.is_empty());

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    assert!(block_assigns_string_literal(entry_block, "before "));
    assert!(block_assigns_string_literal(entry_block, "after"));
}

#[test]
fn escaped_slot_insert_helpers_fail_when_they_reach_hir_runtime_lowering() {
    let mut string_table = StringTable::new();
    let body_slot = string_table.intern("body");
    let location = test_source_location(2);
    let mut builder = setup_builder(&mut string_table);

    let (helper, _helper_registry) = raw_template_expression_for_hir_invariant(
        TemplateType::SlotInsert(SlotKey::named(body_slot)),
        location,
        ValueMode::ImmutableOwned,
    );

    let err = builder
        .lower_expression(&helper)
        .expect_err("escaped helper templates should be rejected in HIR");

    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(
        err.msg
            .contains("Raw template reached HIR runtime-template lowering")
    );
}

#[test]
fn escaped_slot_definition_helpers_fail_when_they_reach_hir_runtime_lowering() {
    let mut string_table = StringTable::new();
    let body_slot = string_table.intern("body");
    let location = test_source_location(2);
    let mut builder = setup_builder(&mut string_table);

    let (helper, _helper_registry) = raw_template_expression_for_hir_invariant(
        TemplateType::SlotDefinition(SlotKey::named(body_slot)),
        location,
        ValueMode::ImmutableOwned,
    );

    let err = builder
        .lower_expression(&helper)
        .expect_err("escaped slot definition helpers should be rejected in HIR");

    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(
        err.msg
            .contains("Raw template reached HIR runtime-template lowering")
    );
}

#[test]
fn runtime_template_without_handoff_reports_compiler_bug() {
    let mut string_table = StringTable::new();
    let location = test_source_location(2);
    let mut builder = setup_builder(&mut string_table);
    let (template, _template_registry) = raw_template_expression_for_hir_invariant(
        TemplateType::StringFunction,
        location,
        ValueMode::ImmutableOwned,
    );

    let err = builder
        .lower_expression(&template)
        .expect_err("runtime templates without an owned handoff should fail");

    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(
        err.msg
            .contains("Raw template reached HIR runtime-template lowering")
    );
}

#[test]
fn top_level_loop_control_handoff_reports_compiler_bug() {
    let mut string_table = StringTable::new();
    let location = test_source_location(2);
    let mut builder = setup_builder(&mut string_table);
    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::LoopControl {
            kind: TemplateLoopControlKind::Break,
            location: location.clone(),
        }),
        location: location.clone(),
    };

    let err = builder
        .lower_expression(&Expression::runtime_template_handoff(
            handoff,
            ValueMode::ImmutableOwned,
        ))
        .expect_err("top-level loop control handoffs should be rejected in HIR");

    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(
        err.msg
            .contains("Template loop-control signal reached HIR outside")
    );
}

#[test]
fn lowers_primitive_literals() {
    let mut string_table = StringTable::new();
    let text = string_table.intern("hello");
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);

    let int_lowered = builder
        .lower_expression(&Expression::int(
            42,
            location.clone(),
            ValueMode::ImmutableOwned,
        ))
        .expect("int lowering should succeed");
    assert!(int_lowered.prelude.is_empty());
    assert_eq!(int_lowered.value.value_kind, ValueKind::Const);
    assert!(matches!(int_lowered.value.kind, HirExpressionKind::Int(42)));

    let float_lowered = builder
        .lower_expression(&Expression::float(
            3.25,
            location.clone(),
            ValueMode::ImmutableOwned,
        ))
        .expect("float lowering should succeed");
    assert!(float_lowered.prelude.is_empty());
    assert_eq!(float_lowered.value.value_kind, ValueKind::Const);
    assert!(matches!(
        float_lowered.value.kind,
        HirExpressionKind::Float(3.25)
    ));

    let bool_lowered = builder
        .lower_expression(&Expression::bool(
            true,
            location.clone(),
            ValueMode::ImmutableOwned,
        ))
        .expect("bool lowering should succeed");
    assert!(bool_lowered.prelude.is_empty());
    assert_eq!(bool_lowered.value.value_kind, ValueKind::Const);
    assert!(matches!(
        bool_lowered.value.kind,
        HirExpressionKind::Bool(true)
    ));

    let char_lowered = builder
        .lower_expression(&Expression::char(
            'x',
            location.clone(),
            ValueMode::ImmutableOwned,
        ))
        .expect("char lowering should succeed");
    assert!(char_lowered.prelude.is_empty());
    assert_eq!(char_lowered.value.value_kind, ValueKind::Const);
    assert!(matches!(
        char_lowered.value.kind,
        HirExpressionKind::Char('x')
    ));

    let string_expr = Expression::string_slice(text, location.clone(), ValueMode::ImmutableOwned);
    let string_lowered = builder
        .lower_expression(&string_expr)
        .expect("string literal lowering should succeed");
    assert!(string_lowered.prelude.is_empty());
    assert_eq!(string_lowered.value.value_kind, ValueKind::Const);
    assert!(matches!(
        string_lowered.value.kind,
        HirExpressionKind::StringLiteral(ref s) if s == "hello"
    ));
}

#[test]
fn lowers_reference_to_registered_local() {
    let mut string_table = StringTable::new();
    let x = super::symbol("x", &mut string_table);
    let location = test_source_location(2);
    let mut builder = setup_builder(&mut string_table);

    register_local(
        &mut builder,
        x.clone(),
        LocalId(10),
        builtin_type_ids::INT,
        location.clone(),
    );

    let expr = inferred_type_reference_expr(
        x,
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::ImmutableReference,
    );
    let lowered = builder
        .lower_expression(&expr)
        .expect("reference lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(lowered.value.value_kind, ValueKind::Place);
    assert!(matches!(
        lowered.value.kind,
        HirExpressionKind::Load(HirPlace::Local(LocalId(10)))
    ));
}

#[test]
fn lowers_reference_to_module_constant_when_local_is_missing() {
    let mut string_table = StringTable::new();
    let third_const = super::symbol("third_const", &mut string_table);
    let location = test_source_location(3);
    let mut builder = setup_builder(&mut string_table);

    builder.test_register_module_constant(
        third_const.clone(),
        Expression::int(3, location.clone(), ValueMode::ImmutableOwned),
    );

    let expr = inferred_type_reference_expr(
        third_const,
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::ImmutableReference,
    );
    let lowered = builder
        .lower_expression(&expr)
        .expect("module constant reference lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(lowered.value.value_kind, ValueKind::Const);
    assert!(matches!(lowered.value.kind, HirExpressionKind::Int(3)));
}

#[test]
fn lowers_runtime_rpn_arithmetic_stack_correctly() {
    let mut string_table = StringTable::new();
    let x = super::symbol("x", &mut string_table);
    let y = super::symbol("y", &mut string_table);
    let location = test_source_location(3);
    let mut builder = setup_builder(&mut string_table);

    register_local(
        &mut builder,
        x.clone(),
        LocalId(10),
        builtin_type_ids::INT,
        location.clone(),
    );
    register_local(
        &mut builder,
        y.clone(),
        LocalId(11),
        builtin_type_ids::INT,
        location.clone(),
    );

    let items = vec![
        runtime_operand_item(inferred_type_reference_expr(
            x,
            builtin_type_ids::INT,
            location.clone(),
            ValueMode::ImmutableReference,
        )),
        runtime_operand_item(Expression::int(
            2,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        runtime_operand_item(inferred_type_reference_expr(
            y,
            builtin_type_ids::INT,
            location.clone(),
            ValueMode::ImmutableReference,
        )),
        runtime_operator_item(Operator::Multiply, location.clone()),
        runtime_operator_item(Operator::Add, location.clone()),
    ];

    let expr = runtime_expr(
        items,
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::MutableOwned,
    );
    let lowered = builder
        .lower_expression(&expr)
        .expect("runtime arithmetic lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(lowered.value.ty, builtin_type_ids::INT);
    assert!(
        matches!(
            lowered.value.kind,
            HirExpressionKind::Load(HirPlace::Local(_))
        ),
        "checked integer addition should return a load of the NumericOp result local"
    );
}

#[test]
fn runtime_division_subexpression_infers_float_type_in_hir() {
    let mut string_table = StringTable::new();
    let location = test_source_location(3);
    let mut builder = setup_builder(&mut string_table);
    let expected_float = builtin_type_ids::FLOAT;

    let items = vec![
        runtime_operand_item(Expression::int(
            5,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        runtime_operand_item(Expression::int(
            2,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        runtime_operator_item(Operator::Divide, location.clone()),
        runtime_operand_item(Expression::int(
            1,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        runtime_operator_item(Operator::Add, location.clone()),
    ];

    let expr = runtime_expr(
        items,
        builtin_type_ids::FLOAT,
        location.clone(),
        ValueMode::MutableOwned,
    );
    let lowered = builder
        .lower_expression(&expr)
        .expect("runtime division lowering should succeed");

    assert_eq!(lowered.value.ty, expected_float);
    assert!(
        matches!(
            lowered.value.kind,
            HirExpressionKind::Load(HirPlace::Local(_))
        ),
        "checked addition should return a load of the NumericOp result local"
    );
}

#[test]
fn runtime_integer_division_lowers_to_hir_int_div_with_int_type() {
    let mut string_table = StringTable::new();
    let location = test_source_location(3);
    let mut builder = setup_builder(&mut string_table);
    let expected_int = builtin_type_ids::INT;

    let items = vec![
        runtime_operand_item(Expression::int(
            5,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        runtime_operand_item(Expression::int(
            2,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        runtime_operator_item(Operator::IntDivide, location.clone()),
    ];

    let expr = runtime_expr(
        items,
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::MutableOwned,
    );
    let lowered = builder
        .lower_expression(&expr)
        .expect("runtime integer division lowering should succeed");

    assert_eq!(lowered.value.ty, expected_int);
    assert!(
        matches!(
            lowered.value.kind,
            HirExpressionKind::Load(HirPlace::Local(_))
        ),
        "checked integer division should return a load of the NumericOp result local"
    );
}

#[test]
fn lowers_unary_not_in_runtime_rpn() {
    let mut string_table = StringTable::new();
    let location = test_source_location(4);
    let mut builder = setup_builder(&mut string_table);

    let items = vec![
        runtime_operand_item(Expression::bool(
            true,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        runtime_operator_item(Operator::Not, location.clone()),
    ];

    let expr = runtime_expr(
        items,
        builtin_type_ids::BOOL,
        location.clone(),
        ValueMode::MutableOwned,
    );
    let lowered = builder
        .lower_expression(&expr)
        .expect("unary not lowering should succeed");

    assert!(matches!(
        lowered.value.kind,
        HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Not,
            ..
        }
    ));
}

#[test]
fn lowers_range_operator_in_runtime_rpn() {
    let mut string_table = StringTable::new();
    let location = test_source_location(5);
    let mut builder = setup_builder(&mut string_table);

    let items = vec![
        runtime_operand_item(Expression::int(
            1,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        runtime_operand_item(Expression::int(
            9,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        runtime_operator_item(Operator::Range, location.clone()),
    ];

    let expr = runtime_expr(
        items,
        builtin_type_ids::RANGE,
        location.clone(),
        ValueMode::MutableOwned,
    );
    let lowered = builder
        .lower_expression(&expr)
        .expect("range lowering should succeed");

    assert!(matches!(
        lowered.value.kind,
        HirExpressionKind::Range { .. }
    ));
}

#[test]
fn lowers_function_call_to_call_statement_and_temp_load() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("sum", &mut string_table);
    let location = test_source_location(6);
    let mut builder = setup_builder(&mut string_table);
    builder.test_register_function_name(function_name.clone(), FunctionId(2));

    let call_expr = Expression::function_call(
        function_name.clone(),
        vec![Expression::int(
            7,
            location.clone(),
            ValueMode::ImmutableOwned,
        )],
        vec![builtin_type_ids::INT],
        location.clone(),
    );

    let lowered = builder
        .lower_expression(&call_expr)
        .expect("function call lowering should succeed");
    assert_eq!(lowered.prelude.len(), 1);

    let statement = &lowered.prelude[0];
    let result_local = match &statement.kind {
        HirStatementKind::Call {
            target,
            args,
            result,
        } => {
            assert_eq!(target, &CallTarget::Local(FunctionId(2)));
            assert_eq!(args.len(), 1);
            result.expect("call with return should bind a temp local")
        }
        _ => panic!("expected lowered call statement"),
    };

    assert!(matches!(
        lowered.value.kind,
        HirExpressionKind::Load(HirPlace::Local(local))
        if local == result_local
    ));
    assert_eq!(lowered.value.value_kind, ValueKind::RValue);
}

#[test]
fn expression_function_call_uses_variant_result_type_ids_for_single_return() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("typed_result", &mut string_table);
    let location = test_source_location(6);
    let mut builder = setup_builder(&mut string_table);
    builder.test_register_function_name(function_name.clone(), FunctionId(32));

    let call_expr = Expression::function_call_with_typed_arguments(
        function_name,
        vec![],
        vec![builtin_type_ids::INT],
        &mut builder.type_environment,
        location.clone(),
    );
    assert_eq!(call_expr.type_id, builtin_type_ids::INT);
    assert!(
        matches!(
            &call_expr.kind,
            ExpressionKind::FunctionCall { result_type_ids, .. }
                if result_type_ids.as_slice() == [builtin_type_ids::INT]
        ),
        "typed function-call construction should store canonical result TypeIds immediately"
    );

    let expected_int = builder
        .lower_type_id(builtin_type_ids::INT, &location)
        .expect("builtin Int TypeId should lower in test context");
    let lowered = builder
        .lower_expression(&call_expr)
        .expect("function call lowering should use variant result TypeIds");

    assert_eq!(lowered.value.ty, expected_int);
}

#[test]
fn expression_function_call_uses_variant_result_type_ids_for_no_return() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("no_result", &mut string_table);
    let location = test_source_location(6);
    let mut builder = setup_builder(&mut string_table);
    builder.test_register_function_name(function_name.clone(), FunctionId(33));

    let call_expr = Expression::function_call(function_name, vec![], vec![], location.clone());

    let lowered = builder
        .lower_expression(&call_expr)
        .expect("no-return call lowering should use empty variant result TypeIds");
    let lowered_type = builder.type_environment.get(lowered.value.ty);

    assert!(
        matches!(
            lowered_type,
            Some(TypeDefinition::Builtin(BuiltinTypeDefinition {
                key: BuiltinTypeKey::None,
            }))
        ),
        "expected no-return expression call to lower as Unit, got {lowered_type:?}"
    );
}

#[test]
fn expression_function_call_uses_variant_result_type_ids_for_multi_return() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("multi_result", &mut string_table);
    let location = test_source_location(6);
    let mut builder = setup_builder(&mut string_table);
    builder.test_register_function_name(function_name.clone(), FunctionId(34));

    let call_expr = Expression::function_call_with_typed_arguments(
        function_name,
        vec![],
        vec![builtin_type_ids::INT, builtin_type_ids::BOOL],
        &mut builder.type_environment,
        location.clone(),
    );
    assert!(
        matches!(
            &call_expr.kind,
            ExpressionKind::FunctionCall { result_type_ids, .. }
                if result_type_ids.as_slice() == [builtin_type_ids::INT, builtin_type_ids::BOOL]
        ),
        "typed function-call construction should preserve canonical multi-return TypeIds"
    );

    let lowered = builder
        .lower_expression(&call_expr)
        .expect("multi-return call lowering should use variant result TypeIds");
    let int_type = builder
        .lower_type_id(builtin_type_ids::INT, &location)
        .expect("builtin Int TypeId should lower in test context");
    let bool_type = builder
        .lower_type_id(builtin_type_ids::BOOL, &location)
        .expect("builtin Bool TypeId should lower in test context");
    let lowered_type = builder.type_environment.get(lowered.value.ty);

    assert!(
        matches!(
            lowered_type,
            Some(TypeDefinition::Constructed(ConstructedTypeDefinition {
                constructor: TypeConstructor::Builtin(BuiltinTypeConstructor::Tuple),
                arguments,
            })) if arguments.as_ref() == [int_type, bool_type]
        ),
        "expected multi-return expression call to lower as tuple(Int, Bool), got {lowered_type:?}"
    );
}

#[test]
fn expression_host_call_uses_variant_result_type_ids() {
    let mut string_table = StringTable::new();
    let location = test_source_location(6);
    let mut builder = setup_builder(&mut string_table);

    let call_expr = Expression::host_function_call_with_typed_arguments(
        ExternalFunctionId::IoLine,
        vec![],
        vec![builtin_type_ids::INT],
        &mut builder.type_environment,
        location.clone(),
    );
    assert!(
        matches!(
            &call_expr.kind,
            ExpressionKind::HostFunctionCall { result_type_ids, .. }
                if result_type_ids.as_slice() == [builtin_type_ids::INT]
        ),
        "typed host-call construction should store canonical result TypeIds immediately"
    );

    let expected_int = builder
        .lower_type_id(builtin_type_ids::INT, &location)
        .expect("builtin Int TypeId should lower in test context");
    let lowered = builder
        .lower_expression(&call_expr)
        .expect("host call lowering should use variant result TypeIds");

    assert_eq!(lowered.value.ty, expected_int);
}

#[test]
fn expression_handled_fallible_call_fallback_uses_variant_result_type_ids() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("handled_result", &mut string_table);
    let location = test_source_location(6);
    let mut builder = setup_builder(&mut string_table);
    let ok_type = builder
        .lower_type_id(builtin_type_ids::INT, &location)
        .expect("builtin Int TypeId should lower in test context");
    let err_type = builder
        .lower_type_id(builtin_type_ids::STRING, &location)
        .expect("builtin String TypeId should lower in test context");
    let carrier_type = result_carrier_type_id(&mut builder.type_environment, ok_type, err_type);
    builder.test_register_function_with_return_type(
        function_name.clone(),
        FunctionId(35),
        carrier_type,
    );
    let test_scope = function_name.clone();

    let handled_call_expr = Expression::handled_fallible_function_call_with_typed_arguments(
        function_name,
        vec![],
        vec![builtin_type_ids::INT],
        FallibleExpressionHandling::Recover,
        &mut builder.type_environment,
        location.clone(),
    );
    assert!(
        matches!(
            &handled_call_expr.kind,
            ExpressionKind::HandledFallibleFunctionCall { result_type_ids, .. }
                if result_type_ids.as_slice() == [builtin_type_ids::INT]
        ),
        "typed handled fallible call construction should store canonical result TypeIds immediately"
    );

    let call_expr = wrap_catch_expression(
        handled_call_expr,
        FallibleHandling::Handler {
            error: None,
            body: vec![AstNode {
                kind: NodeKind::ThenValue(ProducedValues {
                    expressions: vec![Expression::int(
                        7,
                        location.clone(),
                        ValueMode::ImmutableOwned,
                    )],
                    location: location.clone(),
                }),
                location: location.clone(),
                scope: test_scope,
            }],
        },
        vec![builtin_type_ids::INT],
    );

    let lowered = builder
        .lower_expression(&call_expr)
        .expect("handled fallible call lowering should use variant result TypeIds");

    assert_eq!(lowered.value.ty, ok_type);
}

#[test]
fn expression_handled_result_derives_success_slots_from_tuple_type_id() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("handled_result_expr", &mut string_table);
    let location = test_source_location(6);
    let mut builder = setup_builder(&mut string_table);
    builder.test_register_function_name(function_name.clone(), FunctionId(36));

    let ok_type = builder
        .type_environment
        .intern_tuple(vec![builtin_type_ids::INT, builtin_type_ids::BOOL]);
    let err_type = builtin_type_ids::STRING;
    let carrier_type = result_carrier_type_id(&mut builder.type_environment, ok_type, err_type);

    let result_expr = Expression::function_call_with_typed_arguments(
        function_name,
        vec![],
        vec![carrier_type],
        &mut builder.type_environment,
        location.clone(),
    );
    assert_eq!(result_expr.type_id, carrier_type);
    let test_scope = InternedPath::new();

    let handled_expr = handled_result_expr(
        result_expr,
        FallibleHandling::Handler {
            error: None,
            body: vec![AstNode {
                kind: NodeKind::ThenValue(ProducedValues {
                    expressions: vec![
                        Expression::int(7, location.clone(), ValueMode::ImmutableOwned),
                        Expression::bool(false, location.clone(), ValueMode::ImmutableOwned),
                    ],
                    location: location.clone(),
                }),
                location: location.clone(),
                scope: test_scope,
            }],
        },
        ok_type,
        vec![builtin_type_ids::INT, builtin_type_ids::BOOL],
        location.clone(),
    );

    let lowered = builder
        .lower_expression(&handled_expr)
        .expect("handled Result expression should preserve multi-success tuple typing");

    assert_eq!(lowered.value.ty, ok_type);
}

#[test]
fn lowers_fresh_mutable_call_argument_via_hidden_local_with_origin_metadata() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("mutate", &mut string_table);
    let location = test_source_location(6);
    let mut builder = setup_builder(&mut string_table);
    builder.test_register_function_name(function_name.clone(), FunctionId(24));

    let fresh_argument = CallArgument::positional(
        Expression::int(7, location.clone(), ValueMode::ImmutableOwned),
        CallAccessMode::Shared,
        location.clone(),
    )
    .with_passing_mode(CallPassingMode::FreshMutableValue);

    let call_expr = Expression::function_call_with_arguments(
        function_name,
        vec![fresh_argument],
        vec![],
        location.clone(),
    );

    let lowered = builder
        .lower_expression(&call_expr)
        .expect("fresh mutable argument lowering should succeed");

    assert_eq!(
        lowered.prelude.len(),
        2,
        "fresh mutable args should materialize assignment before call"
    );

    let temp_local = match &lowered.prelude[0].kind {
        HirStatementKind::Assign { target, value } => {
            assert!(matches!(value.kind, HirExpressionKind::Int(7)));
            match target {
                HirPlace::Local(local) => *local,
                other => panic!("expected local assignment target, got {other:?}"),
            }
        }
        other => panic!("expected first prelude statement to assign fresh arg temp, got {other:?}"),
    };

    match &lowered.prelude[1].kind {
        HirStatementKind::Call { args, .. } => {
            assert_eq!(args.len(), 1);
            assert!(
                matches!(
                    args[0].kind,
                    HirExpressionKind::Load(HirPlace::Local(local)) if local == temp_local
                ),
                "call argument should load synthesized fresh-arg local"
            );
        }
        other => panic!("expected second prelude statement to be call, got {other:?}"),
    }

    let origin = builder
        .side_table
        .local_origin(temp_local)
        .expect("fresh mutable arg local should have side-table origin metadata");
    assert_eq!(origin.kind, HirLocalOriginKind::CompilerFreshMutableArg);
    assert_eq!(origin.argument_index, Some(0));

    let call_location = origin
        .call_location
        .and_then(|id| builder.side_table.source_location(id))
        .expect("fresh mutable arg local should record originating call location");
    assert_eq!(
        call_location.start_pos.line_number,
        location.start_pos.line_number
    );
}

#[test]
fn lowers_receiver_method_call_with_receiver_as_first_argument() {
    let mut string_table = StringTable::new();
    let method_path = super::symbol("Vector2/reset", &mut string_table);
    let receiver_name = super::symbol("vec", &mut string_table);
    let receiver_struct = super::symbol("Vector2", &mut string_table);
    let location = test_source_location(6);
    let mut builder = setup_builder(&mut string_table);

    builder.test_register_function_name(method_path.clone(), FunctionId(22));

    let receiver_type_id =
        builder.test_register_nominal_struct_type(receiver_struct.clone(), vec![], false);
    builder.test_register_struct_with_fields(
        StructId(21),
        receiver_struct.clone(),
        receiver_type_id,
        vec![],
    );
    register_local(
        &mut builder,
        receiver_name.clone(),
        LocalId(23),
        receiver_type_id,
        location.clone(),
    );

    let method_expression = Expression::method_call_with_typed_arguments(
        inferred_type_reference_expr(
            receiver_name,
            receiver_type_id,
            location.clone(),
            ValueMode::MutableReference,
        ),
        method_path.clone(),
        vec![CallArgument::positional(
            Expression::int(7, location.clone(), ValueMode::ImmutableOwned),
            CallAccessMode::Shared,
            location.clone(),
        )],
        vec![builtin_type_ids::INT],
        &mut builder.type_environment,
        location.clone(),
    );

    let lowered = builder
        .lower_expression(&method_expression)
        .expect("receiver method call lowering should succeed");

    assert_eq!(lowered.prelude.len(), 1);

    match &lowered.prelude[0].kind {
        HirStatementKind::Call { target, args, .. } => {
            assert_eq!(target, &CallTarget::Local(FunctionId(22)));
            assert_eq!(args.len(), 2);
            assert!(matches!(
                args[0].kind,
                HirExpressionKind::Load(HirPlace::Local(LocalId(23)))
            ));
            assert!(matches!(args[1].kind, HirExpressionKind::Int(7)));
        }
        other => panic!("expected lowered receiver call statement, got {other:?}"),
    }
}

#[test]
fn lowers_builtin_scalar_receiver_method_call_with_receiver_as_first_argument() {
    let mut string_table = StringTable::new();
    let method_path = super::symbol("Int/double", &mut string_table);
    let receiver_name = super::symbol("value", &mut string_table);
    let location = test_source_location(12);
    let mut builder = setup_builder(&mut string_table);

    builder.test_register_function_name(method_path.clone(), FunctionId(41));

    register_local(
        &mut builder,
        receiver_name.clone(),
        LocalId(42),
        builtin_type_ids::INT,
        location.clone(),
    );

    let method_expression = Expression::method_call_with_typed_arguments(
        inferred_type_reference_expr(
            receiver_name,
            builtin_type_ids::INT,
            location.clone(),
            ValueMode::ImmutableReference,
        ),
        method_path.clone(),
        vec![],
        vec![builtin_type_ids::INT],
        &mut builder.type_environment,
        location.clone(),
    );

    let lowered = builder
        .lower_expression(&method_expression)
        .expect("builtin scalar receiver method call lowering should succeed");

    assert_eq!(lowered.prelude.len(), 1);
    match &lowered.prelude[0].kind {
        HirStatementKind::Call { target, args, .. } => {
            assert_eq!(target, &CallTarget::Local(FunctionId(41)));
            assert_eq!(args.len(), 1);
            assert!(matches!(
                args[0].kind,
                HirExpressionKind::Load(HirPlace::Local(LocalId(42)))
            ));
        }
        other => panic!("expected lowered builtin scalar receiver call statement, got {other:?}"),
    }
}

#[test]
fn lowers_host_call_expression_with_host_target() {
    let mut string_table = StringTable::new();
    let literal_x = string_table.intern("x");
    let location = test_source_location(7);
    let mut builder = setup_builder(&mut string_table);

    let host_call = Expression::host_function_call(
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine,
        vec![Expression::string_slice(
            literal_x,
            location.clone(),
            ValueMode::ImmutableOwned,
        )],
        vec![builtin_type_ids::INT],
        location.clone(),
    );

    let lowered = builder
        .lower_expression(&host_call)
        .expect("host call lowering should succeed");
    assert_eq!(lowered.prelude.len(), 1);
    let target = match &lowered.prelude[0].kind {
        HirStatementKind::Call { target, .. } => target,
        _ => panic!("expected call statement for host call"),
    };
    assert_eq!(
        target,
        &CallTarget::External(
            crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine
        )
    );
}

#[test]
fn preserves_left_to_right_call_prelude_order_in_nested_call_args() {
    let mut string_table = StringTable::new();
    let first = super::symbol("first", &mut string_table);
    let second = super::symbol("second", &mut string_table);
    let outer = super::symbol("outer", &mut string_table);
    let location = test_source_location(8);
    let mut builder = setup_builder(&mut string_table);

    builder.test_register_function_name(first.clone(), FunctionId(1));
    builder.test_register_function_name(second.clone(), FunctionId(2));
    builder.test_register_function_name(outer.clone(), FunctionId(3));

    let arg_one = Expression::function_call(
        first.clone(),
        vec![],
        vec![builtin_type_ids::INT],
        location.clone(),
    );
    let arg_two = Expression::function_call(
        second.clone(),
        vec![],
        vec![builtin_type_ids::INT],
        location.clone(),
    );
    let outer_call = Expression::function_call(
        outer.clone(),
        vec![arg_one, arg_two],
        vec![builtin_type_ids::INT],
        location,
    );

    let lowered = builder
        .lower_expression(&outer_call)
        .expect("nested call lowering should succeed");

    assert_eq!(lowered.prelude.len(), 3);

    let targets = lowered
        .prelude
        .iter()
        .map(|statement| match &statement.kind {
            HirStatementKind::Call { target, .. } => target.clone(),
            _ => panic!("expected call statement in nested call prelude"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        targets,
        vec![
            CallTarget::Local(FunctionId(1)),
            CallTarget::Local(FunctionId(2)),
            CallTarget::Local(FunctionId(3)),
        ]
    );
}

#[test]
fn malformed_runtime_rpn_reports_hir_transformation_error() {
    let mut string_table = StringTable::new();
    let location = test_source_location(9);
    let mut builder = setup_builder(&mut string_table);

    let expr = runtime_expr(
        vec![runtime_operator_item(Operator::Add, location.clone())],
        builtin_type_ids::INT,
        location,
        ValueMode::MutableOwned,
    );

    let err = builder
        .lower_expression(&expr)
        .expect_err("malformed rpn should fail");
    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(
        err.msg.contains("underflow"),
        "expected stack underflow message, got: {}",
        err.msg
    );
}

#[test]
fn runtime_template_expression_lowers_inline_to_accumulator() {
    let mut string_table = StringTable::new();
    let hello = string_table.intern("hello");
    let location = test_source_location(10);
    let mut builder = setup_builder(&mut string_table);

    let expr = runtime_template_expression(
        location.clone(),
        vec![Expression::string_slice(
            hello,
            location,
            ValueMode::ImmutableOwned,
        )],
        builder.string_table,
    );

    let lowered = builder
        .lower_expression(&expr)
        .expect("runtime template lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert!(builder.module.functions.is_empty());
    assert!(matches!(
        lowered.value.kind,
        HirExpressionKind::Copy(HirPlace::Local(_))
    ));

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    assert!(block_assigns_string_literal(entry_block, "hello"));
}

#[test]
fn runtime_template_handoff_expression_lowers_inline_to_accumulator() {
    let mut string_table = StringTable::new();
    let hello = string_table.intern("hello");
    let location = test_source_location(10);
    let mut builder = setup_builder(&mut string_table);

    let expr = runtime_template_expression(
        location.clone(),
        vec![Expression::string_slice(
            hello,
            location,
            ValueMode::ImmutableOwned,
        )],
        builder.string_table,
    );

    let lowered = builder
        .lower_expression(&expr)
        .expect("owned runtime template handoff expression should lower");

    assert!(lowered.prelude.is_empty());
    assert!(builder.module.functions.is_empty());
    assert!(matches!(
        lowered.value.kind,
        HirExpressionKind::Copy(HirPlace::Local(_))
    ));

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    assert!(block_assigns_string_literal(entry_block, "hello"));
}

#[test]
fn runtime_template_handoff_expression_flattens_nested_linear_handoff() {
    let mut string_table = StringTable::new();
    let before = string_table.intern("before ");
    let inner = string_table.intern("inner");
    let after = string_table.intern(" after");
    let location = test_source_location(10);

    let inner_template = runtime_template_expression(
        location.clone(),
        vec![Expression::string_slice(
            inner,
            location.clone(),
            ValueMode::ImmutableOwned,
        )],
        &string_table,
    );
    let outer_template = runtime_template_expression(
        location.clone(),
        vec![
            Expression::string_slice(before, location.clone(), ValueMode::ImmutableOwned),
            inner_template,
            Expression::string_slice(after, location, ValueMode::ImmutableOwned),
        ],
        &string_table,
    );

    let mut builder = setup_builder(&mut string_table);
    let lowered = builder
        .lower_expression(&outer_template)
        .expect("owned nested runtime template handoffs should lower");

    assert!(lowered.prelude.is_empty());

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    assert_eq!(
        count_empty_string_initializers(entry_block),
        1,
        "nested owned linear handoffs should append into the parent accumulator without creating a child accumulator"
    );
    assert!(block_assigns_string_literal(entry_block, "before "));
    assert!(block_assigns_string_literal(entry_block, "inner"));
    assert!(block_assigns_string_literal(entry_block, " after"));
}

#[test]
fn runtime_template_inline_accumulator_coerces_non_string_segments() {
    let mut string_table = StringTable::new();
    let location = test_source_location(11);
    let mut builder = setup_builder(&mut string_table);

    let expr = runtime_template_expression(
        location.clone(),
        vec![Expression::int(5, location, ValueMode::ImmutableOwned)],
        builder.string_table,
    );

    let lowered = builder
        .lower_expression(&expr)
        .expect("runtime template lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert!(builder.module.functions.is_empty());

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    assert!(block_assigns_coerced_int_chunk(entry_block, 5));
}

#[test]
fn reactive_linear_template_keeps_subscription_chunks_lazy() {
    let mut string_table = StringTable::new();
    let location = test_source_location(12);
    let count_path = InternedPath::from_single_str("count", &mut string_table);
    let count_local = LocalId(24);
    let count_source = ReactiveSource {
        path: count_path.clone(),
        kind: ReactiveSourceKind::Declaration,
    };
    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        count_path.clone(),
        count_local,
        builtin_type_ids::INT,
        location.clone(),
    );
    builder.side_table.bind_reactive_source(HirReactiveSource {
        id: ReactiveSourceId(0),
        local_id: count_local,
        path: count_path.clone(),
        kind: HirReactiveSourceKind::Declaration,
        type_id: builtin_type_ids::INT,
        location: location.clone(),
    });

    let count_expression = inferred_type_reference_expr(
        count_path,
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::ImmutableReference,
    )
    .with_reactive_source(count_source.clone());
    let subscription = ReactiveSubscription {
        source: count_source,
        type_id: builtin_type_ids::INT,
        location: location.clone(),
    };
    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
            children: vec![OwnedRuntimeTemplateNode::DynamicExpression {
                expression: Box::new(count_expression),
                reactive_subscription: Some(subscription),
            }],
        }),
        location: location.clone(),
    };
    let expression = Expression::runtime_template_handoff(handoff, ValueMode::ImmutableOwned);

    let lowered = builder
        .lower_expression(&expression)
        .expect("reactive runtime template should lower lazily");

    assert!(
        expression_contains_load_of_local(&lowered.value, count_local),
        "reactive template snapshot body should reread the subscribed source"
    );

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    assert!(
        entry_block.statements.is_empty(),
        "direct subscription chunks should not be materialized into eager snapshot statements"
    );
}

#[test]
fn runtime_template_lowers_nested_templates_in_order() {
    let mut string_table = StringTable::new();
    let a = string_table.intern("A");
    let b = string_table.intern("B");
    let c = string_table.intern("C");
    let location = test_source_location(12);
    let mut builder = setup_builder(&mut string_table);

    let nested = runtime_template_expression(
        location.clone(),
        vec![Expression::string_slice(
            b,
            location.clone(),
            ValueMode::ImmutableOwned,
        )],
        builder.string_table,
    );

    let expr = runtime_template_expression(
        location.clone(),
        vec![
            Expression::string_slice(a, location.clone(), ValueMode::ImmutableOwned),
            nested,
            Expression::string_slice(c, location, ValueMode::ImmutableOwned),
        ],
        builder.string_table,
    );

    let lowered = builder
        .lower_expression(&expr)
        .expect("nested runtime template lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert!(builder.module.functions.is_empty());

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    assert!(block_assigns_string_literal(entry_block, "A"));
    assert!(block_assigns_string_literal(entry_block, "B"));
    assert!(block_assigns_string_literal(entry_block, "C"));
    // The owned runtime-template handoff flattens nested child templates into the
    // same accumulator, so all three literals are appended directly rather than
    // through an intermediate child-template local.
}

#[test]
fn runtime_template_control_flow_bool_if_lowers_inline_without_helper_call() {
    let mut string_table = StringTable::new();
    let show_name = InternedPath::from_single_str("show", &mut string_table);
    let shown = string_table.intern("shown");
    let hidden = string_table.intern("hidden");
    let location = test_source_location(13);
    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        show_name.clone(),
        LocalId(10),
        builtin_type_ids::BOOL,
        location.clone(),
    );

    let condition = inferred_type_reference_expr(
        show_name,
        builtin_type_ids::BOOL,
        location.clone(),
        ValueMode::ImmutableOwned,
    );
    let then_content = vec![Expression::string_slice(
        shown,
        location.clone(),
        ValueMode::ImmutableOwned,
    )];
    let else_content = vec![Expression::string_slice(
        hidden,
        location.clone(),
        ValueMode::ImmutableOwned,
    )];
    let expr = runtime_template_bool_if_expression(
        condition,
        then_content,
        Some(else_content),
        location,
        builder.string_table,
    );

    let lowered = builder
        .lower_expression(&expr)
        .expect("runtime Bool template if should lower inline");

    assert!(lowered.prelude.is_empty());
    assert!(builder.module.functions.is_empty());
    assert!(matches!(
        lowered.value.kind,
        HirExpressionKind::Copy(HirPlace::Local(_))
    ));

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    assert_eq!(entry_block.statements.len(), 1);

    let (then_block, else_block) = match &entry_block.terminator {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (*then_block, *else_block),
        other => panic!("expected inline template if terminator, got {other:?}"),
    };

    let then_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == then_block)
        .expect("then block should exist");
    let else_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == else_block)
        .expect("else block should exist");

    assert!(block_assigns_string_literal(then_block, "shown"));
    assert!(block_assigns_string_literal(else_block, "hidden"));
}

#[test]
fn runtime_template_control_flow_bool_if_branch_preserves_fallible_propagation_cfg() {
    let mut string_table = StringTable::new();
    let enclosing_name = InternedPath::from_single_str("__expr_test_fn", &mut string_table);
    let can_fail_name = InternedPath::from_single_str("can_fail", &mut string_table);
    let show_name = InternedPath::from_single_str("show", &mut string_table);
    let fallback = string_table.intern("fallback");
    let location = test_source_location(14);
    let mut builder = setup_builder(&mut string_table);

    let enclosing_return_type = result_carrier_type_id(
        &mut builder.type_environment,
        builtin_type_ids::STRING,
        builtin_type_ids::STRING,
    );
    let callee_return_type = result_carrier_type_id(
        &mut builder.type_environment,
        builtin_type_ids::STRING,
        builtin_type_ids::STRING,
    );

    builder.test_register_function_with_return_type(
        enclosing_name,
        FunctionId(0),
        enclosing_return_type,
    );
    builder.test_register_function_with_return_type(
        can_fail_name.clone(),
        FunctionId(7),
        callee_return_type,
    );
    register_local(
        &mut builder,
        show_name.clone(),
        LocalId(11),
        builtin_type_ids::BOOL,
        location.clone(),
    );

    let condition = inferred_type_reference_expr(
        show_name,
        builtin_type_ids::BOOL,
        location.clone(),
        ValueMode::ImmutableOwned,
    );
    let propagated_call = Expression::handled_fallible_function_call_with_typed_arguments(
        can_fail_name,
        vec![],
        vec![builtin_type_ids::STRING],
        FallibleExpressionHandling::Propagate,
        &mut builder.type_environment,
        location.clone(),
    );
    let then_content = vec![propagated_call];
    let else_content = vec![Expression::string_slice(
        fallback,
        location.clone(),
        ValueMode::ImmutableOwned,
    )];
    let expr = runtime_template_bool_if_expression(
        condition,
        then_content,
        Some(else_content),
        location,
        builder.string_table,
    );

    builder
        .lower_expression(&expr)
        .expect("fallible runtime template branch should lower inline");

    assert_eq!(
        builder.module.functions.len(),
        2,
        "control-flow template lowering must not synthesize a helper that could intercept `!`"
    );

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    let then_block_id = match &entry_block.terminator {
        HirTerminator::If { then_block, .. } => *then_block,
        other => panic!("expected inline template if terminator, got {other:?}"),
    };
    let then_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == then_block_id)
        .expect("then block should exist");

    let HirTerminator::FallibleBranch { error_block, .. } = then_block.terminator else {
        panic!("selected template branch should preserve expression-level fallible propagation");
    };
    let error_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == error_block)
        .expect("fallible branch error block should exist");
    assert!(
        matches!(error_block.terminator, HirTerminator::ReturnError(_)),
        "template branch `!` should return through the enclosing function error slot"
    );
}

#[test]
fn runtime_template_control_flow_bool_if_without_else_appends_nothing_on_false_path() {
    let mut string_table = StringTable::new();
    let show_name = InternedPath::from_single_str("show", &mut string_table);
    let shown = string_table.intern("shown");
    let location = test_source_location(14);
    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        show_name.clone(),
        LocalId(11),
        builtin_type_ids::BOOL,
        location.clone(),
    );

    let condition = inferred_type_reference_expr(
        show_name,
        builtin_type_ids::BOOL,
        location.clone(),
        ValueMode::ImmutableOwned,
    );
    let then_content = vec![Expression::string_slice(
        shown,
        location.clone(),
        ValueMode::ImmutableOwned,
    )];
    let expr = runtime_template_bool_if_expression(
        condition,
        then_content,
        None,
        location,
        builder.string_table,
    );

    builder
        .lower_expression(&expr)
        .expect("runtime Bool template if without else should lower inline");

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    let (then_block, else_block) = match &entry_block.terminator {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (*then_block, *else_block),
        other => panic!("expected inline template if terminator, got {other:?}"),
    };

    let then_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == then_block)
        .expect("then block should exist");
    let else_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == else_block)
        .expect("else block should exist");

    assert!(block_assigns_string_literal(then_block, "shown"));
    assert!(
        else_block.statements.is_empty(),
        "false/no-else path should not append to the runtime template accumulator"
    );
}

#[test]
fn runtime_template_control_flow_bool_if_coerces_dynamic_branch_chunks() {
    let mut string_table = StringTable::new();
    let show_name = InternedPath::from_single_str("show", &mut string_table);
    let location = test_source_location(15);
    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        show_name.clone(),
        LocalId(12),
        builtin_type_ids::BOOL,
        location.clone(),
    );

    let condition = inferred_type_reference_expr(
        show_name,
        builtin_type_ids::BOOL,
        location.clone(),
        ValueMode::ImmutableOwned,
    );
    let then_content = vec![Expression::int(
        5,
        location.clone(),
        ValueMode::ImmutableOwned,
    )];
    let expr = runtime_template_bool_if_expression(
        condition,
        then_content,
        None,
        location,
        builder.string_table,
    );

    builder
        .lower_expression(&expr)
        .expect("runtime Bool template if should coerce dynamic branch chunks");

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    let then_block = match &entry_block.terminator {
        HirTerminator::If { then_block, .. } => *then_block,
        other => panic!("expected inline template if terminator, got {other:?}"),
    };
    let then_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == then_block)
        .expect("then block should exist");

    assert!(block_assigns_coerced_int_chunk(then_block, 5));
}

#[test]
fn runtime_template_control_flow_option_capture_lowers_match_and_payload_binding() {
    let mut string_table = StringTable::new();
    let maybe_name = InternedPath::from_single_str("maybe_name", &mut string_table);
    let capture_name = string_table.intern("name");
    let capture_path = InternedPath::from_single_str("name", &mut string_table);
    let hidden = string_table.intern("hidden");
    let location = test_source_location(16);
    let mut builder = setup_builder(&mut string_table);
    let option_string = builder
        .type_environment
        .intern_option(builtin_type_ids::STRING);
    register_local(
        &mut builder,
        maybe_name.clone(),
        LocalId(12),
        option_string,
        location.clone(),
    );

    let scrutinee = inferred_type_reference_expr(
        maybe_name,
        option_string,
        location.clone(),
        ValueMode::ImmutableOwned,
    );
    let then_content = vec![inferred_type_reference_expr(
        capture_path.clone(),
        builtin_type_ids::STRING,
        location.clone(),
        ValueMode::ImmutableReference,
    )];
    let else_content = vec![Expression::string_slice(
        hidden,
        location.clone(),
        ValueMode::ImmutableOwned,
    )];
    let expr = runtime_template_option_capture_expression(
        scrutinee,
        capture_name,
        capture_path,
        builtin_type_ids::STRING,
        then_content,
        Some(else_content),
        location,
        builder.string_table,
    );

    let lowered = builder
        .lower_expression(&expr)
        .expect("runtime option-present template if should lower inline");

    assert!(lowered.prelude.is_empty());
    assert!(builder.module.functions.is_empty());

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    assert_eq!(
        entry_block.statements.len(),
        2,
        "entry block should initialize the accumulator and materialize the option scrutinee once"
    );

    let (present_block_id, absent_block_id) = match &entry_block.terminator {
        HirTerminator::Match { arms, .. } => {
            assert_eq!(arms.len(), 2);
            assert!(matches!(arms[0].pattern, HirPattern::OptionPresent));
            assert!(matches!(arms[1].pattern, HirPattern::OptionNone));
            (arms[0].body, arms[1].body)
        }
        other => panic!("expected option-present template if match terminator, got {other:?}"),
    };

    let present_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == present_block_id)
        .expect("present block should exist");
    let absent_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == absent_block_id)
        .expect("absent block should exist");

    assert!(
        present_block
            .locals
            .iter()
            .any(|local| local.ty == builtin_type_ids::STRING),
        "present branch should register the capture as an ordinary branch local"
    );
    assert!(
        present_block.statements.iter().any(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Assign {
                    target: HirPlace::Local(_),
                    value,
                } if matches!(
                    &value.kind,
                    HirExpressionKind::VariantPayloadGet {
                        carrier: HirVariantCarrier::Option,
                        variant_index: OPTION_SOME_VARIANT_INDEX,
                        field_index: 0,
                        ..
                    }
                )
            )
        }),
        "present branch should assign the option some payload into the capture local"
    );
    assert!(
        absent_block.locals.is_empty(),
        "absent branch must not bind the option capture local"
    );
    assert!(block_assigns_string_literal(absent_block, "hidden"));
}

#[test]
fn runtime_template_control_flow_option_capture_without_else_appends_nothing_when_absent() {
    let mut string_table = StringTable::new();
    let maybe_name = InternedPath::from_single_str("maybe_name", &mut string_table);
    let capture_name = string_table.intern("name");
    let capture_path = InternedPath::from_single_str("name", &mut string_table);
    let location = test_source_location(17);
    let mut builder = setup_builder(&mut string_table);
    let option_string = builder
        .type_environment
        .intern_option(builtin_type_ids::STRING);
    register_local(
        &mut builder,
        maybe_name.clone(),
        LocalId(13),
        option_string,
        location.clone(),
    );

    let scrutinee = inferred_type_reference_expr(
        maybe_name,
        option_string,
        location.clone(),
        ValueMode::ImmutableOwned,
    );
    let then_content = vec![inferred_type_reference_expr(
        capture_path.clone(),
        builtin_type_ids::STRING,
        location.clone(),
        ValueMode::ImmutableReference,
    )];
    let expr = runtime_template_option_capture_expression(
        scrutinee,
        capture_name,
        capture_path,
        builtin_type_ids::STRING,
        then_content,
        None,
        location,
        builder.string_table,
    );

    builder
        .lower_expression(&expr)
        .expect("runtime option-present template if without else should lower inline");

    let entry_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == BlockId(0))
        .expect("entry block should exist");
    let absent_block_id = match &entry_block.terminator {
        HirTerminator::Match { arms, .. } => arms[1].body,
        other => panic!("expected option-present template if match terminator, got {other:?}"),
    };
    let absent_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == absent_block_id)
        .expect("absent block should exist");

    assert!(
        absent_block.statements.is_empty(),
        "absent/no-else branch should not append to the runtime template accumulator"
    );
    assert!(
        absent_block.locals.is_empty(),
        "absent branch must not bind the option capture local"
    );
}

#[test]
fn runtime_template_control_flow_loop_range_lowers_inline_and_wraps_aggregate_when_emitted() {
    let mut string_table = StringTable::new();
    let prefix = string_table.intern("<card>");
    let suffix = string_table.intern("</card>");
    let location = test_source_location(18);
    let limit_path = InternedPath::from_single_str("limit", &mut string_table);
    let item_binding = loop_binding("i", builtin_type_ids::INT, &mut string_table);
    let item_path = item_binding.id.clone();
    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        limit_path.clone(),
        LocalId(20),
        builtin_type_ids::INT,
        location.clone(),
    );
    let body_content = vec![inferred_type_reference_expr(
        item_path,
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::ImmutableReference,
    )];
    let range = RangeLoopSpec {
        start: Expression::int(0, location.clone(), ValueMode::ImmutableOwned),
        end: inferred_type_reference_expr(
            limit_path,
            builtin_type_ids::INT,
            location.clone(),
            ValueMode::ImmutableReference,
        ),
        end_kind: RangeEndKind::Exclusive,
        step: None,
    };
    let expr = runtime_template_range_loop_expression(
        LoopBindings {
            item: Some(item_binding),
            index: None,
        },
        range,
        body_content,
        prefix,
        suffix,
        location,
        builder.string_table,
    );

    builder
        .lower_expression(&expr)
        .expect("runtime range template loop should lower inline");

    assert!(builder.module.functions.is_empty());
    assert!(
        builder.module.blocks.iter().any(block_marks_loop_emitted),
        "range template loop body should mark that at least one iteration emitted"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_assigns_string_literal(block, "<card>")),
        "emitted aggregate should apply the owning head before the aggregate"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_assigns_string_literal(block, "</card>")),
        "emitted aggregate should apply the owning head after the aggregate"
    );
    assert!(
        builder.module.blocks.iter().any(block_appends_local_string),
        "emitted aggregate should append the loop-local aggregate string"
    );
}

#[test]
fn runtime_template_control_flow_loop_collection_materializes_iterable_and_length_once() {
    let mut string_table = StringTable::new();
    let prefix = string_table.intern("");
    let suffix = string_table.intern("");
    let location = test_source_location(19);
    let items_path = InternedPath::from_single_str("items", &mut string_table);
    let item_binding = loop_binding("item", builtin_type_ids::INT, &mut string_table);
    let item_path = item_binding.id.clone();
    let mut builder = setup_builder(&mut string_table);
    let collection_type = builder
        .type_environment
        .intern_collection(builtin_type_ids::INT, None);
    register_local(
        &mut builder,
        items_path.clone(),
        LocalId(21),
        collection_type,
        location.clone(),
    );
    let body_content = vec![inferred_type_reference_expr(
        item_path,
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::ImmutableReference,
    )];
    let iterable = inferred_type_reference_expr(
        items_path,
        collection_type,
        location.clone(),
        ValueMode::ImmutableReference,
    );
    let expr = runtime_template_collection_loop_expression(
        LoopBindings {
            item: Some(item_binding),
            index: None,
        },
        iterable,
        body_content,
        prefix,
        suffix,
        location,
        builder.string_table,
    );

    builder
        .lower_expression(&expr)
        .expect("runtime collection template loop should lower inline");

    let length_calls = builder
        .module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::Call {
                    target: CallTarget::External(ExternalFunctionId::CollectionLength),
                    ..
                }
            )
        })
        .count();

    assert_eq!(
        length_calls, 1,
        "collection template loop should compute iterable length once before iteration"
    );
    assert!(
        builder.module.blocks.iter().any(block_marks_loop_emitted),
        "collection template loop body should mark emitted iterations independently from string length"
    );
}

#[test]
fn runtime_template_control_flow_conditional_loop_rechecks_condition_and_wraps_when_emitted() {
    let mut string_table = StringTable::new();
    let prefix = string_table.intern("<wrap>");
    let suffix = string_table.intern("</wrap>");
    let tick = string_table.intern("tick");
    let location = test_source_location(20);
    let keep_going_path = InternedPath::from_single_str("keep_going", &mut string_table);
    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        keep_going_path.clone(),
        LocalId(22),
        builtin_type_ids::BOOL,
        location.clone(),
    );
    let body_content = vec![Expression::string_slice(
        tick,
        location.clone(),
        ValueMode::ImmutableOwned,
    )];
    let condition = inferred_type_reference_expr(
        keep_going_path,
        builtin_type_ids::BOOL,
        location.clone(),
        ValueMode::ImmutableReference,
    );
    let expr = runtime_template_conditional_loop_expression(
        condition,
        body_content,
        prefix,
        suffix,
        location,
        builder.string_table,
    );

    builder
        .lower_expression(&expr)
        .expect("runtime conditional template loop should lower inline");

    assert!(builder.module.functions.is_empty());
    assert!(
        builder.module.blocks.iter().any(block_marks_loop_emitted),
        "conditional template loop body should mark emitted output when its body has render pieces"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_assigns_string_literal(block, "<wrap>")),
        "emitted aggregate should apply the owning wrapper"
    );
}

#[test]
fn runtime_template_control_flow_loop_empty_body_does_not_mark_iteration_emitted() {
    let mut string_table = StringTable::new();
    let prefix = string_table.intern("<wrap>");
    let suffix = string_table.intern("</wrap>");
    let location = test_source_location(21);
    let limit_path = InternedPath::from_single_str("limit", &mut string_table);
    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        limit_path.clone(),
        LocalId(23),
        builtin_type_ids::INT,
        location.clone(),
    );
    let body_content: Vec<Expression> = vec![];
    let range = RangeLoopSpec {
        start: Expression::int(0, location.clone(), ValueMode::ImmutableOwned),
        end: inferred_type_reference_expr(
            limit_path,
            builtin_type_ids::INT,
            location.clone(),
            ValueMode::ImmutableReference,
        ),
        end_kind: RangeEndKind::Exclusive,
        step: None,
    };
    let expr = runtime_template_range_loop_expression(
        LoopBindings {
            item: None,
            index: None,
        },
        range,
        body_content,
        prefix,
        suffix,
        location,
        builder.string_table,
    );

    builder
        .lower_expression(&expr)
        .expect("runtime empty-body template loop should lower without marking output");

    assert!(
        !builder.module.blocks.iter().any(block_marks_loop_emitted),
        "empty loop bodies should not mark the aggregate as structurally emitted"
    );
}

fn block_assigns_string_literal(block: &HirBlock, expected: &str) -> bool {
    block.statements.iter().any(|statement| {
        let HirStatementKind::Assign { value, .. } = &statement.kind else {
            return false;
        };

        let HirExpressionKind::BinOp {
            op: HirBinOp::StringAppend,
            right,
            ..
        } = &value.kind
        else {
            return false;
        };

        matches!(
            right.kind,
            HirExpressionKind::StringLiteral(ref value) if value == expected
        )
    })
}

fn count_empty_string_initializers(block: &HirBlock) -> usize {
    block
        .statements
        .iter()
        .filter(|statement| {
            let HirStatementKind::Assign { value, .. } = &statement.kind else {
                return false;
            };

            matches!(
                value.kind,
                HirExpressionKind::StringLiteral(ref value) if value.is_empty()
            )
        })
        .count()
}

fn expression_contains_load_of_local(
    expression: &crate::compiler_frontend::hir::expressions::HirExpression,
    expected: LocalId,
) -> bool {
    match &expression.kind {
        HirExpressionKind::Load(HirPlace::Local(local))
        | HirExpressionKind::Copy(HirPlace::Local(local)) => *local == expected,

        HirExpressionKind::BinOp { left, right, .. } => {
            expression_contains_load_of_local(left, expected)
                || expression_contains_load_of_local(right, expected)
        }

        _ => false,
    }
}

fn block_marks_loop_emitted(block: &HirBlock) -> bool {
    block.statements.iter().any(|statement| {
        matches!(
            statement.kind,
            HirStatementKind::Assign {
                value: crate::compiler_frontend::hir::expressions::HirExpression {
                    kind: HirExpressionKind::Bool(true),
                    ..
                },
                ..
            }
        )
    })
}

fn block_appends_local_string(block: &HirBlock) -> bool {
    count_block_appends_local_string(block) > 0
}

fn count_block_appends_local_string(block: &HirBlock) -> usize {
    block
        .statements
        .iter()
        .filter(|statement| {
            let HirStatementKind::Assign { value, .. } = &statement.kind else {
                return false;
            };

            let HirExpressionKind::BinOp {
                op: HirBinOp::StringAppend,
                right,
                ..
            } = &value.kind
            else {
                return false;
            };

            matches!(
                right.kind,
                HirExpressionKind::Load(HirPlace::Local(_))
                    | HirExpressionKind::Copy(HirPlace::Local(_))
            )
        })
        .count()
}

fn block_assigns_coerced_int_chunk(block: &HirBlock, expected: i32) -> bool {
    block.statements.iter().any(|statement| {
        let HirStatementKind::Assign { value, .. } = &statement.kind else {
            return false;
        };

        let HirExpressionKind::BinOp {
            op: HirBinOp::StringAppend,
            right,
            ..
        } = &value.kind
        else {
            return false;
        };

        let HirExpressionKind::BinOp {
            op: HirBinOp::StringAppend,
            left,
            right,
        } = &right.kind
        else {
            return false;
        };

        matches!(
            left.kind,
            HirExpressionKind::StringLiteral(ref value) if value.is_empty()
        ) && matches!(right.kind, HirExpressionKind::Int(value) if value == expected)
    })
}

#[test]
fn local_resolution_uses_full_path_identity_not_leaf_name() {
    let mut string_table = StringTable::new();
    let x_leaf = string_table.intern("x");
    let scope_a = InternedPath::from_single_str("scope_a", &mut string_table);
    let scope_b = InternedPath::from_single_str("scope_b", &mut string_table);
    let local_a = scope_a.append(x_leaf);
    let local_b = scope_b.append(x_leaf);
    let location = test_source_location(10);
    let mut builder = setup_builder(&mut string_table);

    register_local(
        &mut builder,
        local_a,
        LocalId(22),
        builtin_type_ids::INT,
        location.clone(),
    );

    let expr = inferred_type_reference_expr(
        local_b,
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::ImmutableReference,
    );
    let err = builder
        .lower_expression(&expr)
        .expect_err("unregistered full-path symbol should not resolve by leaf name");

    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(err.msg.contains("Unresolved local"));
}

#[test]
fn nominal_struct_identity_uses_field_parent_path() {
    let mut string_table = StringTable::new();
    let location = test_source_location(11);
    let struct_path = super::symbol("MyStruct", &mut string_table);
    let field_path = field_symbol(&struct_path, "value", &mut string_table);
    let mut builder = setup_builder(&mut string_table);
    let int_type = builtin_type_ids::INT;

    builder.test_register_struct_with_fields(
        StructId(1),
        struct_path.clone(),
        int_type,
        vec![(FieldId(3), field_path.clone(), int_type)],
    );

    let struct_type_id = builder.test_register_nominal_struct_type(
        struct_path.clone(),
        vec![(field_path.clone(), int_type, location.clone())],
        false,
    );

    let expr_fields = vec![Declaration {
        id: field_path.clone(),
        value: Expression::int(42, location.clone(), ValueMode::ImmutableOwned),
    }];

    let expression = Expression::struct_instance(
        struct_path.clone(),
        expr_fields.clone(),
        location.clone(),
        ValueMode::MutableOwned,
        false,
        None,
        struct_type_id,
    );

    let lowered = builder
        .lower_expression(&expression)
        .expect("struct instance lowering should succeed");

    match lowered.value.kind {
        HirExpressionKind::StructConstruct { struct_id, fields } => {
            assert_eq!(struct_id, StructId(1));
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].0, FieldId(3));
        }
        other => panic!("expected StructConstruct, got {other:?}"),
    }
}

#[test]
fn rejects_const_record_struct_instance_runtime_lowering() {
    let mut string_table = StringTable::new();
    let location = test_source_location(12);
    let struct_path = super::symbol("Palette", &mut string_table);
    let field_path = field_symbol(&struct_path, "red", &mut string_table);
    let field_value = string_table.intern("red");
    let mut builder = setup_builder(&mut string_table);

    let const_record_type_id = builder.test_register_nominal_struct_type(
        struct_path.clone(),
        vec![(
            field_path.clone(),
            builtin_type_ids::STRING,
            location.clone(),
        )],
        true,
    );

    let expression = Expression::struct_instance(
        struct_path,
        vec![Declaration {
            id: field_path,
            value: Expression::string_slice(
                field_value,
                location.clone(),
                ValueMode::ImmutableOwned,
            ),
        }],
        location.clone(),
        ValueMode::ImmutableOwned,
        true,
        None,
        const_record_type_id,
    );

    let error = builder
        .lower_expression(&expression)
        .expect_err("const record should not lower as a runtime struct construct");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Const record reached runtime HIR struct lowering")
    );
}

#[test]
fn temp_locals_are_not_resolvable_as_user_symbols() {
    let mut string_table = StringTable::new();
    let callee = super::symbol("callee", &mut string_table);
    let temp_name = super::symbol("__hir_tmp_0", &mut string_table);
    let location = test_source_location(12);
    let mut builder = setup_builder(&mut string_table);

    builder.test_register_function_name(callee.clone(), FunctionId(8));

    let call_expr = Expression::function_call(
        callee,
        vec![],
        vec![builtin_type_ids::INT],
        location.clone(),
    );
    let lowered = builder
        .lower_expression(&call_expr)
        .expect("call lowering should succeed");

    assert_eq!(lowered.prelude.len(), 1);
    assert!(matches!(
        lowered.prelude[0].kind,
        HirStatementKind::Call {
            result: Some(_),
            ..
        }
    ));

    let temp_reference = inferred_type_reference_expr(
        temp_name,
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::ImmutableReference,
    );

    let error = builder
        .lower_expression(&temp_reference)
        .expect_err("compiler temp local should not resolve through locals_by_name");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved local"));
}

#[test]
fn field_access_uses_base_struct_identity_not_global_leaf_lookup() {
    let mut string_table = StringTable::new();
    let location = test_source_location(13);
    let struct_a = super::symbol("StructA", &mut string_table);
    let struct_b = super::symbol("StructB", &mut string_table);
    let field_leaf = string_table.intern("value");
    let field_a = struct_a.append(field_leaf);
    let field_b = struct_b.append(field_leaf);
    let local_name = super::symbol("my_struct", &mut string_table);
    let mut builder = setup_builder(&mut string_table);
    let int_type = builtin_type_ids::INT;

    builder.test_register_struct_with_fields(
        StructId(10),
        struct_a.clone(),
        int_type,
        vec![(FieldId(100), field_a.clone(), int_type)],
    );
    builder.test_register_struct_with_fields(
        StructId(11),
        struct_b.clone(),
        int_type,
        vec![(FieldId(101), field_b.clone(), int_type)],
    );

    let local_struct_type_id = builder.test_register_nominal_struct_type(
        struct_a.clone(),
        vec![(field_a.clone(), int_type, location.clone())],
        false,
    );
    register_local(
        &mut builder,
        local_name.clone(),
        LocalId(30),
        local_struct_type_id,
        location.clone(),
    );

    let base_expression = inferred_type_reference_expr(
        local_name,
        local_struct_type_id,
        location.clone(),
        ValueMode::ImmutableReference,
    );

    let field_access = field_access_node(
        base_expression,
        field_leaf,
        builtin_type_ids::INT,
        ConstRecordState::RuntimeValue,
        ValueMode::ImmutableReference,
        location.clone(),
    );

    let (_prelude, place) = builder
        .lower_ast_node_to_place(&field_access)
        .expect("field access should lower via base struct identity");

    match place {
        HirPlace::Field { field, .. } => assert_eq!(field, FieldId(100)),
        other => panic!("expected field place, got {other:?}"),
    }
}

#[test]
fn field_access_from_module_constant_base_materializes_temp_place() {
    let mut string_table = StringTable::new();
    let location = test_source_location(14);
    let format_name = super::symbol("format", &mut string_table);
    let format_struct = super::symbol("Format", &mut string_table);
    let center_leaf = string_table.intern("center");
    let center_field = format_struct.append(center_leaf);
    let center_value = string_table.intern("<div></div>");
    let mut builder = setup_builder(&mut string_table);

    let template_type = builtin_type_ids::STRING;

    builder.test_register_struct_with_fields(
        StructId(20),
        format_struct.clone(),
        template_type,
        vec![(FieldId(200), center_field.clone(), template_type)],
    );

    let format_type_id = builder.test_register_nominal_struct_type(
        format_struct.clone(),
        vec![(
            center_field.clone(),
            builtin_type_ids::STRING,
            location.clone(),
        )],
        false,
    );

    let format_constant = Expression::struct_instance(
        format_struct.clone(),
        vec![Declaration {
            id: center_field.clone(),
            value: Expression::string_slice(
                center_value,
                location.clone(),
                ValueMode::ImmutableOwned,
            ),
        }],
        location.clone(),
        ValueMode::ImmutableOwned,
        false,
        None,
        format_type_id,
    );

    builder.test_register_module_constant(format_name.clone(), format_constant);

    let format_reference = inferred_type_reference_expr(
        format_name,
        format_type_id,
        location.clone(),
        ValueMode::ImmutableReference,
    );
    let field_access = field_access_node(
        format_reference,
        center_leaf,
        builtin_type_ids::STRING,
        ConstRecordState::RuntimeValue,
        ValueMode::ImmutableReference,
        location.clone(),
    );

    let lowered = builder
        .lower_ast_node_as_expression(&field_access)
        .expect("module constant field access should lower");

    assert!(
        lowered
            .prelude
            .iter()
            .any(|statement| matches!(statement.kind, HirStatementKind::Assign { .. })),
        "expected module constant base to be materialized into a temporary local"
    );

    match lowered.value.kind {
        HirExpressionKind::Load(HirPlace::Field { field, base }) => {
            assert_eq!(field, FieldId(200));
            assert!(matches!(*base, HirPlace::Local(_)));
        }
        other => panic!("expected field load expression, got {other:?}"),
    }
}

#[test]
fn const_record_module_constant_field_access_lowers_field_value_without_struct_construct() {
    let mut string_table = StringTable::new();
    let location = test_source_location(15);
    let palette_name = super::symbol("palette", &mut string_table);
    let palette_struct = super::symbol("Palette", &mut string_table);
    let red_leaf = string_table.intern("red");
    let red_field = palette_struct.append(red_leaf);
    let red_value = string_table.intern("red");
    let mut builder = setup_builder(&mut string_table);

    let palette_type_id = builder.test_register_nominal_struct_type(
        palette_struct.clone(),
        vec![(
            red_field.clone(),
            builtin_type_ids::STRING,
            location.clone(),
        )],
        true,
    );

    let palette_constant = Expression::struct_instance(
        palette_struct.clone(),
        vec![Declaration {
            id: red_field,
            value: Expression::string_slice(red_value, location.clone(), ValueMode::ImmutableOwned),
        }],
        location.clone(),
        ValueMode::ImmutableOwned,
        true,
        None,
        palette_type_id,
    );

    builder.test_register_module_constant(palette_name.clone(), palette_constant);

    let palette_reference = const_record_reference_expr(
        palette_name,
        palette_type_id,
        location.clone(),
        ValueMode::ImmutableReference,
    );

    let field_access = field_access_node(
        palette_reference,
        red_leaf,
        builtin_type_ids::STRING,
        ConstRecordState::RuntimeValue,
        ValueMode::ImmutableOwned,
        location.clone(),
    );

    let lowered = builder
        .lower_ast_node_as_expression(&field_access)
        .expect("const-record field access should lower the selected field value");

    assert!(
        lowered.prelude.is_empty(),
        "const-record field access should not materialize the whole record"
    );

    match lowered.value.kind {
        HirExpressionKind::StringLiteral(ref value) if value == "red" => {}
        other => panic!("expected direct string field value, got {other:?}"),
    }
}

#[test]
fn lowers_collection_builtin_host_calls_from_explicit_ast_nodes() {
    let mut string_table = StringTable::new();
    let location = test_source_location(15);
    let receiver_name = super::symbol("values", &mut string_table);
    let get_id = crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionGet;
    let set_id = crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionSet;
    let push_id = crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionPush;
    let remove_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionRemove;
    let length_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionLength;
    let mut builder = setup_builder(&mut string_table);

    let receiver_type_id = builder
        .type_environment
        .intern_collection(builtin_type_ids::INT, None);
    register_local(
        &mut builder,
        receiver_name.clone(),
        LocalId(70),
        receiver_type_id,
        location.clone(),
    );

    let receiver_expression = inferred_type_reference_expr(
        receiver_name,
        receiver_type_id,
        location.clone(),
        ValueMode::MutableReference,
    );

    let fallible_int_result = result_carrier_type_id(
        &mut builder.type_environment,
        builtin_type_ids::INT,
        builtin_type_ids::INT,
    );
    let fallible_none_result = result_carrier_type_id(
        &mut builder.type_environment,
        builtin_type_ids::NONE,
        builtin_type_ids::INT,
    );

    let cases = vec![
        (
            CollectionBuiltinOp::Get,
            vec![CallArgument::positional(
                Expression::int(1, location.clone(), ValueMode::ImmutableOwned),
                CallAccessMode::Shared,
                location.clone(),
            )],
            vec![fallible_int_result],
            get_id,
        ),
        (
            CollectionBuiltinOp::Set,
            vec![
                CallArgument::positional(
                    Expression::int(0, location.clone(), ValueMode::ImmutableOwned),
                    CallAccessMode::Shared,
                    location.clone(),
                ),
                CallArgument::positional(
                    Expression::int(99, location.clone(), ValueMode::ImmutableOwned),
                    CallAccessMode::Shared,
                    location.clone(),
                ),
            ],
            vec![fallible_none_result],
            set_id,
        ),
        (
            CollectionBuiltinOp::Push,
            vec![CallArgument::positional(
                Expression::int(4, location.clone(), ValueMode::ImmutableOwned),
                CallAccessMode::Shared,
                location.clone(),
            )],
            vec![fallible_none_result],
            push_id,
        ),
        (
            CollectionBuiltinOp::Remove,
            vec![CallArgument::positional(
                Expression::int(0, location.clone(), ValueMode::ImmutableOwned),
                CallAccessMode::Shared,
                location.clone(),
            )],
            vec![fallible_int_result],
            remove_id,
        ),
        (
            CollectionBuiltinOp::Length,
            vec![],
            vec![builtin_type_ids::INT],
            length_id,
        ),
    ];

    for (op, args, result_type_ids, expected_id) in cases {
        let expects_result = !result_type_ids.is_empty();
        let receiver_requires_mutable = matches!(
            op,
            CollectionBuiltinOp::Set | CollectionBuiltinOp::Push | CollectionBuiltinOp::Remove
        );
        let call_expression = Expression::collection_builtin_call_with_typed_arguments(
            receiver_expression.clone(),
            op,
            receiver_requires_mutable,
            args,
            result_type_ids,
            &mut builder.type_environment,
            location.clone(),
        );

        let lowered = builder
            .lower_expression(&call_expression)
            .expect("collection builtin call lowering should succeed");

        assert_eq!(lowered.prelude.len(), 1);
        match &lowered.prelude[0].kind {
            HirStatementKind::Call {
                target,
                args,
                result,
            } => {
                assert_eq!(target, &CallTarget::External(expected_id));
                assert_eq!(
                    result.is_some(),
                    expects_result,
                    "{op:?} HIR call result should match its AST result type list"
                );
                assert!(
                    !args.is_empty(),
                    "collection host calls should include receiver as first argument"
                );
            }
            other => panic!("expected host call statement for collection builtin, got {other:?}"),
        }
    }
}

#[test]
fn map_literal_lowering_preserves_entry_order() {
    let mut string_table = StringTable::new();
    let location = test_source_location(37);
    let priya = string_table.intern("Priya");
    let grace = string_table.intern("Grace");
    let mut builder = setup_builder(&mut string_table);
    let map_type = builder
        .type_environment
        .intern_map(builtin_type_ids::STRING, builtin_type_ids::INT);

    let expression = Expression::new(
        ExpressionKind::MapLiteral(vec![
            MapLiteralEntry {
                key: Expression::string_slice(priya, location.clone(), ValueMode::ImmutableOwned),
                value: Expression::int(10, location.clone(), ValueMode::ImmutableOwned),
            },
            MapLiteralEntry {
                key: Expression::string_slice(grace, location.clone(), ValueMode::ImmutableOwned),
                value: Expression::int(12, location.clone(), ValueMode::ImmutableOwned),
            },
        ]),
        location,
        map_type,
        crate::compiler_frontend::datatypes::DataType::Inferred,
        ValueMode::ImmutableOwned,
    );

    let lowered = builder
        .lower_expression(&expression)
        .expect("map literal should lower to first-class HIR");

    assert!(lowered.prelude.is_empty());
    assert_eq!(lowered.value.ty, map_type);

    let HirExpressionKind::MapLiteral(entries) = &lowered.value.kind else {
        panic!("expected HIR map literal, got {:?}", lowered.value.kind);
    };
    assert_eq!(entries.len(), 2);
    assert!(matches!(
        (&entries[0].key.kind, &entries[0].value.kind),
        (HirExpressionKind::StringLiteral(key), HirExpressionKind::Int(10)) if key == "Priya"
    ));
    assert!(matches!(
        (&entries[1].key.kind, &entries[1].value.kind),
        (HirExpressionKind::StringLiteral(key), HirExpressionKind::Int(12)) if key == "Grace"
    ));
}

#[test]
fn map_builtin_calls_lower_to_first_class_hir_ops() {
    let mut string_table = StringTable::new();
    let location = test_source_location(41);
    let scores_name = InternedPath::from_single_str("scores", &mut string_table);
    let key_name = string_table.intern("Priya");
    let mut builder = setup_builder(&mut string_table);
    let map_type = builder
        .type_environment
        .intern_map(builtin_type_ids::STRING, builtin_type_ids::INT);
    register_local(
        &mut builder,
        scores_name.clone(),
        LocalId(80),
        map_type,
        location.clone(),
    );

    let receiver_expression = inferred_type_reference_expr(
        scores_name,
        map_type,
        location.clone(),
        ValueMode::MutableReference,
    );
    let fallible_int_result = result_carrier_type_id(
        &mut builder.type_environment,
        builtin_type_ids::INT,
        builtin_type_ids::INT,
    );
    let fallible_none_result = result_carrier_type_id(
        &mut builder.type_environment,
        builtin_type_ids::NONE,
        builtin_type_ids::INT,
    );
    let key = CallArgument::positional(
        Expression::string_slice(key_name, location.clone(), ValueMode::ImmutableOwned),
        CallAccessMode::Shared,
        location.clone(),
    );

    let cases = vec![
        (
            MapBuiltinOp::Get,
            vec![key.clone()],
            vec![fallible_int_result],
            HirMapOp::Get,
            1,
        ),
        (
            MapBuiltinOp::Set,
            vec![
                key.clone(),
                CallArgument::positional(
                    Expression::int(99, location.clone(), ValueMode::ImmutableOwned),
                    CallAccessMode::Shared,
                    location.clone(),
                ),
            ],
            vec![fallible_none_result],
            HirMapOp::Set,
            2,
        ),
        (
            MapBuiltinOp::Length,
            vec![],
            vec![builtin_type_ids::INT],
            HirMapOp::Length,
            0,
        ),
    ];

    for (op, args, result_type_ids, expected_op, expected_arg_count) in cases {
        let call_expression = Expression::map_builtin_call_with_typed_arguments(
            receiver_expression.clone(),
            op,
            expected_op.requires_mutable_receiver(),
            args,
            result_type_ids.clone(),
            &mut builder.type_environment,
            location.clone(),
        );

        let lowered = builder
            .lower_expression(&call_expression)
            .expect("map builtin call should lower to first-class HIR");

        assert_eq!(lowered.prelude.len(), 1);
        let HirStatementKind::MapOp {
            op,
            receiver,
            args,
            result,
        } = &lowered.prelude[0].kind
        else {
            panic!(
                "expected map op statement, got {:?}",
                lowered.prelude[0].kind
            );
        };
        assert_eq!(*op, expected_op);
        assert_eq!(args.len(), expected_arg_count);
        assert!(result.is_some());
        assert!(
            matches!(
                receiver.kind,
                HirExpressionKind::Load(HirPlace::Local(LocalId(80)))
            ),
            "map receiver should lower as the original local place"
        );

        let result_local = result.expect("map op should produce a result local");
        assert!(
            matches!(lowered.value.kind, HirExpressionKind::Load(HirPlace::Local(local)) if local == result_local),
            "result-bearing map ops should return a load of the result local"
        );
        let local_ty = builder
            .module
            .blocks
            .iter()
            .flat_map(|block| block.locals.iter())
            .find(|local| local.id == result_local)
            .map(|local| local.ty)
            .expect("result local should be registered in the current block");
        assert_eq!(
            local_ty, result_type_ids[0],
            "map op result local should keep the AST-selected result type"
        );
    }
}

/// Verifies that `ExpressionKind::ChoiceConstruct` lowers to `HirExpressionKind::VariantConstruct`
/// with the correct tag index, and that the result type is registered as a choice in
/// `TypeEnvironment`.
///
/// WHY: this is the core contract of the Choice Hardening refactor — choice values must not
/// masquerade as `HirExpressionKind::Int` in HIR.
#[test]
fn lowers_choice_variant_expression_to_hir_variant_construct() {
    let mut string_table = StringTable::new();
    let location = test_source_location(1);

    let status_path = InternedPath::from_single_str("Status", &mut string_table);
    let ready_name = string_table.intern("Ready");
    let busy_name = string_table.intern("Busy");

    let mut builder = setup_builder(&mut string_table);

    let choice_variants = vec![
        ChoiceVariant {
            id: ready_name,
            payload: ChoiceVariantPayload::Unit,
            location: location.clone(),
        },
        ChoiceVariant {
            id: busy_name,
            payload: ChoiceVariantPayload::Unit,
            location: location.clone(),
        },
    ];
    let choice_type_id =
        builder.test_register_nominal_choice_type(status_path.clone(), &choice_variants);
    builder.register_choice_id(&status_path, &location).unwrap();

    let choice_expr = choice_construct_expr(
        status_path.clone(),
        0,
        vec![],
        choice_type_id,
        location.clone(),
        ValueMode::ImmutableOwned,
    );

    let lowered = builder
        .lower_expression(&choice_expr)
        .expect("choice variant lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(lowered.value.value_kind, ValueKind::Const);

    let (choice_id, variant_index) = match &lowered.value.kind {
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Choice { choice_id },
            variant_index,
            fields,
        } => {
            assert!(
                fields.is_empty(),
                "unit variant should have no payload fields"
            );
            (*choice_id, *variant_index)
        }
        other => panic!("expected VariantConstruct, got {other:?}"),
    };

    assert_eq!(variant_index, 0, "expected tag 0 for Ready variant");
    assert_eq!(
        choice_id,
        ChoiceId(0),
        "first choice should receive ChoiceId(0)"
    );

    let hir_type = builder.type_environment.get(lowered.value.ty);
    assert!(
        matches!(
            hir_type,
            Some(TypeDefinition::Choice(ChoiceTypeDefinition {
                id: NominalTypeId(0),
                ..
            }))
        ),
        "expected Choice type with NominalTypeId(0), got {hir_type:?}",
    );
}

#[test]
fn collection_expression_lowering_preserves_fixed_type_identity() {
    let mut string_table = StringTable::new();
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);
    let int_type = builder.type_environment.builtins().int;
    let fixed_collection = builder
        .type_environment
        .intern_collection(int_type, Some(4));

    let expression = Expression::new(
        ExpressionKind::Collection(vec![Expression::int(
            1,
            location.clone(),
            ValueMode::ImmutableOwned,
        )]),
        location,
        fixed_collection,
        crate::compiler_frontend::datatypes::DataType::Inferred,
        ValueMode::ImmutableOwned,
    );

    let lowered = builder
        .lower_expression(&expression)
        .expect("fixed collection expression should lower");

    assert!(
        lowered.prelude.is_empty(),
        "literal collection elements should lower inline"
    );
    assert_eq!(
        lowered.value.ty, fixed_collection,
        "HIR collection expression should preserve the exact AST collection TypeId"
    );

    let shape = builder
        .type_environment
        .collection_shape(lowered.value.ty)
        .expect("lowered expression type should remain a collection");
    assert_eq!(shape.fixed_capacity, Some(4));

    match &lowered.value.kind {
        HirExpressionKind::Collection(elements) => {
            assert_eq!(elements.len(), 1);
            assert_eq!(elements[0].ty, int_type);
        }
        other => panic!("expected HIR collection expression, got {other:?}"),
    }
}

/// Verifies that `ExpressionKind::OptionNone` lowers to `HirExpressionKind::VariantConstruct`
/// with `HirVariantCarrier::Option` and zero fields.
#[test]
fn lowers_option_none_to_hir_variant_construct() {
    let mut string_table = StringTable::new();
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);

    let option_expr = option_none_expr(
        builtin_type_ids::STRING,
        &mut builder.type_environment,
        location.clone(),
    );

    let lowered = builder
        .lower_expression(&option_expr)
        .expect("option none lowering should succeed");

    assert!(lowered.prelude.is_empty());

    match &lowered.value.kind {
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 0,
            fields,
        } => {
            assert!(fields.is_empty(), "Option none should have no fields");
        }
        other => panic!("expected VariantConstruct(Option, 0, []), got {other:?}"),
    }
}

/// Verifies that `ExpressionKind::FallibleCarrierConstruct` lowers to `HirExpressionKind::VariantConstruct`
/// with `HirVariantCarrier::Fallible` and a single value field.
#[test]
fn lowers_fallible_success_to_hir_variant_construct() {
    let mut string_table = StringTable::new();
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);

    let ok_type_id = builtin_type_ids::INT;
    let err_type_id = builtin_type_ids::STRING;
    let result_type_id =
        result_carrier_type_id(&mut builder.type_environment, ok_type_id, err_type_id);

    let value_expr = Expression::int(42, location.clone(), ValueMode::ImmutableOwned);

    let result_expr = Expression::result_construct(
        AstFallibleCarrierVariant::Success,
        value_expr,
        result_type_id,
        location.clone(),
        ValueMode::ImmutableOwned,
    );

    let lowered = builder
        .lower_expression(&result_expr)
        .expect("result ok lowering should succeed");

    assert!(lowered.prelude.is_empty());

    match &lowered.value.kind {
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Fallible,
            variant_index: 0,
            fields,
        } => {
            assert_eq!(fields.len(), 1, "Result Ok should have one field");
            assert!(
                fields[0].name.is_some(),
                "Result Ok field should have a name"
            );
            assert!(
                matches!(fields[0].value.kind, HirExpressionKind::Int(42)),
                "Result Ok field value should be Int(42)"
            );
        }
        other => panic!("expected VariantConstruct(Result, 0, [_]), got {other:?}"),
    }
}

/// Verifies that an `Error` fallible carrier lowers to `HirExpressionKind::VariantConstruct`
/// with `HirVariantCarrier::Fallible` and variant index 1, mirroring the success path.
#[test]
fn lowers_fallible_error_to_hir_variant_construct() {
    let mut string_table = StringTable::new();
    let error_text = string_table.intern("oops");
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);

    let ok_type_id = builtin_type_ids::INT;
    let err_type_id = builtin_type_ids::STRING;
    let result_type_id =
        result_carrier_type_id(&mut builder.type_environment, ok_type_id, err_type_id);

    let value_expr =
        Expression::string_slice(error_text, location.clone(), ValueMode::ImmutableOwned);

    let result_expr = Expression::result_construct(
        AstFallibleCarrierVariant::Error,
        value_expr,
        result_type_id,
        location.clone(),
        ValueMode::ImmutableOwned,
    );

    let lowered = builder
        .lower_expression(&result_expr)
        .expect("result err lowering should succeed");

    assert!(lowered.prelude.is_empty());

    match &lowered.value.kind {
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Fallible,
            variant_index: 1,
            fields,
        } => {
            assert_eq!(fields.len(), 1, "Result Err should have one field");
            assert!(
                fields[0].name.is_some(),
                "Result Err field should have a name"
            );
            assert!(
                matches!(&fields[0].value.kind, HirExpressionKind::StringLiteral(s) if s == "oops"),
                "Result Err field value should be the lowered error string"
            );
        }
        other => panic!("expected VariantConstruct(Result, 1, [_]), got {other:?}"),
    }
}

#[test]
fn external_float_call_emits_validate_float_in_current_block() {
    let mut string_table = StringTable::new();
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);
    let float_type = builder
        .lower_type_id(builtin_type_ids::FLOAT, &location)
        .expect("builtin Float TypeId should lower in test context");

    let call_expr = Expression::host_function_call_with_typed_arguments(
        ExternalFunctionId::Synthetic(0),
        vec![],
        vec![builtin_type_ids::FLOAT],
        &mut builder.type_environment,
        location.clone(),
    );

    let lowered = builder
        .lower_expression_value_to_current_block(&call_expr)
        .expect("external Float call should lower");

    assert_eq!(lowered.ty, float_type);

    let statements = builder.test_current_block_statements();
    assert!(
        statements.iter().any(|statement| matches!(
            &statement.kind,
            HirStatementKind::Call {
                target: CallTarget::External(ExternalFunctionId::Synthetic(0)),
                ..
            }
        )),
        "external Float call should emit the raw Call statement"
    );
    assert!(
        statements
            .iter()
            .any(|statement| matches!(&statement.kind, HirStatementKind::ValidateFloat { .. })),
        "external Float call should emit ValidateFloat after the raw call"
    );
}

#[test]
fn external_float_call_in_builtin_error_function_validates_with_return_error() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("external_float_boundary", &mut string_table);
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);
    let float_type = builder
        .lower_type_id(builtin_type_ids::FLOAT, &location)
        .expect("builtin Float TypeId should lower in test context");
    let int_type = builder
        .lower_type_id(builtin_type_ids::INT, &location)
        .expect("builtin Int TypeId should lower in test context");
    let error_type = builder.test_register_builtin_error_type();
    let return_type = result_carrier_type_id(&mut builder.type_environment, int_type, error_type);

    builder.test_register_function_with_return_type(function_name, FunctionId(98), return_type);
    builder.test_set_current_function(FunctionId(98));

    let call_expr = Expression::host_function_call_with_typed_arguments(
        ExternalFunctionId::Synthetic(1),
        vec![],
        vec![builtin_type_ids::FLOAT],
        &mut builder.type_environment,
        location.clone(),
    );

    let lowered = builder
        .lower_expression_value_to_current_block(&call_expr)
        .expect("external Float call should lower in builtin Error function");

    assert_eq!(lowered.ty, float_type);

    let validate_modes: Vec<_> = builder
        .module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::ValidateFloat { failure_mode, .. } => Some(*failure_mode),
            _ => None,
        })
        .collect();

    assert_eq!(
        validate_modes,
        vec![NumericFailureMode::ReturnError],
        "external Float boundary validation inside builtin Error! functions should be recoverable"
    );
}

#[test]
fn external_int_call_does_not_emit_validate_float() {
    let mut string_table = StringTable::new();
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);

    let call_expr = Expression::host_function_call_with_typed_arguments(
        ExternalFunctionId::Synthetic(0),
        vec![],
        vec![builtin_type_ids::INT],
        &mut builder.type_environment,
        location.clone(),
    );

    let lowered = builder
        .lower_expression(&call_expr)
        .expect("external Int call should lower");

    assert_eq!(lowered.value.ty, builtin_type_ids::INT);

    let prelude_has_validate = lowered
        .prelude
        .iter()
        .any(|statement| matches!(&statement.kind, HirStatementKind::ValidateFloat { .. }));
    assert!(
        !prelude_has_validate,
        "external Int call should not emit ValidateFloat"
    );
}

#[test]
fn external_fallible_float_call_propagation_validates_success() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("fallible_float", &mut string_table);
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);
    let float_type = builder
        .lower_type_id(builtin_type_ids::FLOAT, &location)
        .expect("builtin Float TypeId should lower in test context");
    let error_type = builder.test_register_builtin_error_type();
    let carrier_type =
        result_carrier_type_id(&mut builder.type_environment, float_type, error_type);

    builder.test_register_function_with_return_type(
        function_name.clone(),
        FunctionId(99),
        carrier_type,
    );
    builder.test_set_current_function(FunctionId(99));

    let handled_call_expr =
        Expression::handled_fallible_host_function_call_with_typed_arguments(
            crate::compiler_frontend::ast::expressions::expression::HandledFallibleHostFunctionCallInput {
                id: ExternalFunctionId::Synthetic(1),
                args: vec![],
                result_type_ids: vec![builtin_type_ids::FLOAT],
                error_type_id: error_type,
                handling: FallibleExpressionHandling::Propagate,
                location: location.clone(),
            },
            &mut builder.type_environment,
        );

    let lowered = builder
        .lower_expression_value_to_current_block(&handled_call_expr)
        .expect("fallible external Float call should lower");

    assert_eq!(lowered.ty, float_type);

    let all_statements: Vec<_> = builder
        .module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .collect();
    assert!(
        all_statements.iter().any(|statement| matches!(
            &statement.kind,
            HirStatementKind::Call {
                target: CallTarget::External(ExternalFunctionId::Synthetic(1)),
                ..
            }
        )),
        "fallible external Float call should emit the raw Call statement"
    );
    assert!(
        all_statements
            .iter()
            .any(|statement| matches!(&statement.kind, HirStatementKind::ValidateFloat { .. })),
        "fallible external Float call should validate the unwrapped success Float"
    );
}

#[test]
fn external_fallible_float_call_catch_validates_success() {
    let mut string_table = StringTable::new();
    let function_name = super::symbol("fallible_float_catch", &mut string_table);
    let location = test_source_location(1);
    let mut builder = setup_builder(&mut string_table);
    let float_type = builder
        .lower_type_id(builtin_type_ids::FLOAT, &location)
        .expect("builtin Float TypeId should lower in test context");
    let error_type = builder.test_register_builtin_error_type();
    let carrier_type =
        result_carrier_type_id(&mut builder.type_environment, float_type, error_type);

    builder.test_register_function_with_return_type(
        function_name.clone(),
        FunctionId(100),
        carrier_type,
    );

    let handled_call_expr =
        Expression::handled_fallible_host_function_call_with_typed_arguments(
            crate::compiler_frontend::ast::expressions::expression::HandledFallibleHostFunctionCallInput {
                id: ExternalFunctionId::Synthetic(2),
                args: vec![],
                result_type_ids: vec![builtin_type_ids::FLOAT],
                error_type_id: error_type,
                handling: FallibleExpressionHandling::Propagate,
                location: location.clone(),
            },
            &mut builder.type_environment,
        );

    let catch_expr = wrap_catch_expression(
        handled_call_expr,
        FallibleHandling::Handler {
            error: None,
            body: vec![AstNode {
                kind: NodeKind::ThenValue(ProducedValues {
                    expressions: vec![Expression::float(
                        0.0,
                        location.clone(),
                        ValueMode::ImmutableOwned,
                    )],
                    location: location.clone(),
                }),
                location: location.clone(),
                scope: function_name,
            }],
        },
        vec![builtin_type_ids::FLOAT],
    );

    let lowered = builder
        .lower_expression(&catch_expr)
        .expect("fallible external Float catch should lower");

    assert_eq!(lowered.value.ty, float_type);

    let all_statements: Vec<_> = builder
        .module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .collect();
    assert!(
        all_statements.iter().any(|statement| matches!(
            &statement.kind,
            HirStatementKind::Call {
                target: CallTarget::External(ExternalFunctionId::Synthetic(2)),
                ..
            }
        )),
        "fallible external Float catch should emit the raw Call statement"
    );
    assert!(
        all_statements
            .iter()
            .any(|statement| matches!(&statement.kind, HirStatementKind::ValidateFloat { .. })),
        "fallible external Float catch should validate the unwrapped success Float before merge"
    );
}
