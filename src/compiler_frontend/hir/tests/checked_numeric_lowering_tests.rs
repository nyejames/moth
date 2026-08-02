//! Focused tests for checked numeric HIR lowering (Phase 6b).
//!
//! WHAT: verifies that runtime arithmetic is lowered into `HirStatementKind::NumericOp` with the
//!       correct `HirNumericOp`, operand conversion, and failure-mode selection.
//! WHY: the checked numeric path is new HIR surface; dedicated tests guard against regressions back
//!      to plain `BinOp`/`UnaryOp` arithmetic and against wrong failure-mode wiring.

use crate::compiler_frontend::ast::ast_nodes::SourceLocation;
use crate::compiler_frontend::ast::expressions::expression::{Expression, Operator};
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{FunctionId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::tests::hir_expression_lowering_tests::{
    location, register_local, setup_builder,
};
use crate::compiler_frontend::hir::tests::symbol;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::type_id_fixture_support::{
    reference_expr, runtime_expr, runtime_operand_item, runtime_operator_item,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn int_expr(value: i32, location: SourceLocation) -> Expression {
    Expression::int(value, location, ValueMode::ImmutableOwned)
}

fn float_expr(value: f64, location: SourceLocation) -> Expression {
    Expression::float(value, location, ValueMode::ImmutableOwned)
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

fn set_current_function_return_type(
    builder: &mut HirBuilder<'_>,
    function_id: FunctionId,
    return_type: crate::compiler_frontend::datatypes::ids::TypeId,
    name: InternedPath,
) {
    builder.test_register_function_with_return_type(name, function_id, return_type);
    builder.test_set_current_function(function_id);
}

#[test]
fn checked_int_addition_lowers_to_int_add_numeric_op() {
    let mut string_table = StringTable::new();
    let loc = location(1);
    let x_name = symbol("x", &mut string_table);
    let x_ref = reference_expr(
        x_name.clone(),
        builtin_type_ids::INT,
        loc.clone(),
        ValueMode::ImmutableReference,
    );
    let two = int_expr(2, loc.clone());

    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        x_name,
        LocalId(10),
        builtin_type_ids::INT,
        loc.clone(),
    );

    let expr = runtime_expr(
        vec![
            runtime_operand_item(x_ref),
            runtime_operand_item(two),
            runtime_operator_item(Operator::Add, loc.clone()),
        ],
        builtin_type_ids::INT,
        loc.clone(),
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
    assert!(matches!(op, HirNumericOp::IntAdd));
    assert!(matches!(failure_mode, NumericFailureMode::Trap));
}

#[test]
fn checked_int_subtraction_lowers_to_int_sub_numeric_op() {
    let mut string_table = StringTable::new();
    let loc = location(1);
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(5, loc.clone())),
            runtime_operand_item(int_expr(3, loc.clone())),
            runtime_operator_item(Operator::Subtract, loc.clone()),
        ],
        builtin_type_ids::INT,
        loc.clone(),
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table);
    let _ = builder
        .lower_expression(&expr)
        .expect("int subtraction lowering should succeed");

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(matches!(op, HirNumericOp::IntSub));
}

#[test]
fn checked_regular_division_lowers_to_float_div_numeric_op() {
    let mut string_table = StringTable::new();
    let loc = location(1);
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(5, loc.clone())),
            runtime_operand_item(int_expr(2, loc.clone())),
            runtime_operator_item(Operator::Divide, loc.clone()),
        ],
        builtin_type_ids::FLOAT,
        loc.clone(),
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table);
    let lowered = builder
        .lower_expression(&expr)
        .expect("regular division lowering should succeed");

    assert_eq!(lowered.value.ty, builtin_type_ids::FLOAT);

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(matches!(op, HirNumericOp::FloatDiv));

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
        panic!("FloatDiv should be binary");
    };
    assert!(matches!(
        left.kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::IntToFloat,
            ..
        }
    ));
    assert!(matches!(
        right.kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::IntToFloat,
            ..
        }
    ));
}

#[test]
fn mixed_int_float_addition_converts_int_operand() {
    let mut string_table = StringTable::new();
    let loc = location(1);
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(1, loc.clone())),
            runtime_operand_item(float_expr(2.5, loc.clone())),
            runtime_operator_item(Operator::Add, loc.clone()),
        ],
        builtin_type_ids::FLOAT,
        loc.clone(),
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table);
    let _ = builder
        .lower_expression(&expr)
        .expect("mixed addition lowering should succeed");

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(matches!(op, HirNumericOp::FloatAdd));

    let numeric_op = builder
        .test_current_block_statements()
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp { operands, .. } => Some(operands.clone()),
            _ => None,
        })
        .expect("NumericOp statement should exist");
    let HirNumericOperands::Binary { left, right } = numeric_op else {
        panic!("FloatAdd should be binary");
    };
    assert!(matches!(
        left.kind,
        HirExpressionKind::Cast {
            policy: BuiltinCastPolicyId::IntToFloat,
            ..
        }
    ));
    assert!(matches!(right.kind, HirExpressionKind::Float(_)));
}

#[test]
fn unary_int_negation_lowers_to_int_neg_numeric_op() {
    let mut string_table = StringTable::new();
    let loc = location(1);
    let x_name = symbol("x", &mut string_table);
    let x_ref = reference_expr(
        x_name.clone(),
        builtin_type_ids::INT,
        loc.clone(),
        ValueMode::ImmutableReference,
    );

    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        x_name,
        LocalId(10),
        builtin_type_ids::INT,
        loc.clone(),
    );

    let expr = runtime_expr(
        vec![
            runtime_operand_item(x_ref),
            runtime_operator_item(Operator::Negate, loc.clone()),
        ],
        builtin_type_ids::INT,
        loc.clone(),
        ValueMode::MutableOwned,
    );

    let lowered = builder
        .lower_expression(&expr)
        .expect("unary negation lowering should succeed");

    assert_eq!(lowered.value.ty, builtin_type_ids::INT);

    let (op, _) = find_single_numeric_op(&builder).expect("expected a NumericOp");
    assert!(matches!(op, HirNumericOp::IntNeg));
}

#[test]
fn numeric_failure_mode_is_return_error_for_builtin_error_function() {
    let mut string_table = StringTable::new();
    let loc = location(1);
    let fn_name = symbol("__test_fn_error", &mut string_table);
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(1, loc.clone())),
            runtime_operand_item(int_expr(2, loc.clone())),
            runtime_operator_item(Operator::Add, loc.clone()),
        ],
        builtin_type_ids::INT,
        loc.clone(),
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table);
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
    let mut string_table = StringTable::new();
    let loc = location(1);
    let fn_name = symbol("__test_fn_string_error", &mut string_table);
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(1, loc.clone())),
            runtime_operand_item(int_expr(2, loc.clone())),
            runtime_operator_item(Operator::Add, loc.clone()),
        ],
        builtin_type_ids::INT,
        loc.clone(),
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table);
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
    let mut string_table = StringTable::new();
    let loc = location(1);
    let fn_name = symbol("__test_fn_non_fallible", &mut string_table);
    let expr = runtime_expr(
        vec![
            runtime_operand_item(int_expr(1, loc.clone())),
            runtime_operand_item(int_expr(2, loc.clone())),
            runtime_operator_item(Operator::Add, loc.clone()),
        ],
        builtin_type_ids::INT,
        loc.clone(),
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table);
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
