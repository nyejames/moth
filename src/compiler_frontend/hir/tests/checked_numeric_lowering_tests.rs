//! Focused tests for checked numeric HIR lowering (Phase 6b).
//!
//! WHAT: verifies that runtime arithmetic is lowered into `HirStatementKind::NumericOp` with the
//!       correct `HirNumericOp`, operand conversion, and failure-mode selection.
//! WHY: the checked numeric path is new HIR surface; dedicated tests guard against regressions back
//!      to plain `BinOp`/`UnaryOp` arithmetic and against wrong failure-mode wiring.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::{Expression, Operator};
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expression_store::{HirConstructionFailure, HirExpressionStore};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_builder::{register_local, setup_builder};
use crate::compiler_frontend::hir::ids::{FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{HirTerminator, RuntimeFailureCause};
use crate::compiler_frontend::hir::tests::symbol;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, node, reference_expr_with_type_id,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    runtime_expr, runtime_operand_item, runtime_operator_item,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn int_expr(value: i64, span: Option<crate::compiler_frontend::source::SourceSpan>) -> Expression {
    Expression::int(value, span, ValueMode::ImmutableOwned)
}

fn float_expr(
    value: f64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> Expression {
    Expression::float(value, span, ValueMode::ImmutableOwned)
}

fn find_single_numeric_op(builder: &HirBuilder<'_>) -> Option<(HirNumericOp, NumericFailureMode)> {
    builder
        .module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp {
                op, failure_mode, ..
            } => Some((*op, *failure_mode)),
            _ => None,
        })
}

fn fixed_type(scalar: FixedScalar) -> crate::compiler_frontend::datatypes::ids::TypeId {
    builtin_type_ids::fixed_scalar(scalar)
}

fn find_single_numeric_op_with_operands(
    builder: &HirBuilder<'_>,
) -> Option<(HirNumericOp, HirNumericOperands)> {
    builder
        .module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp { op, operands, .. } => Some((*op, operands.clone())),
            _ => None,
        })
}

