//! Focused tests for checked numeric HIR lowering (Phase 6b).
//!
//! Verifies numeric domains, operand conversions and native failure delivery. Recoverable
//! operations define success and error locals on Invoke edges; discharged and trapping operations
//! remain statements.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::eval_expression::result_type::resolved_operation;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CastFacts, ExprId,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expression_store::{HirConstructionFailure, HirExpressionStore};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_builder::{register_local, setup_builder};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{
    HirFallibleOperation, HirTerminator, RuntimeFailureCause,
};
use crate::compiler_frontend::hir::tests::symbol;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, node, reference_expr_with_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn int_expr(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> ExprId {
    expressions
        .fixed_scalar(
            constants,
            FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range"),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal fits store")
        .id()
}

fn float_expr(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: f64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> ExprId {
    expressions
        .fixed_scalar(
            constants,
            FixedScalarValue::binary_float(FixedScalar::F64, value)
                .expect("F64 literal is exactly representable"),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal fits store")
        .id()
}

/// Builds a resolved operation node in the expression store with its canonical operator facts.
#[allow(
    clippy::too_many_arguments,
    reason = "test fixture keeps operands, operator, result type, span, mode and environment separate"
)]
fn resolved_numeric_expression(
    expressions: &mut AstExpressionBuilder,
    left: ExprId,
    right: Option<ExprId>,
    operator: Operator,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
    mode: ValueMode,
    environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
) -> ExprId {
    let left_type = expressions
        .view(left)
        .expect("fixture left operand")
        .value_type_id()
        .expect("single fixture operand");
    let right_type = right.map(|right| {
        expressions
            .view(right)
            .expect("fixture right operand")
            .value_type_id()
            .expect("single fixture operand")
    });
    let facts = resolved_operation(
        operator,
        span,
        result_type,
        left_type,
        right_type,
        environment,
    )
    .unwrap_or_else(|_| panic!("fixture operation facts resolve"));
    expressions
        .resolved_operation(
            expressions.own(left),
            right.map(|right| expressions.own(right)),
            facts,
            mode,
        )
        .expect("fixture operation fits store")
        .id()
}

/// Delivery is separate from the numeric domain and from the operation's stable node identity.
#[derive(Debug, PartialEq, Eq)]
enum NumericDelivery {
    Statement(NumericFailureMode),
    Invoke,
}

#[derive(Debug)]
struct FoundNumericOp {
    id: HirNodeId,
    block: BlockId,
    op: HirNumericOp,
    failure_mode: NumericDelivery,
    operands: HirNumericOperands,
    result: LocalId,
}

fn find_numeric_op_statement(builder: &HirBuilder<'_>) -> Option<FoundNumericOp> {
    builder.module.blocks.iter().find_map(|block| {
        for statement in &block.statements {
            if let HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            } = &statement.kind
            {
                return Some(FoundNumericOp {
                    id: statement.id,
                    block: block.id,
                    op: *op,
                    failure_mode: NumericDelivery::Statement(*failure_mode),
                    operands: operands.clone(),
                    result: result.local(),
                });
            }
        }
        match &block.terminator {
            HirTerminator::Invoke {
                id,
                operation: HirFallibleOperation::NumericOp { op, operands },
                success_results,
                ..
            } => {
                assert_eq!(
                    success_results.len(),
                    1,
                    "numeric operations define one success slot"
                );
                Some(FoundNumericOp {
                    id: *id,
                    block: block.id,
                    op: *op,
                    failure_mode: NumericDelivery::Invoke,
                    operands: operands.clone(),
                    result: success_results[0],
                })
            }
            _ => None,
        }
    })
}

fn find_single_numeric_op(builder: &HirBuilder<'_>) -> Option<(HirNumericOp, NumericDelivery)> {
    find_numeric_op_statement(builder).map(|found| (found.op, found.failure_mode))
}

fn fixed_type(scalar: FixedScalar) -> crate::compiler_frontend::datatypes::ids::TypeId {
    builtin_type_ids::fixed_scalar(scalar)
}

fn find_single_numeric_op_with_operands(
    builder: &HirBuilder<'_>,
) -> Option<(HirNumericOp, HirNumericOperands)> {
    find_numeric_op_statement(builder).map(|found| (found.op, found.operands))
}

#[derive(Debug)]
struct LoweredNumericFixture {
    expressions: HirExpressionStore,
    value: HirValueId,
    numeric_op: Option<(HirNumericOp, HirNumericOperands)>,
    failure_mode: Option<NumericDelivery>,
}

