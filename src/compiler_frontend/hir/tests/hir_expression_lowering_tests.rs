//! HIR expression lowering regression tests.
//!
//! WHAT: covers how typed AST expressions become HIR values, preludes, and places.
//! WHY: expression lowering is broad and subtle enough that behavior changes need focused regression tests.

use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, LoopBindings, NodeKind, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValueField, ConstValueMetadata, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode,
};
use crate::compiler_frontend::ast::expressions::expression::{
    CollectionExpressionType, HandledFallibleHostFunctionCallInput, MapLiteralExpressionType,
};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CastFacts, ConstructorField, ExprId, MapEntry, PlaceProjection,
    PlaceRecord,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ConstRecordState, ConstValueKind, FallibleExpressionHandling, FallibleHandling,
    ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::expressions::failure_facts::FailureDisposition;
use crate::compiler_frontend::ast::statements::fallible_handling::wrap_catch_expression;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::ProducedValues;
use crate::compiler_frontend::ast::templates::template::{SlotKey, TemplateType};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::definitions::{ChoiceTypeDefinition, TypeDefinition};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::folded_value::OwnedFoldedString;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::{HirConstructionFailure, HirProjection};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapOp, HirVariantCarrier, OPTION_SOME_VARIANT_INDEX,
    ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::{
    HirBuilder, expressions_to_owned_render_node, register_local, runtime_template_expression,
    setup_builder,
};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{
    ChoiceId, FieldId, FunctionId, HirValueId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    reference_expr_with_type_id, runtime_template_handoff_expr,
};
use crate::compiler_frontend::tests::hir_fixture_support::raw_template_expression_for_hir_invariant;
use crate::compiler_frontend::tests::type_id_fixture_support::{
    choice_construct_expr, const_record_reference_expr, option_none_expr, runtime_expr,
    runtime_operand_item, runtime_operator_item,
};
use crate::compiler_frontend::traits::ids::TraitEvidenceId;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// One exact `I32` value for lowering fixtures.
fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range")
}

/// One exact `I64` value for fixtures that deliberately exceed `i32::MAX`.
fn i64_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I64, value).expect("I64 literal in range")
}

/// One exact `U32` value for lowering fixtures.
fn u32_value(value: u64) -> FixedScalarValue {
    FixedScalarValue::unsigned(FixedScalar::U32, value).expect("U32 literal in range")
}

/// One exact `F64` value for lowering fixtures.
fn f64_value(value: f64) -> FixedScalarValue {
    FixedScalarValue::binary_float(FixedScalar::F64, value)
        .expect("F64 literal is exactly representable")
}

/// One exact `I32` literal expression for lowering fixtures.
fn int_expr(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
    span: Option<SourceSpan>,
) -> ExprId {
    builder
        .fixed_scalar(constants, i32_value(value), span, ValueMode::ImmutableOwned)
        .expect("fixture scalar")
        .id()
}

/// One exact `U32` literal expression for lowering fixtures.
fn uint_expr(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: u64,
    span: Option<SourceSpan>,
) -> ExprId {
    builder
        .fixed_scalar(constants, u32_value(value), span, ValueMode::ImmutableOwned)
        .expect("fixture scalar")
        .id()
}

/// One exact `F64` literal expression for lowering fixtures.
fn float_expr(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: f64,
    span: Option<SourceSpan>,
) -> ExprId {
    builder
        .fixed_scalar(constants, f64_value(value), span, ValueMode::ImmutableOwned)
        .expect("fixture scalar")
        .id()
}

fn field_symbol(
    parent: &PathId,
    field_name: &str,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> PathId {
    path_fork
        .try_intern_child(*parent, string_table.intern(field_name))
        .expect("test path fits")
}

/// Builds the aggregate-wrapper node for a loop's owning head wrapper.
///
/// WHAT: a `Sequence` of prefix `Text`, `AggregateOutput`, suffix `Text`.
/// WHY: keeping this boundary shape local makes the aggregate splice explicit.
fn text_aggregate_wrapper_node(
    prefix: StringId,
    suffix: StringId,
    string_table: &StringTable,
    span: &Option<SourceSpan>,
) -> OwnedRuntimeTemplateNode {
    OwnedRuntimeTemplateNode::Sequence {
        children: vec![
            OwnedRuntimeTemplateNode::Text {
                text: OwnedFoldedString::Text(string_table.resolve(prefix).to_owned()),
            },
            OwnedRuntimeTemplateNode::AggregateOutput,
            OwnedRuntimeTemplateNode::Text {
                text: OwnedFoldedString::Text(string_table.resolve(suffix).to_owned()),
            },
        ],
        span: span.to_owned(),
    }
}

fn runtime_template_bool_if_expression(
    builder: &mut AstExpressionBuilder,
    constants: &ConstValueStore,
    condition: ExprId,
    then_content: Vec<ExprId>,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> ExprId {
    let body = OwnedRuntimeTemplateNode::Conditional {
        selector: Box::new(TemplateBranchSelector::Bool(condition)),
        body: Box::new(expressions_to_owned_render_node(
            builder,
            constants,
            &then_content,
            string_table,
        )),
        span,
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(body),
    };

    runtime_template_handoff_expr(builder, handoff, span, ValueMode::ImmutableOwned)
}

#[allow(
    clippy::too_many_arguments,
    reason = "test fixture keeps scrutinee, capture name and path, inner type, body and spans separate"
)]
fn runtime_template_option_capture_expression(
    builder: &mut AstExpressionBuilder,
    constants: &ConstValueStore,
    scrutinee: ExprId,
    capture_name: crate::compiler_frontend::symbols::string_interning::StringId,
    capture_path: PathId,
    inner_type_id: TypeId,
    then_content: Vec<ExprId>,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> ExprId {
    let body = OwnedRuntimeTemplateNode::Conditional {
        selector: Box::new(TemplateBranchSelector::OptionPresentCapture {
            scrutinee,
            pattern: Box::new(MatchPattern::OptionPresentCapture {
                name: capture_name,
                binding_path: capture_path,
                inner_type_id,
                span,
                binding_span: span,
            }),
        }),
        body: Box::new(expressions_to_owned_render_node(
            builder,
            constants,
            &then_content,
            string_table,
        )),
        span,
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(body),
    };

    runtime_template_handoff_expr(builder, handoff, span, ValueMode::ImmutableOwned)
}

#[allow(
    clippy::too_many_arguments,
    reason = "test fixture keeps bindings, range, body, aggregate prefix, suffix and span separate"
)]
fn runtime_template_range_loop_expression(
    builder: &mut AstExpressionBuilder,
    constants: &ConstValueStore,
    bindings: LoopBindings,
    range: RangeLoopSpec,
    body_content: Vec<ExprId>,
    aggregate_prefix: StringId,
    aggregate_suffix: StringId,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> ExprId {
    let body = expressions_to_owned_render_node(builder, constants, &body_content, string_table);
    let aggregate_wrapper =
        text_aggregate_wrapper_node(aggregate_prefix, aggregate_suffix, string_table, &span);

    let node = OwnedRuntimeTemplateNode::Loop {
        header: TemplateLoopHeader::Range {
            bindings: Box::new(bindings),
            range: Box::new(range),
        },
        body: Box::new(body),
        aggregate_wrapper: Some(Box::new(aggregate_wrapper)),
        span,
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(node),
    };

    runtime_template_handoff_expr(builder, handoff, span, ValueMode::ImmutableOwned)
}

#[allow(
    clippy::too_many_arguments,
    reason = "test fixture keeps bindings, iterable, body, aggregate prefix, suffix and span separate"
)]
fn runtime_template_collection_loop_expression(
    builder: &mut AstExpressionBuilder,
    constants: &ConstValueStore,
    bindings: LoopBindings,
    iterable: ExprId,
    body_content: Vec<ExprId>,
    aggregate_prefix: StringId,
    aggregate_suffix: StringId,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> ExprId {
    let body = expressions_to_owned_render_node(builder, constants, &body_content, string_table);
    let aggregate_wrapper =
        text_aggregate_wrapper_node(aggregate_prefix, aggregate_suffix, string_table, &span);

    let node = OwnedRuntimeTemplateNode::Loop {
        header: TemplateLoopHeader::Collection {
            bindings: Box::new(bindings),
            iterable,
        },
        body: Box::new(body),
        aggregate_wrapper: Some(Box::new(aggregate_wrapper)),
        span,
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(node),
    };

    runtime_template_handoff_expr(builder, handoff, span, ValueMode::ImmutableOwned)
}

#[allow(
    clippy::too_many_arguments,
    reason = "test fixture keeps condition, body, aggregate prefix, suffix and span separate"
)]
fn runtime_template_conditional_loop_expression(
    builder: &mut AstExpressionBuilder,
    constants: &ConstValueStore,
    condition: ExprId,
    body_content: Vec<ExprId>,
    aggregate_prefix: StringId,
    aggregate_suffix: StringId,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> ExprId {
    let body = expressions_to_owned_render_node(builder, constants, &body_content, string_table);
    let aggregate_wrapper =
        text_aggregate_wrapper_node(aggregate_prefix, aggregate_suffix, string_table, &span);

    let node = OwnedRuntimeTemplateNode::Loop {
        header: TemplateLoopHeader::Conditional { condition },
        body: Box::new(body),
        aggregate_wrapper: Some(Box::new(aggregate_wrapper)),
        span,
    };

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(node),
    };

    runtime_template_handoff_expr(builder, handoff, span, ValueMode::ImmutableOwned)
}

fn loop_binding(
    name: &str,
    type_id: TypeId,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Declaration {
    Declaration {
        id: path_fork
            .try_intern_portable_path(name, string_table)
            .expect("test path fits"),
        type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    }
}
fn declared_entry_block<'b>(builder: &'b HirBuilder<'_>) -> &'b HirBlock {
    let entry_id = super::start_function(&builder.module).entry;
    builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == entry_id)
        .unwrap_or_else(|| panic!("declared HIR entry block {entry_id:?} should exist"))
}

fn expression_row<'a>(builder: &'a HirBuilder<'_>, value: HirValueId) -> &'a HirExpression {
    let row: &'a HirExpression = builder.module.expressions.expression(value);
    row
}

fn expect_infrastructure_error(failure: HirConstructionFailure) -> CompilerError {
    match failure {
        HirConstructionFailure::Infrastructure(error) => error,
        HirConstructionFailure::Diagnosed(diagnostic) => {
            panic!("expected infrastructure failure, got diagnosed failure: {diagnostic:?}")
        }
    }
}

fn assert_no_synthesized_helper_functions(builder: &HirBuilder<'_>) {
    let _path_fork = super::PathInternerFork::empty();
    assert_eq!(
        builder.module.functions.len(),
        1,
        "runtime-template lowering should retain only its declared start function"
    );
    let start_function = builder
        .module
        .start_function
        .expect("expression fixture should declare a start function");
    assert!(
        builder
            .module
            .functions
            .iter()
            .any(|function| function.id == start_function),
        "the sole expression-fixture function should be its declared start function"
    );
}

#[test]
fn runtime_template_slot_placeholder_materializes_as_no_output_owned_node() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let before = string_table.intern("before ");
    let after = string_table.intern("after");
    let span = None;

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
            children: vec![
                OwnedRuntimeTemplateNode::Text {
                    text: OwnedFoldedString::Text(string_table.resolve(before).to_owned()),
                },
                OwnedRuntimeTemplateNode::Slot { span },
                OwnedRuntimeTemplateNode::Text {
                    text: OwnedFoldedString::Text(string_table.resolve(after).to_owned()),
                },
            ],
            span,
        }),
    };
    let body = match &handoff.body {
        crate::compiler_frontend::ast::templates::OwnedRuntimeTemplateBody::Render(node) => node,
        _ => panic!("slot-shaped template should have a render handoff body"),
    };
    assert!(
        matches!(body, OwnedRuntimeTemplateNode::Sequence { children, .. } if children.iter().any(|child| matches!(child, OwnedRuntimeTemplateNode::Slot { .. }))),
        "slot-shaped template handoff should contain a no-output Slot node"
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let expr =
        runtime_template_handoff_expr(&mut expressions, handoff, span, ValueMode::ImmutableOwned);
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime template expression should have a view"),
        )
        .expect("slot-shaped runtime templates should lower in HIR");

    assert!(lowered.prelude.is_empty());
    assert_no_synthesized_helper_functions(&builder);
    let entry_block = declared_entry_block(&builder);
    assert!(block_assigns_string_literal(
        &builder.module,
        entry_block,
        "before "
    ));
    assert!(block_assigns_string_literal(
        &builder.module,
        entry_block,
        "after"
    ));
}

#[test]
fn escaped_slot_insert_helpers_fail_when_they_reach_hir_runtime_lowering() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let body_slot = string_table.intern("body");
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();

    let (helper, _helper_registry) = raw_template_expression_for_hir_invariant(
        &mut expressions,
        TemplateType::SlotInsert(SlotKey::named(body_slot)),
        span,
        ValueMode::ImmutableOwned,
    );

    let err = expect_infrastructure_error(
        builder
            .lower_expression(
                expressions
                    .view(helper)
                    .expect("escaped helper template should have a view"),
            )
            .expect_err("escaped helper templates should be rejected in HIR"),
    );

    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(
        err.msg
            .contains("Raw template reached HIR runtime-template lowering")
    );
}

#[test]
fn escaped_slot_definition_helpers_fail_when_they_reach_hir_runtime_lowering() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let body_slot = string_table.intern("body");
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let (helper, _helper_registry) = raw_template_expression_for_hir_invariant(
        &mut expressions,
        TemplateType::SlotDefinition(SlotKey::named(body_slot)),
        span,
        ValueMode::ImmutableOwned,
    );

    let err = expect_infrastructure_error(
        builder
            .lower_expression(
                expressions
                    .view(helper)
                    .expect("escaped slot definition helper should have a view"),
            )
            .expect_err("escaped slot definition helpers should be rejected in HIR"),
    );

    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(
        err.msg
            .contains("Raw template reached HIR runtime-template lowering")
    );
}

