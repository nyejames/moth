//! Focused tests for checked numeric HIR lowering (Phase 6b).
//!
//! WHAT: verifies that runtime arithmetic is lowered into `HirStatementKind::NumericOp` with the
//!       correct `HirNumericOp`, operand conversion, and failure-mode selection.
//! WHY: the checked numeric path is new HIR surface; dedicated tests guard against regressions back
//!      to plain `BinOp`/`UnaryOp` arithmetic and against wrong failure-mode wiring.

use crate::compiler_frontend::ast::expressions::expression::{Expression, Operator};
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_builder::{register_local, setup_builder};
use crate::compiler_frontend::hir::ids::{FunctionId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::tests::symbol;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::compiler_frontend::tests::ast_fixture_support::reference_expr_with_type_id;
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

fn lower_binary_with_types(
    op: Operator,
    left_type: crate::compiler_frontend::datatypes::ids::TypeId,
    right_type: crate::compiler_frontend::datatypes::ids::TypeId,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> (HirExpression, Option<(HirNumericOp, HirNumericOperands)>) {
    try_lower_binary_with_types(op, left_type, right_type, result_type)
        .expect("typed binary operator lowering should succeed")
}

fn try_lower_binary_with_types(
    op: Operator,
    left_type: crate::compiler_frontend::datatypes::ids::TypeId,
    right_type: crate::compiler_frontend::datatypes::ids::TypeId,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> Result<
    (HirExpression, Option<(HirNumericOp, HirNumericOperands)>),
    crate::compiler_frontend::compiler_errors::CompilerError,
> {
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

    Ok((
        lowered.value,
        find_single_numeric_op_with_operands(&builder),
    ))
}

fn lower_negation_with_type(
    operand_type: crate::compiler_frontend::datatypes::ids::TypeId,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> (HirExpression, HirNumericOp, HirNumericOperands) {
    try_lower_negation_with_type(operand_type, result_type)
        .expect("typed negation lowering should succeed")
}

fn try_lower_negation_with_type(
    operand_type: crate::compiler_frontend::datatypes::ids::TypeId,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> Result<
    (HirExpression, HirNumericOp, HirNumericOperands),
    crate::compiler_frontend::compiler_errors::CompilerError,
> {
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

    Ok((lowered.value, op, operands))
}

fn numeric_conversion(expression: &HirExpression) -> Option<(NumericScalar, NumericScalar)> {
    match &expression.kind {
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion { source, target },
            ..
        } => Some((*source, *target)),
        _ => None,
    }
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
    assert_eq!(lowered.value.ty, builtin_type_ids::INT);
    assert!(
        matches!(
            lowered.value.kind,
            HirExpressionKind::Load(HirPlace::Local(_))
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

    assert_eq!(lowered.value.ty, builtin_type_ids::FLOAT);

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
        left.kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Int,
                target: NumericScalar::Float,
            },
            ..
        }
    ));
    assert!(matches!(
        right.kind,
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
        left.kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Int,
                target: NumericScalar::Float,
            },
            ..
        }
    ));
    assert!(matches!(right.kind, HirExpressionKind::Float(_)));
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

    assert_eq!(lowered.value.ty, builtin_type_ids::INT);

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
            lowered.value.kind,
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
    let (value, numeric_op) = lower_binary_with_types(Operator::Add, u8_type, u8_type, u32_type);

    assert_eq!(value.ty, u32_type);
    let (op, operands) = numeric_op.expect("fixed addition should emit a NumericOp");
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
    assert_eq!(left.ty, u32_type);
    assert_eq!(right.ty, u32_type);
    assert_eq!(numeric_conversion(&left), expected_conversion);
    assert_eq!(numeric_conversion(&right), expected_conversion);
}

#[test]
fn fixed_i32_i64_addition_promotes_only_the_left_operand() {
    let i32_type = fixed_type(FixedScalar::I32);
    let i64_type = fixed_type(FixedScalar::I64);
    let (value, numeric_op) = lower_binary_with_types(Operator::Add, i32_type, i64_type, i64_type);

    assert_eq!(value.ty, i64_type);
    let (op, operands) = numeric_op.expect("fixed addition should emit a NumericOp");
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
        numeric_conversion(&left),
        Some((
            NumericScalar::Fixed(FixedScalar::I32),
            NumericScalar::Fixed(FixedScalar::I64),
        ))
    );
    assert_eq!(left.ty, i64_type);
    assert_eq!(right.ty, i64_type);
    assert!(numeric_conversion(&right).is_none());
}

