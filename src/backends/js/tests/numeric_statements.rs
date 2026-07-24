//! Checked numeric operation lowering tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::ids::TypeId;
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

    let float_statement = statement(1, statement_kind, 1);

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
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
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
            "__moth_assign_value(moth_result_l0, __moth_numeric_trap(__moth_format_float(1.5)));"
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
        output.contains("__moth_assign_value(moth_result_l0, __moth_format_float(1.5));"),
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
    let mut string_table = StringTable::new();
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
        1,
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
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed")
    .source
}

/// Verifies that trap-mode `IntAdd` assigns the scalar success value to the result local.
#[test]
fn trap_mode_int_add_lowers_to_trapped_helper() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        HirNumericOp::IntAdd,
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.int,
    );

    assert!(
        output.contains("__moth_assign_value(moth_result_l0, __moth_numeric_trap(__moth_int_add(1, 2)));"),
        "trap-mode IntAdd must assign the scalar trap result"
    );
}

/// Verifies that return-error-mode `IntAdd` assigns the fallible carrier directly.
#[test]
fn return_error_mode_int_add_lowers_to_carrier() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        HirNumericOp::IntAdd,
        NumericFailureMode::ReturnError,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.fallible_int_string,
    );

    assert!(
        output.contains("__moth_assign_value(moth_result_l0, __moth_int_add(1, 2));"),
        "ReturnError IntAdd must assign the helper carrier directly"
    );
    assert!(
        !output.contains("__moth_numeric_trap(__moth_int_add"),
        "ReturnError IntAdd must not wrap the helper in __moth_numeric_trap"
    );
}

/// Verifies that a unary numeric operation lowers through the helper path.
#[test]
fn int_neg_lowers_to_unary_helper() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        HirNumericOp::IntNeg,
        NumericFailureMode::Trap,
        HirNumericOperands::Unary {
            operand: int_expression(1, 1, types.int, region),
        },
        types.int,
    );

    assert!(
        output.contains("__moth_assign_value(moth_result_l0, __moth_numeric_trap(__moth_int_neg(1)));"),
        "trap-mode IntNeg must lower to the unary helper"
    );
}

/// Verifies that float operations also lower to the checked helper path.
#[test]
fn float_div_lowers_to_helper() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let output = lower_minimal_module_with_numeric_op(
        HirNumericOp::FloatDiv,
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: float_expression(1, 1.0, types.float, region),
            right: float_expression(2, 2.0, types.float, region),
        },
        types.float,
    );

    assert!(
        output
            .contains("__moth_assign_value(moth_result_l0, __moth_numeric_trap(__moth_float_div(1, 2)));"),
        "trap-mode FloatDiv must lower to the checked float helper"
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
        !source.contains("const __BS_INT_MIN ="),
        "modules without NumericOp must not emit numeric range constants"
    );
}

/// Verifies that the numeric helper group is emitted when a NumericOp is reachable.
#[test]
fn numeric_helpers_emitted_when_numeric_op_reachable() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        HirNumericOp::IntAdd,
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
        source.contains("const __BS_INT_MIN ="),
        "numeric modules must emit numeric range constants"
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
        HirNumericOp::IntAdd,
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
        HirNumericOp::IntNeg,
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

/// Verifies that the shared `__moth_int_check` helper enforces the i32 range and reports
/// `IntOverflow` on failure.
#[test]
fn int_check_helper_contains_overflow_error() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        HirNumericOp::IntAdd,
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.int,
    );

    let check = helper_source(&source, "__moth_int_check");
    let overflow = BuiltinErrorCode::IntOverflow;
    let expected = format!(
        r#"__moth_error_result("{}", {})"#,
        overflow.default_message(),
        overflow.as_i32()
    );

    assert!(
        check.contains(&expected),
        "__moth_int_check must use IntOverflow error result"
    );
}

/// Verifies that integer helpers delegate final i32 validation to `__moth_int_check`.
#[test]
fn int_helpers_delegate_to_int_check() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        HirNumericOp::IntAdd,
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.int,
    );

    let add = helper_source(&source, "__moth_int_add");

    assert!(
        add.contains("return __moth_int_check(result);"),
        "__moth_int_add must delegate i32 validation to __moth_int_check"
    );
    assert!(
        !add.contains("Number.isInteger(result)"),
        "__moth_int_add must not duplicate the i32 range check"
    );
}

/// Verifies that `DivideByZero` code and message appear in the division helpers.
#[test]
fn int_div_helper_contains_divide_by_zero_error() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        HirNumericOp::IntDiv,
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, 1, types.int, region),
            right: int_expression(2, 2, types.int, region),
        },
        types.int,
    );

    let div = helper_source(&source, "__moth_int_div");
    let divide_by_zero = BuiltinErrorCode::DivideByZero;
    let expected = format!(
        r#"__moth_error_result("{}", {})"#,
        divide_by_zero.default_message(),
        divide_by_zero.as_i32()
    );

    assert!(
        div.contains(&expected),
        "__moth_int_div must use DivideByZero error result"
    );
}

/// Verifies that `InvalidExponent` code and message appear in `__moth_int_pow`.
#[test]
fn int_pow_helper_contains_invalid_exponent_error() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        HirNumericOp::IntPow,
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: int_expression(1, 2, types.int, region),
            right: int_expression(2, 3, types.int, region),
        },
        types.int,
    );

    let pow = helper_source(&source, "__moth_int_pow");
    let invalid_exponent = BuiltinErrorCode::InvalidExponent;
    let expected = format!(
        r#"__moth_error_result("{}", {})"#,
        invalid_exponent.default_message(),
        invalid_exponent.as_i32()
    );

    assert!(
        pow.contains(&expected),
        "__moth_int_pow must use InvalidExponent error result"
    );
}

/// Verifies that `FloatNonFinite` code and message appear in the float helpers.
#[test]
fn float_helpers_contain_non_finite_error() {
    let region = RegionId(0);
    let (_, types) = build_type_environment();

    let source = lower_minimal_module_with_numeric_op(
        HirNumericOp::FloatAdd,
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: float_expression(1, 1.0, types.float, region),
            right: float_expression(2, 2.0, types.float, region),
        },
        types.float,
    );

    let add = helper_source(&source, "__moth_float_add");
    let non_finite = BuiltinErrorCode::FloatNonFinite;
    let expected = format!(
        r#"__moth_error_result("{}", {})"#,
        non_finite.default_message(),
        non_finite.as_i32()
    );

    assert!(
        add.contains(&expected),
        "__moth_float_add must use FloatNonFinite error result"
    );
}

/// Verifies that malformed HIR arity produces a compiler error rather than invalid JS.
#[test]
fn numeric_op_arity_mismatch_returns_error() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    // IntAdd is binary but we supply unary operands.
    let numeric_statement = statement(
        1,
        HirStatementKind::NumericOp {
            op: HirNumericOp::IntAdd,
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Unary {
                operand: int_expression(1, 1, types.int, region),
            },
            result: LocalId(0),
        },
        1,
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
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    let result = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    );

    assert!(
        result.is_err(),
        "NumericOp arity mismatch must fail lowering with a compiler error"
    );
}