#[derive(Debug)]
struct LoweredNumericFixture {
    expressions: HirExpressionStore,
    value: HirValueId,
    numeric_op: Option<(HirNumericOp, HirNumericOperands)>,
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
    let loc = None;
    let left_name = symbol("left", &mut path_fork, &mut string_table);
    let right_name = symbol("right", &mut path_fork, &mut string_table);
    let left =
        reference_expr_with_type_id(left_name, left_type, loc, ValueMode::ImmutableReference);
    let right =
        reference_expr_with_type_id(right_name, right_type, loc, ValueMode::ImmutableReference);
    let expression = runtime_expr(
        vec![
            runtime_operand_item(left),
            runtime_operand_item(right),
            runtime_operator_item(op, loc),
        ],
        result_type,
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(&mut builder, left_name, LocalId(10), left_type, loc);
    register_local(&mut builder, right_name, LocalId(11), right_type, loc);
    let lowered = builder.lower_expression(&expression)?;

    let numeric_op = find_single_numeric_op_with_operands(&builder);
    Ok(LoweredNumericFixture {
        expressions: builder.module.expressions,
        value: lowered.value,
        numeric_op,
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
    let loc = None;
    let operand_name = symbol("operand", &mut path_fork, &mut string_table);
    let operand = reference_expr_with_type_id(
        operand_name,
        operand_type,
        loc,
        ValueMode::ImmutableReference,
    );
    let expression = runtime_expr(
        vec![
            runtime_operand_item(operand),
            runtime_operator_item(Operator::Negate, loc),
        ],
        result_type,
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(&mut builder, operand_name, LocalId(10), operand_type, loc);
    let lowered = builder.lower_expression(&expression)?;
    let (op, operands) = find_single_numeric_op_with_operands(&builder)
        .expect("numeric negation should emit one NumericOp");

    Ok(LoweredNumericFixture {
        expressions: builder.module.expressions,
        value: lowered.value,
        numeric_op: Some((op, operands)),
    })
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
    builder.test_register_function_with_return_type(name, function_id, return_type);
    builder.test_set_current_function(function_id);
}

fn lower_u8_compound_add_assignment(
    builder: &mut HirBuilder<'_>,
    target_name: PathId,
    right_name: PathId,
) -> Result<(), HirConstructionFailure> {
    let u8_type = fixed_type(FixedScalar::U8);
    let u32_type = fixed_type(FixedScalar::U32);
    let source = runtime_expr(
        vec![
            runtime_operand_item(reference_expr_with_type_id(
                target_name,
                u8_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operand_item(reference_expr_with_type_id(
                right_name,
                u8_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operator_item(Operator::Add, None),
        ],
        u32_type,
        None,
        ValueMode::MutableOwned,
    );
    let value = Expression::cast(
        ResolvedCastExpression {
            source: Box::new(source),
            source_type_id: u32_type,
            target_type_id: u8_type,
            target: BuiltinCastTarget::Fixed(FixedScalar::U8),
            requires_optional_wrap_after_cast: false,
            evidence: ResolvedCastEvidence::Builtin {
                policy: BuiltinCastPolicyId::NumericConversion {
                    source: NumericScalar::Fixed(FixedScalar::U32),
                    target: NumericScalar::Fixed(FixedScalar::U8),
                },
            },
            handling: CastHandling::StoreConversion,
            span: None,
        },
        u8_type,
        &builder.type_environment,
    );
    builder.lower_statement_node(&node(
        NodeKind::Assignment {
            target: assignment_target(target_name, DataType::Inferred, u8_type, None),
            value,
        },
        None,
    ))
}

fn store_conversion_branch(
    builder: &HirBuilder<'_>,
) -> (
    crate::compiler_frontend::hir::ids::BlockId,
    crate::compiler_frontend::hir::ids::BlockId,
) {
    let cast_block = builder
        .module
        .blocks
        .iter()
        .find(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::CastOp {
                        policy: BuiltinCastPolicyId::NumericConversion {
                            source: NumericScalar::Fixed(FixedScalar::U32),
                            target: NumericScalar::Fixed(FixedScalar::U8),
                        },
                        ..
                    }
                )
            })
        })
        .expect("compound U8 narrowing should emit one CastOp");
    match &cast_block.terminator {
        HirTerminator::FallibleBranch {
            success_block,
            error_block,
            ..
        } => (*success_block, *error_block),
        _ => panic!("compound U8 conversion must branch on its fallible carrier"),
    }
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

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_builtin_error_type();
    set_current_function_return_type(&mut builder, FunctionId(1), u8_type, function_name);
    register_local(&mut builder, target_name, LocalId(10), u8_type, None);
    register_local(&mut builder, right_name, LocalId(11), u8_type, None);
    lower_u8_compound_add_assignment(&mut builder, target_name, right_name)
        .expect("compound U8 store lowering should succeed");
    let (op, failure_mode) =
        find_single_numeric_op(&builder).expect("compound U8 addition should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        }
    );
    // The U8 + U8 arithmetic is discharged to U32 (F06): it carries no failure edge. The
    // narrowing store conversion back to U8 remains the failing check under test.
    assert_eq!(failure_mode, NumericFailureMode::Infallible);

    let (success_block, error_block) = store_conversion_branch(&builder);
    let error_block = &builder.module.blocks[error_block.0 as usize];
    let carrier = match error_block.terminator {
        HirTerminator::RuntimeFailure {
            cause: Some(RuntimeFailureCause::StoreConversion { carrier }),
            ..
        } => carrier,
        _ => panic!("non-fallible conversion failure must retain its store-conversion cause"),
    };
    let stored_value = compound_assignment_store_value(&builder, LocalId(10), success_block);
    assert!(
        matches!(
            &stored_value.kind,
            HirExpressionKind::FallibleUnwrapSuccess { result }
                if matches!(
                    &builder.module.expressions.expression(*result).kind,
                    HirExpressionKind::Load(place)
                        if place.root == carrier && super::is_local_place(place)
                )
        ),
        "the store must consume the successful conversion payload"
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

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let error_type = builder.test_register_builtin_error_type();
    let return_type = builder
        .type_environment
        .intern_fallible_carrier(u8_type, error_type);
    set_current_function_return_type(&mut builder, FunctionId(1), return_type, function_name);
    register_local(&mut builder, target_name, LocalId(10), u8_type, None);
    register_local(&mut builder, right_name, LocalId(11), u8_type, None);
    lower_u8_compound_add_assignment(&mut builder, target_name, right_name)
        .expect("compound U8 store lowering should succeed");
    let (op, failure_mode) =
        find_single_numeric_op(&builder).expect("compound U8 addition should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        }
    );
    // Discharged arithmetic stays Infallible even inside Error! functions: only the store
    // conversion takes the recoverable path asserted below.
    assert_eq!(failure_mode, NumericFailureMode::Infallible);

    let (success_block, error_block) = store_conversion_branch(&builder);
    let error_block = &builder.module.blocks[error_block.0 as usize];
    assert!(
        matches!(&error_block.terminator, HirTerminator::ReturnError(_)),
        "builtin Error! conversion failure must return through the error edge"
    );
    let stored_value = compound_assignment_store_value(&builder, LocalId(10), success_block);
    assert!(
        matches!(
            &stored_value.kind,
            HirExpressionKind::FallibleUnwrapSuccess { .. }
        ),
        "the store must consume the successful conversion payload"
    );
}

#[test]
fn equal_type_compound_store_emits_no_conversion_carrier() {
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
    let value = runtime_expr(
        vec![
            runtime_operand_item(reference_expr_with_type_id(
                target_name,
                u32_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operand_item(reference_expr_with_type_id(
                right_name,
                u32_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operator_item(Operator::Add, None),
        ],
        u32_type,
        None,
        ValueMode::MutableOwned,
    );
    builder
        .lower_statement_node(&node(
            NodeKind::Assignment {
                target: assignment_target(target_name, DataType::Inferred, u32_type, None),
                value,
            },
            None,
        ))
        .expect("same-type compound assignment lowering should succeed");

    assert!(
        !builder
            .module
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(&statement.kind, HirStatementKind::CastOp { .. })),
        "an assignment-compatible result should not emit a cast carrier"
    );
    assert!(
        !builder
            .module
            .blocks
            .iter()
            .any(|block| matches!(&block.terminator, HirTerminator::FallibleBranch { .. })),
        "same-type arithmetic should not emit a conversion branch"
    );
    let _stored_value = compound_assignment_store_value(
        &builder,
        LocalId(10),
        crate::compiler_frontend::hir::ids::BlockId(0),
    );
}

#[test]
fn checked_int_addition_lowers_to_int_add_numeric_op() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let x_name = symbol("x", &mut path_fork, &mut string_table);
    let x_ref = reference_expr_with_type_id(
        x_name,
        builtin_type_ids::INT,
        loc,
        ValueMode::ImmutableReference,
    );
    let two = int_expr(2, loc);

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
        builtin_type_ids::INT,
        loc,
    );

    let expr = runtime_expr(
        vec![
            runtime_operand_item(x_ref),
            runtime_operand_item(two),
            runtime_operator_item(Operator::Add, loc),
        ],
        builtin_type_ids::INT,
        loc,
        ValueMode::MutableOwned,
    );

    let lowered = builder
        .lower_expression(&expr)
        .expect("int addition lowering should succeed");

    assert!(lowered.prelude.is_empty());
    assert_eq!(
        expression_row(&builder, lowered.value).ty,
        builtin_type_ids::INT
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
            domain: NumericScalar::Int
        }
    );
    assert!(matches!(failure_mode, NumericFailureMode::Trap));
}

#[test]
fn checked_int_subtraction_lowers_to_int_sub_numeric_op() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(5, loc)),
            runtime_operand_item(int_expr(3, loc)),
            runtime_operator_item(Operator::Subtract, loc),
        ],
        builtin_type_ids::INT,
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let _ = builder
        .lower_expression(&expr)
        .expect("int subtraction lowering should succeed");

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        op == HirNumericOp {
            operator: NumericOperator::Subtract,
            domain: NumericScalar::Int
        }
    );
}

#[test]
fn checked_regular_division_lowers_to_float_div_numeric_op() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(5, loc)),
            runtime_operand_item(int_expr(2, loc)),
            runtime_operator_item(Operator::Divide, loc),
        ],
        builtin_type_ids::FLOAT,
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let lowered = builder
        .lower_expression(&expr)
        .expect("regular division lowering should succeed");

    assert_eq!(
        expression_row(&builder, lowered.value).ty,
        builtin_type_ids::FLOAT
    );

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        op == HirNumericOp {
            operator: NumericOperator::Divide,
            domain: NumericScalar::Float
        }
    );

    // Both Int operands must have been explicitly converted to Float before the division.
    let numeric_op = builder
        .test_current_block_statements()
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp { operands, .. } => Some(operands.clone()),
            _ => None,
        })
        .expect("NumericOp statement should exist");
    let HirNumericOperands::Binary { left, right } = numeric_op else {
        panic!("Float divide should be binary");
    };
    assert!(matches!(
        &builder.module.expressions.expression(left).kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Int,
                target: NumericScalar::Float,
            },
            ..
        }
    ));
    assert!(matches!(
        &builder.module.expressions.expression(right).kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Int,
                target: NumericScalar::Float,
            },
            ..
        }
    ));
}

