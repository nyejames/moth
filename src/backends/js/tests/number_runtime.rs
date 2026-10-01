//! Executable behavior tests for exact Dec JavaScript runtime helpers.

use super::support::*;
use crate::backends::js::JsEmitter;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::number::NumberScale;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::module::HirModule;
use std::process::Command;

fn emit_cast_helpers(policies: &[BuiltinCastPolicyId]) -> String {
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let path_table = path_fork.snapshot_table();
    let (type_environment, _) = build_type_environment();
    let hir = HirModule::new();
    let borrow_analysis = BorrowCheckReport::default();
    let numeric_proofs = NumericProofs::default();
    let mut emitter = JsEmitter::new(
        &hir,
        &borrow_analysis,
        &numeric_proofs,
        &string_table,
        &path_table,
        default_config(),
        &type_environment,
    );
    for policy in policies {
        emitter.used_cast_policies.insert(*policy);
    }
    emitter.emit_runtime_error_helpers();
    emitter.emit_runtime_cast_helpers();
    std::mem::take(&mut emitter.out)
}

#[test]
fn number_conversion_helper_artifacts_are_deterministic_for_the_same_demands() {
    let scale_one = NumberScale::new(1).expect("scale one is valid");
    let scale_two = NumberScale::new(2).expect("scale two is valid");
    let scale_three = NumberScale::new(3).expect("scale three is valid");
    let scale_four = NumberScale::new(4).expect("scale four is valid");
    let demands = [
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Number(scale_four),
            target: NumericScalar::Number(scale_two),
        },
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Number(scale_three),
            target: NumericScalar::Number(scale_one),
        },
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Number(scale_two),
            target: NumericScalar::Fixed(FixedScalar::U8),
        },
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Number(scale_one),
            target: NumericScalar::Fixed(FixedScalar::I16),
        },
    ];
    let expected_artifact = emit_cast_helpers(&demands);

    // Each emitter owns a freshly seeded policy HashSet; vary insertion order as well.
    for rotation in 0..16 {
        let mut reordered_demands = demands;
        reordered_demands.rotate_left(rotation % demands.len());
        if rotation % 2 != 0 {
            reordered_demands.reverse();
        }

        assert_eq!(
            emit_cast_helpers(&reordered_demands),
            expected_artifact,
            "helper artifact changed for policy insertion order {rotation}"
        );
    }

    let helper_free_demands = [
        demands[0],
        demands[1],
        demands[2],
        demands[3],
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Number(scale_one),
            target: NumericScalar::Number(scale_four),
        },
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::U8),
            target: NumericScalar::Number(scale_two),
        },
    ];
    assert_eq!(
        emit_cast_helpers(&helper_free_demands),
        expected_artifact,
        "infallible widening demands added runtime helpers"
    );

    let mut script = expected_artifact;
    script.push_str(
        "\nconst narrowed = __moth_cast_number_scale(123400n, 100n, 4, \"Dec4\", \"Dec2\");\n\
         const integer = __moth_cast_number_to_integer(1200n, 100n, 2, 0, 255, \"Dec2\", \"U8\");\n\
         console.log([narrowed.tag, String(narrowed.value), integer.tag, String(integer.value)].join(\":\"));",
    );
    assert_eq!(run_javascript(&script).trim(), "ok:1234:ok:12");
}

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

