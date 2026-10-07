//! Executable behavior tests for `Uint` JavaScript runtime helpers.
//!
//! WHAT: executes the shared integer helper families with `Uint` bounds, the
//!       direct `BigInt` to binary-float rounding and the integer cast helpers
//!       in Node.js using adversarial boundary values.
//! WHY: `Uint32` rides the exact `Number` family and `Uint64` the exact `BigInt`
//!      family with their own unsigned bounds; only execution proves values
//!      above the signed maxima stay exact and out-of-range results fail.

use super::support::*;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::RegionId;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};
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

fn profile(int_width: IntWidth) -> NumericProfile {
    NumericProfile {
        int_width,
        float_precision: FloatPrecision::Bits64,
    }
}

/// Lowers one `Uint` addition so the prelude carries the profile-selected
/// integer helper family with `Uint` bounds.
fn lower_uint_operation_source(numeric_profile: NumericProfile) -> String {
    let mut expressions = HirExpressionStore::default();
    let (type_environment, _) = build_type_environment();
    let uint_type: TypeId = type_environment.builtins().uint;
    let region = RegionId(0);
    lower_minimal_module_with_numeric_op_for_profile(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Uint,
        },
        NumericFailureMode::Trap,
        HirNumericOperands::Binary {
            left: expression(
                HirExpressionKind::Uint(1),
                uint_type,
                region,
                ValueKind::Const,
                &mut expressions,
            ),
            right: expression(
                HirExpressionKind::Uint(2),
                uint_type,
                region,
                ValueKind::Const,
                &mut expressions,
            ),
        },
        uint_type,
        numeric_profile,
        expressions,
    )
}

fn emit_uint_cast_helpers(
    policies: &[BuiltinCastPolicyId],
    numeric_profile: NumericProfile,
) -> String {
    emit_cast_helpers_for_profile(policies, numeric_profile)
}

#[test]
fn uint32_operations_use_shared_int_helpers_with_unsigned_bounds() {
    let source = lower_uint_operation_source(profile(IntWidth::Bits32));

    assert!(
        source.contains("__moth_int_add(1, 2, 0, 4294967295)"),
        "Uint32 addition must demand the shared int helper with unsigned bounds"
    );
    assert!(
        !source.contains("__moth_bigint_add"),
        "Uint32 programs must not gain BigInt helpers or global state"
    );
    assert!(
        !source.contains("__moth_bigint_to_binary_float"),
        "Uint32 arithmetic must not demand binary-float conversion helpers"
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
    let source = lower_uint_operation_source(profile(IntWidth::Bits64));

    assert!(
        source.contains("__moth_bigint_add(1n, 2n, 0n, 18446744073709551615n)"),
        "Uint64 addition must demand the shared bigint helper with unsigned bounds"
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
    let mut script = emit_uint_cast_helpers(
        &[BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Uint,
            target: NumericScalar::Float,
        }],
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        },
    );
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
    let mut script = emit_uint_cast_helpers(
        &[
            BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Uint,
                target: NumericScalar::Int,
            },
            BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Int,
                target: NumericScalar::Uint,
            },
            BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Float,
                target: NumericScalar::Uint,
            },
            BuiltinCastPolicyId::StringToNumeric(NumericScalar::Uint),
            BuiltinCastPolicyId::NumericToString(NumericScalar::Uint),
        ],
        profile(IntWidth::Bits64),
    );
    script.push_str(
        "\nconst summarize = result => result.tag + \":\" + String(result.value);\n\
         const fail = result => result.tag + \":\" + __moth_error_code(result.value);\n",
    );
    script.push_str(
        "console.log(JSON.stringify([\
            summarize(__moth_cast_integer_to_integer(100n, -9223372036854775808n, 9223372036854775807n, \"Uint\", \"Int\")),\
            fail(__moth_cast_integer_to_integer(18446744073709551615n, -9223372036854775808n, 9223372036854775807n, \"Uint\", \"Int\")),\
            summarize(__moth_cast_integer_to_integer(41, 0n, 18446744073709551615n, \"Int\", \"Uint\")),\
            fail(__moth_cast_integer_to_integer(-1, 0n, 18446744073709551615n, \"Int\", \"Uint\")),\
            summarize(__moth_cast_float_to_int(-0.5, 0n, 18446744073709551615n, \"Float\", \"Uint\")),\
            fail(__moth_cast_float_to_int(-1.0, 0n, 18446744073709551615n, \"Float\", \"Uint\")),\
            summarize(__moth_cast_int(\"18446744073709551615\", 0n, 18446744073709551615n, \"Uint\")),\
            fail(__moth_cast_int(\"-0\", 0n, 18446744073709551615n, \"Uint\")),\
            fail(__moth_cast_int(\"4a\", 0n, 18446744073709551615n, \"Uint\")),\
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
