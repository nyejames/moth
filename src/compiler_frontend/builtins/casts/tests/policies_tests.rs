//! Policy unit tests for the builtin cast surface.
//!
//! WHAT: covers every builtin cast policy row, error code selection, and edge
//!      cases called out in the cast plan.
//! WHY: the policy owner is the single source of truth for cast rules. These
//!      tests pin the policy behaviour down so later phases can rely on it
//!      without re-deriving the expected outcomes in code.

use crate::compiler_frontend::builtins::casts::numeric_limits::{I32_MAX, I32_MIN};
use crate::compiler_frontend::builtins::casts::policies::{
    BuiltinCastLiteral, apply_builtin_cast_policy,
};
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

#[test]
fn float_to_int_truncates_toward_zero() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float(1.9),
    )
    .expect("1.9 should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(1));

    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float(-1.9),
    )
    .expect("-1.9 should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(-1));
}

#[test]
fn float_to_int_rejects_non_finite_with_invalid_value_code() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float(f64::NAN),
    )
    .expect_err("NaN should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntInvalidValue);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float(f64::INFINITY),
    )
    .expect_err("infinity should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntInvalidValue);
}

#[test]
fn float_to_int_rejects_out_of_i32_range_with_out_of_range_code() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float(3_000_000_000.0),
    )
    .expect_err("above i32 range should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float(-3_000_000_000.0),
    )
    .expect_err("below i32 range should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn float_to_int_accepts_i32_max_boundary() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float(i32::MAX as f64),
    )
    .expect("exact i32 max should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(i32::MAX));
}

#[test]
fn float_to_int_accepts_i32_min_boundary() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float(i32::MIN as f64),
    )
    .expect("exact i32 min should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(i32::MIN));
}

#[test]
fn float_to_int_rejects_one_above_i32_max() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float((i32::MAX as f64) + 1.0),
    )
    .expect_err("one above i32 max should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn float_to_int_rejects_one_below_i32_min() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToInt,
        &BuiltinCastLiteral::Float((i32::MIN as f64) - 1.0),
    )
    .expect_err("one below i32 min should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn int_to_char_accepts_valid_unicode_scalars() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::IntToChar,
        &BuiltinCastLiteral::Int(0x41),
    )
    .expect("'A' should fold");
    assert_eq!(result, BuiltinCastLiteral::Char('A'));
}

#[test]
fn int_to_char_rejects_negatives_with_invalid_codepoint_code() {
    let error =
        apply_builtin_cast_policy(BuiltinCastPolicyId::IntToChar, &BuiltinCastLiteral::Int(-1))
            .expect_err("negative codepoint should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntCastToCharInvalidCodepoint);
}

#[test]
fn int_to_char_rejects_surrogate_range_with_invalid_codepoint_code() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::IntToChar,
        &BuiltinCastLiteral::Int(0xD800),
    )
    .expect_err("surrogate codepoint should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntCastToCharInvalidCodepoint);
}

#[test]
fn int_to_char_rejects_above_max_scalar_with_invalid_codepoint_code() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::IntToChar,
        &BuiltinCastLiteral::Int(0x110000),
    )
    .expect_err("above max scalar should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntCastToCharInvalidCodepoint);
}

#[test]
fn char_to_int_returns_unicode_scalar_value() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::CharToInt,
        &BuiltinCastLiteral::Char('A'),
    )
    .expect("Char -> Int should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(0x41));
}

#[test]
fn string_to_int_is_strict_base_10_with_optional_sign() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String("-42".to_string()),
    )
    .expect("signed integer should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(-42));

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String("3.14".to_string()),
    )
    .expect_err("decimal text should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);

    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String("1_000".to_string()),
    )
    .expect("underscore-separated text should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(1000));

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String("1__000".to_string()),
    )
    .expect_err("invalid underscore placement should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);
}

#[test]
fn string_to_int_rejects_surrounding_whitespace() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String(" 42 ".to_string()),
    )
    .expect_err("surrounding whitespace should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);
}