#[test]
fn number_operation_helpers_round_signed_ties_and_keep_exact_power_intermediates() {
    let scale = NumberScale::new(1).expect("scale one is valid");
    let scale_factor = "10n";
    let mut script = lower_number_operation_source(scale);
    script.push_str("\nconst summarize = result => result.tag + \" :\" + String(result.value);\n");
    script.push_str(&format!(
        "console.log(JSON.stringify([\
            __moth_number_mul(15n, 1n, {scale_factor}),\
            __moth_number_mul(25n, 1n, {scale_factor}),\
            __moth_number_mul(-15n, 1n, {scale_factor}),\
            __moth_number_mul(-25n, 1n, {scale_factor}),\
            __moth_number_div(1n, 4n, {scale_factor}),\
            __moth_number_div(3n, 4n, {scale_factor}),\
            __moth_number_div(-1n, 4n, {scale_factor}),\
            __moth_number_div(3n, -4n, {scale_factor}),\
            __moth_number_pow(15n, 2n, {scale_factor}),\
            __moth_number_pow(-15n, 3n, {scale_factor}),\
            __moth_number_idiv(-7n, 3n),\
            __moth_number_mod(-7n, 3n),\
            __moth_number_add(123456789012345678901234567890n, 1n),\
            __moth_number_neg(123456789012345678901234567890n),\
            __moth_number_pow({scale_factor}, 9223372036854775807n, {scale_factor}),\
            __moth_number_pow(-{scale_factor}, 9223372036854775807n, {scale_factor}),\
            __moth_number_pow(0n, 9223372036854775807n, {scale_factor}),\
            __moth_number_pow(999999999999999999999999999999999999n, 0n, {scale_factor})\
        ].map(summarize)));"
    ));

    assert_eq!(
        run_javascript(&script).trim(),
        r#"["ok :2","ok :2","ok :-2","ok :-2","ok :2","ok :8","ok :-2","ok :-8","ok :22","ok :-34","ok :-2","ok :-1","ok :123456789012345678901234567891","ok :-123456789012345678901234567890","ok :10","ok :-10","ok :0","ok :10"]"#
    );
}

#[test]
fn number_text_and_integer_casts_keep_exact_scale_bounds_and_error_codes() {
    let scale_two = NumberScale::new(2).expect("scale two is valid");
    let parse_helpers = emit_cast_helpers(&[BuiltinCastPolicyId::StringToNumeric(
        NumericScalar::Number(scale_two),
    )]);
    let invalid_format = BuiltinErrorCode::NumberParseInvalidFormat.as_u32();
    let inexact_scale = BuiltinErrorCode::NumberParseInexactScale.as_u32();
    let mut parse_script = parse_helpers;
    parse_script.push_str(
        "\nconst summarize = result => result.tag === \"ok\" ? \"ok:\" + String(result.value) : \"err:\" + __moth_error_code(result.value);\n",
    );
    parse_script.push_str(
        "const zeroText = \"0e\" + \"9\".repeat(10000);\nconsole.log(JSON.stringify([\
            __moth_cast_number(\"1_234.50e-1\", 2, \"Dec2\"),\
            __moth_cast_number(\"3.14e2\", 2, \"Dec2\"),\
            __moth_cast_number(zeroText, 2, \"Dec2\"),\
            __moth_cast_number(\"1.239\", 2, \"Dec2\"),\
            __moth_cast_number(\"1\\n\", 2, \"Dec2\"),\
            __moth_cast_number(\"1e-1000000\", 2, \"Dec2\")\
        ].map(summarize)));",
    );
    assert_eq!(
        run_javascript(&parse_script).trim(),
        format!(
            r#"["ok:12345","ok:31400","ok:0","err:{inexact_scale}","err:{invalid_format}","err:{inexact_scale}"]"#
        )
    );

    let integer_cast_helpers = emit_cast_helpers(&[BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Number(scale_two),
        target: NumericScalar::Fixed(FixedScalar::U8),
    }]);
    let cast_inexact = BuiltinErrorCode::NumberCastInexact.as_u32();
    let out_of_range = BuiltinErrorCode::IntCastOutOfRange.as_u32();
    let mut integer_script = integer_cast_helpers;
    integer_script.push_str(
        "\nconst summarize = result => result.tag === \"ok\" ? \"ok:\" + String(result.value) : \"err:\" + __moth_error_code(result.value);\n",
    );
    integer_script.push_str(
        "console.log(JSON.stringify([\
            __moth_cast_number_to_integer(1200n, 100n, 2, 0, 255, \"Dec2\", \"U8\"),\
            __moth_cast_number_to_integer(1234n, 100n, 2, 0, 255, \"Dec2\", \"U8\"),\
            __moth_cast_number_to_integer(25600n, 100n, 2, 0, 255, \"Dec2\", \"U8\")\
        ].map(summarize)));",
    );
    assert_eq!(
        run_javascript(&integer_script).trim(),
        format!(r#"["ok:12","err:{cast_inexact}","err:{out_of_range}"]"#)
    );

    let scale_four = NumberScale::new(4).expect("scale four is valid");
    let scale_narrow_helpers = emit_cast_helpers(&[BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Number(scale_four),
        target: NumericScalar::Number(scale_two),
    }]);
    let mut scale_narrow_script = scale_narrow_helpers;
    scale_narrow_script.push_str(
        "\nconst summarize = result => result.tag === \"ok\" ? \"ok:\" + String(result.value) : \"err:\" + __moth_error_code(result.value);\n",
    );
    scale_narrow_script.push_str(
        "console.log(JSON.stringify([\
            __moth_cast_number_scale(123400n, 100n, 4, \"Dec4\", \"Dec2\"),\
            __moth_cast_number_scale(123456n, 100n, 4, \"Dec4\", \"Dec2\")\
        ].map(summarize)));",
    );
    assert_eq!(
        run_javascript(&scale_narrow_script).trim(),
        format!(r#"["ok:1234","err:{cast_inexact}"]"#)
    );
}