#[test]
fn runtime_template_without_handoff_reports_compiler_bug() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let (template, _template_registry) = raw_template_expression_for_hir_invariant(
        &mut expressions,
        TemplateType::StringFunction,
        span,
        ValueMode::ImmutableOwned,
    );

    let err = expect_infrastructure_error(
        builder
            .lower_expression(
                expressions
                    .view(template)
                    .expect("raw template expression should have a view"),
            )
            .expect_err("runtime templates without an owned handoff should fail"),
    );

    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(
        err.msg
            .contains("Raw template reached HIR runtime-template lowering")
    );
}

#[test]
fn lowers_primitive_literals() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let text = string_table.intern("hello");
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();

    let int = int_expr(&mut expressions, &mut builder.module_const_values, 42, span);
    let int_lowered = builder
        .lower_expression(
            expressions
                .view(int)
                .expect("int literal should have a view"),
        )
        .expect("int lowering should succeed");
    assert!(int_lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, int_lowered.value).value_kind,
        ValueKind::Const
    );
    assert!(matches!(
        &expression_row(&builder, int_lowered.value).kind,
        HirExpressionKind::FixedScalar(value) if *value == i32_value(42)
    ));

    let float = float_expr(
        &mut expressions,
        &mut builder.module_const_values,
        3.25,
        span,
    );
    let float_lowered = builder
        .lower_expression(
            expressions
                .view(float)
                .expect("float literal should have a view"),
        )
        .expect("float lowering should succeed");
    assert!(float_lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, float_lowered.value).value_kind,
        ValueKind::Const
    );
    assert!(matches!(
        &expression_row(&builder, float_lowered.value).kind,
        HirExpressionKind::FixedScalar(value) if *value == f64_value(3.25)
    ));

    let bool_expr = expressions
        .bool(
            &mut builder.module_const_values,
            true,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture bool");
    let bool_lowered = builder
        .lower_expression(
            expressions
                .view_owned(bool_expr)
                .expect("bool literal should have a view"),
        )
        .expect("bool lowering should succeed");
    assert!(bool_lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, bool_lowered.value).value_kind,
        ValueKind::Const
    );
    assert!(matches!(
        &expression_row(&builder, bool_lowered.value).kind,
        HirExpressionKind::Bool(true)
    ));

    let char_expr = expressions
        .char(
            &mut builder.module_const_values,
            'x',
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture char");
    let char_lowered = builder
        .lower_expression(
            expressions
                .view_owned(char_expr)
                .expect("char literal should have a view"),
        )
        .expect("char lowering should succeed");
    assert!(char_lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, char_lowered.value).value_kind,
        ValueKind::Const
    );
    assert!(matches!(
        &expression_row(&builder, char_lowered.value).kind,
        HirExpressionKind::Char('x')
    ));

    let string_expr = expressions
        .string_slice(
            &mut builder.module_const_values,
            text,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string");
    let string_lowered = builder
        .lower_expression(
            expressions
                .view_owned(string_expr)
                .expect("string literal should have a view"),
        )
        .expect("string literal lowering should succeed");
    assert!(string_lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, string_lowered.value).value_kind,
        ValueKind::Const
    );
    assert!(matches!(
        &expression_row(&builder, string_lowered.value).kind,
        HirExpressionKind::StringLiteral(s) if s == "hello"
    ));
}

#[test]
fn lowers_int_above_i32_range_without_truncation() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    // Above `i32::MAX` so an `i32` carrier would truncate; the `i64` carrier preserves it.
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let wide = expressions
        .fixed_scalar(
            &mut builder.module_const_values,
            i64_value(3_000_000_000),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar");
    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(wide)
                .expect("wide int should have a view"),
        )
        .expect("wide int lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, lowered.value).value_kind,
        ValueKind::Const
    );
    assert!(matches!(
        &expression_row(&builder, lowered.value).kind,
        HirExpressionKind::FixedScalar(value) if *value == i64_value(3_000_000_000)
    ));
}

#[test]
fn lowers_reference_to_registered_local() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    register_local(
        &mut builder,
        x,
        LocalId(10),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let expr = reference_expr_with_type_id(
        &mut expressions,
        x,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    );
    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("reference should have a view"),
        )
        .expect("reference lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, lowered.value).value_kind,
        ValueKind::Place
    );
    assert!(matches!(
        &expression_row(&builder, lowered.value).kind,
        HirExpressionKind::Load(place)
            if place.root == LocalId(10) && super::is_local_place(place)
    ));
}

#[test]
fn lowers_reference_to_module_constant_when_local_is_missing() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let third_const = super::symbol("third_const", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let three = constants
        .insert_resolved(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ConstValuePayload::FixedScalar(i32_value(3)),
            None,
        )
        .expect("module constant value should be resolved");
    builder.module_const_values = std::mem::take(&mut constants);
    builder.test_register_module_constant(third_const, three);

    let expr = reference_expr_with_type_id(
        &mut expressions,
        third_const,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    );
    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("module constant reference should have a view"),
        )
        .expect("module constant reference lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, lowered.value).value_kind,
        ValueKind::Const
    );
    assert!(matches!(
        &expression_row(&builder, lowered.value).kind,
        HirExpressionKind::FixedScalar(value) if *value == i32_value(3)
    ));
}

#[test]
fn lowers_runtime_rpn_arithmetic_stack_correctly() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let y = super::symbol("y", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();

    register_local(
        &mut builder,
        x,
        LocalId(10),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
    );
    register_local(
        &mut builder,
        y,
        LocalId(11),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
    );

    let x_ref = reference_expr_with_type_id(
        &mut expressions,
        x,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    );
    let two = int_expr(&mut expressions, &mut builder.module_const_values, 2, span);
    let y_ref = reference_expr_with_type_id(
        &mut expressions,
        y,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    );

    let items = vec![
        runtime_operand_item(x_ref),
        runtime_operand_item(two),
        runtime_operand_item(y_ref),
        runtime_operator_item(Operator::Multiply, span),
        runtime_operator_item(Operator::Add, span),
    ];

    let expr = runtime_expr(
        &mut expressions,
        &mut builder.module_const_values,
        items,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime arithmetic should have a view"),
        )
        .expect("runtime arithmetic lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, lowered.value).ty,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert!(
        matches!(
            &expression_row(&builder, lowered.value).kind,
            HirExpressionKind::Load(place) if super::is_local_place(place)
        ),
        "checked integer addition should return a load of the NumericOp result local"
    );
}

#[test]
fn runtime_division_subexpression_infers_float_type_in_hir() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let expected_float = builtin_type_ids::fixed_scalar(FixedScalar::F64);

    let five = int_expr(&mut expressions, &mut builder.module_const_values, 5, span);
    let two = int_expr(&mut expressions, &mut builder.module_const_values, 2, span);
    let one = float_expr(
        &mut expressions,
        &mut builder.module_const_values,
        1.0,
        span,
    );
    let items = vec![
        runtime_operand_item(five),
        runtime_operand_item(two),
        runtime_operator_item(Operator::Divide, span),
        // The division result is `F64`, so the literal addend materialises at that
        // receiving type instead of mixing an `I32` literal with an `F64` operand.
        runtime_operand_item(one),
        runtime_operator_item(Operator::Add, span),
    ];

    let expr = runtime_expr(
        &mut expressions,
        &mut builder.module_const_values,
        items,
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        span,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime division should have a view"),
        )
        .expect("runtime division lowering should succeed");

    assert_eq!(expression_row(&builder, lowered.value).ty, expected_float);
    assert!(
        matches!(
            &expression_row(&builder, lowered.value).kind,
            HirExpressionKind::Load(place) if super::is_local_place(place)
        ),
        "checked addition should return a load of the NumericOp result local"
    );
}

#[test]
fn runtime_integer_division_lowers_to_hir_int_div_with_int_type() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let expected_int = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let five = int_expr(&mut expressions, &mut builder.module_const_values, 5, span);
    let two = int_expr(&mut expressions, &mut builder.module_const_values, 2, span);
    let items = vec![
        runtime_operand_item(five),
        runtime_operand_item(two),
        runtime_operator_item(Operator::IntDivide, span),
    ];

    let expr = runtime_expr(
        &mut expressions,
        &mut builder.module_const_values,
        items,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime integer division should have a view"),
        )
        .expect("runtime integer division lowering should succeed");

    assert_eq!(expression_row(&builder, lowered.value).ty, expected_int);
    assert!(
        matches!(
            &expression_row(&builder, lowered.value).kind,
            HirExpressionKind::Load(place) if super::is_local_place(place)
        ),
        "checked integer division should return a load of the NumericOp result local"
    );
}

#[test]
fn lowers_unary_not_in_runtime_rpn() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    // Authored operator spans for `not (not (not true))`, ordered outermost first.
    let mut extended_spans = ExtendedSpanBuilder::new();
    let not_spans = [0, 5, 10].map(|start| {
        Some(SourceSpan::new(
            SourceId::from_index(2),
            LocalSpan::exact(start, 3, &mut extended_spans).expect("not operator span should fit"),
        ))
    });
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();

    let truth = expressions
        .bool(
            &mut builder.module_const_values,
            true,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("bool literal should construct");
    let items = vec![
        runtime_operand_item(truth.id()),
        runtime_operator_item(Operator::Not, not_spans[2]),
        runtime_operator_item(Operator::Not, not_spans[1]),
        runtime_operator_item(Operator::Not, not_spans[0]),
    ];

    let expr = runtime_expr(
        &mut expressions,
        &mut builder.module_const_values,
        items,
        builtin_type_ids::BOOL,
        span,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("unary not should have a view"),
        )
        .expect("unary not lowering should succeed");

    let mut value = lowered.value;
    for expected_span in not_spans {
        let row = expression_row(&builder, value);
        assert_eq!(row.ty, builtin_type_ids::BOOL);
        assert_eq!(row.span, expected_span);
        assert_eq!(builder.side_table.value_source_span(value), expected_span);
        let HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Not,
            operand,
        } = &row.kind
        else {
            panic!("each authored not should retain its own HIR unary row");
        };
        assert_ne!(value, *operand, "not should have a separate operand row");
        value = *operand;
    }
    assert!(matches!(
        &expression_row(&builder, value).kind,
        HirExpressionKind::Bool(true)
    ));
}

#[test]
fn lowers_range_operator_in_runtime_rpn() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();

    let one = int_expr(&mut expressions, &mut builder.module_const_values, 1, span);
    let nine = int_expr(&mut expressions, &mut builder.module_const_values, 9, span);
    let items = vec![
        runtime_operand_item(one),
        runtime_operand_item(nine),
        runtime_operator_item(Operator::Range, span),
    ];

    let expr = runtime_expr(
        &mut expressions,
        &mut builder.module_const_values,
        items,
        builtin_type_ids::RANGE,
        span,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    let lowered = builder
        .lower_expression(expressions.view(expr).expect("range should have a view"))
        .expect("range lowering should succeed");

    assert!(matches!(
        &expression_row(&builder, lowered.value).kind,
        HirExpressionKind::Range { .. }
    ));
}

#[test]
fn lowers_function_call_to_call_statement_and_temp_load() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("sum", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_function_name(function_name, FunctionId(2));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let seven = int_expr(&mut expressions, &mut builder.module_const_values, 7, span);
    let call_expr = expressions
        .function_call_with_typed_arguments(
            function_name,
            vec![CallArgument::positional(
                expressions.own(seven),
                CallAccessMode::Shared,
                span,
            )],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            span,
        )
        .expect("function call should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(call_expr)
                .expect("call expression should have a view"),
        )
        .expect("function call lowering should succeed");
    assert_eq!(lowered.prelude.len(), 1);

    let statement = &lowered.prelude[0];
    let result_local = match &statement.kind {
        HirStatementKind::Call {
            target,
            args,
            results,
        } => {
            assert_eq!(target, &CallTarget::Local(FunctionId(2)));
            assert_eq!(args.len(), 1);
            let [HirLocalDestination::Define(local)] = results.as_slice() else {
                panic!("call result must define one fresh caller-local binding");
            };
            *local
        }
        _ => panic!("expected lowered call statement"),
    };

    assert!(matches!(
        &expression_row(&builder, lowered.value).kind,
        HirExpressionKind::Load(place)
            if place.root == result_local && super::is_local_place(place)
    ));
    assert_eq!(
        expression_row(&builder, lowered.value).value_kind,
        ValueKind::RValue
    );
}

#[test]
fn expression_function_call_uses_success_shape_for_single_return() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("typed_result", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_function_name(function_name, FunctionId(32));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call_expr = expressions
        .function_call_with_typed_arguments(
            function_name,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            span,
        )
        .expect("single-result call should construct");
    let call_view = expressions
        .view_owned(call_expr)
        .expect("call expression should have a view");
    assert_eq!(
        call_view
            .value_type_id()
            .expect("single-result call should have one value type"),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_eq!(
        call_view.success_types(),
        &[builtin_type_ids::fixed_scalar(FixedScalar::I32)]
    );

    let expected_int = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::I32), &span)
        .expect("fixed I32 TypeId should lower in test context");
    let lowered = builder
        .lower_expression(call_view)
        .expect("function call lowering should use the expression success shape");

    assert_eq!(expression_row(&builder, lowered.value).ty, expected_int);
}