fn lower_binary_with_types(
    op: Operator,
    left_type: crate::compiler_frontend::datatypes::ids::TypeId,
    right_type: crate::compiler_frontend::datatypes::ids::TypeId,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> LoweredNumericFixture {
    try_lower_binary_with_types(op, left_type, right_type, result_type)
        .expect("typed binary operator lowering should succeed")
}

fn try_lower_binary_with_types(
    op: Operator,
    left_type: crate::compiler_frontend::datatypes::ids::TypeId,
    right_type: crate::compiler_frontend::datatypes::ids::TypeId,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> Result<LoweredNumericFixture, HirConstructionFailure> {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let left_name = symbol("left", &mut path_fork, &mut string_table);
    let right_name = symbol("right", &mut path_fork, &mut string_table);
    let left = reference_expr_with_type_id(
        &mut expressions,
        left_name,
        left_type,
        loc,
        ValueMode::ImmutableReference,
    );
    let right = reference_expr_with_type_id(
        &mut expressions,
        right_name,
        right_type,
        loc,
        ValueMode::ImmutableReference,
    );
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let expression = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        op,
        result_type,
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    register_local(&mut builder, left_name, LocalId(10), left_type, loc);
    register_local(&mut builder, right_name, LocalId(11), right_type, loc);
    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expression)
        .expect("fixture expression belongs to builder");
    let lowered = builder.lower_expression(view)?;

    let found = find_numeric_op_statement(&builder);
    Ok(LoweredNumericFixture {
        expressions: builder.module.expressions,
        value: lowered.value,
        numeric_op: found
            .as_ref()
            .map(|found| (found.op, found.operands.clone())),
        failure_mode: found.map(|found| found.failure_mode),
    })
}

fn lower_negation_with_type(
    operand_type: crate::compiler_frontend::datatypes::ids::TypeId,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> LoweredNumericFixture {
    try_lower_negation_with_type(operand_type, result_type)
        .expect("typed negation lowering should succeed")
}

fn try_lower_negation_with_type(
    operand_type: crate::compiler_frontend::datatypes::ids::TypeId,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> Result<LoweredNumericFixture, HirConstructionFailure> {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let operand_name = symbol("operand", &mut path_fork, &mut string_table);
    let operand = reference_expr_with_type_id(
        &mut expressions,
        operand_name,
        operand_type,
        loc,
        ValueMode::ImmutableReference,
    );
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let expression = resolved_numeric_expression(
        &mut expressions,
        operand,
        None,
        Operator::Negate,
        result_type,
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    register_local(&mut builder, operand_name, LocalId(10), operand_type, loc);
    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expression)
        .expect("fixture expression belongs to builder");
    let lowered = builder.lower_expression(view)?;
    let found =
        find_numeric_op_statement(&builder).expect("numeric negation should emit one NumericOp");

    Ok(LoweredNumericFixture {
        expressions: builder.module.expressions,
        value: lowered.value,
        numeric_op: Some((found.op, found.operands)),
        failure_mode: Some(found.failure_mode),
    })
}

/// Lowers `1 (coerced to F64) <op> 2.5` in either operand order.
///
/// WHAT: builds the explicitly converted integer operand the AST records for a supported
///       fixed-integer/binary-float comparison so the conversion is visible to lowering.
/// WHY: completed I32 and F64 operands have no direct comparison; only the converted form is a
///      valid HIR shape, so the fixture must exercise the conversion rather than retype it.
fn lower_explicitly_converted_int_float_comparison(
    operator: Operator,
    int_on_left: bool,
) -> LoweredNumericFixture {
    let loc = None;
    let float_type = fixed_type(FixedScalar::F64);
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let int_value = int_expr(&mut expressions, &mut constants, 1, loc);
    let int_operand = expressions
        .coerced(expressions.own(int_value), float_type)
        .expect("fixture coercion fits store");
    let float_operand = float_expr(&mut expressions, &mut constants, 2.5, loc);
    let (left, right) = if int_on_left {
        (int_operand.id(), float_operand)
    } else {
        (float_operand, int_operand.id())
    };
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let expression = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        operator,
        builtin_type_ids::BOOL,
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expression)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("explicitly converted int/float comparison lowering should succeed");
    let found = find_numeric_op_statement(&builder);
    LoweredNumericFixture {
        expressions: builder.module.expressions,
        value: lowered.value,
        numeric_op: found
            .as_ref()
            .map(|found| (found.op, found.operands.clone())),
        failure_mode: found.map(|found| found.failure_mode),
    }
}

fn numeric_conversion(
    expressions: &HirExpressionStore,
    value: HirValueId,
) -> Option<(NumericScalar, NumericScalar)> {
    match &expressions.expression(value).kind {
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion { source, target },
            ..
        } => Some((*source, *target)),
        _ => None,
    }
}

fn expression_row<'a>(builder: &'a HirBuilder<'_>, value: HirValueId) -> &'a HirExpression {
    let row: &'a HirExpression = builder.module.expressions.expression(value);
    row
}

fn set_current_function_return_type(
    builder: &mut HirBuilder<'_>,
    function_id: FunctionId,
    return_type: crate::compiler_frontend::datatypes::ids::TypeId,
    name: PathId,
) {
    builder.test_register_function_with_returns(name, function_id, vec![return_type], None);
    builder.test_set_current_function(function_id);
}

fn lower_u8_compound_add_assignment(
    builder: &mut HirBuilder<'_>,
    target_name: PathId,
    right_name: PathId,
) -> Result<(), HirConstructionFailure> {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let u8_type = fixed_type(FixedScalar::U8);
    let i8_type = fixed_type(FixedScalar::I8);
    let i16_type = fixed_type(FixedScalar::I16);
    let left = reference_expr_with_type_id(
        &mut expressions,
        target_name,
        u8_type,
        None,
        ValueMode::ImmutableReference,
    );
    let right = reference_expr_with_type_id(
        &mut expressions,
        right_name,
        i8_type,
        None,
        ValueMode::ImmutableReference,
    );
    let source = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        Operator::Add,
        i16_type,
        None,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    let value = expressions
        .cast(
            expressions.own(source),
            CastFacts {
                source_type_id: i16_type,
                target_type_id: u8_type,
                requires_optional_wrap_after_cast: false,
                fallibility: BuiltinCastFallibility::Fallible,
                evidence: ResolvedCastEvidence::Builtin {
                    policy: BuiltinCastPolicyId::NumericConversion {
                        source: NumericScalar::Fixed(FixedScalar::I16),
                        target: NumericScalar::Fixed(FixedScalar::U8),
                    },
                },
                handling: CastHandling::StoreConversion,
                span: None,
            },
            u8_type,
        )
        .expect("fixture cast fits store");
    let statement = node(
        NodeKind::Assignment {
            target: assignment_target(
                &mut expressions,
                target_name,
                DataType::Inferred,
                u8_type,
                None,
            ),
            value: value.id(),
        },
        None,
    );
    builder.lower_statement_node(expressions.store(), &statement)
}

/// Lowers a same-type compound update (`target scalar += right scalar`).
///
/// WHAT: builds the declaration-compatible arithmetic value the AST produces when the promoted
///       result already matches the target's exact type, so no store conversion exists.
/// WHY: same-type narrow arithmetic fails inside its own domain; its fixture must stay separate
///      from the mixed-domain write-back fixture above.
fn lower_same_type_compound_add_assignment(
    builder: &mut HirBuilder<'_>,
    target_name: PathId,
    right_name: PathId,
    scalar: FixedScalar,
) -> Result<(), HirConstructionFailure> {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let scalar_type = fixed_type(scalar);
    let left = reference_expr_with_type_id(
        &mut expressions,
        target_name,
        scalar_type,
        None,
        ValueMode::ImmutableReference,
    );
    let right = reference_expr_with_type_id(
        &mut expressions,
        right_name,
        scalar_type,
        None,
        ValueMode::ImmutableReference,
    );
    let value = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        Operator::Add,
        scalar_type,
        None,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    let statement = node(
        NodeKind::Assignment {
            target: assignment_target(
                &mut expressions,
                target_name,
                DataType::Inferred,
                scalar_type,
                None,
            ),
            value,
        },
        None,
    );
    builder.lower_statement_node(expressions.store(), &statement)
}

fn store_conversion_branch(builder: &HirBuilder<'_>) -> (BlockId, BlockId, LocalId, LocalId) {
    builder
        .module
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            HirTerminator::Invoke {
                operation:
                    HirFallibleOperation::CastOp {
                        policy:
                            BuiltinCastPolicyId::NumericConversion {
                                source: NumericScalar::Fixed(FixedScalar::I16),
                                target: NumericScalar::Fixed(FixedScalar::U8),
                            },
                        ..
                    },
                success,
                failure,
                success_results,
                error_result,
                ..
            } => {
                assert_eq!(success_results.len(), 1);
                assert_ne!(success_results[0], *error_result);
                Some((*success, *failure, success_results[0], *error_result))
            }
            _ => None,
        })
        .expect("compound U8 narrowing should invoke a checked conversion")
}