#[test]
fn string_to_int_rejects_unary_plus() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String("+42".to_string()),
    )
    .expect_err("unary plus should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);
}

#[test]
fn string_to_int_rejects_exponent_forms() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String("1e3".to_string()),
    )
    .expect_err("lowercase exponent text should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String("1E3".to_string()),
    )
    .expect_err("uppercase exponent text should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);
}

#[test]
fn string_to_int_reports_overflow_as_out_of_range() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String("2147483648".to_string()),
    )
    .expect_err("one above i32::MAX should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String("-2147483649".to_string()),
    )
    .expect_err("one below i32::MIN should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);
}

#[test]
fn string_to_int_accepts_i32_max_boundary() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String(I32_MAX.to_string()),
    )
    .expect("i32 max should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(I32_MAX));
}

#[test]
fn string_to_int_rejects_one_above_i32_max() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String((I32_MAX as i64 + 1).to_string()),
    )
    .expect_err("one above i32 max should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);
}

#[test]
fn string_to_int_accepts_i32_min_boundary() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String(I32_MIN.to_string()),
    )
    .expect("i32 min should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(I32_MIN));
}

#[test]
fn string_to_int_rejects_one_below_i32_min() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToInt,
        &BuiltinCastLiteral::String((I32_MIN as i64 - 1).to_string()),
    )
    .expect_err("one below i32 min should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);
}

#[test]
fn string_to_float_rejects_nan_and_infinity_as_invalid_format() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToFloat,
        &BuiltinCastLiteral::String("NaN".to_string()),
    )
    .expect_err("NaN text should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseInvalidFormat);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToFloat,
        &BuiltinCastLiteral::String("Infinity".to_string()),
    )
    .expect_err("Infinity text should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseInvalidFormat);
}

#[test]
fn string_to_float_parses_ordinary_decimal_text() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToFloat,
        &BuiltinCastLiteral::String("3.5e2".to_string()),
    )
    .expect("decimal exponent should fold");
    assert_eq!(result, BuiltinCastLiteral::Float(350.0));
}

#[test]
fn string_to_float_uses_shared_numeric_text_grammar() {
    let valid_cases = [
        ("1", 1.0),
        ("1.5", 1.5),
        ("-1.5", -1.5),
        ("1e6", 1e6),
        ("1e+21", 1e21),
        ("1e-6", 1e-6),
        ("1_000.5e-2", 1000.5e-2),
    ];

    for (source, expected) in valid_cases {
        let result = apply_builtin_cast_policy(
            BuiltinCastPolicyId::StringToFloat,
            &BuiltinCastLiteral::String(source.to_string()),
        )
        .expect(source);
        assert_eq!(result, BuiltinCastLiteral::Float(expected), "{source}");
    }

    let invalid_format_cases = [
        "1E6",
        "NaN",
        "Infinity",
        "-Infinity",
        "+1.0",
        " 1.0",
        "1.0 ",
        ".5",
        "1.",
        "1e",
        "1e+",
        "1__0",
        "1e_2",
    ];

    for source in invalid_format_cases {
        let error = apply_builtin_cast_policy(
            BuiltinCastPolicyId::StringToFloat,
            &BuiltinCastLiteral::String(source.to_string()),
        )
        .expect_err(source);
        assert_eq!(
            error.code,
            BuiltinErrorCode::FloatParseInvalidFormat,
            "{source}"
        );
    }

    let out_of_range_error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToFloat,
        &BuiltinCastLiteral::String("1e10000".to_string()),
    )
    .expect_err("non-finite grammar should fail");
    assert_eq!(
        out_of_range_error.code,
        BuiltinErrorCode::FloatParseOutOfRange
    );
}

#[test]
fn string_to_float_rejects_surrounding_whitespace() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToFloat,
        &BuiltinCastLiteral::String(" 1.0 ".to_string()),
    )
    .expect_err("surrounding whitespace should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseInvalidFormat);
}