#[test]
fn expression_function_call_uses_success_shape_for_no_return() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("no_result", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_function_name(function_name, FunctionId(33));
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call_expr = expressions
        .function_call_with_typed_arguments(function_name, vec![], vec![], span)
        .expect("zero-result call should construct");
    assert!(
        expressions
            .view_owned(call_expr)
            .expect("call expression should have a view")
            .success_types()
            .is_empty()
    );

    let lowered = builder
        .lower_expression_results(
            expressions
                .view_owned(call_expr)
                .expect("call expression should have a view"),
        )
        .expect("no-return call lowering should use the zero-success shape");
    assert!(
        lowered.values.is_empty(),
        "zero-success calls produce no value"
    );
}

#[test]
fn expression_function_call_uses_success_shape_for_multi_return() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("multi_result", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_function_name(function_name, FunctionId(34));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call_expr = expressions
        .function_call_with_typed_arguments(
            function_name,
            vec![],
            vec![
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                builtin_type_ids::BOOL,
            ],
            span,
        )
        .expect("multi-result call should construct");
    assert_eq!(
        expressions
            .view_owned(call_expr)
            .expect("call expression should have a view")
            .success_types(),
        &[
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            builtin_type_ids::BOOL,
        ]
    );

    let lowered = builder
        .lower_expression_results(
            expressions
                .view_owned(call_expr)
                .expect("call expression should have a view"),
        )
        .expect("multi-return call lowering should retain the expression success slots");
    let int_type = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::I32), &span)
        .expect("fixed I32 TypeId should lower in test context");
    let bool_type = builder
        .lower_type_id(builtin_type_ids::BOOL, &span)
        .expect("builtin Bool TypeId should lower in test context");
    assert_eq!(lowered.values.len(), 2);
    assert_eq!(expression_row(&builder, lowered.values[0]).ty, int_type);
    assert_eq!(expression_row(&builder, lowered.values[1]).ty, bool_type);
}

#[test]
fn forwarded_multiple_return_wraps_each_optional_slot_without_repeating_call() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("forward_pair", &mut path_fork, &mut string_table);
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_function_name(function_name, FunctionId(34));
    let integer_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let producer = expressions
        .function_call_with_typed_arguments(
            function_name,
            vec![],
            vec![integer_type, builtin_type_ids::BOOL],
            None,
        )
        .expect("the producer call should construct");
    let lowered = builder
        .lower_expression_results(
            expressions
                .view_owned(producer)
                .expect("producer call should have a view"),
        )
        .expect("the producer should execute once");
    assert_eq!(lowered.prelude.len(), 1);
    assert!(
        matches!(&lowered.prelude[0].kind, HirStatementKind::Call { results, .. } if results.len() == 2)
    );
    let optional_types = [
        builder.type_environment.intern_option(integer_type),
        builder
            .type_environment
            .intern_option(builtin_type_ids::BOOL),
    ];
    assert_eq!(lowered.values.len(), optional_types.len());
    for (value, expected_type) in lowered.values.into_iter().zip(optional_types) {
        let wrapped = builder
            .wrap_value_to_optional_type(value, expected_type, &None)
            .expect("validated forwarding should wrap each slot");
        assert_eq!(expression_row(&builder, wrapped).ty, expected_type);
        assert!(matches!(
            &expression_row(&builder, wrapped).kind,
            HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: OPTION_SOME_VARIANT_INDEX,
                ..
            }
        ));
    }
}

#[test]
fn expression_host_call_uses_success_shape() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call_expr = expressions
        .host_function_call_with_typed_arguments(
            ExternalFunctionId::IoLine,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            false,
            span,
        )
        .expect("host call should construct");
    let call_view = expressions
        .view_owned(call_expr)
        .expect("host call should have a view");
    assert_eq!(
        call_view.success_types(),
        &[builtin_type_ids::fixed_scalar(FixedScalar::I32)]
    );

    let expected_int = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::I32), &span)
        .expect("fixed I32 TypeId should lower in test context");
    let lowered = builder
        .lower_expression(call_view)
        .expect("host call lowering should use the expression success shape");

    assert_eq!(expression_row(&builder, lowered.value).ty, expected_int);
}

#[test]
fn expression_handled_fallible_call_fallback_uses_success_shape() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("handled_result", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let ok_type = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::I32), &span)
        .expect("fixed I32 TypeId should lower in test context");
    let err_type = builder
        .lower_type_id(builtin_type_ids::STRING, &span)
        .expect("builtin String TypeId should lower in test context");
    builder.test_register_function_with_returns(
        function_name,
        FunctionId(35),
        vec![ok_type],
        Some(err_type),
    );
    let test_scope = function_name;

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let handled_call = expressions
        .handled_fallible_function_call_with_typed_arguments(
            function_name,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Recover,
            span,
        )
        .expect("handled fallible call should construct");
    // Parser-produced catches carry a resolved HandledByCatch disposition on the protected
    // node; synthetic fixtures set it explicitly to match.
    let mut handled_facts = expressions
        .view_owned(handled_call)
        .expect("handled call should have a view")
        .failure_facts()
        .cloned()
        .unwrap_or_default();
    handled_facts.disposition = FailureDisposition::HandledByCatch {
        error_type_id: builtin_type_ids::STRING,
    };
    let handled_call = expressions
        .with_failure_facts(handled_call, handled_facts)
        .expect("handled call failure facts should attach");
    assert_eq!(
        expressions
            .view_owned(handled_call)
            .expect("handled call should have a view")
            .success_types(),
        &[builtin_type_ids::fixed_scalar(FixedScalar::I32)]
    );

    let seven = int_expr(&mut expressions, &mut builder.module_const_values, 7, span);
    let call_expr = wrap_catch_expression(
        &mut expressions,
        handled_call,
        FallibleHandling::Handler {
            error: None,
            body: vec![AstNode {
                kind: NodeKind::ThenValue(ProducedValues {
                    expressions: vec![seven],
                    span,
                }),
                span,
                scope: test_scope,
            }],
        },
        vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
    )
    .expect("catch wrapping should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(call_expr)
                .expect("catch expression should have a view"),
        )
        .expect("handled fallible call lowering should use variant result TypeIds");

    assert_eq!(expression_row(&builder, lowered.value).ty, ok_type);
}

#[test]
fn optional_user_defined_cast_recovery_merges_the_inner_target_type() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let method_path = super::symbol("Label/try_to_string", &mut path_fork, &mut string_table);
    let fallback_text = string_table.intern("fallback");
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let optional_string_type = builder
        .type_environment
        .intern_option(builtin_type_ids::STRING);
    let error_type_id = builder.test_register_builtin_error_type();
    builder.test_register_function_with_returns(
        method_path,
        FunctionId(101),
        vec![builtin_type_ids::STRING],
        Some(error_type_id),
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let source = int_expr(&mut expressions, &mut builder.module_const_values, 1, span);
    let cast = expressions
        .cast(
            expressions.own(source),
            CastFacts {
                source_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                target_type_id: builtin_type_ids::STRING,
                requires_optional_wrap_after_cast: true,
                fallibility: BuiltinCastFallibility::Fallible,
                evidence: ResolvedCastEvidence::UserDefined {
                    evidence_id: TraitEvidenceId(0),
                    method_path,
                },
                handling: CastHandling::Recover,
                span,
            },
            optional_string_type,
        )
        .expect("fixture cast fits store");
    let mut cast_facts = expressions
        .view_owned(cast)
        .expect("cast should have a view")
        .failure_facts()
        .cloned()
        .unwrap_or_default();
    cast_facts.disposition = FailureDisposition::HandledByCatch { error_type_id };
    let cast = expressions
        .with_failure_facts(cast, cast_facts)
        .expect("cast failure facts should attach");
    let fallback = expressions
        .string_slice(
            &mut builder.module_const_values,
            fallback_text,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fallback string should construct");
    let catch_expression = wrap_catch_expression(
        &mut expressions,
        cast,
        FallibleHandling::Handler {
            error: None,
            body: vec![AstNode {
                kind: NodeKind::ThenValue(ProducedValues {
                    expressions: vec![fallback.id()],
                    span,
                }),
                span,
                scope: method_path,
            }],
        },
        vec![builtin_type_ids::STRING],
    )
    .expect("catch wrapping should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(catch_expression)
                .expect("catch expression should have a view"),
        )
        .expect("optional user-defined cast recovery should lower");
    assert_eq!(
        builder.module.expressions.expression(lowered.value).ty,
        optional_string_type
    );

    let mut found_inner_string_write = false;
    for block in &builder.module.blocks {
        for statement in &block.statements {
            let HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local_id),
                value,
            } = &statement.kind
            else {
                continue;
            };
            let destination = builder
                .module
                .blocks
                .iter()
                .flat_map(|block| &block.locals)
                .find(|local| local.id == *local_id);
            if destination.is_some_and(|local| local.ty == builtin_type_ids::STRING) {
                found_inner_string_write = true;
                assert_eq!(
                    builder.module.expressions.expression(*value).ty,
                    builtin_type_ids::STRING,
                    "optional catch success and recovery must merge the cast's inner target type"
                );
            }
        }
    }
    assert!(
        found_inner_string_write,
        "expected the catch join to write its inner String result"
    );
}

