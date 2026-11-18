//! Executable behavior tests for JavaScript fast scalar arithmetic.
//!
//! WHAT: lowers one reachable fast operation so the prelude carries its helper family, then runs
//!       the emitted helpers and lowered expressions under Node with boundary values.
//! WHY: only execution proves that fast integers wrap at the semantic result width, that U32
//!       products keep their low bits, that division/remainder/power traps are fatal, and that
//!       binary floats round per operation while rejecting non-finite results and signed-zero
//!       divisors. Source inspection alone cannot distinguish wrapping from checked arithmetic.

use super::support::*;
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_operators::{NumericOperator, NumericSemantics};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionReturnContract};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::{
    HirFastStoreConversion, HirNumericOp, HirNumericOperands, LoopStepFailureCause,
    NumericFailureMode,
};
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// JavaScript capture helper shared by the fast helper-body tests.
///
/// WHY: bigint results need their exact decimal text and traps need their canonical message, so
///      one snippet keeps every expected JSON row directly comparable.
const CAPTURE_SCALAR: &str = r#"
const capture = operation => {
    try {
        const value = operation();
        return typeof value === "bigint" ? value.toString() + "n" : String(value);
    } catch (error) {
        return "trap:" + error.message;
    }
};
"#;

fn fast_op(operator: NumericOperator, scalar: FixedScalar) -> HirNumericOp {
    HirNumericOp {
        operator,
        domain: NumericScalar::Fixed(scalar),
        semantics: NumericSemantics::Fast,
    }
}

fn lower_fast_binary(
    operator: NumericOperator,
    scalar: FixedScalar,
    left: FixedScalarValue,
    right: FixedScalarValue,
) -> String {
    let ty = builtin_type_ids::fixed_scalar(scalar);
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let left = fixed_scalar_expression(left, ty, region, &mut expressions);
    let right = fixed_scalar_expression(right, ty, region, &mut expressions);
    let op = fast_op(operator, scalar);
    lower_minimal_module_with_numeric_op(
        op,
        op.fast_statement_failure_mode(),
        HirNumericOperands::Binary { left, right },
        ty,
        expressions,
    )
}

/// Lowers one fast unary operation as the entry's returned scalar.
fn lower_fast_unary_entry(
    operator: NumericOperator,
    scalar: FixedScalar,
    operand: FixedScalarValue,
) -> ScalarEntry {
    let ty = builtin_type_ids::fixed_scalar(scalar);
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let operand = fixed_scalar_expression(operand, ty, region, &mut expressions);
    let op = fast_op(operator, scalar);
    lower_fast_entry(
        op,
        op.fast_statement_failure_mode(),
        HirNumericOperands::Unary { operand },
        ty,
        expressions,
    )
}

/// Lowers one fast binary operation as the entry's returned scalar.
fn lower_fast_binary_entry(
    operator: NumericOperator,
    scalar: FixedScalar,
    left: FixedScalarValue,
    right: FixedScalarValue,
) -> ScalarEntry {
    let ty = builtin_type_ids::fixed_scalar(scalar);
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let left = fixed_scalar_expression(left, ty, region, &mut expressions);
    let right = fixed_scalar_expression(right, ty, region, &mut expressions);
    let op = fast_op(operator, scalar);
    lower_fast_entry(
        op,
        op.fast_statement_failure_mode(),
        HirNumericOperands::Binary { left, right },
        ty,
        expressions,
    )
}

