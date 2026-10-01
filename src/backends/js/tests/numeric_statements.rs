//! Checked numeric operation lowering tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;

// FormatFloat and ValidateFloat statement lowering tests [float]
// ---------------------------------------------------------------------------

/// Builds and lowers a minimal module containing one `FormatFloat` or `ValidateFloat` statement.
///
/// WHY: Float statement lowering tests need the same HIR scaffolding every time; keeping it in one
/// helper lets each public fixture name only the statement kind, failure mode, source, and result
/// type.
fn lower_minimal_module_with_float_statement(
    kind: HirFloatStatementKind,
    failure_mode: NumericFailureMode,
    source: HirExpression,
    result_type: TypeId,
) -> String {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let statement_kind = match kind {
        HirFloatStatementKind::Format => HirStatementKind::FormatFloat {
            source,
            failure_mode,
            result: LocalId(0),
        },
        HirFloatStatementKind::Validate => HirStatementKind::ValidateFloat {
            source,
            failure_mode,
            result: LocalId(0),
        },
    };

    let float_statement = statement(1, statement_kind);

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, result_type, region)],
        statements: vec![float_statement],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed")
    .source
}

#[derive(Clone, Copy)]
enum HirFloatStatementKind {
    Format,
    Validate,
}

/// Verifies that trap-mode `FormatFloat` assigns the scalar formatted string to the result local.
#[test]
fn trap_mode_format_float_lowers_to_trapped_helper() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Format,
        NumericFailureMode::Trap,
        float_expression(1, 1.5, types.float, region),
        types.string,
    );

    assert!(
        output.contains(
            "__moth_assign_value(moth_result_l0, __moth_numeric_trap(__moth_format_float(1.5, 64, \"Float\")));"
        ),
        "trap-mode FormatFloat must assign the scalar trap result"
    );
}

/// Verifies that return-error-mode `FormatFloat` assigns the fallible carrier directly.
#[test]
fn return_error_mode_format_float_lowers_to_carrier() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Format,
        NumericFailureMode::ReturnError,
        float_expression(1, 1.5, types.float, region),
        types.fallible_int_string,
    );

    assert!(
        output.contains(
            "__moth_assign_value(moth_result_l0, __moth_format_float(1.5, 64, \"Float\"));"
        ),
        "ReturnError FormatFloat must assign the helper carrier directly"
    );
    assert!(
        !output.contains("__moth_numeric_trap(__moth_format_float"),
        "ReturnError FormatFloat must not wrap the helper in __moth_numeric_trap"
    );
}

/// Verifies that trap-mode `ValidateFloat` assigns the scalar finite Float to the result local.
#[test]
fn trap_mode_validate_float_lowers_to_trapped_helper() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Validate,
        NumericFailureMode::Trap,
        float_expression(1, 1.5, types.float, region),
        types.float,
    );

    assert!(
        output.contains(
            "__moth_assign_value(moth_result_l0, __moth_numeric_trap(__moth_float_validate(1.5)));"
        ),
        "trap-mode ValidateFloat must assign the scalar trap result"
    );
}

/// Verifies that return-error-mode `ValidateFloat` assigns the fallible carrier directly.
#[test]
fn return_error_mode_validate_float_lowers_to_carrier() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Validate,
        NumericFailureMode::ReturnError,
        float_expression(1, 1.5, types.float, region),
        types.fallible_int_string,
    );

    assert!(
        output.contains("__moth_assign_value(moth_result_l0, __moth_float_validate(1.5));"),
        "ReturnError ValidateFloat must assign the helper carrier directly"
    );
    assert!(
        !output.contains("__moth_numeric_trap(__moth_float_validate"),
        "ReturnError ValidateFloat must not wrap the helper in __moth_numeric_trap"
    );
}

/// Verifies that the Float formatting helper is emitted when `FormatFloat` is reachable.
#[test]
fn format_float_helper_emitted_when_format_float_reachable() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Format,
        NumericFailureMode::Trap,
        float_expression(1, 1.5, types.float, region),
        types.string,
    );

    assert!(
        source.contains("function __moth_format_float("),
        "modules with FormatFloat must emit __moth_format_float"
    );
    assert!(
        !source.contains("function __moth_float_validate("),
        "FormatFloat should not emit the separate boundary-validation helper"
    );
    assert!(
        source.contains("function __moth_numeric_trap("),
        "modules with FormatFloat must emit __moth_numeric_trap"
    );
}