#[test]
fn expression_handled_result_preserves_success_shape() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("handled_result_expr", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let success_types = vec![
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        builtin_type_ids::BOOL,
    ];
    let err_type = builtin_type_ids::STRING;
    builder.test_register_function_with_returns(
        function_name,
        FunctionId(36),
        success_types.clone(),
        Some(err_type),
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let producer = expressions
        .function_call_with_typed_arguments(
            function_name,
            vec![],
            vec![
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                builtin_type_ids::BOOL,
            ],
            span,
        )
        .expect("typed error producer call should construct");
    let producer = expressions
        .with_typed_error_producer(producer, err_type, None)
        .expect("typed error producer should attach");
    let producer_view = expressions
        .view_owned(producer)
        .expect("producer call should have a view");
    assert_eq!(producer_view.success_types().len(), 2);
    assert_eq!(producer_view.error_type(), Some(err_type));
    let test_scope = crate::compiler_frontend::symbols::path_interner::PathId::ROOT;

    let seven = int_expr(&mut expressions, &mut builder.module_const_values, 7, span);
    let false_value = expressions
        .bool(
            &mut builder.module_const_values,
            false,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture bool fits store");
    let handled = expressions
        .handled_result(producer, FallibleExpressionHandling::Recover, span)
        .expect("handled result should construct");
    // Parser-produced catches carry a resolved HandledByCatch disposition on the protected
    // node; synthetic fixtures set it explicitly to match.
    let mut handled_facts = expressions
        .view_owned(handled)
        .expect("handled result should have a view")
        .failure_facts()
        .cloned()
        .unwrap_or_default();
    handled_facts.disposition = FailureDisposition::HandledByCatch {
        error_type_id: builtin_type_ids::STRING,
    };
    let handled = expressions
        .with_failure_facts(handled, handled_facts)
        .expect("handled result failure facts should attach");
    let handled_expr = wrap_catch_expression(
        &mut expressions,
        handled,
        FallibleHandling::Handler {
            error: None,
            body: vec![AstNode {
                kind: NodeKind::ThenValue(ProducedValues {
                    expressions: vec![seven, false_value.id()],
                    span,
                }),
                span,
                scope: test_scope,
            }],
        },
        vec![
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            builtin_type_ids::BOOL,
        ],
    )
    .expect("fixture catch fits expression store");

    let lowered = builder
        .lower_expression_results(
            expressions
                .view_owned(handled_expr)
                .expect("handled expression should have a view"),
        )
        .expect("handled expression should preserve ordered success slots");

    assert_eq!(lowered.values.len(), success_types.len());
    for (value, expected_type) in lowered.values.iter().zip(success_types) {
        assert_eq!(expression_row(&builder, *value).ty, expected_type);
    }
}

#[test]
fn lowers_fresh_mutable_call_argument_via_hidden_local_with_origin_metadata() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("mutate", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_function_name(function_name, FunctionId(24));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let seven = int_expr(&mut expressions, &mut builder.module_const_values, 7, span);
    let fresh_argument =
        CallArgument::positional(expressions.own(seven), CallAccessMode::Shared, span)
            .with_passing_mode(CallPassingMode::FreshMutableValue);

    let call_expr = expressions
        .function_call_with_typed_arguments(function_name, vec![fresh_argument], vec![], span)
        .expect("fresh mutable call should construct");

    let lowered = builder
        .lower_expression_results(
            expressions
                .view_owned(call_expr)
                .expect("call expression should have a view"),
        )
        .expect("fresh mutable argument lowering should succeed");
    assert!(
        lowered.values.is_empty(),
        "no-success call produces no values"
    );

    assert_eq!(
        lowered.prelude.len(),
        2,
        "fresh mutable args should define their temporary before call"
    );

    let temp_local = match &lowered.prelude[0].kind {
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(local),
            value,
        } => {
            assert!(matches!(
                &builder.module.expressions.expression(*value).kind,
                HirExpressionKind::FixedScalar(value) if *value == i32_value(7)
            ));
            *local
        }
        other => panic!("expected first prelude statement to define fresh arg temp, got {other:?}"),
    };

    match &lowered.prelude[1].kind {
        HirStatementKind::Call { args, .. } => {
            assert_eq!(args.len(), 1);
            let argument = builder.module.expressions.values(*args)[0];
            assert!(
                matches!(
                    &builder.module.expressions.expression(argument).kind,
                    HirExpressionKind::Load(place)
                        if place.root == temp_local && super::is_local_place(place)
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

    assert!(
        origin.call_span.is_none(),
        "synthetic fresh mutable argument should not fabricate source provenance"
    );
}

#[test]
fn lowers_receiver_method_call_with_receiver_as_first_argument() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let method_path = super::symbol("Vector2/reset", &mut path_fork, &mut string_table);
    let receiver_name = super::symbol("vec", &mut path_fork, &mut string_table);
    let receiver_struct = super::symbol("Vector2", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    builder.test_register_function_name(method_path, FunctionId(22));

    let receiver_type_id =
        builder.test_register_nominal_struct_type(receiver_struct, vec![], false);
    builder.test_register_struct_with_fields(
        StructId(21),
        receiver_struct,
        receiver_type_id,
        vec![],
    );
    register_local(
        &mut builder,
        receiver_name,
        LocalId(23),
        receiver_type_id,
        span,
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let receiver = reference_expr_with_type_id(
        &mut expressions,
        receiver_name,
        receiver_type_id,
        span,
        ValueMode::MutableReference,
    );
    let seven = int_expr(&mut expressions, &mut builder.module_const_values, 7, span);
    let method_expression = expressions
        .method_call_with_typed_arguments(
            expressions.own(receiver),
            method_path,
            vec![CallArgument::positional(
                expressions.own(seven),
                CallAccessMode::Shared,
                span,
            )],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            span,
        )
        .expect("receiver method call should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(method_expression)
                .expect("method call should have a view"),
        )
        .expect("receiver method call lowering should succeed");

    assert_eq!(lowered.prelude.len(), 1);

    match &lowered.prelude[0].kind {
        HirStatementKind::Call { target, args, .. } => {
            assert_eq!(target, &CallTarget::Local(FunctionId(22)));
            assert_eq!(args.len(), 2);
            let args = builder.module.expressions.values(*args);
            assert!(matches!(
                &builder.module.expressions.expression(args[0]).kind,
                HirExpressionKind::Load(place)
                    if place.root == LocalId(23) && super::is_local_place(place)
            ));
            assert!(matches!(
                &builder.module.expressions.expression(args[1]).kind,
                HirExpressionKind::FixedScalar(value) if *value == i32_value(7)
            ));
        }
        other => panic!("expected lowered receiver call statement, got {other:?}"),
    }
}

#[test]
fn lowers_builtin_scalar_receiver_method_call_with_receiver_as_first_argument() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let method_path = super::symbol("I32/double", &mut path_fork, &mut string_table);
    let receiver_name = super::symbol("value", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    builder.test_register_function_name(method_path, FunctionId(41));

    register_local(
        &mut builder,
        receiver_name,
        LocalId(42),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let receiver = reference_expr_with_type_id(
        &mut expressions,
        receiver_name,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    );
    let method_expression = expressions
        .method_call_with_typed_arguments(
            expressions.own(receiver),
            method_path,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            span,
        )
        .expect("builtin scalar receiver method call should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(method_expression)
                .expect("method call should have a view"),
        )
        .expect("builtin scalar receiver method call lowering should succeed");

    assert_eq!(lowered.prelude.len(), 1);
    match &lowered.prelude[0].kind {
        HirStatementKind::Call { target, args, .. } => {
            assert_eq!(target, &CallTarget::Local(FunctionId(41)));
            assert_eq!(args.len(), 1);
            assert!(matches!(
                &builder
                    .module
                    .expressions
                    .expression(builder.module.expressions.values(*args)[0])
                    .kind,
                HirExpressionKind::Load(place)
                    if place.root == LocalId(42) && super::is_local_place(place)
            ));
        }
        other => panic!("expected lowered builtin scalar receiver call statement, got {other:?}"),
    }
}

#[test]
fn lowers_host_call_expression_with_host_target() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let literal_x = string_table.intern("x");
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let text = expressions
        .string_slice(
            &mut builder.module_const_values,
            literal_x,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("string literal should construct");
    let host_call = expressions
        .host_function_call_with_typed_arguments(
            crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine,
            vec![CallArgument::positional(text, CallAccessMode::Shared, span)],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            false,
            span,
        )
        .expect("host call should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(host_call)
                .expect("host call should have a view"),
        )
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
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let first = super::symbol("first", &mut path_fork, &mut string_table);
    let second = super::symbol("second", &mut path_fork, &mut string_table);
    let outer = super::symbol("outer", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    builder.test_register_function_name(first, FunctionId(1));
    builder.test_register_function_name(second, FunctionId(2));
    builder.test_register_function_name(outer, FunctionId(3));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let arg_one = expressions
        .function_call_with_typed_arguments(
            first,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            span,
        )
        .expect("first nested call should construct");
    let arg_two = expressions
        .function_call_with_typed_arguments(
            second,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            span,
        )
        .expect("second nested call should construct");
    let outer_call = expressions
        .function_call_with_typed_arguments(
            outer,
            vec![
                CallArgument::positional(arg_one, CallAccessMode::Shared, span),
                CallArgument::positional(arg_two, CallAccessMode::Shared, span),
            ],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            span,
        )
        .expect("outer call should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(outer_call)
                .expect("outer call should have a view"),
        )
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
fn runtime_template_expression_lowers_inline_to_accumulator() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let hello = string_table.intern("hello");
    let span = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let hello_expr = expressions
        .string_slice(&mut constants, hello, span, ValueMode::ImmutableOwned)
        .expect("fixture string should construct");
    let expr = runtime_template_expression(
        &mut expressions,
        &constants,
        span,
        vec![hello_expr.id()],
        &string_table,
    );
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime template should have a view"),
        )
        .expect("runtime template lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_no_synthesized_helper_functions(&builder);
    assert!(matches!(
        &expression_row(&builder, lowered.value).kind,
        HirExpressionKind::Copy(place) if super::is_local_place(place)
    ));
    let entry_block = declared_entry_block(&builder);
    assert!(block_assigns_string_literal(
        &builder.module,
        entry_block,
        "hello"
    ));
}

#[test]
fn runtime_template_handoff_expression_flattens_nested_linear_handoff() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let before = string_table.intern("before ");
    let inner = string_table.intern("inner");
    let after = string_table.intern(" after");
    let span = None;

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let inner_str = expressions
        .string_slice(&mut constants, inner, span, ValueMode::ImmutableOwned)
        .expect("fixture string should construct");
    let inner_template = runtime_template_expression(
        &mut expressions,
        &constants,
        span,
        vec![inner_str.id()],
        &string_table,
    );
    let before_str = expressions
        .string_slice(&mut constants, before, span, ValueMode::ImmutableOwned)
        .expect("fixture string should construct");
    let after_str = expressions
        .string_slice(&mut constants, after, span, ValueMode::ImmutableOwned)
        .expect("fixture string should construct");
    let outer_template = runtime_template_expression(
        &mut expressions,
        &constants,
        span,
        vec![before_str.id(), inner_template, after_str.id()],
        &string_table,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let lowered = builder
        .lower_expression(
            expressions
                .view(outer_template)
                .expect("outer template should have a view"),
        )
        .expect("owned nested runtime template handoffs should lower");

    assert!(lowered.prelude.is_empty());
    let entry_block = declared_entry_block(&builder);
    assert_eq!(
        count_empty_string_initializers(&builder.module, entry_block),
        1,
        "nested owned linear handoffs should append into the parent accumulator without creating a child accumulator"
    );
    assert!(block_assigns_string_literal(
        &builder.module,
        entry_block,
        "before "
    ));
    assert!(block_assigns_string_literal(
        &builder.module,
        entry_block,
        "inner"
    ));
    assert!(block_assigns_string_literal(
        &builder.module,
        entry_block,
        " after"
    ));
}

#[test]
fn runtime_template_inline_accumulator_coerces_non_string_segments() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let five = int_expr(&mut expressions, &mut builder.module_const_values, 5, span);
    let expr = runtime_template_expression(
        &mut expressions,
        &builder.module_const_values,
        span,
        vec![five],
        builder.string_table,
    );

    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime template should have a view"),
        )
        .expect("runtime template lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_no_synthesized_helper_functions(&builder);
    let entry_block = declared_entry_block(&builder);
    assert!(block_assigns_coerced_int_chunk(
        &builder.module,
        entry_block,
        5
    ));
}

#[test]
fn runtime_template_lowers_nested_templates_in_order() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let a = string_table.intern("A");
    let b = string_table.intern("B");
    let c = string_table.intern("C");
    let span = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let a_expr = expressions
        .string_slice(&mut constants, a, span, ValueMode::ImmutableOwned)
        .expect("fixture string should construct");
    let b_expr = expressions
        .string_slice(&mut constants, b, span, ValueMode::ImmutableOwned)
        .expect("fixture string should construct");
    let c_expr = expressions
        .string_slice(&mut constants, c, span, ValueMode::ImmutableOwned)
        .expect("fixture string should construct");
    let nested = runtime_template_expression(
        &mut expressions,
        &constants,
        span,
        vec![b_expr.id()],
        &string_table,
    );
    let expr = runtime_template_expression(
        &mut expressions,
        &constants,
        span,
        vec![a_expr.id(), nested, c_expr.id()],
        &string_table,
    );
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("nested runtime template should have a view"),
        )
        .expect("nested runtime template lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_no_synthesized_helper_functions(&builder);
    let entry_block = declared_entry_block(&builder);
    assert!(block_assigns_string_literal(
        &builder.module,
        entry_block,
        "A"
    ));
    assert!(block_assigns_string_literal(
        &builder.module,
        entry_block,
        "B"
    ));
    assert!(block_assigns_string_literal(
        &builder.module,
        entry_block,
        "C"
    ));
    // The owned runtime-template handoff flattens nested child templates into the
    // same accumulator, so all three literals are appended directly rather than
    // through an intermediate child-template local.
}

#[test]
fn runtime_template_control_flow_bool_if_lowers_inline_without_helper_call() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let show_name = super::symbol("show", &mut path_fork, &mut string_table);
    let shown = string_table.intern("shown");
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(
        &mut builder,
        show_name,
        LocalId(10),
        builtin_type_ids::BOOL,
        span,
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let condition = reference_expr_with_type_id(
        &mut expressions,
        show_name,
        builtin_type_ids::BOOL,
        span,
        ValueMode::ImmutableOwned,
    );
    let shown_expr = expressions
        .string_slice(&mut constants, shown, span, ValueMode::ImmutableOwned)
        .expect("fixture string should construct");
    let expr = runtime_template_bool_if_expression(
        &mut expressions,
        &constants,
        condition,
        vec![shown_expr.id()],
        span,
        builder.string_table,
    );

    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime Bool template if should have a view"),
        )
        .expect("runtime Bool template if should lower inline");

    assert!(lowered.prelude.is_empty());
    assert_no_synthesized_helper_functions(&builder);
    assert!(matches!(
        &expression_row(&builder, lowered.value).kind,
        HirExpressionKind::Copy(place) if super::is_local_place(place)
    ));
    let entry_block = declared_entry_block(&builder);
    assert_eq!(
        count_empty_string_initializers(&builder.module, entry_block),
        1,
        "inline template if should initialize exactly one runtime accumulator"
    );

    let (then_block, else_block) = match &entry_block.terminator {
        &HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (then_block, else_block),
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

    assert!(block_assigns_string_literal(
        &builder.module,
        then_block,
        "shown"
    ));
    assert!(
        else_block
            .statements
            .iter()
            .all(|statement| !matches!(&statement.kind, HirStatementKind::Write { .. })),
        "the unselected path must not append to the runtime template accumulator"
    );
}

#[test]
fn runtime_template_control_flow_bool_if_branch_preserves_fallible_propagation_cfg() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let enclosing_name = super::symbol("__expr_test_fn", &mut path_fork, &mut string_table);
    let can_fail_name = super::symbol("can_fail", &mut path_fork, &mut string_table);
    let show_name = super::symbol("show", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    builder.test_register_function_with_returns(
        enclosing_name,
        FunctionId(0),
        vec![builtin_type_ids::STRING],
        Some(builtin_type_ids::STRING),
    );
    builder.test_register_function_with_returns(
        can_fail_name,
        FunctionId(7),
        vec![builtin_type_ids::STRING],
        Some(builtin_type_ids::STRING),
    );
    register_local(
        &mut builder,
        show_name,
        LocalId(11),
        builtin_type_ids::BOOL,
        span,
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let condition = reference_expr_with_type_id(
        &mut expressions,
        show_name,
        builtin_type_ids::BOOL,
        span,
        ValueMode::ImmutableOwned,
    );
    let propagated_call = expressions
        .handled_fallible_function_call_with_typed_arguments(
            can_fail_name,
            vec![],
            vec![builtin_type_ids::STRING],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Propagate,
            span,
        )
        .expect("propagated fallible call should construct");
    let expr = runtime_template_bool_if_expression(
        &mut expressions,
        &constants,
        condition,
        vec![propagated_call.id()],
        span,
        builder.string_table,
    );

    builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("fallible runtime template branch should have a view"),
        )
        .expect("fallible runtime template branch should lower inline");

    assert_eq!(
        builder.module.functions.len(),
        2,
        "control-flow template lowering must not synthesize a helper that could intercept `!`"
    );
    let entry_block = declared_entry_block(&builder);
    let then_block_id = match &entry_block.terminator {
        &HirTerminator::If { then_block, .. } => then_block,
        other => panic!("expected inline template if terminator, got {other:?}"),
    };
    let then_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == then_block_id)
        .expect("then block should exist");

    let HirTerminator::Invoke {
        failure: error_block,
        ..
    } = then_block.terminator
    else {
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
fn runtime_template_control_flow_bool_if_coerces_dynamic_branch_chunks() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let show_name = super::symbol("show", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(
        &mut builder,
        show_name,
        LocalId(12),
        builtin_type_ids::BOOL,
        span,
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let condition = reference_expr_with_type_id(
        &mut expressions,
        show_name,
        builtin_type_ids::BOOL,
        span,
        ValueMode::ImmutableOwned,
    );
    let five = int_expr(&mut expressions, &mut builder.module_const_values, 5, span);
    let expr = runtime_template_bool_if_expression(
        &mut expressions,
        &builder.module_const_values,
        condition,
        vec![five],
        span,
        builder.string_table,
    );

    builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime Bool template if should have a view"),
        )
        .expect("runtime Bool template if should coerce dynamic branch chunks");
    let entry_block = declared_entry_block(&builder);
    let then_block = match &entry_block.terminator {
        &HirTerminator::If { then_block, .. } => then_block,
        other => panic!("expected inline template if terminator, got {other:?}"),
    };
    let then_block = builder
        .module
        .blocks
        .iter()
        .find(|block| block.id == then_block)
        .expect("then block should exist");

    assert!(block_assigns_coerced_int_chunk(
        &builder.module,
        then_block,
        5
    ));
}

#[test]
fn runtime_template_control_flow_option_capture_lowers_match_and_payload_binding() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let maybe_name = super::symbol("maybe_name", &mut path_fork, &mut string_table);
    let capture_name = string_table.intern("name");
    let capture_path = super::symbol("name", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let option_string = builder
        .type_environment
        .intern_option(builtin_type_ids::STRING);
    register_local(&mut builder, maybe_name, LocalId(12), option_string, span);

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let scrutinee = reference_expr_with_type_id(
        &mut expressions,
        maybe_name,
        option_string,
        span,
        ValueMode::ImmutableOwned,
    );
    let then_content = vec![reference_expr_with_type_id(
        &mut expressions,
        capture_path,
        builtin_type_ids::STRING,
        span,
        ValueMode::ImmutableReference,
    )];
    let expr = runtime_template_option_capture_expression(
        &mut expressions,
        &constants,
        scrutinee,
        capture_name,
        capture_path,
        builtin_type_ids::STRING,
        then_content,
        span,
        builder.string_table,
    );

    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime option-present template if should have a view"),
        )
        .expect("runtime option-present template if should lower inline");

    assert!(lowered.prelude.is_empty());
    assert_no_synthesized_helper_functions(&builder);
    let entry_block = declared_entry_block(&builder);
    assert_eq!(
        count_empty_string_initializers(&builder.module, entry_block),
        1,
        "option template should initialize exactly one runtime accumulator"
    );
    assert_eq!(
        entry_block
            .statements
            .iter()
            .filter(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(_),
                        value,
                    }
                        if matches!(
                            &builder.module.expressions.expression(*value).kind,
                            HirExpressionKind::Load(place) | HirExpressionKind::Copy(place)
                                if super::is_local_place(place)
                        )
                )
            })
            .count(),
        1,
        "option template should materialize the scrutinee exactly once before matching"
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
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(_),
                    value,
                }
                if matches!(
                        &builder.module.expressions.expression(*value).kind,
                        HirExpressionKind::VariantPayloadGet {
                            carrier: HirVariantCarrier::Option,
                            variant_index: OPTION_SOME_VARIANT_INDEX,
                            field_index: 0,
                            ..
                        }
                    )
            )
        }),
        "present branch should define the option some payload in the capture local"
    );
    assert!(
        absent_block.locals.is_empty(),
        "absent branch must not bind the option capture local"
    );
    assert!(
        absent_block
            .statements
            .iter()
            .all(|statement| !matches!(&statement.kind, HirStatementKind::Write { .. })),
        "the absent path must not append to the runtime template accumulator"
    );
}