fn lower_fast_entry(
    op: HirNumericOp,
    failure_mode: NumericFailureMode,
    operands: HirNumericOperands,
    result_type: crate::compiler_frontend::datatypes::ids::TypeId,
    expressions: HirExpressionStore,
) -> ScalarEntry {
    let (type_environment, _) = build_type_environment();
    lower_returned_scalar_entry(
        type_environment,
        expressions,
        HirStatementKind::NumericOp {
            op,
            failure_mode,
            operands,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        result_type,
    )
}

/// Executes the lowered entry and renders its scalar result or thrown message.
fn run_fast_entry(entry: &ScalarEntry) -> String {
    let mut script = entry.source.clone();
    script.push_str(&format!(
        "\ntry {{ console.log(String({}())); }} catch (error) {{ console.log(\"trap:\" + error.message); }}",
        entry.function_name
    ));
    run_javascript(&script)
}

fn signed(scalar: FixedScalar, value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(scalar, value).expect("fixture value fits its signed scalar")
}

fn unsigned(scalar: FixedScalar, value: u64) -> FixedScalarValue {
    FixedScalarValue::unsigned(scalar, value).expect("fixture value fits its unsigned scalar")
}

fn float(scalar: FixedScalar, value: f64) -> FixedScalarValue {
    FixedScalarValue::binary_float(scalar, value).expect("fixture value is exact at its precision")
}

/// A fast operation with literal operands must lower to its plain wrapping expression and
/// produce the wrapped value.
#[test]
fn fast_integer_expressions_wrap_at_the_semantic_result_width() {
    let number_cases = [
        (
            NumericOperator::Add,
            FixedScalar::I8,
            signed(FixedScalar::I8, 127),
            signed(FixedScalar::I8, 1),
            "__moth_fast_int_wrap((127 + 1), 8, true)",
            "-128",
        ),
        (
            NumericOperator::Add,
            FixedScalar::U8,
            unsigned(FixedScalar::U8, 255),
            unsigned(FixedScalar::U8, 1),
            "__moth_fast_int_wrap((255 + 1), 8, false)",
            "0",
        ),
        (
            NumericOperator::Subtract,
            FixedScalar::I16,
            signed(FixedScalar::I16, -32768),
            signed(FixedScalar::I16, 1),
            "__moth_fast_int_wrap((-32768 - 1), 16, true)",
            "32767",
        ),
        (
            NumericOperator::Subtract,
            FixedScalar::U16,
            unsigned(FixedScalar::U16, 0),
            unsigned(FixedScalar::U16, 1),
            "__moth_fast_int_wrap((0 - 1), 16, false)",
            "65535",
        ),
        (
            NumericOperator::Multiply,
            FixedScalar::I32,
            signed(FixedScalar::I32, 2147483647),
            signed(FixedScalar::I32, 2),
            "__moth_fast_int_wrap(Math.imul(2147483647, 2), 32, true)",
            "-2",
        ),
        // A Number product would lose low bits here; `Math.imul` keeps the exact low word.
        (
            NumericOperator::Multiply,
            FixedScalar::U32,
            unsigned(FixedScalar::U32, 4294967295),
            unsigned(FixedScalar::U32, 4294967295),
            "__moth_fast_int_wrap(Math.imul(4294967295, 4294967295), 32, false)",
            "1",
        ),
        (
            NumericOperator::Add,
            FixedScalar::U32,
            unsigned(FixedScalar::U32, 4294967295),
            unsigned(FixedScalar::U32, 1),
            "__moth_fast_int_wrap((4294967295 + 1), 32, false)",
            "0",
        ),
    ];
    for (operator, scalar, left, right, expression, expected) in number_cases {
        let entry = lower_fast_binary_entry(operator, scalar, left, right);
        assert!(
            entry.source.contains(expression),
            "fast {scalar:?} {operator:?} must lower to `{expression}`:\n{}",
            entry.source
        );
        assert!(
            !entry.source.contains("__moth_numeric_trap("),
            "a total fast operation must not consume the checked trap carrier"
        );
        assert_eq!(
            run_fast_entry(&entry).trim(),
            expected,
            "fast {scalar:?} {operator:?} on the fixture operands must wrap"
        );
    }

    let bigint_cases = [
        (
            NumericOperator::Add,
            FixedScalar::I64,
            signed(FixedScalar::I64, i64::MAX),
            signed(FixedScalar::I64, 1),
            "BigInt.asIntN(64, (9223372036854775807n + 1n))",
            "-9223372036854775808",
        ),
        (
            NumericOperator::Multiply,
            FixedScalar::U64,
            unsigned(FixedScalar::U64, u64::MAX),
            unsigned(FixedScalar::U64, 2),
            "BigInt.asUintN(64, (18446744073709551615n * 2n))",
            "18446744073709551614",
        ),
        (
            NumericOperator::Subtract,
            FixedScalar::U64,
            unsigned(FixedScalar::U64, 0),
            unsigned(FixedScalar::U64, 1),
            "BigInt.asUintN(64, (0n - 1n))",
            "18446744073709551615",
        ),
    ];
    for (operator, scalar, left, right, expression, expected) in bigint_cases {
        let entry = lower_fast_binary_entry(operator, scalar, left, right);
        assert!(
            entry.source.contains(expression),
            "fast {scalar:?} {operator:?} must lower to `{expression}`:\n{}",
            entry.source
        );
        assert_eq!(run_fast_entry(&entry).trim(), expected);
    }
}

#[test]
fn fast_integer_negation_wraps_instead_of_trapping_at_the_signed_minimum() {
    let cases = [
        (
            FixedScalar::I8,
            signed(FixedScalar::I8, -128),
            "__moth_fast_int_wrap(-(-128), 8, true)",
            "-128",
        ),
        (
            FixedScalar::I32,
            signed(FixedScalar::I32, i64::from(i32::MIN)),
            "__moth_fast_int_wrap(-(-2147483648), 32, true)",
            "-2147483648",
        ),
        (
            FixedScalar::I64,
            signed(FixedScalar::I64, i64::MIN),
            "BigInt.asIntN(64, -(-9223372036854775808n))",
            "-9223372036854775808",
        ),
    ];
    for (scalar, operand, expression, expected) in cases {
        let entry = lower_fast_unary_entry(NumericOperator::Negate, scalar, operand);
        assert!(
            entry.source.contains(expression),
            "fast {scalar:?} negation must lower to `{expression}`:\n{}",
            entry.source
        );
        assert!(!entry.source.contains("__moth_numeric_trap("));
        assert_eq!(run_fast_entry(&entry).trim(), expected);
    }
}

/// The Number-carried width normaliser must reduce every fast integer width to its semantic
/// signedness and keep exact low product bits.
#[test]
fn fast_number_integer_wrap_helper_covers_width_boundaries() {
    let mut source = lower_fast_binary(
        NumericOperator::Add,
        FixedScalar::I32,
        signed(FixedScalar::I32, 0),
        signed(FixedScalar::I32, 1),
    );
    source.push_str(
        r#"
const wrap = (value, bits, signed) => __moth_fast_int_wrap(value, bits, signed);
console.log(JSON.stringify([
    wrap(127 + 1, 8, true), wrap(-128 - 1, 8, true),
    wrap(Math.imul(127, 2), 8, true), wrap(Math.imul(-128, -128), 8, true),
    wrap(255 + 1, 8, false), wrap(0 - 1, 8, false), wrap(Math.imul(255, 255), 8, false),
    wrap(32767 + 1, 16, true), wrap(-32768 - 1, 16, true),
    wrap(Math.imul(32767, 2), 16, true),
    wrap(65535 + 1, 16, false), wrap(0 - 1, 16, false),
    wrap(Math.imul(65535, 65535), 16, false),
    wrap(2147483647 + 1, 32, true), wrap(-2147483648 - 1, 32, true),
    wrap(Math.imul(2147483647, 2), 32, true),
    wrap(4294967295 + 1, 32, false), wrap(0 - 1, 32, false),
    wrap(Math.imul(4294967295, 4294967295), 32, false),
    wrap(Math.imul(65536, 65536), 32, false), wrap(-0, 32, false)
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        "[-128,127,-2,0,0,255,1,-32768,32767,-2,0,65535,1,-2147483648,2147483647,-2,0,4294967295,1,0,0]",
        "the fast Number width normaliser must wrap every semantic width"
    );
}

/// Number-carried fast division must truncate toward zero and trap only a zero divisor or the
/// signed minimum divided by -1.
#[test]
fn fast_number_integer_division_helper_truncates_and_traps() {
    let mut source = lower_fast_binary(
        NumericOperator::IntegerDivide,
        FixedScalar::I32,
        signed(FixedScalar::I32, 7),
        signed(FixedScalar::I32, 2),
    );
    source.push_str(CAPTURE_SCALAR);
    source.push_str(
        r#"
console.log(JSON.stringify([
    capture(() => __moth_fast_int_div(7, 2, 32, true, -2147483648)),
    capture(() => __moth_fast_int_div(-7, 2, 32, true, -2147483648)),
    capture(() => __moth_fast_int_div(7, -2, 32, true, -2147483648)),
    capture(() => __moth_fast_int_div(4294967295, 3, 32, false, 0)),
    capture(() => __moth_fast_int_div(-2147483648, -1, 32, true, -2147483648)),
    capture(() => __moth_fast_int_div(-128, -1, 8, true, -128)),
    capture(() => __moth_fast_int_div(-128, 2, 8, true, -128)),
    capture(() => __moth_fast_int_div(255, 2, 8, false, 0)),
    capture(() => __moth_fast_int_div(5, 0, 32, true, -2147483648))
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        r#"["3","-3","-3","1431655765","trap:Integer operation overflowed","trap:Integer operation overflowed","-64","127","trap:Division by zero"]"#,
        "fast Number division must truncate and trap at the domain minimum"
    );
}

/// Number-carried fast remainder must keep the dividend sign and trap only a zero divisor.
#[test]
fn fast_number_integer_remainder_helper_keeps_dividend_sign() {
    let mut source = lower_fast_binary(
        NumericOperator::Remainder,
        FixedScalar::I32,
        signed(FixedScalar::I32, 7),
        signed(FixedScalar::I32, 3),
    );
    source.push_str(CAPTURE_SCALAR);
    source.push_str(
        r#"
console.log(JSON.stringify([
    capture(() => __moth_fast_int_mod(7, 3, 32, true)),
    capture(() => __moth_fast_int_mod(-7, 3, 32, true)),
    capture(() => __moth_fast_int_mod(7, -3, 32, true)),
    capture(() => __moth_fast_int_mod(-2147483648, -1, 32, true)),
    capture(() => __moth_fast_int_mod(-128, -1, 8, true)),
    capture(() => __moth_fast_int_mod(-128, 3, 8, true)),
    capture(() => __moth_fast_int_mod(5, 0, 32, true))
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        r#"["1","-1","1","0","0","-2","trap:Division by zero"]"#,
        "fast Number remainder must keep the dividend sign and canonical minimum remainder"
    );
}

/// Number-carried fast power must stay modular at every width and trap a negative exponent.
#[test]
fn fast_number_integer_power_helper_stays_modular() {
    let mut source = lower_fast_binary(
        NumericOperator::Power,
        FixedScalar::I32,
        signed(FixedScalar::I32, 2),
        signed(FixedScalar::I32, 31),
    );
    source.push_str(CAPTURE_SCALAR);
    source.push_str(
        r#"
console.log(JSON.stringify([
    capture(() => __moth_fast_int_pow(3, 5, 32, true)),
    capture(() => __moth_fast_int_pow(-2, 3, 32, true)),
    capture(() => __moth_fast_int_pow(2, 31, 32, true)),
    capture(() => __moth_fast_int_pow(0, 0, 32, true)),
    capture(() => __moth_fast_int_pow(2, -1, 32, true)),
    capture(() => __moth_fast_int_pow(2, 7, 8, true)),
    capture(() => __moth_fast_int_pow(3, 5, 8, false)),
    capture(() => __moth_fast_int_pow(2, 32, 32, false)),
    capture(() => __moth_fast_int_pow(4294967295, 2, 32, false)),
    capture(() => __moth_fast_int_pow(2, 15, 16, true)),
    capture(() => __moth_fast_int_pow(3, 5, 16, false))
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        r#"["243","-8","-2147483648","1","trap:Invalid exponent","-128","243","0","1","-32768","243"]"#,
        "fast Number power must wrap at every width and trap a negative exponent"
    );
}

/// BigInt division must stay exact at 64 bits and trap only its fast causes.
#[test]
fn fast_bigint_division_helper_uses_64_bit_modular_quotients() {
    let mut source = lower_fast_binary(
        NumericOperator::IntegerDivide,
        FixedScalar::I64,
        signed(FixedScalar::I64, 1),
        signed(FixedScalar::I64, 1),
    );
    assert!(source.contains("__moth_fast_bigint_div(1n, 1n, -9223372036854775808n)"));
    source.push_str(CAPTURE_SCALAR);
    source.push_str(
        r#"
const min = -9223372036854775808n;
console.log(JSON.stringify([
    capture(() => __moth_fast_bigint_div(7n, 2n, min)),
    capture(() => __moth_fast_bigint_div(-7n, 2n, min)),
    capture(() => __moth_fast_bigint_div(7n, -2n, min)),
    capture(() => __moth_fast_bigint_div(4294967295n, 3n, 0n)),
    capture(() => __moth_fast_bigint_div(min, -1n, min)),
    capture(() => __moth_fast_bigint_div(5n, 0n, min))
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        r#"["3n","-3n","-3n","1431655765n","trap:Integer operation overflowed","trap:Division by zero"]"#,
        "fast BigInt division must truncate and trap at the 64-bit minimum"
    );
}

/// BigInt remainder must stay exact at 64 bits and trap only a zero divisor.
#[test]
fn fast_bigint_remainder_helper_keeps_dividend_sign() {
    let mut source = lower_fast_binary(
        NumericOperator::Remainder,
        FixedScalar::I64,
        signed(FixedScalar::I64, 7),
        signed(FixedScalar::I64, 3),
    );
    source.push_str(CAPTURE_SCALAR);
    source.push_str(
        r#"
const min = -9223372036854775808n;
console.log(JSON.stringify([
    capture(() => __moth_fast_bigint_mod(7n, 3n)),
    capture(() => __moth_fast_bigint_mod(-7n, 3n)),
    capture(() => __moth_fast_bigint_mod(7n, -3n)),
    capture(() => __moth_fast_bigint_mod(min, -1n)),
    capture(() => __moth_fast_bigint_mod(5n, 0n))
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        r#"["1n","-1n","1n","0n","trap:Division by zero"]"#,
        "fast BigInt remainder must keep the dividend sign and canonical minimum remainder"
    );
}

/// BigInt power must wrap each accumulator step to the 64-bit signed or unsigned width.
#[test]
fn fast_bigint_power_helper_wraps_each_accumulator_step() {
    let mut source = lower_fast_binary(
        NumericOperator::Power,
        FixedScalar::I64,
        signed(FixedScalar::I64, 2),
        signed(FixedScalar::I64, 63),
    );
    source.push_str(CAPTURE_SCALAR);
    source.push_str(
        r#"
console.log(JSON.stringify([
    capture(() => __moth_fast_bigint_pow(3n, 5n, true)),
    capture(() => __moth_fast_bigint_pow(-2n, 3n, true)),
    capture(() => __moth_fast_bigint_pow(2n, 63n, true)),
    capture(() => __moth_fast_bigint_pow(2n, 64n, true)),
    capture(() => __moth_fast_bigint_pow(0n, 0n, true)),
    capture(() => __moth_fast_bigint_pow(2n, -1n, true)),
    capture(() => __moth_fast_bigint_pow(2n, 64n, false)),
    capture(() => __moth_fast_bigint_pow(18446744073709551615n, 2n, false))
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        r#"["243n","-8n","-9223372036854775808n","0n","1n","trap:Invalid exponent","0n","1n"]"#,
        "fast BigInt power must wrap at the 64-bit signed or unsigned width"
    );
}

/// F64 fast helpers must trap non-finite results and signed-zero divisors while preserving
/// signed zero for total operations.
#[test]
fn fast_f64_helpers_trap_non_finite_and_signed_zero_divisors() {
    let mut source = lower_fast_binary(
        NumericOperator::Power,
        FixedScalar::F64,
        float(FixedScalar::F64, 2.0),
        float(FixedScalar::F64, 2.0),
    );
    source.push_str(
        r#"
const capture = operation => {
    try {
        const value = operation();
        return (Object.is(value, -0) ? "-0" : String(value));
    } catch (error) {
        return "trap:" + error.message;
    }
};
console.log(JSON.stringify([
    capture(() => __moth_fast_float_add(1, 2)),
    capture(() => __moth_fast_float_sub(1, 2)),
    capture(() => __moth_fast_float_mul(1.5, 2)),
    capture(() => __moth_fast_float_mul(-1, 0)),
    capture(() => __moth_fast_float_add(-0, -0)),
    capture(() => __moth_fast_float_div(1, 2)),
    capture(() => __moth_fast_float_div(1, 0)),
    capture(() => __moth_fast_float_div(1, -0)),
    capture(() => __moth_fast_float_mod(5, 2)),
    capture(() => __moth_fast_float_mod(-0, 3)),
    capture(() => __moth_fast_float_mod(1, 0)),
    capture(() => __moth_fast_float_pow(2, 10)),
    capture(() => __moth_fast_float_pow(-1, 0.5)),
    capture(() => __moth_fast_float_mul(1e308, 1e308)),
    capture(() => __moth_fast_float_mul(5e-324, 1))
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        r#"["3","-1","3","-0","-0","0.5","trap:Division by zero","trap:Division by zero","1","-0","trap:Division by zero","1024","trap:Floating-point operation produced a non-finite value","trap:Floating-point operation produced a non-finite value","5e-324"]"#,
        "fast F64 operations must preserve signed zero and trap on non-finite results"
    );
}

/// F32 fast helpers must round after every operation at binary32.
#[test]
fn fast_f32_helpers_round_each_operation_and_keep_subnormals() {
    let mut source = lower_fast_binary(
        NumericOperator::Power,
        FixedScalar::F32,
        float(FixedScalar::F32, 2.0),
        float(FixedScalar::F32, 2.0),
    );
    source.push_str(
        r#"
const capture = operation => {
    try {
        const value = operation();
        return (Object.is(value, -0) ? "-0" : String(value));
    } catch (error) {
        return "trap:" + error.message;
    }
};
console.log(JSON.stringify([
    capture(() => __moth_fast_float32_add(16777216, 1)),
    capture(() => __moth_fast_float32_add(1, 5.960464477539063e-8)),
    capture(() => __moth_fast_float32_mul(-1, 0)),
    capture(() => __moth_fast_float32_div(1, 0)),
    capture(() => __moth_fast_float32_div(1.1754943508222875e-38, 2)),
    capture(() => __moth_fast_float32_mul(3.4028234663852886e38, 2)),
    capture(() => __moth_fast_float32_mod(-0, 3)),
    capture(() => __moth_fast_float32_mod(1, -0)),
    capture(() => __moth_fast_float32_pow(2, 0.5))
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        r#"["16777216","1","-0","trap:Division by zero","5.877471754111438e-39","trap:Floating-point operation produced a non-finite value","-0","trap:Division by zero","1.4142135381698608"]"#,
        "fast F32 operations must complete at binary32 and keep subnormals"
    );
}

/// F16 fast helpers must use binary16 rounding at every operation and trap overflow.
#[test]
fn fast_f16_helpers_round_each_operation_and_trap_overflow() {
    let mut source = lower_fast_binary(
        NumericOperator::Power,
        FixedScalar::F16,
        float(FixedScalar::F16, 2.0),
        float(FixedScalar::F16, 2.0),
    );
    source.push_str(
        r#"
const capture = operation => {
    try {
        const value = operation();
        return (Object.is(value, -0) ? "-0" : String(value));
    } catch (error) {
        return "trap:" + error.message;
    }
};
console.log(JSON.stringify([
    capture(() => __moth_fast_float16_add(1, 0.00048828125)),
    capture(() => __moth_fast_float16_add(65504, 1)),
    capture(() => __moth_fast_float16_mul(-1, 0)),
    capture(() => __moth_fast_float16_div(-5.960464477539063e-8, 2)),
    capture(() => __moth_fast_float16_div(1, 0)),
    capture(() => __moth_fast_float16_mul(60000, 60000)),
    capture(() => __moth_fast_float16_mod(-0, 3)),
    capture(() => __moth_fast_float16_mod(1, -0)),
    capture(() => __moth_fast_float16_pow(2, 0.5))
]));
"#,
    );
    assert_eq!(
        run_javascript(&source).trim(),
        r#"["1","65504","-0","-0","trap:Division by zero","trap:Floating-point operation produced a non-finite value","-0","trap:Division by zero","1.4140625"]"#,
        "fast F16 operations must complete at binary16"
    );
}

/// Fast binary-float negation is total and rounds inline without a trap helper.
#[test]
fn fast_float_negation_is_total_and_preserves_signed_zero() {
    let entry = lower_fast_unary_entry(
        NumericOperator::Negate,
        FixedScalar::F32,
        float(FixedScalar::F32, -0.0),
    );
    assert!(
        entry.source.contains("Math.fround(-(-0))"),
        "fast F32 negation must round inline at binary32:\n{}",
        entry.source
    );
    assert!(!entry.source.contains("__moth_numeric_trap("));
    let mut script = entry.source;
    script.push_str(&format!(
        "\nconst value = {}();\nconsole.log(String(value) + \":\" + Object.is(value, -0));",
        entry.function_name
    ));
    assert_eq!(run_javascript(&script).trim(), "0:false");

    let entry = lower_fast_unary_entry(
        NumericOperator::Negate,
        FixedScalar::F64,
        float(FixedScalar::F64, 1.5),
    );
    assert!(entry.source.contains("(-(1.5))"));
    assert_eq!(run_fast_entry(&entry).trim(), "-1.5");
}

/// Ordinary arithmetic delivered by a fatal statement keeps the checked helper and the
/// carrier-consuming trap transport. Fatal delivery alone must never select fast wrapping.
#[test]
fn ordinary_fatal_arithmetic_keeps_the_checked_trap_path() {
    let ty = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let left = fixed_scalar_expression(
        signed(FixedScalar::I32, i64::from(i32::MAX)),
        ty,
        region,
        &mut expressions,
    );
    let right = fixed_scalar_expression(signed(FixedScalar::I32, 1), ty, region, &mut expressions);
    let entry = lower_fast_entry(
        HirNumericOp::ordinary(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::I32)),
        NumericFailureMode::Fatal,
        HirNumericOperands::Binary { left, right },
        ty,
        expressions,
    );
    assert!(
        entry.source.contains(
            "__moth_numeric_trap(__moth_int_add(2147483647, 1, -2147483648, 2147483647))"
        ),
        "an ordinary fatal operation must keep the checked trapping path:\n{}",
        entry.source
    );
    assert_eq!(
        run_fast_entry(&entry).trim(),
        "trap:Integer operation overflowed"
    );
}

/// Fast helpers are demanded only by fast operations, and Dec values never demand them.
#[test]
fn fast_helpers_are_absent_from_ordinary_and_dec_modules() {
    let ordinary = {
        let ty = builtin_type_ids::fixed_scalar(FixedScalar::I32);
        let mut expressions = HirExpressionStore::default();
        let region = RegionId(0);
        let left =
            fixed_scalar_expression(signed(FixedScalar::I32, 1), ty, region, &mut expressions);
        let right =
            fixed_scalar_expression(signed(FixedScalar::I32, 2), ty, region, &mut expressions);
        lower_minimal_module_with_numeric_op(
            HirNumericOp::ordinary(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::I32)),
            NumericFailureMode::Trap,
            HirNumericOperands::Binary { left, right },
            ty,
            expressions,
        )
    };
    assert!(!ordinary.contains("function __moth_fast_"));
    assert!(ordinary.contains("__moth_int_add("));

    let dec = lower_dec_multiplication();
    assert!(dec.contains("function __moth_number_round_half_even("));
    assert!(
        dec.contains("__moth_number_round_half_even((10n * 10n), 10n)"),
        "a total Dec multiply must lower to the shared half-even kernel"
    );
    for absent in [
        "function __moth_number_add(",
        "function __moth_number_sub(",
        "function __moth_number_mul(",
        "function __moth_number_neg(",
        "function __moth_number_div(",
        "function __moth_number_idiv(",
        "function __moth_number_mod(",
        "function __moth_number_pow(",
        "function __moth_numeric_trap(",
    ] {
        assert!(
            !dec.contains(absent),
            "a total Dec multiply must not emit or reference {absent}"
        );
    }
    assert!(!dec.contains("function __moth_fast_"));
    assert!(!dec.contains("BigInt.asIntN(64"));

    let fast_i32 = lower_fast_binary(
        NumericOperator::Add,
        FixedScalar::I32,
        signed(FixedScalar::I32, 1),
        signed(FixedScalar::I32, 2),
    );
    assert!(fast_i32.contains("function __moth_fast_int_wrap("));
    assert!(!fast_i32.contains("function __moth_fast_int_div("));
    assert!(!fast_i32.contains("function __moth_fast_int_mod("));
    assert!(!fast_i32.contains("function __moth_fast_int_pow("));
    assert!(!fast_i32.contains("function __moth_int_add("));
    assert!(!fast_i32.contains("function __moth_numeric_trap("));

    let fast_i64 = lower_fast_binary(
        NumericOperator::Add,
        FixedScalar::I64,
        signed(FixedScalar::I64, 1),
        signed(FixedScalar::I64, 2),
    );
    assert!(!fast_i64.contains("function __moth_fast_bigint_"));
    assert!(!fast_i64.contains("function __moth_bigint_add("));
    assert!(!fast_i64.contains("function __moth_numeric_trap("));

    let fast_i64_division = lower_fast_binary(
        NumericOperator::IntegerDivide,
        FixedScalar::I64,
        signed(FixedScalar::I64, 1),
        signed(FixedScalar::I64, 1),
    );
    assert!(fast_i64_division.contains("function __moth_fast_bigint_div("));
    assert!(!fast_i64_division.contains("function __moth_fast_bigint_mod("));
    assert!(!fast_i64_division.contains("function __moth_fast_bigint_pow("));
    assert!(!fast_i64_division.contains("function __moth_bigint_div("));
    assert!(!fast_i64_division.contains("function __moth_numeric_trap("));

    let fast_f64 = lower_fast_binary(
        NumericOperator::Add,
        FixedScalar::F64,
        float(FixedScalar::F64, 1.0),
        float(FixedScalar::F64, 2.0),
    );
    assert!(fast_f64.contains("function __moth_fast_float_add("));
    assert!(!fast_f64.contains("function __moth_float_add("));
    assert!(!fast_f64.contains("function __moth_numeric_trap("));

    // A wrap-only fast write-back needs its total helper but never the checked trap carrier.
    let i64_type = builtin_type_ids::fixed_scalar(FixedScalar::I64);
    let wrap_only = lower_minimal_module_with_fast_store_conversion(
        HirFastStoreConversion {
            source: NumericScalar::Fixed(FixedScalar::I64),
            target: NumericScalar::Fixed(FixedScalar::U32),
        },
        |_, region, expressions| {
            fixed_scalar_expression(signed(FixedScalar::I64, 1), i64_type, region, expressions)
        },
    );
    assert!(wrap_only.contains("function __moth_fast_wrap_integer("));
    assert!(!wrap_only.contains("function __moth_numeric_trap("));
    assert!(!wrap_only.contains("function __moth_int_"));
}

/// A binary-float family emits its `pow` member and the shared power helper only together.
///
/// WHY: the helper is large, so a page without a selected Power op must carry neither the
///      member nor the helper and must reference no undefined function. The pow pages run both
///      members to prove the references resolve at runtime.
#[test]
fn power_demand_emits_the_float_pow_member_and_shared_helper() {
    let fast_add = lower_fast_binary(
        NumericOperator::Add,
        FixedScalar::F64,
        float(FixedScalar::F64, 1.0),
        float(FixedScalar::F64, 2.0),
    );
    assert!(!fast_add.contains("__moth_float_power"));
    assert!(!fast_add.contains("function __moth_fast_float_pow("));

    let ty = builtin_type_ids::fixed_scalar(FixedScalar::F64);
    let region = RegionId(0);
    let mut add_expressions = HirExpressionStore::default();
    let left = fixed_scalar_expression(
        float(FixedScalar::F64, 1.0),
        ty,
        region,
        &mut add_expressions,
    );
    let right = fixed_scalar_expression(
        float(FixedScalar::F64, 2.0),
        ty,
        region,
        &mut add_expressions,
    );
    let checked_add = lower_minimal_module_with_numeric_op(
        HirNumericOp::ordinary(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::F64)),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary { left, right },
        ty,
        add_expressions,
    );
    assert!(!checked_add.contains("__moth_float_power"));
    assert!(!checked_add.contains("function __moth_float_pow("));

    let mut fast_pow = lower_fast_binary(
        NumericOperator::Power,
        FixedScalar::F64,
        float(FixedScalar::F64, 2.0),
        float(FixedScalar::F64, 2.0),
    );
    assert!(fast_pow.contains("function __moth_float_power("));
    assert!(fast_pow.contains("function __moth_fast_float_pow("));
    fast_pow.push_str("\nconsole.log(String(__moth_fast_float_pow(2, 10)));\n");
    assert_eq!(run_javascript(&fast_pow).trim(), "1024");

    let mut pow_expressions = HirExpressionStore::default();
    let left = fixed_scalar_expression(
        float(FixedScalar::F64, 2.0),
        ty,
        region,
        &mut pow_expressions,
    );
    let right = fixed_scalar_expression(
        float(FixedScalar::F64, 10.0),
        ty,
        region,
        &mut pow_expressions,
    );
    let checked_pow = lower_minimal_module_with_numeric_op(
        HirNumericOp::ordinary(
            NumericOperator::Power,
            NumericScalar::Fixed(FixedScalar::F64),
        ),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary { left, right },
        ty,
        pow_expressions,
    );
    assert!(checked_pow.contains("function __moth_float_power("));
    assert!(checked_pow.contains("function __moth_float_pow("));
    let mut checked_source = checked_pow;
    checked_source.push_str("\nconsole.log(JSON.stringify(__moth_float_pow(2, 10)));\n");
    assert_eq!(
        run_javascript(&checked_source).trim(),
        r#"{"tag":"ok","value":1024}"#
    );
}

fn lower_dec_multiplication() -> String {
    let scale = NumberScale::new(1).expect("scale one is valid");
    let mut expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, _) = build_type_environment();
    let number_type = type_environment.intern_number(scale);
    let region = RegionId(0);
    let value = NumberValue::from_integer(1, scale);
    let left = expression(
        HirExpressionKind::Number(value.clone()),
        number_type,
        region,
        ValueKind::Const,
        &mut expressions,
    );
    let right = expression(
        HirExpressionKind::Number(value),
        number_type,
        region,
        ValueKind::Const,
        &mut expressions,
    );
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, number_type, region)],
        statements: vec![statement(
            1,
            HirStatementKind::NumericOp {
                op: HirNumericOp::ordinary(NumericOperator::Multiply, NumericScalar::Number(scale)),
                failure_mode: NumericFailureMode::Trap,
                operands: HirNumericOperands::Binary { left, right },
                result: HirLocalDestination::Define(LocalId(0)),
            },
        )],
        terminator: HirTerminator::Return(HirValueRange::empty()),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            vec![],
            None,
        )),
    };
    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );
    lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("Dec HIR should lower to JavaScript")
    .source
}

/// A fast range guard throws its canonical cause directly and never demands the checked carrier
/// trap helper.
#[test]
fn fast_loop_step_failure_throws_without_the_checked_trap_helper() {
    let expressions = HirExpressionStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.boolean, region)],
        statements: vec![statement(
            1,
            HirStatementKind::LoopStepFailure {
                cause: LoopStepFailureCause::ZeroStep,
                semantics: NumericSemantics::Fast,
                result: HirLocalDestination::Define(LocalId(0)),
            },
        )],
        terminator: HirTerminator::Return(HirValueRange::empty()),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            [],
            None,
        )),
    };
    let module = build_module(
        expressions,
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );
    let source = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("fast loop-step failure must lower")
    .source;
    assert!(
        source.contains("throw new Error(\"Loop step cannot be zero\");"),
        "a fast range guard must throw its canonical cause directly:\n{source}"
    );
    assert!(!source.contains("function __moth_numeric_trap("));
    assert!(!source.contains("__moth_error_result(\"Loop step cannot be zero\""));
}
