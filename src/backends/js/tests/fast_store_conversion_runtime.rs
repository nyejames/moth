//! Executable behavior tests for the JavaScript fast compound write-back helpers.
//!
//! WHAT: lowers one reachable fast store conversion so the prelude carries its helper, then runs
//!       that helper directly under Node with adversarial boundary values.
//! WHY: the fast write-back owns wrap/truncation/rounding semantics and its own fatal throws.
//!       Only execution proves the destination low bits, the truncation direction, the exact
//!       bounds and that non-finite values never reach the store; source inspection alone cannot.

use super::support::*;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::ids::RegionId;
use crate::compiler_frontend::hir::numeric::{
    HirFastStoreConversion, HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// Lowers one fast conversion whose source literal is `source_value`.
fn lower_conversion(conversion: HirFastStoreConversion, source_value: FixedScalarValue) -> String {
    let source_type: TypeId = builtin_type_ids::fixed_scalar(source_value.scalar());
    assert_eq!(
        conversion.source,
        NumericScalar::Fixed(source_value.scalar()),
        "the fixture literal must inhabit the conversion's source domain"
    );
    lower_minimal_module_with_fast_store_conversion(conversion, move |_, region, expressions| {
        fixed_scalar_expression(source_value, source_type, region, expressions)
    })
}

fn signed(scalar: FixedScalar, value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(scalar, value).expect("fixture value fits its signed scalar")
}

/// Lowers an ordinary checked I32 addition, which demands no fast write-back helper.
fn lower_ordinary_i32_addition() -> String {
    let mut expressions = HirExpressionStore::default();
    let region = RegionId(0);
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    lower_minimal_module_with_numeric_op(
        HirNumericOp::ordinary(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::I32)),
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: i32_expression(1, i32_type, region, &mut expressions),
            right: i32_expression(2, i32_type, region, &mut expressions),
        },
        i32_type,
        expressions,
    )
}

#[test]
fn integer_wrap_keeps_destination_low_bits_and_signedness() {
    let source = lower_conversion(
        HirFastStoreConversion {
            source: NumericScalar::Fixed(FixedScalar::I64),
            target: NumericScalar::Fixed(FixedScalar::U32),
        },
        signed(FixedScalar::I64, 1),
    );
    assert!(
        source.contains("Number(__moth_fast_wrap_integer(1n, 32, false))"),
        "a U32 destination must wrap through the shared helper and return a Number"
    );

    let mut script = source;
    script.push_str("console.log(JSON.stringify([");
    script.push_str(
        "String(__moth_fast_wrap_integer(127, 8, true)),\
         String(__moth_fast_wrap_integer(128, 8, true)),\
         String(__moth_fast_wrap_integer(-129, 8, true)),\
         String(__moth_fast_wrap_integer(-1, 8, false)),\
         String(__moth_fast_wrap_integer(256, 8, false)),\
         String(__moth_fast_wrap_integer(32768, 16, true)),\
         String(__moth_fast_wrap_integer(-32769, 16, true)),\
         String(__moth_fast_wrap_integer(-1, 16, false)),\
         String(__moth_fast_wrap_integer(65536, 16, false)),\
         String(__moth_fast_wrap_integer(2147483648, 32, true)),\
         String(__moth_fast_wrap_integer(-2147483649, 32, true)),\
         String(__moth_fast_wrap_integer(-1, 32, false)),\
         String(__moth_fast_wrap_integer(4294967296, 32, false)),\
         String(__moth_fast_wrap_integer(9223372036854775808n, 64, true)),\
         String(__moth_fast_wrap_integer(-9223372036854775809n, 64, true)),\
         String(__moth_fast_wrap_integer(-1n, 64, false)),\
         String(__moth_fast_wrap_integer(18446744073709551616n, 64, false)),\
         String(__moth_fast_wrap_integer(9007199254740993n, 32, false)),\
         String(__moth_fast_wrap_integer(9007199254740993n, 64, true)),\
         String(Number(__moth_fast_wrap_integer(70000, 16, true)))",
    );
    script.push_str("]));");

    assert_eq!(
        run_javascript(&script).trim(),
        "[\"127\",\"-128\",\"127\",\"255\",\"0\",\"-32768\",\"32767\",\"65535\",\"0\",\
         \"-2147483648\",\"2147483647\",\"4294967295\",\"0\",\"-9223372036854775808\",\
         \"9223372036854775807\",\"18446744073709551615\",\"0\",\"1\",\"9007199254740993\",\"4464\"]",
        "every width must keep only its low bits, in its own signedness"
    );
}