#[test]
fn runtime_template_control_flow_loop_range_lowers_inline_and_wraps_aggregate_when_emitted() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let prefix = string_table.intern("<card>");
    let suffix = string_table.intern("</card>");
    let span = None;
    let limit_path = super::symbol("limit", &mut path_fork, &mut string_table);
    let item_binding = loop_binding(
        "i",
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        &mut path_fork,
        &mut string_table,
    );
    let item_path = item_binding.id;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(
        &mut builder,
        limit_path,
        LocalId(20),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
    );
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let body_content = vec![reference_expr_with_type_id(
        &mut expressions,
        item_path,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    )];
    let start = int_expr(&mut expressions, &mut builder.module_const_values, 0, span);
    let end = reference_expr_with_type_id(
        &mut expressions,
        limit_path,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    );
    let range = RangeLoopSpec {
        start,
        end,
        end_kind: RangeEndKind::Exclusive,
        step: None,
        domain: NumericScalar::Fixed(FixedScalar::I32),
    };
    let expr = runtime_template_range_loop_expression(
        &mut expressions,
        &builder.module_const_values,
        LoopBindings {
            item: Some(item_binding),
            index: None,
        },
        range,
        body_content,
        prefix,
        suffix,
        span,
        builder.string_table,
    );

    builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime range template loop should have a view"),
        )
        .expect("runtime range template loop should lower inline");

    assert_no_synthesized_helper_functions(&builder);
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_marks_loop_emitted(&builder.module, block)),
        "range template loop body should mark that at least one iteration emitted"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_assigns_string_literal(&builder.module, block, "<card>")),
        "emitted aggregate should apply the owning head before the aggregate"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_assigns_string_literal(&builder.module, block, "</card>")),
        "emitted aggregate should apply the owning head after the aggregate"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_appends_local_string(&builder.module, block)),
        "emitted aggregate should append the loop-local aggregate string"
    );
}

#[test]
fn runtime_template_control_flow_loop_collection_materializes_iterable_and_length_once() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let prefix = string_table.intern("");
    let suffix = string_table.intern("");
    let span = None;
    let items_path = super::symbol("items", &mut path_fork, &mut string_table);
    let item_binding = loop_binding(
        "item",
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        &mut path_fork,
        &mut string_table,
    );
    let item_path = item_binding.id;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let collection_type = builder
        .type_environment
        .intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::I32), None);
    register_local(&mut builder, items_path, LocalId(21), collection_type, span);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let body_content = vec![reference_expr_with_type_id(
        &mut expressions,
        item_path,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    )];
    let iterable = reference_expr_with_type_id(
        &mut expressions,
        items_path,
        collection_type,
        span,
        ValueMode::ImmutableReference,
    );
    let expr = runtime_template_collection_loop_expression(
        &mut expressions,
        &constants,
        LoopBindings {
            item: Some(item_binding),
            index: None,
        },
        iterable,
        body_content,
        prefix,
        suffix,
        span,
        builder.string_table,
    );

    builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime collection template loop should have a view"),
        )
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
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_marks_loop_emitted(&builder.module, block)),
        "collection template loop body should mark emitted iterations independently from string length"
    );
}

#[test]
fn runtime_template_control_flow_conditional_loop_rechecks_condition_and_wraps_when_emitted() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let prefix = string_table.intern("<wrap>");
    let suffix = string_table.intern("</wrap>");
    let tick = string_table.intern("tick");
    let span = None;
    let keep_going_path = super::symbol("keep_going", &mut path_fork, &mut string_table);
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(
        &mut builder,
        keep_going_path,
        LocalId(22),
        builtin_type_ids::BOOL,
        span,
    );
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let tick_expr = expressions
        .string_slice(&mut constants, tick, span, ValueMode::ImmutableOwned)
        .expect("fixture string should construct");
    let body_content = vec![tick_expr.id()];
    let condition = reference_expr_with_type_id(
        &mut expressions,
        keep_going_path,
        builtin_type_ids::BOOL,
        span,
        ValueMode::ImmutableReference,
    );
    let expr = runtime_template_conditional_loop_expression(
        &mut expressions,
        &constants,
        condition,
        body_content,
        prefix,
        suffix,
        span,
        builder.string_table,
    );

    builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime conditional template loop should have a view"),
        )
        .expect("runtime conditional template loop should lower inline");

    assert_no_synthesized_helper_functions(&builder);
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_marks_loop_emitted(&builder.module, block)),
        "conditional template loop body should mark emitted output when its body has render pieces"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| block_assigns_string_literal(&builder.module, block, "<wrap>")),
        "emitted aggregate should apply the owning wrapper"
    );
}

#[test]
fn runtime_template_control_flow_loop_empty_body_does_not_mark_iteration_emitted() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let prefix = string_table.intern("<wrap>");
    let suffix = string_table.intern("</wrap>");
    let span = None;
    let limit_path = super::symbol("limit", &mut path_fork, &mut string_table);
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(
        &mut builder,
        limit_path,
        LocalId(23),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
    );
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let body_content: Vec<ExprId> = vec![];
    let start = int_expr(&mut expressions, &mut builder.module_const_values, 0, span);
    let end = reference_expr_with_type_id(
        &mut expressions,
        limit_path,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    );
    let range = RangeLoopSpec {
        start,
        end,
        end_kind: RangeEndKind::Exclusive,
        step: None,
        domain: NumericScalar::Fixed(FixedScalar::I32),
    };
    let expr = runtime_template_range_loop_expression(
        &mut expressions,
        &builder.module_const_values,
        LoopBindings {
            item: None,
            index: None,
        },
        range,
        body_content,
        prefix,
        suffix,
        span,
        builder.string_table,
    );

    builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("runtime empty-body template loop should have a view"),
        )
        .expect("runtime empty-body template loop should lower without marking output");

    assert!(
        !builder
            .module
            .blocks
            .iter()
            .any(|block| block_marks_loop_emitted(&builder.module, block)),
        "empty loop bodies should not mark the aggregate as structurally emitted"
    );
}

fn block_assigns_string_literal(module: &HirModule, block: &HirBlock, expected: &str) -> bool {
    block.statements.iter().any(|statement| {
        let HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(_),
            value,
        } = &statement.kind
        else {
            return false;
        };

        let value = module.expressions.expression(*value);
        let HirExpressionKind::BinOp {
            op: HirBinOp::StringAppend,
            right,
            ..
        } = &value.kind
        else {
            return false;
        };

        matches!(
            &module.expressions.expression(*right).kind,
            HirExpressionKind::StringLiteral(value) if value == expected
        )
    })
}

fn count_empty_string_initializers(module: &HirModule, block: &HirBlock) -> usize {
    block
        .statements
        .iter()
        .filter(|statement| {
            let HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(_),
                value,
            } = &statement.kind
            else {
                return false;
            };

            let value = module.expressions.expression(*value);
            matches!(
                &value.kind,
                HirExpressionKind::StringLiteral(value) if value.is_empty()
            )
        })
        .count()
}

fn block_marks_loop_emitted(module: &HirModule, block: &HirBlock) -> bool {
    block.statements.iter().any(|statement| {
        super::write_value(module, statement)
            .is_some_and(|value| matches!(&value.kind, HirExpressionKind::Bool(true)))
    })
}

fn block_appends_local_string(module: &HirModule, block: &HirBlock) -> bool {
    count_block_appends_local_string(module, block) > 0
}

fn count_block_appends_local_string(module: &HirModule, block: &HirBlock) -> usize {
    block
        .statements
        .iter()
        .filter(|statement| {
            let HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(_),
                value,
            } = &statement.kind
            else {
                return false;
            };

            let value = module.expressions.expression(*value);
            let HirExpressionKind::BinOp {
                op: HirBinOp::StringAppend,
                right,
                ..
            } = &value.kind
            else {
                return false;
            };

            matches!(
                &module.expressions.expression(*right).kind,
                HirExpressionKind::Load(place) | HirExpressionKind::Copy(place)
                    if super::is_local_place(place)
            )
        })
        .count()
}

fn block_assigns_coerced_int_chunk(module: &HirModule, block: &HirBlock, expected: i64) -> bool {
    block.statements.iter().any(|statement| {
        let HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(_),
            value,
        } = &statement.kind
        else {
            return false;
        };

        let value = module.expressions.expression(*value);
        let HirExpressionKind::BinOp {
            op: HirBinOp::StringAppend,
            right,
            ..
        } = &value.kind
        else {
            return false;
        };

        // A fixed numeric chunk renders through the numeric text policy, so the appended
        // value is the string conversion of the literal rather than a raw number.
        let HirExpressionKind::Cast {
            source,
            policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::I32)),
        } = &module.expressions.expression(*right).kind
        else {
            return false;
        };

        matches!(
            &module.expressions.expression(*source).kind,
            HirExpressionKind::FixedScalar(value) if *value == i32_value(expected)
        )
    })
}

#[test]
fn local_resolution_uses_full_path_identity_not_leaf_name() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let x_leaf = string_table.intern("x");
    let scope_a = super::symbol("scope_a", &mut path_fork, &mut string_table);
    let scope_b = super::symbol("scope_b", &mut path_fork, &mut string_table);
    let local_a = path_fork
        .try_intern_child(scope_a, x_leaf)
        .expect("test path fits");
    let local_b = path_fork
        .try_intern_child(scope_b, x_leaf)
        .expect("test path fits");
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    register_local(
        &mut builder,
        local_a,
        LocalId(22),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let expr = reference_expr_with_type_id(
        &mut expressions,
        local_b,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    );
    let err = expect_infrastructure_error(
        builder
            .lower_expression(
                expressions
                    .view(expr)
                    .expect("reference expression should have a view"),
            )
            .expect_err("unregistered full-path symbol should not resolve by leaf name"),
    );

    assert_eq!(err.error_type, ErrorType::HirTransformation);
    assert!(err.msg.contains("Unresolved local"));
}