/// Verifies that the Float validation helper is emitted when `ValidateFloat` is reachable.
#[test]
fn validate_float_helper_emitted_when_validate_float_reachable() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_float_statement(
        HirFloatStatementKind::Validate,
        NumericFailureMode::Trap,
        float_expression(1, 1.5, types.float, region),
        types.float,
    );

    assert!(
        source.contains("function __moth_float_validate("),
        "modules with ValidateFloat must emit __moth_float_validate"
    );
    assert!(
        !source.contains("function __moth_format_float("),
        "ValidateFloat should not emit the separate formatting helper"
    );
    assert!(
        source.contains("function __moth_numeric_trap("),
        "modules with ValidateFloat must emit __moth_numeric_trap"
    );
}

/// Verifies that Float helpers are not emitted for modules without Float statements.
#[test]
fn float_helpers_not_emitted_without_float_statement() {
    let source = lower_minimal_module("main");

    assert!(
        !source.contains("function __moth_format_float("),
        "modules without Float statements must not emit __moth_format_float"
    );
    assert!(
        !source.contains("function __moth_float_validate("),
        "modules without Float statements must not emit __moth_float_validate"
    );
}

// Numeric operation statement lowering tests [numeric]
// ---------------------------------------------------------------------------

/// Builds and lowers a minimal module containing one `NumericOp` statement.
///
/// WHY: numeric lowering tests need the same HIR scaffolding every time; keeping it in one
/// helper lets each public fixture name only the operation, failure mode, operands, and result
/// type.
fn lower_minimal_module_with_numeric_op(
    op: HirNumericOp,
    failure_mode: NumericFailureMode,
    operands: HirNumericOperands,
    result_type: TypeId,
) -> String {
    lower_minimal_module_with_numeric_op_for_profile(
        op,
        failure_mode,
        operands,
        result_type,
        NumericProfile::STANDARD,
    )
}

fn lower_minimal_module_with_numeric_op_for_profile(
    op: HirNumericOp,
    failure_mode: NumericFailureMode,
    operands: HirNumericOperands,
    result_type: TypeId,
    numeric_profile: NumericProfile,
) -> String {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let numeric_statement = statement(
        1,
        HirStatementKind::NumericOp {
            op,
            failure_mode,
            operands,
            result: LocalId(0),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, result_type, region)],
        statements: vec![numeric_statement],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &NumericProofs::default(),
        &string_table,
        JsLoweringConfig::direct_js(false, numeric_profile),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed")
    .source
}

fn int_op(operator: NumericOperator) -> HirNumericOp {
    HirNumericOp {
        operator,
        domain: NumericScalar::Int,
    }
}

fn float_op(operator: NumericOperator) -> HirNumericOp {
    HirNumericOp {
        operator,
        domain: NumericScalar::Float,
    }
}

/// Verifies that trap-mode Int addition assigns the scalar success value to the result local.
#[test]
fn trap_mode_int_add_lowers_to_trapped_helper() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.int,
    );

    assert!(
        output.contains(
            "__moth_assign_value(moth_result_l0, __moth_numeric_trap(__moth_int_add(1, 2, -2147483648, 2147483647)));"
        ),
        "trap-mode Int addition must assign the checked Number carrier result"
    );
}

/// Verifies that return-error-mode Int addition assigns the fallible carrier directly.
#[test]
fn return_error_mode_int_add_lowers_to_carrier() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Add),
        NumericFailureMode::ReturnError,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.fallible_int_string,
    );

    assert!(
        output.contains(
            "__moth_assign_value(moth_result_l0, __moth_int_add(1, 2, -2147483648, 2147483647));"
        ),
        "ReturnError Int addition must assign the helper carrier directly"
    );
    assert!(
        !output.contains("__moth_numeric_trap(__moth_int_add"),
        "ReturnError Int addition must not wrap the helper in __moth_numeric_trap"
    );
}

/// Verifies that a unary numeric operation lowers through the helper path.
#[test]
fn int_neg_lowers_to_unary_helper() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Negate),
        NumericFailureMode::Trap,
        HirNumericOperands::Unary {
            operand: int_expression(1, 1, types.int, region),
        },
        types.int,
    );

    assert!(
        output.contains(
            "__moth_assign_value(moth_result_l0, __moth_numeric_trap(__moth_int_neg(1, -2147483648, 2147483647)));"
        ),
        "trap-mode Int negation must lower to the checked unary helper"
    );
}