fn compound_assignment_store_value<'a>(
    builder: &'a HirBuilder<'_>,
    target_local: LocalId,
    expected_block: crate::compiler_frontend::hir::ids::BlockId,
) -> &'a HirExpression {
    let assignment_blocks: Vec<_> = builder
        .module
        .blocks
        .iter()
        .filter(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::AssignPlace(target),
                        ..
                    } if target.root == target_local && super::is_local_place(target)
                )
            })
        })
        .map(|block| block.id)
        .collect();
    assert_eq!(
        assignment_blocks,
        vec![expected_block],
        "the compound assignment store must occur exactly once on the success continuation"
    );

    let success_block = &builder.module.blocks[expected_block.0 as usize];
    success_block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(target),
                value,
            } if target.root == target_local && super::is_local_place(target) => {
                Some(builder.module.expressions.expression(*value))
            }
            _ => None,
        })
        .expect("success continuation should update the compound target")
}

#[test]
fn compound_u8_store_conversion_traps_before_writing_target() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = symbol(
        "__test_compound_store_trap",
        &mut path_fork,
        &mut string_table,
    );
    let target_name = symbol("target", &mut path_fork, &mut string_table);
    let right_name = symbol("right", &mut path_fork, &mut string_table);
    let u8_type = fixed_type(FixedScalar::U8);
    let i8_type = fixed_type(FixedScalar::I8);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_builtin_error_type();
    set_current_function_return_type(&mut builder, FunctionId(1), u8_type, function_name);
    register_local(&mut builder, target_name, LocalId(10), u8_type, None);
    register_local(&mut builder, right_name, LocalId(11), i8_type, None);
    lower_u8_compound_add_assignment(&mut builder, target_name, right_name)
        .expect("compound U8 store lowering should succeed");
    let (op, failure_mode) =
        find_single_numeric_op(&builder).expect("compound U8 addition should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::I16),
        }
    );
    // The mixed-width U8 + I8 arithmetic completes inside its I16 common domain: it carries
    // no failure edge. The narrowing store conversion back to U8 remains the failing check
    // under test.
    assert_eq!(
        failure_mode,
        NumericDelivery::Statement(NumericFailureMode::Infallible)
    );

    let (success_block, error_block, converted, error) = store_conversion_branch(&builder);
    let error_block = &builder.module.blocks[error_block.0 as usize];
    assert!(
        matches!(
            error_block.terminator,
            HirTerminator::RuntimeFailure {
                cause: Some(RuntimeFailureCause::StoreConversion { error: cause_error }),
                ..
            } if cause_error == error
        ),
        "non-fallible conversion failure must retain its edge-local error"
    );
    let stored_value = compound_assignment_store_value(&builder, LocalId(10), success_block);
    assert!(
        matches!(
            &stored_value.kind,
            HirExpressionKind::Load(place) if place.root == converted && super::is_local_place(place)
        ),
        "the store must consume the successful conversion local"
    );
}

#[test]
fn compound_u8_store_conversion_returns_error_before_writing_target() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = symbol(
        "__test_compound_store_error",
        &mut path_fork,
        &mut string_table,
    );
    let target_name = symbol("target", &mut path_fork, &mut string_table);
    let right_name = symbol("right", &mut path_fork, &mut string_table);
    let u8_type = fixed_type(FixedScalar::U8);
    let i8_type = fixed_type(FixedScalar::I8);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let error_type = builder.test_register_builtin_error_type();
    builder.test_register_function_with_returns(
        function_name,
        FunctionId(1),
        vec![u8_type],
        Some(error_type),
    );
    builder.test_set_current_function(FunctionId(1));
    register_local(&mut builder, target_name, LocalId(10), u8_type, None);
    register_local(&mut builder, right_name, LocalId(11), i8_type, None);
    lower_u8_compound_add_assignment(&mut builder, target_name, right_name)
        .expect("compound U8 store lowering should succeed");
    let (op, failure_mode) =
        find_single_numeric_op(&builder).expect("compound U8 addition should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::I16),
        }
    );
    // Discharged arithmetic stays Infallible even inside Error! functions: only the store
    // conversion takes the recoverable path asserted below.
    assert_eq!(
        failure_mode,
        NumericDelivery::Statement(NumericFailureMode::Infallible)
    );

    let (success_block, error_block, converted, error) = store_conversion_branch(&builder);
    let error_block = &builder.module.blocks[error_block.0 as usize];
    assert!(
        matches!(&error_block.terminator, HirTerminator::ReturnError(value)
            if matches!(&expression_row(&builder, *value).kind,
                HirExpressionKind::Load(place) if place.root == error)),
        "builtin Error! conversion failure must return through the error edge"
    );
    let stored_value = compound_assignment_store_value(&builder, LocalId(10), success_block);
    assert!(
        matches!(
            &stored_value.kind,
            HirExpressionKind::Load(place) if place.root == converted && super::is_local_place(place)
        ),
        "the store must consume the successful conversion payload"
    );
}

#[test]
fn equal_type_compound_store_emits_no_conversion() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = symbol(
        "__test_compound_store_equal_type",
        &mut path_fork,
        &mut string_table,
    );
    let target_name = symbol("target", &mut path_fork, &mut string_table);
    let right_name = symbol("right", &mut path_fork, &mut string_table);
    let u32_type = fixed_type(FixedScalar::U32);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    set_current_function_return_type(&mut builder, FunctionId(1), u32_type, function_name);
    register_local(&mut builder, target_name, LocalId(10), u32_type, None);
    register_local(&mut builder, right_name, LocalId(11), u32_type, None);
    lower_same_type_compound_add_assignment(
        &mut builder,
        target_name,
        right_name,
        FixedScalar::U32,
    )
    .expect("same-type compound assignment lowering should succeed");

    assert!(
        !builder.module.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::CastOp { .. },
                ..
            }
        )),
        "same-type arithmetic should not emit a conversion branch"
    );
    let _stored_value = compound_assignment_store_value(
        &builder,
        LocalId(10),
        crate::compiler_frontend::hir::ids::BlockId(0),
    );
}