#[test]
fn string_to_bool_accepts_only_lowercase_true_and_false() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToBool,
        &BuiltinCastLiteral::String(" true ".to_string()),
    )
    .expect("lowercase true should fold");
    assert_eq!(result, BuiltinCastLiteral::Bool(true));

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToBool,
        &BuiltinCastLiteral::String("TRUE".to_string()),
    )
    .expect_err("uppercase true should fail");
    assert_eq!(error.code, BuiltinErrorCode::StringParseBoolInvalidFormat);
}

#[test]
fn string_to_char_succeeds_only_for_single_scalar() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToChar,
        &BuiltinCastLiteral::String("A".to_string()),
    )
    .expect("single char should fold");
    assert_eq!(result, BuiltinCastLiteral::Char('A'));

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToChar,
        &BuiltinCastLiteral::String("AB".to_string()),
    )
    .expect_err("multi-char string should fail");
    assert_eq!(error.code, BuiltinErrorCode::StringParseCharInvalidFormat);
}

#[test]
fn string_to_char_rejects_empty_with_invalid_format_code() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToChar,
        &BuiltinCastLiteral::String(String::new()),
    )
    .expect_err("empty string should fail");
    assert_eq!(error.code, BuiltinErrorCode::StringParseCharInvalidFormat);
}

#[test]
fn int_to_string_uses_signed_base_10() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::IntToString,
        &BuiltinCastLiteral::Int(-42),
    )
    .expect("Int -> String should fold");
    assert_eq!(result, BuiltinCastLiteral::String("-42".to_string()));
}

#[test]
fn float_to_string_uses_stable_decimal() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::FloatToString,
        &BuiltinCastLiteral::Float(1.5),
    )
    .expect("Float -> String should fold");
    assert_eq!(result, BuiltinCastLiteral::String("1.5".to_string()));
}

#[test]
fn float_to_string_follows_moth_contract() {
    let cases: &[(f64, &str)] = &[
        (1.0, "1"),
        (1.5, "1.5"),
        (0.000001, "0.000001"),
        (0.0000001, "1e-7"),
        (1e21, "1e+21"),
        (-0.0, "0"),
    ];

    for (value, expected) in cases {
        let result = apply_builtin_cast_policy(
            BuiltinCastPolicyId::FloatToString,
            &BuiltinCastLiteral::Float(*value),
        )
        .expect("Float -> String should fold");
        assert_eq!(
            result,
            BuiltinCastLiteral::String(expected.to_string()),
            "Float -> String for {value}"
        );
    }
}

#[test]
fn bool_to_string_returns_true_or_false() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::BoolToString,
        &BuiltinCastLiteral::Bool(true),
    )
    .expect("Bool -> String should fold");
    assert_eq!(result, BuiltinCastLiteral::String("true".to_string()));
}

#[test]
fn char_to_string_returns_one_character_string() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::CharToString,
        &BuiltinCastLiteral::Char('Z'),
    )
    .expect("Char -> String should fold");
    assert_eq!(result, BuiltinCastLiteral::String("Z".to_string()));
}

#[test]
fn string_to_error_uses_text_as_message_and_default_code() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToError,
        &BuiltinCastLiteral::String("Missing number".to_string()),
    )
    .expect("String -> Error should fold");

    assert_eq!(
        result,
        BuiltinCastLiteral::Error {
            message: "Missing number".to_string(),
            code: 0,
        }
    );
}

#[test]
fn error_to_string_returns_error_message_only() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::ErrorToString,
        &BuiltinCastLiteral::Error {
            message: "Missing number".to_string(),
            code: 200,
        },
    )
    .expect("Error -> String should fold");

    assert_eq!(
        result,
        BuiltinCastLiteral::String("Missing number".to_string())
    );
}

#[test]
fn const_foldable_policy_marker_excludes_error_materialization_policies() {
    assert!(!BuiltinCastPolicyId::StringToError.is_const_foldable());
    assert!(!BuiltinCastPolicyId::ErrorToString.is_const_foldable());
    assert!(BuiltinCastPolicyId::StringToInt.is_const_foldable());
    assert!(BuiltinCastPolicyId::IntToString.is_const_foldable());
}