#[test]
fn nominal_struct_identity_uses_field_parent_path() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let struct_path = super::symbol("MyStruct", &mut path_fork, &mut string_table);
    let field_path = field_symbol(&struct_path, "value", &mut path_fork, &mut string_table);
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    builder.test_register_struct_with_fields(
        StructId(1),
        struct_path,
        int_type,
        vec![(FieldId(3), field_path, int_type)],
    );

    let struct_type_id = builder.test_register_nominal_struct_type(
        struct_path,
        vec![(field_path, int_type, span)],
        false,
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let value = int_expr(&mut expressions, &mut builder.module_const_values, 42, span);
    let expression = expressions
        .struct_instance(
            vec![ConstructorField {
                name: field_path,
                slot: 0,
                value: expressions.own(value),
            }],
            struct_type_id,
            span,
            ValueMode::MutableOwned,
            false,
        )
        .expect("struct instance should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(expression)
                .expect("struct instance should have a view"),
        )
        .expect("struct instance lowering should succeed");

    match &expression_row(&builder, lowered.value).kind {
        HirExpressionKind::StructConstruct { struct_id, fields } => {
            assert_eq!(*struct_id, StructId(1));
            let fields = builder.module.expressions.struct_fields(*fields);
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].0, FieldId(3));
        }
        other => panic!("expected StructConstruct, got {other:?}"),
    }
}

#[test]
fn rejects_const_record_struct_instance_runtime_lowering() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let struct_path = super::symbol("Palette", &mut path_fork, &mut string_table);
    let field_path = field_symbol(&struct_path, "red", &mut path_fork, &mut string_table);
    let field_value = string_table.intern("red");
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let const_record_type_id = builder.test_register_nominal_struct_type(
        struct_path,
        vec![(field_path, builtin_type_ids::STRING, span)],
        true,
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let value = expressions
        .string_slice(&mut constants, field_value, span, ValueMode::ImmutableOwned)
        .expect("const record field value should construct");
    let expression = expressions
        .struct_instance(
            vec![ConstructorField {
                name: field_path,
                slot: 0,
                value,
            }],
            const_record_type_id,
            span,
            ValueMode::ImmutableOwned,
            true,
        )
        .expect("const record struct instance should construct");

    let error = expect_infrastructure_error(
        builder
            .lower_expression(
                expressions
                    .view_owned(expression)
                    .expect("const record struct instance should have a view"),
            )
            .expect_err("const record should not lower as a runtime struct construct"),
    );

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Const record reached runtime HIR struct lowering")
    );
}

#[test]
fn temp_locals_are_not_resolvable_as_user_symbols() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let callee = super::symbol("callee", &mut path_fork, &mut string_table);
    let temp_name = super::symbol("__hir_tmp_0", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    builder.test_register_function_name(callee, FunctionId(8));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call_expr = expressions
        .function_call_with_typed_arguments(
            callee,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            span,
        )
        .expect("call construction should succeed");
    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(call_expr)
                .expect("call expression should have a view"),
        )
        .expect("call lowering should succeed");

    assert_eq!(lowered.prelude.len(), 1);
    assert!(matches!(
        &lowered.prelude[0].kind,
        HirStatementKind::Call { results, .. } if results.len() == 1
    ));

    let temp_reference = reference_expr_with_type_id(
        &mut expressions,
        temp_name,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span,
        ValueMode::ImmutableReference,
    );

    let error = expect_infrastructure_error(
        builder
            .lower_expression(
                expressions
                    .view(temp_reference)
                    .expect("temp reference should have a view"),
            )
            .expect_err("compiler temp local should not resolve through locals_by_name"),
    );

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved local"));
}

#[test]
fn field_access_uses_base_struct_identity_not_global_leaf_lookup() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let struct_a = super::symbol("StructA", &mut path_fork, &mut string_table);
    let struct_b = super::symbol("StructB", &mut path_fork, &mut string_table);
    let field_leaf = string_table.intern("value");
    let field_a = path_fork
        .try_intern_child(struct_a, field_leaf)
        .expect("test path fits");
    let field_b = path_fork
        .try_intern_child(struct_b, field_leaf)
        .expect("test path fits");
    let local_name = super::symbol("my_struct", &mut path_fork, &mut string_table);
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    builder.test_register_struct_with_fields(
        StructId(10),
        struct_a,
        int_type,
        vec![(FieldId(100), field_a, int_type)],
    );
    builder.test_register_struct_with_fields(
        StructId(11),
        struct_b,
        int_type,
        vec![(FieldId(101), field_b, int_type)],
    );

    let local_struct_type_id =
        builder.test_register_nominal_struct_type(struct_a, vec![(field_a, int_type, span)], false);
    register_local(
        &mut builder,
        local_name,
        LocalId(30),
        local_struct_type_id,
        span,
    );

    let field_projections = expressions
        .append_place_projections(
            vec![PlaceProjection::Field {
                field: field_leaf,
                result_type: int_type,
            }],
            span,
        )
        .expect("fixture place projections fit store");
    let field_place = expressions
        .append_place(
            PlaceRecord {
                root: local_name,
                root_type_id: local_struct_type_id,
                projections: field_projections,
                type_id: int_type,
                value_mode: ValueMode::ImmutableReference,
                span,
            },
            span,
        )
        .expect("field place fits expression store");

    let (_prelude, place) = builder
        .lower_place_expression_to_hir_place(
            expressions
                .place(field_place)
                .expect("field place should have a view"),
        )
        .expect("field place should lower via its resolved base type");

    assert_eq!(place.root, LocalId(30));
    assert_eq!(
        builder.module.expressions.projections(place.projections),
        &[HirProjection::Field(FieldId(100))]
    );
}

#[test]
fn field_access_from_module_constant_base_materializes_temp_place() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let format_name = super::symbol("format", &mut path_fork, &mut string_table);
    let format_struct = super::symbol("Format", &mut path_fork, &mut string_table);
    let center_leaf = string_table.intern("center");
    let center_field = path_fork
        .try_intern_child(format_struct, center_leaf)
        .expect("test path fits");
    let center_value = string_table.intern("<div></div>");
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let template_type = builtin_type_ids::STRING;

    builder.test_register_struct_with_fields(
        StructId(20),
        format_struct,
        template_type,
        vec![(FieldId(200), center_field, template_type)],
    );

    let format_type_id = builder.test_register_nominal_struct_type(
        format_struct,
        vec![(center_field, builtin_type_ids::STRING, span)],
        false,
    );

    let center_constant = constants
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Text(center_value)),
            None,
        )
        .expect("module constant field value should be resolved");
    let format_constant = constants
        .insert_resolved(
            format_type_id,
            ConstValuePayload::Record(vec![ConstValueField {
                name: center_field,
                value: center_constant,
            }]),
            None,
        )
        .expect("module constant struct should be resolved");

    builder.module_const_values = std::mem::take(&mut constants);
    builder.test_register_module_constant(format_name, format_constant);

    let format_reference = reference_expr_with_type_id(
        &mut expressions,
        format_name,
        format_type_id,
        span,
        ValueMode::ImmutableReference,
    );
    let field_access = expressions
        .field_access(
            expressions.own(format_reference),
            center_leaf,
            builtin_type_ids::STRING,
            span,
            ValueMode::ImmutableReference,
            ConstRecordState::RuntimeValue,
        )
        .expect("module constant field access fits expression store");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(field_access)
                .expect("module constant field access should have a view"),
        )
        .expect("module constant field access should lower");

    assert!(
        lowered
            .prelude
            .iter()
            .any(|statement| matches!(statement.kind, HirStatementKind::Write { .. })),
        "expected module constant base to be materialized into a temporary local"
    );

    match &expression_row(&builder, lowered.value).kind {
        HirExpressionKind::Load(place) => {
            assert_eq!(
                builder.module.expressions.projections(place.projections),
                &[HirProjection::Field(FieldId(200))]
            );
            assert!(
                builder
                    .module
                    .blocks
                    .iter()
                    .flat_map(|block| block.locals.iter())
                    .any(|local| local.id == place.root)
            );
        }
        other => panic!("expected field load expression, got {other:?}"),
    }
}

#[test]
fn const_record_module_constant_field_access_lowers_field_value_without_struct_construct() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let palette_name = super::symbol("palette", &mut path_fork, &mut string_table);
    let palette_struct = super::symbol("Palette", &mut path_fork, &mut string_table);
    let red_leaf = string_table.intern("red");
    let red_field = path_fork
        .try_intern_child(palette_struct, red_leaf)
        .expect("test path fits");
    let red_value = string_table.intern("red");
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let palette_type_id = builder.test_register_nominal_struct_type(
        palette_struct,
        vec![(red_field, builtin_type_ids::STRING, span)],
        true,
    );

    let red_constant = constants
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Text(red_value)),
            None,
        )
        .expect("const-record field value should be resolved");
    let mut palette_metadata =
        ConstValueMetadata::resolved(palette_type_id, span, ValueMode::ImmutableOwned);
    palette_metadata.const_record_state = ConstRecordState::ConstRecord;
    palette_metadata.value_kind = ConstValueKind::Composite;
    palette_metadata.hir_visible = false;
    let palette_constant = constants
        .insert_value(
            palette_metadata,
            ConstValuePayload::Record(vec![ConstValueField {
                name: red_field,
                value: red_constant,
            }]),
        )
        .expect("const-record module constant should be stored");

    builder.module_const_values = std::mem::take(&mut constants);
    builder.test_register_module_constant(palette_name, palette_constant);

    let palette_reference = const_record_reference_expr(
        &mut expressions,
        palette_name,
        palette_type_id,
        span,
        ValueMode::ImmutableReference,
    );

    let field_access = expressions
        .field_access(
            expressions.own(palette_reference),
            red_leaf,
            builtin_type_ids::STRING,
            span,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .expect("const-record field access fits expression store");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(field_access)
                .expect("const-record field access should have a view"),
        )
        .expect("const-record field access should lower the selected field value");

    assert!(
        lowered.prelude.is_empty(),
        "const-record field access should not materialize the whole record"
    );

    match &expression_row(&builder, lowered.value).kind {
        HirExpressionKind::StringLiteral(value) if value == "red" => {}
        other => panic!("expected direct string field value, got {other:?}"),
    }
}