// WHAT: verifies that a same-type narrow compound update keeps its checked arithmetic failure
//       and updates the target only through the success continuation.
// WHY: D15 makes `U8 + U8` potentially fallible inside U8, so the mixed-domain witness (whose
//      failure lives in the store conversion) no longer proves the write-back gate; a stale
//      discharged operation or a partial commit on the failing path must fail this test.
#[test]
fn compound_same_type_u8_arithmetic_returns_error_before_writing_target() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let function_name = symbol(
        "__test_compound_same_type_error",
        &mut path_fork,
        &mut string_table,
    );
    let target_name = symbol("target", &mut path_fork, &mut string_table);
    let right_name = symbol("right", &mut path_fork, &mut string_table);
    let u8_type = fixed_type(FixedScalar::U8);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let error_type = builder.test_register_builtin_error_type();
    builder.test_register_function_with_returns(
        function_name,
        FunctionId(1),
        vec![u8_type],
        Some(error_type),
    );
    builder.test_set_current_function(FunctionId(1));
    register_local(&mut builder, target_name, LocalId(10), u8_type, None);
    register_local(&mut builder, right_name, LocalId(11), u8_type, None);
    lower_same_type_compound_add_assignment(&mut builder, target_name, right_name, FixedScalar::U8)
        .expect("same-type compound U8 addition lowering should succeed");

    let found = find_numeric_op_statement(&builder)
        .expect("same-type compound U8 addition should emit a NumericOp");
    assert_eq!(
        found.op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U8),
        }
    );
    // `255 + 1` overflows U8, so the operation stays checked; the builtin Error! contract must
    // deliver that failure recoverably rather than discharging it or substituting a trap.
    assert_eq!(found.failure_mode, NumericDelivery::Invoke);
    assert!(
        !builder.module.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::CastOp { .. },
                ..
            }
        )),
        "a same-type compound update must not fabricate a store conversion"
    );

    let numeric_block = &builder.module.blocks[found.block.0 as usize];
    let HirTerminator::Invoke {
        id,
        success,
        failure,
        success_results,
        error_result,
        ..
    } = &numeric_block.terminator
    else {
        panic!("checked same-type U8 arithmetic must invoke its failure continuation");
    };
    assert_eq!(*id, found.id);
    assert_eq!(success_results, &[found.result]);
    assert_ne!(found.result, *error_result);
    let (success_block, error_block) = (*success, *failure);
    let error_row = &builder.module.blocks[error_block.0 as usize];
    assert!(
        matches!(&error_row.terminator, HirTerminator::ReturnError(_)),
        "the failing addition must return before the target is updated"
    );

    // The only write to the target sits on the success continuation and consumes this exact
    // addition's success payload, so a failed update can never commit a partial result.
    let stored_value = compound_assignment_store_value(&builder, LocalId(10), success_block);
    assert!(
        matches!(
            &stored_value.kind,
            HirExpressionKind::Load(place)
                if place.root == found.result && super::is_local_place(place)
        ),
        "the store must consume the arithmetic success payload after its failure branch"
    );
}

#[test]
fn checked_int_addition_lowers_to_int_add_numeric_op() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let x_name = symbol("x", &mut path_fork, &mut string_table);
    let x_ref = reference_expr_with_type_id(
        &mut expressions,
        x_name,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::ImmutableReference,
    );
    let two = int_expr(&mut expressions, &mut constants, 2, loc);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    assert_eq!(
        builder.module.start_function,
        Some(FunctionId(0)),
        "the checked addition test is lowered in the top-level start function"
    );
    register_local(
        &mut builder,
        x_name,
        LocalId(10),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
    );

    let expr = resolved_numeric_expression(
        &mut expressions,
        x_ref,
        Some(two),
        Operator::Add,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expr)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("int addition lowering should succeed");

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
        "checked addition should return a load of the NumericOp result"
    );

    let (op, failure_mode) = find_single_numeric_op(&builder)
        .expect("int addition should emit exactly one NumericOp statement");
    assert!(
        op == HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::I32)
        }
    );
    assert!(matches!(
        failure_mode,
        NumericDelivery::Statement(NumericFailureMode::Trap)
    ));
}

#[test]
fn checked_int_subtraction_lowers_to_int_sub_numeric_op() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let left = int_expr(&mut expressions, &mut constants, 5, loc);
    let right = int_expr(&mut expressions, &mut constants, 3, loc);
    let expr = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        Operator::Subtract,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expr)
        .expect("fixture expression belongs to builder");
    let _ = builder
        .lower_expression(view)
        .expect("int subtraction lowering should succeed");

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        op == HirNumericOp {
            operator: NumericOperator::Subtract,
            domain: NumericScalar::Fixed(FixedScalar::I32)
        }
    );
}

#[test]
fn checked_regular_division_lowers_to_float_div_numeric_op() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let left = int_expr(&mut expressions, &mut constants, 5, loc);
    let right = int_expr(&mut expressions, &mut constants, 2, loc);
    let expr = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        Operator::Divide,
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expr)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("regular division lowering should succeed");

    assert_eq!(
        expression_row(&builder, lowered.value).ty,
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        op == HirNumericOp {
            operator: NumericOperator::Divide,
            domain: NumericScalar::Fixed(FixedScalar::F64)
        }
    );

    // Both I32 operands must have been explicitly converted to F64 before the division.
    let (_, numeric_op) =
        find_single_numeric_op_with_operands(&builder).expect("numeric division should exist");
    let HirNumericOperands::Binary { left, right } = numeric_op else {
        panic!("F64 divide should be binary");
    };
    assert!(matches!(
        &builder.module.expressions.expression(left).kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Fixed(FixedScalar::I32),
                target: NumericScalar::Fixed(FixedScalar::F64),
            },
            ..
        }
    ));
    assert!(matches!(
        &builder.module.expressions.expression(right).kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Fixed(FixedScalar::I32),
                target: NumericScalar::Fixed(FixedScalar::F64),
            },
            ..
        }
    ));
}