#[test]
fn mixed_int_float_addition_converts_int_operand() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(1, loc)),
            runtime_operand_item(float_expr(2.5, loc)),
            runtime_operator_item(Operator::Add, loc),
        ],
        builtin_type_ids::FLOAT,
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let _ = builder
        .lower_expression(&expr)
        .expect("mixed addition lowering should succeed");

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        op == HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Float
        }
    );

    let numeric_op = builder
        .test_current_block_statements()
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp { operands, .. } => Some(operands.clone()),
            _ => None,
        })
        .expect("NumericOp statement should exist");
    let HirNumericOperands::Binary { left, right } = numeric_op else {
        panic!("Float add should be binary");
    };
    assert!(matches!(
        &builder.module.expressions.expression(left).kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Int,
                target: NumericScalar::Float,
            },
            ..
        }
    ));
    assert!(matches!(
        &builder.module.expressions.expression(right).kind,
        HirExpressionKind::Float(_)
    ));
}

#[test]
fn unary_int_negation_lowers_to_int_neg_numeric_op() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let x_name = symbol("x", &mut path_fork, &mut string_table);
    let x_ref = reference_expr_with_type_id(
        x_name,
        builtin_type_ids::INT,
        loc,
        ValueMode::ImmutableReference,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(
        &mut builder,
        x_name,
        LocalId(10),
        builtin_type_ids::INT,
        loc,
    );

    let expr = runtime_expr(
        vec![
            runtime_operand_item(x_ref),
            runtime_operator_item(Operator::Negate, loc),
        ],
        builtin_type_ids::INT,
        loc,
        ValueMode::MutableOwned,
    );

    let lowered = builder
        .lower_expression(&expr)
        .expect("unary negation lowering should succeed");

    assert_eq!(
        expression_row(&builder, lowered.value).ty,
        builtin_type_ids::INT
    );

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        op == HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Int
        }
    );
}