#[test]
fn lowers_collection_builtin_host_calls_from_explicit_ast_nodes() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let receiver_name = super::symbol("values", &mut path_fork, &mut string_table);
    let get_id = crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionGet;
    let set_id = crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionSet;
    let growable_push_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionPushGrowable;
    let fixed_push_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionPushFixed;
    let remove_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionRemove;
    let length_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionLength;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let receiver_type_id = builder
        .type_environment
        .intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::I32), None);
    register_local(
        &mut builder,
        receiver_name,
        LocalId(70),
        receiver_type_id,
        span,
    );

    let receiver_expression = reference_expr_with_type_id(
        &mut expressions,
        receiver_name,
        receiver_type_id,
        span,
        ValueMode::MutableReference,
    );

    let error_type = builder.test_register_builtin_error_type();
    let enclosing_name = super::symbol(
        "__collection_fixture",
        builder.path_fork,
        builder.string_table,
    );
    builder.test_register_function_with_returns(
        enclosing_name,
        FunctionId(0),
        Vec::new(),
        Some(error_type),
    );

    let cases = vec![
        (
            CollectionBuiltinOp::Get,
            vec![CallArgument::positional(
                {
                    let value =
                        int_expr(&mut expressions, &mut builder.module_const_values, 1, span);
                    expressions.own(value)
                },
                CallAccessMode::Shared,
                span,
            )],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            Some(error_type),
            get_id,
        ),
        (
            CollectionBuiltinOp::Set,
            vec![
                CallArgument::positional(
                    {
                        let value =
                            int_expr(&mut expressions, &mut builder.module_const_values, 0, span);
                        expressions.own(value)
                    },
                    CallAccessMode::Shared,
                    span,
                ),
                CallArgument::positional(
                    {
                        let value =
                            int_expr(&mut expressions, &mut builder.module_const_values, 99, span);
                        expressions.own(value)
                    },
                    CallAccessMode::Shared,
                    span,
                ),
            ],
            vec![],
            Some(error_type),
            set_id,
        ),
        // PushFixed deliberately reuses the growable receiver below: HIR must consume the
        // explicit operation identity, not reclassify the push from the receiver's shape.
        (
            CollectionBuiltinOp::PushFixed,
            vec![CallArgument::positional(
                {
                    let value =
                        int_expr(&mut expressions, &mut builder.module_const_values, 4, span);
                    expressions.own(value)
                },
                CallAccessMode::Shared,
                span,
            )],
            vec![],
            Some(error_type),
            fixed_push_id,
        ),
        (
            CollectionBuiltinOp::PushGrowable,
            vec![CallArgument::positional(
                {
                    let value =
                        int_expr(&mut expressions, &mut builder.module_const_values, 4, span);
                    expressions.own(value)
                },
                CallAccessMode::Shared,
                span,
            )],
            vec![],
            None,
            growable_push_id,
        ),
        (
            CollectionBuiltinOp::Remove,
            vec![CallArgument::positional(
                {
                    let value =
                        int_expr(&mut expressions, &mut builder.module_const_values, 0, span);
                    expressions.own(value)
                },
                CallAccessMode::Shared,
                span,
            )],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            Some(error_type),
            remove_id,
        ),
        (
            CollectionBuiltinOp::Length,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            None,
            length_id,
        ),
    ];

    for (op, args, result_type_ids, error_type, expected_id) in cases {
        let expected_results = result_type_ids.len();
        let call_expression = expressions
            .collection_builtin_call_with_typed_arguments(
                expressions.own(receiver_expression),
                op,
                args,
                result_type_ids,
                span,
            )
            .expect("collection builtin call construction should succeed");
        let call_expression = match error_type {
            Some(error_type) => expressions
                .with_typed_error_producer(call_expression, error_type, None)
                .expect("typed error producer should attach"),
            None => call_expression,
        };

        let lowered = builder
            .lower_expression_results(
                expressions
                    .view_owned(call_expression)
                    .expect("collection builtin call should be a valid expression"),
            )
            .expect("collection builtin call lowering should succeed");

        assert_eq!(lowered.values.len(), expected_results);
        if error_type.is_some() {
            assert!(lowered.prelude.is_empty());
            let calls = builder
                .module
                .blocks
                .iter()
                .filter_map(|block| match &block.terminator {
                    HirTerminator::Invoke {
                        operation:
                            crate::compiler_frontend::hir::terminators::HirFallibleOperation::Call {
                                target,
                                args,
                            },
                        success_results,
                        ..
                    } if target == &CallTarget::External(expected_id) => {
                        Some((args, success_results))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(calls.len(), 1, "fallible builtin must execute exactly once");
            assert_eq!(calls[0].1.len(), expected_results);
            assert!(calls[0].0.len() > 0, "receiver must be the first argument");
        } else {
            assert_eq!(lowered.prelude.len(), 1);
            let HirStatementKind::Call {
                target,
                args,
                results,
            } = &lowered.prelude[0].kind
            else {
                panic!("expected an infallible collection call");
            };
            assert_eq!(target, &CallTarget::External(expected_id));
            assert_eq!(results.len(), expected_results);
            assert!(
                results
                    .iter()
                    .all(|result| matches!(result, HirLocalDestination::Define(_)))
            );
            assert!(args.len() > 0, "receiver must be the first argument");
        }
    }
}

#[test]
fn map_literal_lowering_preserves_entry_order() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let priya = string_table.intern("Priya");
    let grace = string_table.intern("Grace");
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let map_type = builder.type_environment.intern_map(
        builtin_type_ids::STRING,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    let mut expressions = AstExpressionBuilder::new().unwrap();

    let priya_key = expressions
        .string_slice(
            &mut builder.module_const_values,
            priya,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("map key should construct");
    let priya_value = int_expr(&mut expressions, &mut builder.module_const_values, 10, span);
    let grace_key = expressions
        .string_slice(
            &mut builder.module_const_values,
            grace,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("map key should construct");
    let grace_value = int_expr(&mut expressions, &mut builder.module_const_values, 12, span);
    let expression = expressions
        .map_literal_with_type_id(
            vec![
                MapEntry {
                    key: priya_key,
                    value: expressions.own(priya_value),
                },
                MapEntry {
                    key: grace_key,
                    value: expressions.own(grace_value),
                },
            ],
            MapLiteralExpressionType {
                key_type_id: builtin_type_ids::STRING,
                value_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                map_type_id: Some(map_type),
            },
            &mut builder.type_environment,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("map literal should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(expression)
                .expect("map literal view should resolve"),
        )
        .expect("map literal should lower to first-class HIR");

    assert!(lowered.prelude.is_empty());
    assert_eq!(expression_row(&builder, lowered.value).ty, map_type);

    let HirExpressionKind::MapLiteral(entries) = &expression_row(&builder, lowered.value).kind
    else {
        panic!(
            "expected HIR map literal, got {:?}",
            expression_row(&builder, lowered.value).kind
        );
    };
    let entries = builder.module.expressions.map_entries(*entries);
    assert_eq!(entries.len(), 2);
    assert!(matches!(
        (
            &expression_row(&builder, entries[0].key).kind,
            &expression_row(&builder, entries[0].value).kind
        ),
        (HirExpressionKind::StringLiteral(key), HirExpressionKind::FixedScalar(value))
            if key == "Priya" && *value == i32_value(10)
    ));
    assert!(matches!(
        (
            &expression_row(&builder, entries[1].key).kind,
            &expression_row(&builder, entries[1].value).kind
        ),
        (HirExpressionKind::StringLiteral(key), HirExpressionKind::FixedScalar(value))
            if key == "Grace" && *value == i32_value(12)
    ));
}

#[test]
fn map_builtin_calls_lower_to_first_class_hir_ops() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let scores_name = super::symbol("scores", &mut path_fork, &mut string_table);
    let key_name = string_table.intern("Priya");
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let map_type = builder.type_environment.intern_map(
        builtin_type_ids::STRING,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    register_local(&mut builder, scores_name, LocalId(80), map_type, span);

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let receiver_expression = reference_expr_with_type_id(
        &mut expressions,
        scores_name,
        map_type,
        span,
        ValueMode::MutableReference,
    );
    let key_expression = expressions
        .string_slice(
            &mut builder.module_const_values,
            key_name,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string");
    let key = CallArgument::positional(key_expression, CallAccessMode::Shared, span);

    let cases = vec![
        (
            MapBuiltinOp::Get,
            vec![key.clone()],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            HirMapOp::Get,
            1,
        ),
        (
            MapBuiltinOp::Set,
            vec![
                key.clone(),
                CallArgument::positional(
                    {
                        let value =
                            int_expr(&mut expressions, &mut builder.module_const_values, 99, span);
                        expressions.own(value)
                    },
                    CallAccessMode::Shared,
                    span,
                ),
            ],
            vec![],
            HirMapOp::Set,
            2,
        ),
        (
            MapBuiltinOp::Length,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            HirMapOp::Length,
            0,
        ),
    ];

    for (op, args, result_type_ids, expected_op, expected_arg_count) in cases {
        let call_expression = expressions
            .map_builtin_call_with_typed_arguments(
                expressions.own(receiver_expression),
                op,
                expected_op.requires_mutable_receiver(),
                args,
                result_type_ids.clone(),
                span,
            )
            .expect("map builtin call construction should succeed");

        let lowered = builder
            .lower_expression_results(
                expressions
                    .view_owned(call_expression)
                    .expect("map builtin call should be a valid expression"),
            )
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
        assert!(
            matches!(
                &expression_row(&builder, *receiver).kind,
                HirExpressionKind::Load(place)
                    if place.root == LocalId(80) && super::is_local_place(place)
            ),
            "map receiver should lower as the original local place"
        );

        assert_eq!(lowered.values.len(), result_type_ids.len());
        if let Some(expected_type) = result_type_ids.first() {
            let HirLocalDestination::Define(result_local) =
                result.expect("result-bearing map op must define one result")
            else {
                panic!("map result must define its result local");
            };
            let value = lowered.values[0];
            assert!(matches!(
                &expression_row(&builder, value).kind,
                HirExpressionKind::Load(place)
                    if place.root == result_local && super::is_local_place(place)
            ));
            assert_eq!(expression_row(&builder, value).ty, *expected_type);
        } else {
            assert!(result.is_none(), "zero-success map op has no destination");
        }
    }
}

/// Verifies that a choice construct expression (`AstExpressionBuilder::choice_construct`) lowers to
/// `HirExpressionKind::VariantConstruct` with the correct tag index, and that the result type is
/// registered as a choice in `TypeEnvironment`.
///
/// WHY: this is the core contract of the Choice Hardening refactor — choice values must not
/// masquerade as `HirExpressionKind::FixedScalar` in HIR.
#[test]
fn lowers_choice_variant_expression_to_hir_variant_construct() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;

    let status_path = super::symbol("Status", &mut path_fork, &mut string_table);
    let ready_name = string_table.intern("Ready");
    let busy_name = string_table.intern("Busy");

    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let choice_variants = vec![
        ChoiceVariant {
            id: ready_name,
            payload: ChoiceVariantPayload::Unit,
            span,
        },
        ChoiceVariant {
            id: busy_name,
            payload: ChoiceVariantPayload::Unit,
            span,
        },
    ];
    let choice_type_id = builder.test_register_nominal_choice_type(status_path, &choice_variants);
    builder.register_choice_id(&status_path, &span).unwrap();

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let choice_expr = choice_construct_expr(
        &mut expressions,
        status_path,
        0,
        vec![],
        choice_type_id,
        span,
        ValueMode::ImmutableOwned,
    );

    let lowered = builder
        .lower_expression(
            expressions
                .view(choice_expr)
                .expect("choice variant view should resolve"),
        )
        .expect("choice variant lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, lowered.value).value_kind,
        ValueKind::Const
    );

    let (choice_id, variant_index) = match &expression_row(&builder, lowered.value).kind {
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Choice { choice_id },
            variant_index,
            fields,
        } => {
            assert!(
                fields.len() == 0,
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

    let hir_type = builder
        .type_environment
        .get(expression_row(&builder, lowered.value).ty);
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
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let fixed_collection = builder
        .type_environment
        .intern_collection(int_type, Some(4));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let item = int_expr(&mut expressions, &mut builder.module_const_values, 1, span);
    let expression = expressions
        .collection_with_type_id(
            vec![expressions.own(item)],
            CollectionExpressionType {
                element_type_id: int_type,
                fixed_capacity: Some(4),
                collection_type_id: Some(fixed_collection),
            },
            &mut builder.type_environment,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixed collection expression should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(expression)
                .expect("fixed collection view should resolve"),
        )
        .expect("fixed collection expression should lower");

    assert!(
        lowered.prelude.is_empty(),
        "literal collection elements should lower inline"
    );
    assert_eq!(
        expression_row(&builder, lowered.value).ty,
        fixed_collection,
        "HIR collection expression should preserve the exact AST collection TypeId"
    );

    let shape = builder
        .type_environment
        .collection_shape(expression_row(&builder, lowered.value).ty)
        .expect("lowered expression type should remain a collection");
    assert_eq!(shape.fixed_capacity, Some(4));

    match &expression_row(&builder, lowered.value).kind {
        HirExpressionKind::Collection(elements) => {
            let elements = builder.module.expressions.values(*elements);
            assert_eq!(elements.len(), 1);
            assert_eq!(expression_row(&builder, elements[0]).ty, int_type);
        }
        other => panic!("expected HIR collection expression, got {other:?}"),
    }
}

/// Verifies that an option-none expression (`option_none_with_type_id`) lowers to
/// `HirExpressionKind::VariantConstruct` with `HirVariantCarrier::Option` and zero fields.
#[test]
fn lowers_option_none_to_hir_variant_construct() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let option_expr = option_none_expr(
        &mut expressions,
        &mut builder.module_const_values,
        builtin_type_ids::STRING,
        &mut builder.type_environment,
        span,
    );

    let lowered = builder
        .lower_expression(
            expressions
                .view(option_expr)
                .expect("option none view should resolve"),
        )
        .expect("option none lowering should succeed");

    assert!(lowered.prelude.is_empty());

    match &expression_row(&builder, lowered.value).kind {
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 0,
            fields,
        } => {
            assert!(fields.len() == 0, "Option none should have no fields");
        }
        other => panic!("expected VariantConstruct(Option, 0, []), got {other:?}"),
    }
}

#[test]
fn external_float_call_emits_validate_float_in_current_block() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let float_type = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::F64), &span)
        .expect("fixed F64 TypeId should lower in test context");

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call_expr = expressions
        .host_function_call_with_typed_arguments(
            ExternalFunctionId::Synthetic(0),
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::F64)],
            true,
            span,
        )
        .expect("external F64 call should construct");

    let lowered = builder
        .lower_expression_value_to_current_block(
            expressions
                .view_owned(call_expr)
                .expect("external F64 call view should resolve"),
        )
        .expect("external F64 call should lower");

    assert_eq!(expression_row(&builder, lowered).ty, float_type);

    let statements = builder.test_current_block_statements();
    assert!(
        statements.iter().any(|statement| matches!(
            &statement.kind,
            HirStatementKind::Call {
                target: CallTarget::External(ExternalFunctionId::Synthetic(0)),
                ..
            }
        )),
        "external F64 call should emit the raw Call statement"
    );
    assert!(
        statements
            .iter()
            .any(|statement| matches!(&statement.kind, HirStatementKind::ValidateFloat { .. })),
        "external F64 call should emit ValidateFloat after the raw call"
    );
}

#[test]
fn external_fixed_f64_call_allocates_and_validates_its_exact_type() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let f64_type_id =
        builtin_type_ids::fixed_scalar(moth_lexical::numeric::fixed_scalar::FixedScalar::F64);
    let f64_type = builder
        .lower_type_id(f64_type_id, &span)
        .expect("fixed F64 TypeId should lower in test context");

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call_expr = expressions
        .host_function_call_with_typed_arguments(
            ExternalFunctionId::Synthetic(0),
            vec![],
            vec![f64_type_id],
            true,
            span,
        )
        .expect("raw fixed F64 external call should construct");

    let lowered = builder
        .lower_expression_value_to_current_block(
            expressions
                .view_owned(call_expr)
                .expect("raw fixed F64 external call view should resolve"),
        )
        .expect("raw fixed F64 external call should lower");

    assert_eq!(
        expression_row(&builder, lowered).ty,
        f64_type,
        "the validated value must keep the resolved fixed F64 type"
    );

    let validate_result_local = builder
        .test_current_block_statements()
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::ValidateFloat { result, .. } => Some(result.local()),
            _ => None,
        })
        .expect("raw fixed F64 external call should emit ValidateFloat");

    let result_local_type = builder
        .module
        .blocks
        .iter()
        .flat_map(|block| &block.locals)
        .find(|local| local.id == validate_result_local)
        .map(|local| local.ty)
        .expect("ValidateFloat result local should be registered");
    assert_eq!(
        result_local_type, f64_type,
        "the validated result local must use the resolved fixed F64 type"
    );
}

#[test]
fn external_float_call_in_builtin_error_function_validates_through_invoke() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("external_float_boundary", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let float_type = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::F64), &span)
        .expect("fixed F64 TypeId should lower in test context");
    let int_type = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::I32), &span)
        .expect("fixed I32 TypeId should lower in test context");
    let error_type = builder.test_register_builtin_error_type();
    builder.test_register_function_with_returns(
        function_name,
        FunctionId(98),
        vec![int_type],
        Some(error_type),
    );
    builder.test_set_current_function(FunctionId(98));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call_expr = expressions
        .host_function_call_with_typed_arguments(
            ExternalFunctionId::Synthetic(1),
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::F64)],
            true,
            span,
        )
        .expect("external F64 call should construct");

    let lowered = builder
        .lower_expression_value_to_current_block(
            expressions
                .view_owned(call_expr)
                .expect("external F64 call view should resolve"),
        )
        .expect("external F64 call should lower in builtin Error function");

    assert_eq!(expression_row(&builder, lowered).ty, float_type);

    assert!(
        builder.module.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: crate::compiler_frontend::hir::terminators::HirFallibleOperation::ValidateFloat { .. },
                ..
            }
        )),
        "external F64 boundary validation inside builtin Error! functions should be recoverable"
    );
}