/// Verifies that float operations also lower to the checked helper path.
#[test]
fn float_div_lowers_to_helper() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        float_op(NumericOperator::Divide),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: float_expression(1, 1.0, types.float, region),
            right: float_expression(2, 2.0, types.float, region),
        },
        types.float,
    );

    assert!(
        output.contains(
            "__moth_assign_value(moth_result_l0, __moth_numeric_trap(__moth_float_div(1, 2)));"
        ),
        "trap-mode Float division must lower to the checked float helper"
    );
}

/// Verifies that numeric helpers are not emitted for modules without NumericOp.
#[test]
fn numeric_helpers_not_emitted_without_numeric_op() {
    let source = lower_minimal_module("main");

    assert!(
        !source.contains("function __moth_int_add("),
        "modules without NumericOp must not emit __moth_int_add"
    );
    assert!(
        !source.contains("function __moth_numeric_trap("),
        "modules without NumericOp must not emit __moth_numeric_trap"
    );
    assert!(
        !source.contains("function __moth_bigint_add("),
        "modules without a BigInteger-domain operation must not emit that helper family"
    );
}

/// Verifies that the numeric helper group is emitted when a NumericOp is reachable.
#[test]
fn numeric_helpers_emitted_when_numeric_op_reachable() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.int,
    );

    assert!(
        source.contains("function __moth_int_add("),
        "numeric modules must emit __moth_int_add"
    );
    assert!(
        source.contains("function __moth_int_check("),
        "numeric modules must emit __moth_int_check"
    );
    assert!(
        source.contains("function __moth_numeric_trap("),
        "numeric modules must emit __moth_numeric_trap"
    );
    assert!(
        source.contains("__moth_int_add(1, 2, -2147483648, 2147483647)"),
        "numeric operations must pass the semantic Int bounds to the shared helper"
    );
    assert!(
        !source.contains("function __moth_format_float("),
        "NumericOp should not emit the Float formatting helper"
    );
    assert!(
        !source.contains("function __moth_float_validate("),
        "NumericOp should not emit the Float boundary-validation helper"
    );
}

// Numeric helper contract tests [numeric-helper]
// ---------------------------------------------------------------------------

/// Verifies that the trap helper returns ok values and throws err values.
#[test]
fn numeric_trap_returns_ok_and_throws_err() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.int,
    );

    let trap = helper_source(&source, "__moth_numeric_trap");

    assert!(
        trap.contains("carrier.tag === \"ok\"") && trap.contains("return carrier.value;"),
        "__moth_numeric_trap must return ok values"
    );
    assert!(
        trap.contains("carrier.tag === \"err\"")
            && trap.contains("throw new Error(__moth_error_message(carrier.value));"),
        "__moth_numeric_trap must throw JS errors using the canonical Moth error message"
    );
}

/// Verifies that integer helper successes normalize JS `-0` to the single Moth Int zero.
#[test]
fn int_ok_helper_normalizes_negative_zero() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Negate),
        NumericFailureMode::Trap,
        HirNumericOperands::Unary {
            operand: int_expression(1, 0, types.int, region),
        },
        types.int,
    );

    let helper = helper_source(&source, "__moth_int_ok");

    assert!(
        helper.contains("Object.is(value, -0) ? 0 : value"),
        "integer helpers must normalize JS -0 at the success boundary"
    );
}

/// Verifies that exact-Number integer helpers delegate range checking to `__moth_int_check`.
#[test]
fn int_helpers_delegate_to_int_check() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        int_op(NumericOperator::Add),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.int,
    );

    let add = helper_source(&source, "__moth_int_add");

    assert!(
        add.contains("return __moth_int_check(a + b, min, max);"),
        "__moth_int_add must pass its semantic result range to __moth_int_check"
    );
    assert!(
        !add.contains("Number.isInteger(result)"),
        "__moth_int_add must not duplicate the integer carrier check"
    );
}

/// Verifies that malformed HIR arity produces a compiler error rather than invalid JS.
#[test]
fn numeric_op_arity_mismatch_returns_error() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    // Int addition is binary but we supply unary operands.
    let numeric_statement = statement(
        1,
        HirStatementKind::NumericOp {
            op: int_op(NumericOperator::Add),
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Unary {
                operand: int_expression(1, 1, types.int, region),
            },
            result: LocalId(0),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.int, region)],
        statements: vec![numeric_statement],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    let result = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    );

    assert!(
        result.is_err(),
        "NumericOp arity mismatch must fail lowering with a compiler error"
    );
}
