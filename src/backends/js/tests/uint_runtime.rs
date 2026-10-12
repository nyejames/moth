//! Executable behavior tests for unsigned exact-width JavaScript runtime helpers.
//!
//! WHAT: executes the shared integer helper families with `U32`/`U64` bounds, the
//!       direct `BigInt` to binary-float rounding and the integer cast helpers
//!       in Node.js using adversarial boundary values.
//! WHY: `U32` rides the exact `Number` family and `U64` the exact `BigInt`
//!      family with their own unsigned bounds; only execution proves values
//!      above the signed maxima stay exact and out-of-range results fail.

use super::support::*;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::ids::RegionId;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::process::Command;

fn run_javascript(source: &str) -> String {
    let output = Command::new("node")
        .args(["--eval", source])
        .output()
        .expect("Node.js is required for JavaScript runtime behavior tests");
    assert!(
        output.status.success(),
        "Node.js runtime failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Node.js output is UTF-8")
}

fn unsigned_literal(scalar: FixedScalar, value: u64) -> FixedScalarValue {
    FixedScalarValue::unsigned(scalar, value).expect("fixture value fits its unsigned scalar")
}

/// Lowers one exact-width unsigned addition so the prelude carries the integer
/// helper family for that scalar's carrier with its unsigned bounds.
fn lower_uint_operation_source(scalar: FixedScalar) -> String {
    let mut expressions = HirExpressionStore::default();
    let uint_type: TypeId = builtin_type_ids::fixed_scalar(scalar);
    let region = RegionId(0);
    lower_minimal_module_with_numeric_op(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(scalar),
        },
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: fixed_scalar_expression(
                unsigned_literal(scalar, 1),
                uint_type,
                region,
                &mut expressions,
            ),
            right: fixed_scalar_expression(
                unsigned_literal(scalar, 2),
                uint_type,
                region,
                &mut expressions,
            ),
        },
        uint_type,
        expressions,
    )
}

#[test]
fn uint32_operations_use_shared_int_helpers_with_unsigned_bounds() {
    let source = lower_uint_operation_source(FixedScalar::U32);

    assert!(
        source.contains("__moth_int_add(1, 2, 0, 4294967295)"),
        "U32 addition must demand the shared int helper with unsigned bounds"
    );
    assert!(
        !source.contains("__moth_bigint_add"),
        "U32 programs must not gain BigInt helpers or global state"
    );
    assert!(
        !source.contains("__moth_bigint_to_binary_float"),
        "U32 arithmetic must not demand binary-float conversion helpers"
    );

    let mut script = source;
    script.push_str(
        "\nconst summarize = result => result.tag + \":\" + String(result.value);\n\
         const fail = result => result.tag + \":\" + __moth_error_code(result.value);\n",
    );
    script.push_str(
        "console.log(JSON.stringify([\
            summarize(__moth_int_add(2147483648, 1, 0, 4294967295)),\
            fail(__moth_int_add(4294967295, 1, 0, 4294967295)),\
            fail(__moth_int_sub(0, 1, 0, 4294967295)),\
            summarize(__moth_int_mul(65535, 65537, 0, 4294967295)),\
            fail(__moth_int_mul(100000, 100000, 0, 4294967295)),\
            summarize(__moth_int_div(7, 2, 0, 4294967295)),\
            fail(__moth_int_div(7, 0, 0, 4294967295)),\
            fail(__moth_int_mod(7, 0, 0, 4294967295)),\
            fail(__moth_int_pow(2, 32, 0, 4294967295)),\
            summarize(__moth_int_pow(0, 0, 0, 4294967295)),\
            summarize(__moth_int_mul(0, 5, 0, 4294967295))\
        ]));",
    );

    let overflow = BuiltinErrorCode::IntOverflow.as_u32();
    let divide_by_zero = BuiltinErrorCode::DivideByZero.as_u32();
    assert_eq!(
        run_javascript(&script).trim(),
        format!(
            "[\"ok:2147483649\",\"err:{overflow}\",\"err:{overflow}\",\"ok:4294967295\",\
             \"err:{overflow}\",\"ok:3\",\"err:{divide_by_zero}\",\"err:{divide_by_zero}\",\
             \"err:{overflow}\",\"ok:1\",\"ok:0\"]"
        )
    );
}

