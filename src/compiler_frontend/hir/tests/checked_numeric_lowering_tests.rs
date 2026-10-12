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
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expression_store::{HirConstructionFailure, HirExpressionStore};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_builder::{register_local, setup_builder};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirTerminator, RuntimeFailureCause};
use crate::compiler_frontend::hir::tests::symbol;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, node, reference_expr_with_type_id,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    runtime_expr, runtime_operand_item, runtime_operator_item,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn int_expr(value: i64, span: Option<crate::compiler_frontend::source::SourceSpan>) -> Expression {
    Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range"),
        span,
        ValueMode::ImmutableOwned,
    )
}

fn float_expr(
    value: f64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> Expression {
    Expression::fixed_scalar(
        FixedScalarValue::binary_float(FixedScalar::F64, value)
            .expect("F64 literal is exactly representable"),
        span,
        ValueMode::ImmutableOwned,
    )
}

/// One found `HirStatementKind::NumericOp` with everything the lowering fixtures assert on.
///
/// WHY: the operation, its failure mode, operands, result destination and owning block all belong
///      to the same statement; one scanner keeps the narrower finders from drifting apart.
#[derive(Debug)]
struct FoundNumericOp {
    block: BlockId,
    op: HirNumericOp,
    failure_mode: NumericFailureMode,
    operands: HirNumericOperands,
    result: HirLocalDestination,
}

fn find_numeric_op_statement(builder: &HirBuilder<'_>) -> Option<FoundNumericOp> {
    builder.module.blocks.iter().find_map(|block| {
        block
            .statements
            .iter()
            .find_map(|statement| match &statement.kind {
                HirStatementKind::NumericOp {
                    op,
                    failure_mode,
                    operands,
                    result,
                } => Some(FoundNumericOp {
                    block: block.id,
                    op: *op,
                    failure_mode: *failure_mode,
                    operands: operands.clone(),
                    result: *result,
                }),
                _ => None,
            })
    })
}

fn find_single_numeric_op(builder: &HirBuilder<'_>) -> Option<(HirNumericOp, NumericFailureMode)> {
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
    failure_mode: Option<NumericFailureMode>,
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
    let int_operand = Expression::coerced(int_expr(1, loc), float_type);
    let float_operand = float_expr(2.5, loc);
    let (left, right) = if int_on_left {
        (int_operand, float_operand)
    } else {
        (float_operand, int_operand)
    };
    let expression = runtime_expr(
        vec![
            runtime_operand_item(left),
            runtime_operand_item(right),
            runtime_operator_item(operator, loc),
        ],
        builtin_type_ids::BOOL,
        loc,
        ValueMode::MutableOwned,
    );

    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let lowered = builder
        .lower_expression(&expression)
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
    builder.test_register_function_with_return_type(name, function_id, return_type);
    builder.test_set_current_function(function_id);
}

fn lower_u8_compound_add_assignment(
    builder: &mut HirBuilder<'_>,
    target_name: PathId,
    right_name: PathId,
) -> Result<(), HirConstructionFailure> {
    let u8_type = fixed_type(FixedScalar::U8);
    let i8_type = fixed_type(FixedScalar::I8);
    let i16_type = fixed_type(FixedScalar::I16);
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
                i8_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operator_item(Operator::Add, None),
        ],
        i16_type,
        None,
        ValueMode::MutableOwned,
    );
    let value = Expression::cast(
        ResolvedCastExpression {
            source: Box::new(source),
            source_type_id: i16_type,
            target_type_id: u8_type,
            target: BuiltinCastTarget::Fixed(FixedScalar::U8),
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
    let scalar_type = fixed_type(scalar);
    let value = runtime_expr(
        vec![
            runtime_operand_item(reference_expr_with_type_id(
                target_name,
                scalar_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operand_item(reference_expr_with_type_id(
                right_name,
                scalar_type,
                None,
                ValueMode::ImmutableReference,
            )),
            runtime_operator_item(Operator::Add, None),
        ],
        scalar_type,
        None,
        ValueMode::MutableOwned,
    );
    builder.lower_statement_node(&node(
        NodeKind::Assignment {
            target: assignment_target(target_name, DataType::Inferred, scalar_type, None),
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
                            source: NumericScalar::Fixed(FixedScalar::I16),
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
    let i8_type = fixed_type(FixedScalar::I8);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let error_type = builder.test_register_builtin_error_type();
    let return_type = builder
        .type_environment
        .intern_fallible_carrier(u8_type, error_type);
    set_current_function_return_type(&mut builder, FunctionId(1), return_type, function_name);
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
    lower_same_type_compound_add_assignment(
        &mut builder,
        target_name,
        right_name,
        FixedScalar::U32,
    )
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
    let return_type = builder
        .type_environment
        .intern_fallible_carrier(u8_type, error_type);
    set_current_function_return_type(&mut builder, FunctionId(1), return_type, function_name);
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
    assert_eq!(found.failure_mode, NumericFailureMode::ReturnError);
    assert!(
        !builder
            .module
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(&statement.kind, HirStatementKind::CastOp { .. })),
        "a same-type compound update must not fabricate a store conversion"
    );

    let numeric_block = &builder.module.blocks[found.block.0 as usize];
    let HirTerminator::FallibleBranch {
        success_block,
        error_block,
        ..
    } = &numeric_block.terminator
    else {
        panic!("checked same-type U8 arithmetic must branch on its fallible carrier");
    };
    let (success_block, error_block) = (*success_block, *error_block);
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
            HirExpressionKind::FallibleUnwrapSuccess { result }
                if matches!(
                    &builder.module.expressions.expression(*result).kind,
                    HirExpressionKind::Load(place)
                        if place.root == found.result.local() && super::is_local_place(place)
                )
        ),
        "the store must consume the arithmetic success payload after its failure branch"
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
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
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
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
    );

    let expr = runtime_expr(
        vec![
            runtime_operand_item(x_ref),
            runtime_operand_item(two),
            runtime_operator_item(Operator::Add, loc),
        ],
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
    );

    let lowered = builder
        .lower_expression(&expr)
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
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
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
            domain: NumericScalar::Fixed(FixedScalar::I32)
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
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let lowered = builder
        .lower_expression(&expr)
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
    let numeric_op = builder
        .test_current_block_statements()
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp { operands, .. } => Some(operands.clone()),
            _ => None,
        })
        .expect("NumericOp statement should exist");
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
    let loc = None;
    let f64_type = builtin_type_ids::fixed_scalar(FixedScalar::F64);
    // Bounded fixed domains never mix implicitly, so the integer operand carries an
    // explicit coercion to the receiving F64 domain before the operation.
    let expr = runtime_expr(
        vec![
            runtime_operand_item(Expression::coerced(int_expr(1, loc), f64_type)),
            runtime_operand_item(float_expr(2.5, loc)),
            runtime_operator_item(Operator::Add, loc),
        ],
        f64_type,
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
            domain: NumericScalar::Fixed(FixedScalar::F64)
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
    let loc = None;
    let x_name = symbol("x", &mut path_fork, &mut string_table);
    let x_ref = reference_expr_with_type_id(
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

    let expr = runtime_expr(
        vec![
            runtime_operand_item(x_ref),
            runtime_operator_item(Operator::Negate, loc),
        ],
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
    );

    let lowered = builder
        .lower_expression(&expr)
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
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let error_type_id = builder.test_register_builtin_error_type();
    let return_type = builder.type_environment.intern_fallible_carrier(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        error_type_id,
    );
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
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let return_type = builder.type_environment.intern_fallible_carrier(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        builtin_type_ids::STRING,
    );
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
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        loc,
        ValueMode::MutableOwned,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    set_current_function_return_type(
        &mut builder,
        FunctionId(1),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        fn_name,
    );

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
        Some(NumericFailureMode::Trap),
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
        Some(NumericFailureMode::Trap),
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

#[test]
fn unsupported_arithmetic_pair_is_an_internal_lowering_error() {
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let i64_type = fixed_type(FixedScalar::I64);
    let u64_type = fixed_type(FixedScalar::U64);
    let f64_type = fixed_type(FixedScalar::F64);

    // Fixed integer and binary-float operands never mix implicitly, and a signed
    // type paired with `U64` has no common domain: both require explicit conversion.
    for (left_type, right_type, result_type) in [
        (i32_type, f64_type, f64_type),
        (f64_type, i32_type, f64_type),
        (i64_type, u64_type, i64_type),
    ] {
        let failure =
            try_lower_binary_with_types(Operator::Add, left_type, right_type, result_type)
                .expect_err("unsupported arithmetic must not fall back to plain BinOp");
        let HirConstructionFailure::Infrastructure(error) = failure else {
            panic!("unsupported arithmetic should be an infrastructure failure");
        };

        assert_eq!(
            error.error_type,
            crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation
        );
    }
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
fn number_power_keeps_its_exponent_in_fixed_i32() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let base_name = symbol("base", &mut path_fork, &mut string_table);
    let exponent_name = symbol("exponent", &mut path_fork, &mut string_table);
    let scale = NumberScale::new(3).expect("test Dec scale is valid");
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

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