#[test]
fn fixed_u8_division_converts_both_operands_to_f64() {
    let u8_type = fixed_type(FixedScalar::U8);
    let f64_type = fixed_type(FixedScalar::F64);
    let (value, numeric_op) = lower_binary_with_types(Operator::Divide, u8_type, u8_type, f64_type);

    assert_eq!(value.ty, f64_type);
    let (op, operands) = numeric_op.expect("fixed division should emit a NumericOp");
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
    assert_eq!(left.ty, f64_type);
    assert_eq!(right.ty, f64_type);
    assert_eq!(numeric_conversion(&left), expected_conversion);
    assert_eq!(numeric_conversion(&right), expected_conversion);
}

#[test]
fn fixed_f16_f32_multiplication_promotes_only_the_left_operand() {
    let f16_type = fixed_type(FixedScalar::F16);
    let f32_type = fixed_type(FixedScalar::F32);
    let (value, numeric_op) =
        lower_binary_with_types(Operator::Multiply, f16_type, f32_type, f32_type);

    assert_eq!(value.ty, f32_type);
    let (op, operands) = numeric_op.expect("fixed multiplication should emit a NumericOp");
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
        numeric_conversion(&left),
        Some((
            NumericScalar::Fixed(FixedScalar::F16),
            NumericScalar::Fixed(FixedScalar::F32),
        ))
    );
    assert_eq!(left.ty, f32_type);
    assert_eq!(right.ty, f32_type);
    assert!(numeric_conversion(&right).is_none());
}

#[test]
fn fixed_i8_negation_promotes_operand_to_i32() {
    let i8_type = fixed_type(FixedScalar::I8);
    let i32_type = fixed_type(FixedScalar::I32);
    let (value, op, operands) = lower_negation_with_type(i8_type, i32_type);

    assert_eq!(value.ty, i32_type);
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
        numeric_conversion(&operand),
        Some((
            NumericScalar::Fixed(FixedScalar::I8),
            NumericScalar::Fixed(FixedScalar::I32),
        ))
    );
    assert_eq!(operand.ty, i32_type);
}

#[test]
fn fixed_f64_negation_keeps_its_operand_domain() {
    let f64_type = fixed_type(FixedScalar::F64);
    let (value, op, operands) = lower_negation_with_type(f64_type, f64_type);

    assert_eq!(value.ty, f64_type);
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
    assert_eq!(operand.ty, f64_type);
    assert!(numeric_conversion(&operand).is_none());
}

#[test]
fn fixed_integer_comparison_keeps_mixed_operand_types() {
    let i64_type = fixed_type(FixedScalar::I64);
    let u64_type = fixed_type(FixedScalar::U64);
    let (value, numeric_op) = lower_binary_with_types(
        Operator::LessThan,
        i64_type,
        u64_type,
        builtin_type_ids::BOOL,
    );

    assert!(
        numeric_op.is_none(),
        "comparisons should not emit NumericOp"
    );
    let HirExpressionKind::BinOp { op, left, right } = &value.kind else {
        panic!("fixed integer comparison should remain a plain BinOp");
    };
    assert_eq!(*op, HirBinOp::Lt);
    assert_eq!(left.ty, i64_type);
    assert_eq!(right.ty, u64_type);
    assert!(numeric_conversion(left).is_none());
    assert!(numeric_conversion(right).is_none());
}

#[test]
fn unsupported_arithmetic_pair_is_an_internal_lowering_error() {
    let u8_type = fixed_type(FixedScalar::U8);
    let error = try_lower_binary_with_types(
        Operator::Add,
        builtin_type_ids::INT,
        u8_type,
        builtin_type_ids::INT,
    )
    .expect_err("unsupported arithmetic must not fall back to plain BinOp");

    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation
    );
    assert!(error.msg.contains("without a valid numeric domain"));
}

#[test]
fn unsupported_numeric_negation_is_an_internal_lowering_error() {
    let u32_type = fixed_type(FixedScalar::U32);
    let error = try_lower_negation_with_type(u32_type, u32_type)
        .expect_err("unsupported unsigned negation must not fall back to plain UnaryOp");

    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation
    );
    assert!(error.msg.contains("without a valid numeric domain"));
}