fn lower_number_operation_source(scale: NumberScale) -> String {
    use crate::compiler_frontend::datatypes::number::NumberValue;
    use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
    use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
    use crate::compiler_frontend::hir::blocks::HirBlock;
    use crate::compiler_frontend::hir::functions::HirFunction;
    use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
    use crate::compiler_frontend::hir::numeric::{
        HirNumericOp, HirNumericOperands, NumericFailureMode,
    };
    use crate::compiler_frontend::hir::statements::HirStatementKind;
    use crate::compiler_frontend::hir::terminators::HirTerminator;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, types) = build_type_environment();
    let number_type = type_environment.intern_number(scale);
    let region = RegionId(0);
    let number_value = NumberValue::from_integer(1, scale);
    let left = expression(
        1,
        crate::compiler_frontend::hir::expressions::HirExpressionKind::Number(number_value.clone()),
        number_type,
        region,
        crate::compiler_frontend::hir::expressions::ValueKind::Const,
    );
    let right = expression(
        2,
        crate::compiler_frontend::hir::expressions::HirExpressionKind::Number(number_value),
        number_type,
        region,
        crate::compiler_frontend::hir::expressions::ValueKind::Const,
    );
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, number_type, region)],
        statements: vec![statement(
            1,
            HirStatementKind::NumericOp {
                op: HirNumericOp {
                    operator: NumericOperator::Multiply,
                    domain: NumericScalar::Number(scale),
                },
                failure_mode: NumericFailureMode::Trap,
                operands: HirNumericOperands::Binary { left, right },
                result: LocalId(0),
            },
        )],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, region)),
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
    .expect("Number HIR should lower to JavaScript")
    .source
}

#[test]
fn shared_numeric_text_normalization_rejects_trailing_newlines_for_float() {
    let mut script =
        emit_cast_helpers(&[BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float)]);
    script.push_str(
        "\nconst parsed = __moth_cast_float(\"1.25\", 64, \"Float\");\n\
         const rejected = __moth_cast_float(\"1\\n\", 64, \"Float\");\n\
         console.log(JSON.stringify([parsed.tag, parsed.value, rejected.tag, __moth_error_code(rejected.value)]));",
    );

    assert_eq!(
        run_javascript(&script).trim(),
        format!(
            "[\"ok\",1.25,\"err\",{}]",
            BuiltinErrorCode::FloatParseInvalidFormat.as_u32()
        )
    );
}