#[test]
fn mixed_int_float_addition_converts_the_explicitly_coerced_operand() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let f64_type = builtin_type_ids::fixed_scalar(FixedScalar::F64);
    // Bounded fixed domains never mix implicitly, so the integer operand carries an
    // explicit coercion to the receiving F64 domain before the operation.
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let int_value = int_expr(&mut expressions, &mut constants, 1, loc);
    let left = expressions
        .coerced(expressions.own(int_value), f64_type)
        .expect("fixture coercion fits store");
    let right = float_expr(&mut expressions, &mut constants, 2.5, loc);
    let expr = resolved_numeric_expression(
        &mut expressions,
        left.id(),
        Some(right),
        Operator::Add,
        f64_type,
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expr)
        .expect("fixture expression belongs to builder");
    let _ = builder
        .lower_expression(view)
        .expect("mixed addition lowering should succeed");

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        op == HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::F64)
        }
    );

    let (_, numeric_op) =
        find_single_numeric_op_with_operands(&builder).expect("numeric addition should exist");
    let HirNumericOperands::Binary { left, right } = numeric_op else {
        panic!("F64 add should be binary");
    };
    assert!(matches!(
        &builder.module.expressions.expression(left).kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Fixed(FixedScalar::I32),
                target: NumericScalar::Fixed(FixedScalar::F64),
            },
            ..
        }
    ));
    assert!(matches!(
        &builder.module.expressions.expression(right).kind,
        HirExpressionKind::FixedScalar(value) if value.scalar() == FixedScalar::F64
    ));
}

#[test]
fn unary_int_negation_lowers_to_int_neg_numeric_op() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let x_name = symbol("x", &mut path_fork, &mut string_table);
    let x_ref = reference_expr_with_type_id(
        &mut expressions,
        x_name,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::ImmutableReference,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(
        &mut builder,
        x_name,
        LocalId(10),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
    );

    let expr = resolved_numeric_expression(
        &mut expressions,
        x_ref,
        None,
        Operator::Negate,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expr)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("unary negation lowering should succeed");

    assert_eq!(
        expression_row(&builder, lowered.value).ty,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        op == HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Fixed(FixedScalar::I32)
        }
    );
}

#[test]
fn numeric_failure_uses_invoke_for_builtin_error_function() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let fn_name = symbol("__test_fn_error", &mut path_fork, &mut string_table);
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let left = int_expr(&mut expressions, &mut constants, 1, loc);
    let right = int_expr(&mut expressions, &mut constants, 2, loc);
    let expr = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        Operator::Add,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    let error_type_id = builder.test_register_builtin_error_type();
    builder.test_register_function_with_returns(
        fn_name,
        FunctionId(1),
        vec![fixed_type(FixedScalar::I32)],
        Some(error_type_id),
    );
    builder.test_set_current_function(FunctionId(1));

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expr)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("addition lowering in Error! function should succeed");

    let found = find_numeric_op_statement(&builder).expect("expected a NumericOp");
    assert_eq!(found.failure_mode, NumericDelivery::Invoke);
    assert!(
        matches!(
            &expression_row(&builder, lowered.value).kind,
            HirExpressionKind::Load(place) if place.root == found.result && super::is_local_place(place)
        ),
        "recoverable numeric lowering should continue with its edge-defined success local"
    );
    assert!(
        matches!(
            &builder.module.blocks[found.block.0 as usize].terminator,
            HirTerminator::Invoke { id, success_results, error_result, .. }
                if *id == found.id && success_results == &[found.result] && *error_result != found.result
        ),
        "recoverable numeric lowering preserves operation identity and disjoint edge definitions"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, HirTerminator::ReturnError(_))),
        "recoverable numeric lowering should emit a builtin Error return edge"
    );
}

#[test]
fn numeric_failure_mode_is_trap_for_custom_error_function() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let fn_name = symbol("__test_fn_string_error", &mut path_fork, &mut string_table);
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let left = int_expr(&mut expressions, &mut constants, 1, loc);
    let right = int_expr(&mut expressions, &mut constants, 2, loc);
    let expr = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        Operator::Add,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    builder.test_register_function_with_returns(
        fn_name,
        FunctionId(1),
        vec![fixed_type(FixedScalar::I32)],
        Some(builtin_type_ids::STRING),
    );
    builder.test_set_current_function(FunctionId(1));

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expr)
        .expect("fixture expression belongs to builder");
    let _ = builder
        .lower_expression(view)
        .expect("addition lowering in custom-error function should succeed");

    let (_, failure_mode) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        matches!(
            failure_mode,
            NumericDelivery::Statement(NumericFailureMode::Trap)
        ),
        "custom error functions should trap on numeric failure"
    );
}

#[test]
fn numeric_failure_mode_is_trap_for_non_fallible_function() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let fn_name = symbol("__test_fn_non_fallible", &mut path_fork, &mut string_table);
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let left = int_expr(&mut expressions, &mut constants, 1, loc);
    let right = int_expr(&mut expressions, &mut constants, 2, loc);
    let expr = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        Operator::Add,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    set_current_function_return_type(
        &mut builder,
        FunctionId(1),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        fn_name,
    );

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expr)
        .expect("fixture expression belongs to builder");
    let _ = builder
        .lower_expression(view)
        .expect("addition lowering in non-fallible function should succeed");

    let (_, failure_mode) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        matches!(
            failure_mode,
            NumericDelivery::Statement(NumericFailureMode::Trap)
        ),
        "non-fallible functions should trap on numeric failure"
    );
}

#[test]
fn fixed_u8_addition_keeps_the_operand_domain() {
    let u8_type = fixed_type(FixedScalar::U8);
    let fixture = lower_binary_with_types(Operator::Add, u8_type, u8_type, u8_type);

    assert_eq!(fixture.expressions.expression(fixture.value).ty, u8_type);
    let (op, operands) = fixture
        .numeric_op
        .expect("fixed addition should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U8),
        }
    );
    let HirNumericOperands::Binary { left, right } = operands else {
        panic!("fixed addition should have binary operands");
    };
    assert_eq!(fixture.expressions.expression(left).ty, u8_type);
    assert_eq!(fixture.expressions.expression(right).ty, u8_type);
    assert!(numeric_conversion(&fixture.expressions, left).is_none());
    assert!(numeric_conversion(&fixture.expressions, right).is_none());
    // `255 + 1` overflows U8, so the narrowed domain must stay checked. The fixture lowers in the
    // non-fallible start function, whose exact delivery is a trap; the old promoted U32 domain
    // discharged this operation to `Infallible` and must not return.
    assert_eq!(
        fixture.failure_mode,
        Some(NumericDelivery::Statement(NumericFailureMode::Trap)),
        "same-type U8 addition must keep its checked trap delivery"
    );
}