#[test]
fn uint64_operations_use_shared_bigint_helpers_with_unsigned_bounds() {
    let source = lower_uint_operation_source(FixedScalar::U64);

    assert!(
        source.contains("__moth_bigint_add(1n, 2n, 0n, 18446744073709551615n)"),
        "U64 addition must demand the shared bigint helper with unsigned bounds"
    );

    let mut script = source;
    script.push_str(
        "\nconst summarize = result => result.tag + \":\" + String(result.value);\n\
         const fail = result => result.tag + \":\" + __moth_error_code(result.value);\n",
    );
    script.push_str(
        "console.log(JSON.stringify([\
            summarize(__moth_bigint_add(9223372036854775808n, 100n, 0n, 18446744073709551615n)),\
            fail(__moth_bigint_add(18446744073709551615n, 1n, 0n, 18446744073709551615n)),\
            fail(__moth_bigint_sub(0n, 1n, 0n, 18446744073709551615n)),\
            summarize(__moth_bigint_mul(1000000000n, 18446744073n, 0n, 18446744073709551615n)),\
            fail(__moth_bigint_mul(4294967296n, 4294967296n, 0n, 18446744073709551615n)),\
            summarize(__moth_bigint_div(18446744073709551615n, 9223372036854775808n, 0n, 18446744073709551615n)),\
            fail(__moth_bigint_div(7n, 0n, 0n, 18446744073709551615n)),\
            summarize(__moth_bigint_pow(10n, 19n, 0n, 18446744073709551615n)),\
            fail(__moth_bigint_pow(2n, 64n, 0n, 18446744073709551615n))\
        ]));",
    );

    let overflow = BuiltinErrorCode::IntOverflow.as_u32();
    let divide_by_zero = BuiltinErrorCode::DivideByZero.as_u32();
    assert_eq!(
        run_javascript(&script).trim(),
        format!(
            "[\"ok:9223372036854775908\",\"err:{overflow}\",\"err:{overflow}\",\
             \"ok:18446744073000000000\",\"err:{overflow}\",\"ok:1\",\"err:{divide_by_zero}\",\
             \"ok:10000000000000000000\",\"err:{overflow}\"]"
        )
    );
}

#[test]
fn uint64_to_float32_rounds_directly_without_binary64_detour() {
    let mut script = emit_cast_helpers(&[BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Fixed(FixedScalar::U64),
        target: NumericScalar::Fixed(FixedScalar::F32),
    }]);
    script.push_str(
        "\nconst direct = __moth_bigint_to_binary_float(9007199791611905n, 32);\n\
         const bits = new Uint32Array(new Float32Array([direct]).buffer)[0];\n\
         console.log(JSON.stringify([direct === 9007200328482816, bits === 0x5a000001]));",
    );

    // The forbidden binary64 detour `Math.fround(Number(9007199791611905n))`
    // gives 9007199254740992 (bits 0x5a000000); the helper must not match it.
    assert_eq!(run_javascript(&script).trim(), "[true,true]");
}

#[test]
fn uint_integer_casts_check_complete_ranges_across_carriers() {
    let mut script = emit_cast_helpers(&[
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::U64),
            target: NumericScalar::Fixed(FixedScalar::I64),
        },
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::I64),
            target: NumericScalar::Fixed(FixedScalar::U64),
        },
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::F64),
            target: NumericScalar::Fixed(FixedScalar::U64),
        },
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::U64)),
        BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::U64)),
    ]);
    script.push_str(
        "\nconst summarize = result => result.tag + \":\" + String(result.value);\n\
         const fail = result => result.tag + \":\" + __moth_error_code(result.value);\n",
    );
    script.push_str(
        "console.log(JSON.stringify([\
            summarize(__moth_cast_integer_to_integer(100n, -9223372036854775808n, 9223372036854775807n, \"U64\", \"I64\")),\
            fail(__moth_cast_integer_to_integer(18446744073709551615n, -9223372036854775808n, 9223372036854775807n, \"U64\", \"I64\")),\
            summarize(__moth_cast_integer_to_integer(41, 0n, 18446744073709551615n, \"I64\", \"U64\")),\
            fail(__moth_cast_integer_to_integer(-1, 0n, 18446744073709551615n, \"I64\", \"U64\")),\
            summarize(__moth_cast_float_to_int(-0.5, 0n, 18446744073709551615n, \"F64\", \"U64\")),\
            fail(__moth_cast_float_to_int(-1.0, 0n, 18446744073709551615n, \"F64\", \"U64\")),\
            summarize(__moth_cast_int(\"18446744073709551615\", 0n, 18446744073709551615n, \"U64\")),\
            fail(__moth_cast_int(\"-0\", 0n, 18446744073709551615n, \"U64\")),\
            fail(__moth_cast_int(\"4a\", 0n, 18446744073709551615n, \"U64\")),\
            __moth_cast_int_to_string(18446744073709551615n)\
        ]));",
    );

    let out_of_range = BuiltinErrorCode::IntCastOutOfRange.as_u32();
    let float_out_of_range = BuiltinErrorCode::FloatCastToIntOutOfRange.as_u32();
    let parse_out_of_range = BuiltinErrorCode::IntParseOutOfRange.as_u32();
    let invalid_format = BuiltinErrorCode::IntParseInvalidFormat.as_u32();
    assert_eq!(
        run_javascript(&script).trim(),
        format!(
            "[\"ok:100\",\"err:{out_of_range}\",\"ok:41\",\"err:{out_of_range}\",\
             \"ok:0\",\"err:{float_out_of_range}\",\"ok:18446744073709551615\",\
             \"err:{parse_out_of_range}\",\"err:{invalid_format}\",\"18446744073709551615\"]"
        )
    );
}