#[test]
fn numeric_failure_mode_is_return_error_for_builtin_error_function() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let fn_name = symbol("__test_fn_error", &mut path_fork, &mut string_table);
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(1, loc)),
            runtime_operand_item(int_expr(2, loc)),
            runtime_operator_item(Operator::Add, loc),
        ],
        builtin_type_ids::INT,
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let error_type_id = builder.test_register_builtin_error_type();
    let return_type = builder
        .type_environment
        .intern_fallible_carrier(builtin_type_ids::INT, error_type_id);
    set_current_function_return_type(&mut builder, FunctionId(1), return_type, fn_name);

    let lowered = builder
        .lower_expression(&expr)
        .expect("addition lowering in Error! function should succeed");

    let (_, failure_mode) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        matches!(failure_mode, NumericFailureMode::ReturnError),
        "builtin Error! functions should use ReturnError numeric failure mode"
    );
    assert!(
        matches!(
            expression_row(&builder, lowered.value).kind,
            HirExpressionKind::FallibleUnwrapSuccess { .. }
        ),
        "recoverable numeric lowering should continue with an unwrapped success value"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, HirTerminator::FallibleBranch { .. })),
        "recoverable numeric lowering should branch on the internal carrier"
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
    let loc = None;
    let fn_name = symbol("__test_fn_string_error", &mut path_fork, &mut string_table);
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(1, loc)),
            runtime_operand_item(int_expr(2, loc)),
            runtime_operator_item(Operator::Add, loc),
        ],
        builtin_type_ids::INT,
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let return_type = builder
        .type_environment
        .intern_fallible_carrier(builtin_type_ids::INT, builtin_type_ids::STRING);
    set_current_function_return_type(&mut builder, FunctionId(1), return_type, fn_name);

    let _ = builder
        .lower_expression(&expr)
        .expect("addition lowering in custom-error function should succeed");

    let (_, failure_mode) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        matches!(failure_mode, NumericFailureMode::Trap),
        "custom error functions should trap on numeric failure"
    );
}