#[test]
fn fixed_i32_i64_addition_promotes_only_the_left_operand() {
    let i32_type = fixed_type(FixedScalar::I32);
    let i64_type = fixed_type(FixedScalar::I64);
    let fixture = lower_binary_with_types(Operator::Add, i32_type, i64_type, i64_type);

    assert_eq!(fixture.expressions.expression(fixture.value).ty, i64_type);
    let (op, operands) = fixture
        .numeric_op
        .expect("fixed addition should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::I64),
        }
    );
    let HirNumericOperands::Binary { left, right } = operands else {
        panic!("fixed addition should have binary operands");
    };
    assert_eq!(
        numeric_conversion(&fixture.expressions, left),
        Some((
            NumericScalar::Fixed(FixedScalar::I32),
            NumericScalar::Fixed(FixedScalar::I64),
        ))
    );
    assert_eq!(fixture.expressions.expression(left).ty, i64_type);
    assert_eq!(fixture.expressions.expression(right).ty, i64_type);
    assert!(numeric_conversion(&fixture.expressions, right).is_none());
}

#[test]
fn fixed_u8_division_converts_both_operands_to_f64() {
    let u8_type = fixed_type(FixedScalar::U8);
    let f64_type = fixed_type(FixedScalar::F64);
    let fixture = lower_binary_with_types(Operator::Divide, u8_type, u8_type, f64_type);

    assert_eq!(fixture.expressions.expression(fixture.value).ty, f64_type);
    let (op, operands) = fixture
        .numeric_op
        .expect("fixed division should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Divide,
            domain: NumericScalar::Fixed(FixedScalar::F64),
        }
    );
    let HirNumericOperands::Binary { left, right } = operands else {
        panic!("fixed division should have binary operands");
    };
    let expected_conversion = Some((
        NumericScalar::Fixed(FixedScalar::U8),
        NumericScalar::Fixed(FixedScalar::F64),
    ));
    assert_eq!(fixture.expressions.expression(left).ty, f64_type);
    assert_eq!(fixture.expressions.expression(right).ty, f64_type);
    assert_eq!(
        numeric_conversion(&fixture.expressions, left),
        expected_conversion
    );
    assert_eq!(
        numeric_conversion(&fixture.expressions, right),
        expected_conversion
    );
}

#[test]
fn fixed_f16_f32_multiplication_promotes_only_the_left_operand() {
    let f16_type = fixed_type(FixedScalar::F16);
    let f32_type = fixed_type(FixedScalar::F32);
    let fixture = lower_binary_with_types(Operator::Multiply, f16_type, f32_type, f32_type);

    assert_eq!(fixture.expressions.expression(fixture.value).ty, f32_type);
    let (op, operands) = fixture
        .numeric_op
        .expect("fixed multiplication should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Multiply,
            domain: NumericScalar::Fixed(FixedScalar::F32),
        }
    );
    let HirNumericOperands::Binary { left, right } = operands else {
        panic!("fixed multiplication should have binary operands");
    };
    assert_eq!(
        numeric_conversion(&fixture.expressions, left),
        Some((
            NumericScalar::Fixed(FixedScalar::F16),
            NumericScalar::Fixed(FixedScalar::F32),
        ))
    );
    assert_eq!(fixture.expressions.expression(left).ty, f32_type);
    assert_eq!(fixture.expressions.expression(right).ty, f32_type);
    assert!(numeric_conversion(&fixture.expressions, right).is_none());
}

#[test]
fn fixed_i8_negation_keeps_its_operand_domain() {
    let i8_type = fixed_type(FixedScalar::I8);
    let fixture = lower_negation_with_type(i8_type, i8_type);

    assert_eq!(fixture.expressions.expression(fixture.value).ty, i8_type);
    let (op, operands) = fixture
        .numeric_op
        .expect("negation should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Fixed(FixedScalar::I8),
        }
    );
    let HirNumericOperands::Unary { operand } = operands else {
        panic!("negation should have a unary operand");
    };
    assert_eq!(fixture.expressions.expression(operand).ty, i8_type);
    assert!(numeric_conversion(&fixture.expressions, operand).is_none());
    // Negating the I8 minimum overflows I8, so the narrowed domain must stay checked. The fixture
    // lowers in the non-fallible start function, whose exact delivery is a trap; the old promoted
    // I32 domain made the negation infallible and must not return.
    assert_eq!(
        fixture.failure_mode,
        Some(NumericDelivery::Statement(NumericFailureMode::Trap)),
        "signed minimum negation must keep its checked trap delivery"
    );
}

#[test]
fn fixed_f64_negation_keeps_its_operand_domain() {
    let f64_type = fixed_type(FixedScalar::F64);
    let fixture = lower_negation_with_type(f64_type, f64_type);

    assert_eq!(fixture.expressions.expression(fixture.value).ty, f64_type);
    let (op, operands) = fixture
        .numeric_op
        .expect("negation should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Fixed(FixedScalar::F64),
        }
    );
    let HirNumericOperands::Unary { operand } = operands else {
        panic!("negation should have a unary operand");
    };
    assert_eq!(fixture.expressions.expression(operand).ty, f64_type);
    assert!(numeric_conversion(&fixture.expressions, operand).is_none());
}

#[test]
fn fixed_integer_comparison_keeps_mixed_operand_types() {
    let i64_type = fixed_type(FixedScalar::I64);
    let u64_type = fixed_type(FixedScalar::U64);
    let fixture = lower_binary_with_types(
        Operator::LessThan,
        i64_type,
        u64_type,
        builtin_type_ids::BOOL,
    );

    assert!(
        fixture.numeric_op.is_none(),
        "comparisons should not emit NumericOp"
    );
    let HirExpressionKind::BinOp { op, left, right } =
        &fixture.expressions.expression(fixture.value).kind
    else {
        panic!("fixed integer comparison should remain a plain BinOp");
    };
    assert_eq!(*op, HirBinOp::Lt);
    assert_eq!(fixture.expressions.expression(*left).ty, i64_type);
    assert_eq!(fixture.expressions.expression(*right).ty, u64_type);
    assert!(numeric_conversion(&fixture.expressions, *left).is_none());
    assert!(numeric_conversion(&fixture.expressions, *right).is_none());
}