#[test]
fn float_to_integer_truncates_toward_zero_and_traps_out_of_range() {
    let source = lower_conversion(
        HirFastStoreConversion {
            source: NumericScalar::Fixed(FixedScalar::F64),
            target: NumericScalar::Fixed(FixedScalar::I32),
        },
        FixedScalarValue::binary_float(FixedScalar::F64, 1.0).expect("1.0 is an exact F64"),
    );
    assert!(
        source.contains("__moth_fast_float_to_integer(1, -2147483648, 2147483647)"),
        "an I32 destination must pass its exact bounds to the truncating helper"
    );

    let mut script = source;
    script.push_str(
        "const capture = operation => { try { return \"ok:\" + String(operation()); } \
         catch (error) { return \"err:\" + error.message; } };\n",
    );
    script.push_str("console.log(JSON.stringify([");
    script.push_str(
        "capture(() => __moth_fast_float_to_integer(2.9, -128, 127)),\
         capture(() => __moth_fast_float_to_integer(-2.9, -128, 127)),\
         capture(() => __moth_fast_float_to_integer(0.999, -128, 127)),\
         capture(() => Object.is(__moth_fast_float_to_integer(-0.5, -128, 127), 0)),\
         capture(() => __moth_fast_float_to_integer(127.0, -128, 127)),\
         capture(() => __moth_fast_float_to_integer(128, -128, 127)),\
         capture(() => __moth_fast_float_to_integer(-128.9, -128, 127)),\
         capture(() => __moth_fast_float_to_integer(-129.1, -128, 127)),\
         capture(() => __moth_fast_float_to_integer(NaN, -128, 127)),\
         capture(() => __moth_fast_float_to_integer(Infinity, -128, 127)),\
         capture(() => __moth_fast_float_to_integer(-Infinity, -128, 127)),\
         capture(() => __moth_fast_float_to_integer(1e18, -9223372036854775808n, 9223372036854775807n)),\
         capture(() => __moth_fast_float_to_integer(1e19, -9223372036854775808n, 9223372036854775807n)),\
         capture(() => __moth_fast_float_to_integer(18446744073709551615, 0n, 18446744073709551615n))",
    );
    script.push_str("]));");

    let invalid = "Floating-point value cannot be cast to an integer";
    let out_of_range = "Floating-point value is out of the target integer range";
    assert_eq!(
        run_javascript(&script).trim(),
        format!(
            "[\"ok:2\",\"ok:-2\",\"ok:0\",\"ok:true\",\"ok:127\",\"err:{out_of_range}\",\
             \"ok:-128\",\"err:{out_of_range}\",\"err:{invalid}\",\"err:{invalid}\",\
             \"err:{invalid}\",\"ok:1000000000000000000\",\"err:{out_of_range}\",\
             \"err:{out_of_range}\"]"
        ),
        "truncation is toward zero, and the exact destination range decides the throw"
    );
}

#[test]
fn float_conversion_rounds_at_destination_precision_and_traps_non_finite() {
    let source = lower_conversion(
        HirFastStoreConversion {
            source: NumericScalar::Fixed(FixedScalar::F64),
            target: NumericScalar::Fixed(FixedScalar::F32),
        },
        FixedScalarValue::binary_float(FixedScalar::F64, 1.5).expect("1.5 is an exact F64"),
    );
    assert!(
        source.contains("__moth_fast_float_conversion(1.5, 32)"),
        "an F32 destination must round at binary32 precision"
    );

    let mut script = source;
    script.push_str(
        "const capture = operation => { try { return \"ok:\" + String(operation()); } \
         catch (error) { return \"err:\" + error.message; } };\n",
    );
    script.push_str("console.log(JSON.stringify([");
    script.push_str(
        "capture(() => __moth_fast_float_conversion(16777217, 32)),\
         capture(() => __moth_fast_float_conversion(0.1, 64)),\
         capture(() => __moth_fast_float_conversion(1e300, 32)),\
         capture(() => __moth_fast_float_conversion(Infinity, 64)),\
         capture(() => __moth_fast_float_conversion(1.401298464324817e-45, 32) > 0),\
         capture(() => Object.is(__moth_fast_float_conversion(-0, 32), -0)),\
         capture(() => Object.is(__moth_fast_float_conversion(-0, 64), -0))",
    );
    script.push_str("]));");

    let non_finite = "Floating-point conversion produced a non-finite value";
    assert_eq!(
        run_javascript(&script).trim(),
        format!(
            "[\"ok:16777216\",\"ok:0.1\",\"err:{non_finite}\",\"err:{non_finite}\",\
             \"ok:true\",\"ok:true\",\"ok:true\"]"
        ),
        "rounding must stay at the destination precision and preserve signed zero"
    );
}

#[test]
fn integer_to_float_write_back_reuses_the_shared_bigint_rounding_helper() {
    let source = lower_conversion(
        HirFastStoreConversion {
            source: NumericScalar::Fixed(FixedScalar::I64),
            target: NumericScalar::Fixed(FixedScalar::F32),
        },
        signed(FixedScalar::I64, 1),
    );
    assert!(
        source.contains("__moth_fast_float_conversion(__moth_bigint_to_binary_float(1n, 32), 32)"),
        "an exact 64-bit source must round directly to binary32 before the finite check"
    );
    assert_eq!(
        source
            .matches("function __moth_bigint_to_binary_float(")
            .count(),
        1,
        "the shared rounding helper must be emitted exactly once"
    );
}

#[test]
fn ordinary_modules_emit_no_fast_write_back_helpers() {
    for source in [lower_minimal_module("main"), lower_ordinary_i32_addition()] {
        for helper in [
            "__moth_fast_wrap_integer",
            "__moth_fast_float_to_integer",
            "__moth_fast_float_conversion",
        ] {
            assert!(
                !source.contains(helper),
                "an ordinary module must not emit {helper}"
            );
        }
    }
}