#[test]
fn numeric_failure_mode_is_trap_for_non_fallible_function() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let fn_name = symbol("__test_fn_non_fallible", &mut path_fork, &mut string_table);
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(1, loc)),
            runtime_operand_item(int_expr(2, loc)),
            runtime_operator_item(Operator::Add, loc),
        ],
        builtin_type_ids::INT,
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    set_current_function_return_type(&mut builder, FunctionId(1), builtin_type_ids::INT, fn_name);

    let _ = builder
        .lower_expression(&expr)
        .expect("addition lowering in non-fallible function should succeed");

    let (_, failure_mode) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(
        matches!(failure_mode, NumericFailureMode::Trap),
        "non-fallible functions should trap on numeric failure"
    );
}

#[test]
fn fixed_u8_addition_promotes_both_operands_to_u32() {
    let u8_type = fixed_type(FixedScalar::U8);
    let u32_type = fixed_type(FixedScalar::U32);
    let fixture = lower_binary_with_types(Operator::Add, u8_type, u8_type, u32_type);

    assert_eq!(fixture.expressions.expression(fixture.value).ty, u32_type);
    let (op, operands) = fixture
        .numeric_op
        .expect("fixed addition should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        }
    );
    let HirNumericOperands::Binary { left, right } = operands else {
        panic!("fixed addition should have binary operands");
    };
    let expected_conversion = Some((
        NumericScalar::Fixed(FixedScalar::U8),
        NumericScalar::Fixed(FixedScalar::U32),
    ));
    assert_eq!(fixture.expressions.expression(left).ty, u32_type);
    assert_eq!(fixture.expressions.expression(right).ty, u32_type);
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
fn fixed_i8_negation_promotes_operand_to_i32() {
    let i8_type = fixed_type(FixedScalar::I8);
    let i32_type = fixed_type(FixedScalar::I32);
    let fixture = lower_negation_with_type(i8_type, i32_type);

    assert_eq!(fixture.expressions.expression(fixture.value).ty, i32_type);
    let (op, operands) = fixture
        .numeric_op
        .expect("negation should emit a NumericOp");
    assert_eq!(
        op,
        HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Fixed(FixedScalar::I32),
        }
    );
    let HirNumericOperands::Unary { operand } = operands else {
        panic!("negation should have a unary operand");
    };
    assert_eq!(
        numeric_conversion(&fixture.expressions, operand),
        Some((
            NumericScalar::Fixed(FixedScalar::I8),
            NumericScalar::Fixed(FixedScalar::I32),
        ))
    );
    assert_eq!(fixture.expressions.expression(operand).ty, i32_type);
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
fn mixed_int_float_comparison_converts_int_operand_to_float() {
    let float_type = builtin_type_ids::FLOAT;

    let int_on_left = lower_binary_with_types(
        Operator::LessThan,
        builtin_type_ids::INT,
        float_type,
        builtin_type_ids::BOOL,
    );
    assert!(int_on_left.numeric_op.is_none());
    let HirExpressionKind::BinOp {
        op: HirBinOp::Lt,
        left,
        right,
    } = &int_on_left.expressions.expression(int_on_left.value).kind
    else {
        panic!("mixed Int/Float comparison should remain a BinOp");
    };
    assert_eq!(
        numeric_conversion(&int_on_left.expressions, *left),
        Some((NumericScalar::Int, NumericScalar::Float))
    );
    assert_eq!(int_on_left.expressions.expression(*left).ty, float_type);
    assert!(numeric_conversion(&int_on_left.expressions, *right).is_none());

    let int_on_right = lower_binary_with_types(
        Operator::GreaterThan,
        float_type,
        builtin_type_ids::INT,
        builtin_type_ids::BOOL,
    );
    assert!(int_on_right.numeric_op.is_none());
    let HirExpressionKind::BinOp { left, right, .. } =
        &int_on_right.expressions.expression(int_on_right.value).kind
    else {
        panic!("mixed Float/Int comparison should remain a BinOp");
    };
    assert!(numeric_conversion(&int_on_right.expressions, *left).is_none());
    assert_eq!(
        numeric_conversion(&int_on_right.expressions, *right),
        Some((NumericScalar::Int, NumericScalar::Float))
    );
    assert_eq!(int_on_right.expressions.expression(*right).ty, float_type);
}

#[test]
fn unsupported_arithmetic_pair_is_an_internal_lowering_error() {
    let u8_type = fixed_type(FixedScalar::U8);
    let failure = try_lower_binary_with_types(
        Operator::Add,
        builtin_type_ids::INT,
        u8_type,
        builtin_type_ids::INT,
    )
    .expect_err("unsupported arithmetic must not fall back to plain BinOp");
    let HirConstructionFailure::Infrastructure(error) = failure else {
        panic!("unsupported arithmetic should be an infrastructure failure");
    };

    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation
    );
}