#[test]
fn mixed_int_float_comparison_consumes_the_explicit_operand_conversion() {
    // WHAT: verifies the valid converted form of a fixed-integer/binary-float comparison: the
    //       explicitly converted operand is consumed by the comparison in either operand order.
    // WHY: completed I32 and F64 operands never compare directly. The shared comparison policy
    //      owns that rejection
    //      (`src/compiler_frontend/datatypes/tests/numeric_operators_tests.rs::
    //      fixed_integer_and_binary_float_families_never_mix_implicitly`) and the HIR validator
    //      owns the unconverted HIR shape
    //      (`src/compiler_frontend/hir/tests/hir_validation_tests.rs::
    //      validator_uses_numeric_comparison_compatibility_and_keeps_same_type_other_values`),
    //      so this fixture must pin only the converted operands and keep the conversion visible.
    let float_type = fixed_type(FixedScalar::F64);
    let expected_conversion = Some((
        NumericScalar::Fixed(FixedScalar::I32),
        NumericScalar::Fixed(FixedScalar::F64),
    ));

    let int_on_left = lower_explicitly_converted_int_float_comparison(Operator::LessThan, true);
    assert!(int_on_left.numeric_op.is_none());
    let HirExpressionKind::BinOp {
        op: HirBinOp::Lt,
        left,
        right,
    } = &int_on_left.expressions.expression(int_on_left.value).kind
    else {
        panic!("converted I32/F64 comparison should remain a BinOp");
    };
    assert_eq!(
        int_on_left.expressions.expression(int_on_left.value).ty,
        builtin_type_ids::BOOL
    );
    assert_eq!(int_on_left.expressions.expression(*left).ty, float_type);
    assert_eq!(int_on_left.expressions.expression(*right).ty, float_type);
    assert_eq!(
        numeric_conversion(&int_on_left.expressions, *left),
        expected_conversion,
        "the comparison must consume the converted integer operand"
    );
    assert!(numeric_conversion(&int_on_left.expressions, *right).is_none());

    let int_on_right =
        lower_explicitly_converted_int_float_comparison(Operator::GreaterThan, false);
    assert!(int_on_right.numeric_op.is_none());
    let HirExpressionKind::BinOp {
        op: HirBinOp::Gt,
        left,
        right,
    } = &int_on_right.expressions.expression(int_on_right.value).kind
    else {
        panic!("converted F64/I32 comparison should remain a BinOp");
    };
    assert_eq!(int_on_right.expressions.expression(*left).ty, float_type);
    assert_eq!(int_on_right.expressions.expression(*right).ty, float_type);
    assert!(numeric_conversion(&int_on_right.expressions, *left).is_none());
    assert_eq!(
        numeric_conversion(&int_on_right.expressions, *right),
        expected_conversion,
        "the reversed comparison must consume the converted integer operand"
    );
}

#[test]
fn mixed_signed_unsigned_addition_promotes_to_the_common_signed_width() {
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let u8_type = fixed_type(FixedScalar::U8);
    let fixture = lower_binary_with_types(Operator::Add, i32_type, u8_type, i32_type);

    assert_eq!(fixture.expressions.expression(fixture.value).ty, i32_type);
    let (op, operands) = fixture
        .numeric_op
        .expect("mixed fixed addition should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::I32),
        }
    );
    let HirNumericOperands::Binary { left, right } = operands else {
        panic!("fixed addition should have binary operands");
    };
    assert_eq!(fixture.expressions.expression(left).ty, i32_type);
    assert_eq!(fixture.expressions.expression(right).ty, i32_type);
    assert!(numeric_conversion(&fixture.expressions, left).is_none());
    assert_eq!(
        numeric_conversion(&fixture.expressions, right),
        Some((
            NumericScalar::Fixed(FixedScalar::U8),
            NumericScalar::Fixed(FixedScalar::I32),
        ))
    );
}

// WHAT: verifies that fixed-width numeric HIR keeps the builtin Error! failure mode.
// WHY: operation-domain coverage alone would not catch a regression in failure-channel selection.
#[test]
fn fixed_u32_addition_uses_native_error_delivery() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let function_name = symbol("__test_fixed_u32_error", &mut path_fork, &mut string_table);
    let left_name = symbol("left", &mut path_fork, &mut string_table);
    let right_name = symbol("right", &mut path_fork, &mut string_table);
    let u32_type = fixed_type(FixedScalar::U32);
    let left = reference_expr_with_type_id(
        &mut expressions,
        left_name,
        u32_type,
        loc,
        ValueMode::ImmutableReference,
    );
    let right = reference_expr_with_type_id(
        &mut expressions,
        right_name,
        u32_type,
        loc,
        ValueMode::ImmutableReference,
    );
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let expression = resolved_numeric_expression(
        &mut expressions,
        left,
        Some(right),
        Operator::Add,
        u32_type,
        loc,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );

    let error_type_id = builder.test_register_builtin_error_type();
    builder.test_register_function_with_returns(
        function_name,
        FunctionId(1),
        vec![u32_type],
        Some(error_type_id),
    );
    builder.test_set_current_function(FunctionId(1));
    register_local(&mut builder, left_name, LocalId(10), u32_type, loc);
    register_local(&mut builder, right_name, LocalId(11), u32_type, loc);

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expression)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("U32 addition in a builtin Error! function should lower");

    let (op, failure_mode) =
        find_single_numeric_op(&builder).expect("expected a fixed-width NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        }
    );
    assert!(
        matches!(failure_mode, NumericDelivery::Invoke),
        "fixed-width numeric failures in builtin Error! functions should invoke the error edge"
    );
    let found = find_numeric_op_statement(&builder).expect("expected the numeric Invoke");
    assert!(matches!(
        &expression_row(&builder, lowered.value).kind,
        HirExpressionKind::Load(place) if place.root == found.result && super::is_local_place(place)
    ));
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, HirTerminator::ReturnError(_))),
        "fixed-width numeric failure should return through the builtin Error slot"
    );
}