#[test]
fn external_int_call_does_not_emit_validate_float() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call_expr = expressions
        .host_function_call_with_typed_arguments(
            ExternalFunctionId::Synthetic(0),
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            false,
            span,
        )
        .expect("external I32 call should construct");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(call_expr)
                .expect("external I32 call view should resolve"),
        )
        .expect("external I32 call should lower");

    assert_eq!(
        expression_row(&builder, lowered.value).ty,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );

    let prelude_has_validate = lowered
        .prelude
        .iter()
        .any(|statement| matches!(&statement.kind, HirStatementKind::ValidateFloat { .. }));
    assert!(
        !prelude_has_validate,
        "external I32 call should not emit ValidateFloat"
    );
}

#[test]
fn external_fallible_float_call_propagation_validates_success() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("fallible_float", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let float_type = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::F64), &span)
        .expect("fixed F64 TypeId should lower in test context");
    let error_type = builder.test_register_builtin_error_type();
    builder.test_register_function_with_returns(
        function_name,
        FunctionId(99),
        vec![float_type],
        Some(error_type),
    );
    builder.test_set_current_function(FunctionId(99));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let handled_call_expr = expressions
        .handled_fallible_host_function_call_with_typed_arguments(
            HandledFallibleHostFunctionCallInput {
                id: ExternalFunctionId::Synthetic(1),
                args: vec![],
                result_type_ids: vec![builtin_type_ids::fixed_scalar(FixedScalar::F64)],
                requires_external_float_validation: true,
                error_type_id: error_type,
                handling: FallibleExpressionHandling::Propagate,
                span,
            },
        )
        .expect("fallible external F64 call should construct");

    let lowered = builder
        .lower_expression_value_to_current_block(
            expressions
                .view_owned(handled_call_expr)
                .expect("fallible external F64 call view should resolve"),
        )
        .expect("fallible external F64 call should lower");

    assert_eq!(expression_row(&builder, lowered).ty, float_type);

    let call_success = builder
        .module
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            HirTerminator::Invoke {
                operation:
                    crate::compiler_frontend::hir::terminators::HirFallibleOperation::Call {
                        target: CallTarget::External(ExternalFunctionId::Synthetic(1)),
                        ..
                    },
                success,
                ..
            } => Some(*success),
            _ => None,
        })
        .expect("fallible external F64 call should invoke the raw ABI call");
    let success_block = &builder.module.blocks[call_success.0 as usize];
    assert!(
        matches!(
            &success_block.terminator,
            HirTerminator::Invoke {
                operation: crate::compiler_frontend::hir::terminators::HirFallibleOperation::ValidateFloat { .. }, ..
            }
        ) || success_block.statements.iter().any(|statement|
            matches!(&statement.kind, HirStatementKind::ValidateFloat { .. })
        ),
        "fallible external F64 call must validate only its successful result"
    );
}

#[test]
fn external_fallible_float_call_catch_validates_success() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = super::symbol("fallible_float_catch", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let float_type = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::F64), &span)
        .expect("fixed F64 TypeId should lower in test context");
    let error_type = builder.test_register_builtin_error_type();
    builder.test_register_function_with_returns(
        function_name,
        FunctionId(100),
        vec![float_type],
        Some(error_type),
    );

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let handled_call_expr = expressions
        .handled_fallible_host_function_call_with_typed_arguments(
            HandledFallibleHostFunctionCallInput {
                id: ExternalFunctionId::Synthetic(2),
                args: vec![],
                result_type_ids: vec![builtin_type_ids::fixed_scalar(FixedScalar::F64)],
                requires_external_float_validation: true,
                error_type_id: error_type,
                handling: FallibleExpressionHandling::Propagate,
                span,
            },
        )
        .expect("fallible external F64 catch call should construct");
    // Parser-produced catches carry a resolved HandledByCatch disposition on the protected
    // node; synthetic fixtures set it explicitly to match.
    let mut handled_facts = expressions
        .view_owned(handled_call_expr)
        .expect("fallible external F64 catch call view should resolve")
        .failure_facts()
        .cloned()
        .unwrap_or_default();
    handled_facts.disposition = FailureDisposition::HandledByCatch {
        error_type_id: error_type,
    };
    let handled_call_expr = expressions
        .with_failure_facts(handled_call_expr, handled_facts)
        .expect("catch disposition should attach to the protected call");

    let float_value = float_expr(
        &mut expressions,
        &mut builder.module_const_values,
        0.0,
        span,
    );
    let catch_expr = wrap_catch_expression(
        &mut expressions,
        handled_call_expr,
        FallibleHandling::Handler {
            error: None,
            body: vec![AstNode {
                kind: NodeKind::ThenValue(ProducedValues {
                    expressions: vec![float_value],
                    span,
                }),
                span,
                scope: function_name,
            }],
        },
        vec![builtin_type_ids::fixed_scalar(FixedScalar::F64)],
    )
    .expect("fallible external F64 catch should wrap");

    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(catch_expr)
                .expect("fallible external F64 catch view should resolve"),
        )
        .expect("fallible external F64 catch should lower");

    assert_eq!(expression_row(&builder, lowered.value).ty, float_type);

    let call_success = builder
        .module
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            HirTerminator::Invoke {
                operation:
                    crate::compiler_frontend::hir::terminators::HirFallibleOperation::Call {
                        target: CallTarget::External(ExternalFunctionId::Synthetic(2)),
                        ..
                    },
                success,
                ..
            } => Some(*success),
            _ => None,
        })
        .expect("fallible external F64 catch should invoke the raw ABI call");
    let success_block = &builder.module.blocks[call_success.0 as usize];
    assert!(
        matches!(
            &success_block.terminator,
            HirTerminator::Invoke {
                operation: crate::compiler_frontend::hir::terminators::HirFallibleOperation::ValidateFloat { .. }, ..
            }
        ) || success_block.statements.iter().any(|statement|
            matches!(&statement.kind, HirStatementKind::ValidateFloat { .. })
        ),
        "fallible external F64 catch must validate its successful result before merge"
    );
}

/// Verifies that an implicit numeric promotion through `AstExpressionBuilder::coerced`
/// lowers to an explicit infallible `Cast` instead of a bare type override.
///
/// WHY: backends select the value conversion from the cast policy. A bare override
/// would leave the source carrier (notably a 64-bit BigInt) under the target
/// `F64` type, so implicit `U32`/`I32` returns formatted as non-finite.
#[test]
fn coerced_numeric_promotion_lowers_to_explicit_infallible_cast() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let float_type = builder
        .lower_type_id(builtin_type_ids::fixed_scalar(FixedScalar::F64), &span)
        .expect("fixed F64 TypeId should lower in test context");

    let mut expressions = AstExpressionBuilder::new().unwrap();
    for (expected_source, inner) in [
        (
            NumericScalar::Fixed(FixedScalar::U32),
            uint_expr(&mut expressions, &mut builder.module_const_values, 41, span),
        ),
        (
            NumericScalar::Fixed(FixedScalar::I32),
            int_expr(&mut expressions, &mut builder.module_const_values, 41, span),
        ),
    ] {
        let coerced = expressions
            .coerced(
                expressions.own(inner),
                builtin_type_ids::fixed_scalar(FixedScalar::F64),
            )
            .expect("fixture coercion");
        let lowered = builder
            .lower_expression(
                expressions
                    .view_owned(coerced)
                    .expect("numeric promotion view should resolve"),
            )
            .expect("numeric promotion should lower in HIR");

        assert!(
            lowered.prelude.is_empty(),
            "{expected_source:?} promotion should convert inline without prelude statements"
        );
        assert_eq!(expression_row(&builder, lowered.value).ty, float_type);
        let (policy_source, policy_target, converted) =
            match &expression_row(&builder, lowered.value).kind {
                HirExpressionKind::Cast { source, policy } => {
                    let BuiltinCastPolicyId::NumericConversion {
                        source: domain_source,
                        target: domain_target,
                    } = *policy
                    else {
                        panic!(
                            "{expected_source:?} promotion should use a numeric conversion policy"
                        );
                    };
                    (domain_source, domain_target, *source)
                }
                other => {
                    panic!("{expected_source:?} promotion should lower to Cast, got {other:?}")
                }
            };
        assert_eq!(policy_source, expected_source);
        assert_eq!(policy_target, NumericScalar::Fixed(FixedScalar::F64));
        match expected_source {
            NumericScalar::Fixed(FixedScalar::U32) => assert!(
                matches!(
                    &builder.module.expressions.expression(converted).kind,
                    HirExpressionKind::FixedScalar(value) if *value == u32_value(41)
                ),
                "U32 promotion should convert the original U32 literal value"
            ),
            NumericScalar::Fixed(FixedScalar::I32) => assert!(
                matches!(
                    &builder.module.expressions.expression(converted).kind,
                    HirExpressionKind::FixedScalar(value) if *value == i32_value(41)
                ),
                "I32 promotion should convert the original I32 literal value"
            ),
            _ => unreachable!("promotion test only covers U32 and I32 sources"),
        }
    }
}

#[test]
fn metadata_replacement_preserves_distinct_occurrence_mappings() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut extended = ExtendedSpanBuilder::new();
    let ast_span = SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(3, 4, &mut extended).unwrap(),
    );
    let source_span = SourceSpan::new(
        SourceId::from_index(2),
        LocalSpan::exact(17, 4, &mut extended).unwrap(),
    );
    let original = builder
        .make_expression(
            &Some(source_span),
            HirExpressionKind::FixedScalar(i32_value(7)),
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ValueKind::Const,
            RegionId(0),
        )
        .unwrap();
    builder
        .side_table
        .map_value(Some(ast_span), original, Some(source_span));
    let before = builder.module.expressions.measurements();
    let unchanged = builder
        .replace_expression_metadata(original, &Some(source_span), None, None)
        .unwrap();
    assert_eq!(unchanged, original);
    assert_eq!(builder.module.expressions.measurements(), before);

    let replacement = builder
        .replace_expression_metadata(original, &Some(source_span), None, Some(RegionId(1)))
        .unwrap();
    assert_ne!(replacement, original);
    for value in [original, replacement] {
        assert_eq!(builder.side_table.value_ast_span(value), Some(ast_span));
        assert_eq!(
            builder.side_table.value_source_span(value),
            Some(source_span)
        );
        assert_eq!(
            builder.module.expressions.expression(value).span,
            Some(source_span)
        );
    }
    assert_eq!(
        builder.module.expressions.expression(original).region,
        RegionId(0)
    );
    assert_eq!(
        builder.module.expressions.expression(replacement).region,
        RegionId(1)
    );

    let new_span = SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(29, 4, &mut extended).unwrap(),
    );
    let retagged = builder
        .replace_expression_metadata(replacement, &Some(new_span), None, None)
        .unwrap();
    assert_ne!(retagged, replacement);
    assert_eq!(builder.side_table.value_ast_span(retagged), Some(new_span));
    assert_eq!(
        builder.side_table.value_source_span(retagged),
        Some(new_span)
    );
    assert_eq!(
        builder.module.expressions.expression(retagged).span,
        Some(new_span)
    );
    assert_eq!(
        builder.side_table.value_ast_span(replacement),
        Some(ast_span)
    );
    assert_eq!(
        builder.side_table.value_source_span(replacement),
        Some(source_span)
    );
}

/// Lowers a 4096-level runtime-leaf numeric chain on a 1 MiB thread.
///
/// WHAT: `x + 1 + 1 ...` builds a left-deep binop tree whose depth equals the chain length.
/// WHY: AST-to-HIR lowering must walk the chain with a heap stack, not one native frame per level.
#[test]
fn lowers_deep_runtime_numeric_chain_without_stack_growth() {
    const DEPTH: usize = 4096;
    const ONE_MEBIBYTE: usize = 1024 * 1024;
    let worker = std::thread::Builder::new()
        .name("deep-numeric-lowering".to_owned())
        .stack_size(ONE_MEBIBYTE)
        .spawn(move || lower_runtime_numeric_chain(DEPTH))
        .expect("deep lowering test thread should spawn");
    if let Err(payload) = worker.join() {
        std::panic::resume_unwind(payload);
    }
}

fn lower_runtime_numeric_chain(depth: usize) {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let span = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    register_local(&mut builder, x, LocalId(10), i32_type, span);

    let x_ref = reference_expr_with_type_id(
        &mut expressions,
        x,
        i32_type,
        span,
        ValueMode::ImmutableReference,
    );
    let mut items = Vec::with_capacity(depth * 2 + 1);
    items.push(runtime_operand_item(x_ref));
    for _ in 0..depth {
        let one = int_expr(&mut expressions, &mut builder.module_const_values, 1, span);
        items.push(runtime_operand_item(one));
        items.push(runtime_operator_item(Operator::Add, span));
    }
    let root = runtime_expr(
        &mut expressions,
        &mut builder.module_const_values,
        items,
        i32_type,
        span,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    let lowered = builder
        .lower_expression(
            expressions
                .view(root)
                .expect("deep numeric chain should have a view"),
        )
        .expect("deep numeric chain lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(expression_row(&builder, lowered.value).ty, i32_type);
    assert!(
        matches!(
            &expression_row(&builder, lowered.value).kind,
            HirExpressionKind::Load(place) if super::is_local_place(place)
        ),
        "deep checked addition should return a load of the final NumericOp result local"
    );
}