#[test]
fn unsupported_numeric_negation_is_an_internal_lowering_error() {
    let u32_type = fixed_type(FixedScalar::U32);
    let failure = try_lower_negation_with_type(u32_type, u32_type)
        .expect_err("unsupported unsigned negation must not fall back to plain UnaryOp");
    let HirConstructionFailure::Infrastructure(error) = failure else {
        panic!("unsupported unsigned negation should be an infrastructure failure");
    };

    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation
    );
}

// WHAT: verifies that fixed-width numeric HIR keeps the builtin Error! failure mode.
// WHY: operation-domain coverage alone would not catch a regression in failure-channel selection.
#[test]
fn fixed_u32_addition_uses_return_error_failure_mode() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let function_name = symbol("__test_fixed_u32_error", &mut path_fork, &mut string_table);
    let left_name = symbol("left", &mut path_fork, &mut string_table);
    let right_name = symbol("right", &mut path_fork, &mut string_table);
    let u32_type = fixed_type(FixedScalar::U32);
    let left = reference_expr_with_type_id(left_name, u32_type, loc, ValueMode::ImmutableReference);
    let right =
        reference_expr_with_type_id(right_name, u32_type, loc, ValueMode::ImmutableReference);
    let expression = runtime_expr(
        vec![
            runtime_operand_item(left),
            runtime_operand_item(right),
            runtime_operator_item(Operator::Add, loc),
        ],
        u32_type,
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let error_type_id = builder.test_register_builtin_error_type();
    let return_type = builder
        .type_environment
        .intern_fallible_carrier(u32_type, error_type_id);
    set_current_function_return_type(&mut builder, FunctionId(1), return_type, function_name);
    register_local(&mut builder, left_name, LocalId(10), u32_type, loc);
    register_local(&mut builder, right_name, LocalId(11), u32_type, loc);

    let lowered = builder
        .lower_expression(&expression)
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
        matches!(failure_mode, NumericFailureMode::ReturnError),
        "fixed-width numeric failures in builtin Error! functions should use ReturnError"
    );
    assert!(matches!(
        expression_row(&builder, lowered.value).kind,
        HirExpressionKind::FallibleUnwrapSuccess { .. }
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
    let integer_name = symbol("integer", &mut path_fork, &mut string_table);
    let number_name = symbol("number", &mut path_fork, &mut string_table);
    let integer_type = fixed_type(FixedScalar::U64);
    let scale = NumberScale::new(2).expect("test Dec scale is valid");

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let number_type = builder.type_environment.intern_number(scale);
    let expression = runtime_expr(
        vec![
            runtime_operand_item(reference_expr_with_type_id(
                integer_name,
                integer_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operand_item(reference_expr_with_type_id(
                number_name,
                number_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operator_item(Operator::Add, None),
        ],
        number_type,
        None,
        ValueMode::MutableOwned,
    );
    register_local(&mut builder, integer_name, LocalId(10), integer_type, None);
    register_local(&mut builder, number_name, LocalId(11), number_type, None);

    let lowered = builder
        .lower_expression(&expression)
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
fn number_power_keeps_its_exponent_in_profile_int() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let base_name = symbol("base", &mut path_fork, &mut string_table);
    let exponent_name = symbol("exponent", &mut path_fork, &mut string_table);
    let scale = NumberScale::new(3).expect("test Dec scale is valid");
    let int_type = builtin_type_ids::INT;

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let number_type = builder.type_environment.intern_number(scale);
    let expression = runtime_expr(
        vec![
            runtime_operand_item(reference_expr_with_type_id(
                base_name,
                number_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operand_item(reference_expr_with_type_id(
                exponent_name,
                int_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operator_item(Operator::Exponent, None),
        ],
        number_type,
        None,
        ValueMode::MutableOwned,
    );
    register_local(&mut builder, base_name, LocalId(10), number_type, None);
    register_local(&mut builder, exponent_name, LocalId(11), int_type, None);

    let lowered = builder
        .lower_expression(&expression)
        .expect("Dec power with an Int exponent should lower");
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
    let integer_name = symbol("integer", &mut path_fork, &mut string_table);
    let number_name = symbol("number", &mut path_fork, &mut string_table);
    let integer_type = fixed_type(FixedScalar::U64);
    let scale = NumberScale::new(4).expect("test Dec scale is valid");
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let number_type = builder.type_environment.intern_number(scale);
    register_local(&mut builder, integer_name, LocalId(10), integer_type, None);
    register_local(&mut builder, number_name, LocalId(11), number_type, None);

    let number_on_left = runtime_expr(
        vec![
            runtime_operand_item(reference_expr_with_type_id(
                number_name,
                number_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operand_item(reference_expr_with_type_id(
                integer_name,
                integer_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operator_item(Operator::LessThan, None),
        ],
        builtin_type_ids::BOOL,
        None,
        ValueMode::MutableOwned,
    );
    let lowered = builder
        .lower_expression(&number_on_left)
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

    let integer_on_left = runtime_expr(
        vec![
            runtime_operand_item(reference_expr_with_type_id(
                integer_name,
                integer_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operand_item(reference_expr_with_type_id(
                number_name,
                number_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operator_item(Operator::LessThan, None),
        ],
        builtin_type_ids::BOOL,
        None,
        ValueMode::MutableOwned,
    );
    let lowered = builder
        .lower_expression(&integer_on_left)
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