#[test]
fn number_arithmetic_converts_mixed_integer_operands_to_the_number_scale() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let integer_name = symbol("integer", &mut path_fork, &mut string_table);
    let number_name = symbol("number", &mut path_fork, &mut string_table);
    let integer_type = fixed_type(FixedScalar::U64);
    let scale = NumberScale::new(2).expect("test Dec scale is valid");

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let number_type = builder.type_environment.intern_number(scale);
    let integer = reference_expr_with_type_id(
        &mut expressions,
        integer_name,
        integer_type,
        None,
        ValueMode::ImmutableReference,
    );
    let number = reference_expr_with_type_id(
        &mut expressions,
        number_name,
        number_type,
        None,
        ValueMode::ImmutableReference,
    );
    let expression = resolved_numeric_expression(
        &mut expressions,
        integer,
        Some(number),
        Operator::Add,
        number_type,
        None,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    register_local(&mut builder, integer_name, LocalId(10), integer_type, None);
    register_local(&mut builder, number_name, LocalId(11), number_type, None);

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expression)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("Dec addition with a fixed integer should lower");
    let (op, operands) = find_single_numeric_op_with_operands(&builder)
        .expect("Dec addition should emit one checked NumericOp");

    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Number(scale),
        }
    );
    assert_eq!(expression_row(&builder, lowered.value).ty, number_type);

    let HirNumericOperands::Binary { left, right } = operands else {
        panic!("Dec addition must have two operands");
    };
    assert_eq!(builder.module.expressions.expression(left).ty, number_type);
    assert_eq!(
        numeric_conversion(&builder.module.expressions, left),
        Some((
            NumericScalar::Fixed(FixedScalar::U64),
            NumericScalar::Number(scale)
        ))
    );
    assert_eq!(builder.module.expressions.expression(right).ty, number_type);
    assert!(numeric_conversion(&builder.module.expressions, right).is_none());
}

#[test]
fn number_power_keeps_its_exponent_in_fixed_i32() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let base_name = symbol("base", &mut path_fork, &mut string_table);
    let exponent_name = symbol("exponent", &mut path_fork, &mut string_table);
    let scale = NumberScale::new(3).expect("test Dec scale is valid");
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let number_type = builder.type_environment.intern_number(scale);
    let base = reference_expr_with_type_id(
        &mut expressions,
        base_name,
        number_type,
        None,
        ValueMode::ImmutableReference,
    );
    let exponent = reference_expr_with_type_id(
        &mut expressions,
        exponent_name,
        int_type,
        None,
        ValueMode::ImmutableReference,
    );
    let expression = resolved_numeric_expression(
        &mut expressions,
        base,
        Some(exponent),
        Operator::Exponent,
        number_type,
        None,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    register_local(&mut builder, base_name, LocalId(10), number_type, None);
    register_local(&mut builder, exponent_name, LocalId(11), int_type, None);

    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(expression)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("Dec power with an I32 exponent should lower");
    let (op, operands) = find_single_numeric_op_with_operands(&builder)
        .expect("Dec power should emit one checked NumericOp");

    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Power,
            domain: NumericScalar::Number(scale),
        }
    );
    assert_eq!(expression_row(&builder, lowered.value).ty, number_type);

    let HirNumericOperands::Binary { left, right } = operands else {
        panic!("Dec power must have two operands");
    };
    assert_eq!(builder.module.expressions.expression(left).ty, number_type);
    assert!(numeric_conversion(&builder.module.expressions, left).is_none());
    assert_eq!(builder.module.expressions.expression(right).ty, int_type);
    assert!(numeric_conversion(&builder.module.expressions, right).is_none());
}

#[test]
fn mixed_number_comparisons_convert_the_integer_side_to_the_number_scale() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let integer_name = symbol("integer", &mut path_fork, &mut string_table);
    let number_name = symbol("number", &mut path_fork, &mut string_table);
    let integer_type = fixed_type(FixedScalar::U64);
    let scale = NumberScale::new(4).expect("test Dec scale is valid");
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let number_type = builder.type_environment.intern_number(scale);
    register_local(&mut builder, integer_name, LocalId(10), integer_type, None);
    register_local(&mut builder, number_name, LocalId(11), number_type, None);

    let number = reference_expr_with_type_id(
        &mut expressions,
        number_name,
        number_type,
        None,
        ValueMode::ImmutableReference,
    );
    let integer = reference_expr_with_type_id(
        &mut expressions,
        integer_name,
        integer_type,
        None,
        ValueMode::ImmutableReference,
    );
    let number_on_left = resolved_numeric_expression(
        &mut expressions,
        number,
        Some(integer),
        Operator::LessThan,
        builtin_type_ids::BOOL,
        None,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    builder.module_const_values = std::mem::take(&mut constants);
    let view = expressions
        .view(number_on_left)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("Dec and fixed integer comparison should lower");
    let HirExpressionKind::BinOp { left, right, .. } =
        &expression_row(&builder, lowered.value).kind
    else {
        panic!("Dec comparison should remain a plain BinOp");
    };
    assert_eq!(builder.module.expressions.expression(*left).ty, number_type);
    assert!(numeric_conversion(&builder.module.expressions, *left).is_none());
    assert_eq!(
        builder.module.expressions.expression(*right).ty,
        number_type
    );
    assert_eq!(
        numeric_conversion(&builder.module.expressions, *right),
        Some((
            NumericScalar::Fixed(FixedScalar::U64),
            NumericScalar::Number(scale)
        ))
    );

    let integer = reference_expr_with_type_id(
        &mut expressions,
        integer_name,
        integer_type,
        None,
        ValueMode::ImmutableReference,
    );
    let number = reference_expr_with_type_id(
        &mut expressions,
        number_name,
        number_type,
        None,
        ValueMode::ImmutableReference,
    );
    let integer_on_left = resolved_numeric_expression(
        &mut expressions,
        integer,
        Some(number),
        Operator::LessThan,
        builtin_type_ids::BOOL,
        None,
        ValueMode::MutableOwned,
        &builder.type_environment,
    );
    let view = expressions
        .view(integer_on_left)
        .expect("fixture expression belongs to builder");
    let lowered = builder
        .lower_expression(view)
        .expect("reversed Dec and fixed integer comparison should lower");
    let HirExpressionKind::BinOp { left, right, .. } =
        &expression_row(&builder, lowered.value).kind
    else {
        panic!("Dec comparison should remain a plain BinOp");
    };
    assert_eq!(builder.module.expressions.expression(*left).ty, number_type);
    assert_eq!(
        numeric_conversion(&builder.module.expressions, *left),
        Some((
            NumericScalar::Fixed(FixedScalar::U64),
            NumericScalar::Number(scale)
        ))
    );
    assert_eq!(
        builder.module.expressions.expression(*right).ty,
        number_type
    );
    assert!(numeric_conversion(&builder.module.expressions, *right).is_none());
}
