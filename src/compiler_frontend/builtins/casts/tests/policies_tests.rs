//! Policy unit tests for the builtin cast surface.
//!
//! WHAT: covers every builtin cast policy row, error code selection, and edge
//!      cases called out in the cast plan.
//! WHY: the policy owner is the single source of truth for cast rules. These
//!      tests pin the policy behaviour down so later phases can rely on it
//!      without re-deriving the expected outcomes in code.

use crate::compiler_frontend::builtins::casts::policies::{
    BuiltinCastLiteral, apply_builtin_cast_policy, builtin_cast_failure_codes,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::grammar::NumericLiteralSign;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

fn number_value(text: &str, scale: u16) -> NumberValue {
    let scale = NumberScale::new(scale).expect("test scale is valid");
    NumberValue::from_normalized(text, NumericLiteralSign::Positive, scale)
        .expect("test Dec value is exact")
}

#[test]
fn float_to_int_truncates_toward_zero() {
    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Float,
        target: NumericScalar::Int,
    };
    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(1.9),
        NumericProfile::STANDARD,
    )
    .expect("1.9 should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(1));

    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(-1.9),
        NumericProfile::STANDARD,
    )
    .expect("-1.9 should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(-1));
}

#[test]
fn float_to_int_rejects_non_finite_with_invalid_value_code() {
    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Float,
        target: NumericScalar::Int,
    };
    let error = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(f64::NAN),
        NumericProfile::STANDARD,
    )
    .expect_err("NaN should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntInvalidValue);
    assert!(error.code.is_implicit_failure());

    let error = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(f64::INFINITY),
        NumericProfile::STANDARD,
    )
    .expect_err("infinity should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntInvalidValue);
    assert!(error.code.is_implicit_failure());
    assert_eq!(
        builtin_cast_failure_codes(policy, BuiltinCastFallibility::Fallible),
        &[
            BuiltinErrorCode::FloatCastToIntInvalidValue,
            BuiltinErrorCode::FloatCastToIntOutOfRange,
        ]
    );
}

#[test]
fn float_to_int_rejects_out_of_i32_range_with_out_of_range_code() {
    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Float,
        target: NumericScalar::Int,
    };
    let error = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(3_000_000_000.0),
        NumericProfile::STANDARD,
    )
    .expect_err("above i32 range should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
    assert!(error.code.is_implicit_failure());

    let error = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(-3_000_000_000.0),
        NumericProfile::STANDARD,
    )
    .expect_err("below i32 range should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn failure_code_projection_tracks_supported_numeric_pair_failures() {
    let profile = NumericProfile::STANDARD;
    let integer_narrowing = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Int,
        target: NumericScalar::Fixed(FixedScalar::I8),
    };
    let error =
        apply_builtin_cast_policy(integer_narrowing, &BuiltinCastLiteral::Int(128), profile)
            .expect_err("128 is outside I8");
    assert_eq!(error.code, BuiltinErrorCode::IntCastOutOfRange);
    assert!(error.code.is_implicit_failure());
    assert_eq!(
        builtin_cast_failure_codes(integer_narrowing, BuiltinCastFallibility::Fallible),
        &[BuiltinErrorCode::IntCastOutOfRange]
    );

    let float_narrowing = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Float,
        target: NumericScalar::Fixed(FixedScalar::F16),
    };
    let error = apply_builtin_cast_policy(
        float_narrowing,
        &BuiltinCastLiteral::Float(f64::INFINITY),
        profile,
    )
    .expect_err("a non-finite value cannot materialise as F16");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastNonFinite);
    assert!(error.code.is_implicit_failure());
    assert_eq!(
        builtin_cast_failure_codes(float_narrowing, BuiltinCastFallibility::Fallible),
        &[BuiltinErrorCode::FloatCastNonFinite]
    );

    let infallible = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Fixed(FixedScalar::U8),
        target: NumericScalar::Fixed(FixedScalar::U16),
    };
    assert!(builtin_cast_failure_codes(infallible, BuiltinCastFallibility::Infallible).is_empty());

    let unsupported_float_to_number = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Float,
        target: NumericScalar::Number(NumberScale::ZERO),
    };
    assert!(
        builtin_cast_failure_codes(
            unsupported_float_to_number,
            BuiltinCastFallibility::Fallible
        )
        .is_empty()
    );
    assert!(!BuiltinErrorCode::FloatBoundaryNonFinite.is_implicit_failure());
    assert!(!BuiltinErrorCode::FloatFormatInvariant.is_implicit_failure());
    assert!(!BuiltinErrorCode::CollectionIndexOutOfBounds.is_implicit_failure());
    assert!(!BuiltinErrorCode::HostInvalidArgument.is_implicit_failure());
}

#[test]
fn float_to_int_accepts_i32_max_boundary() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Float,
            target: NumericScalar::Int,
        },
        &BuiltinCastLiteral::Float(i32::MAX as f64),
        NumericProfile::STANDARD,
    )
    .expect("exact i32 max should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(i64::from(i32::MAX)));
}

#[test]
fn float_to_int_accepts_i32_min_boundary() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Float,
            target: NumericScalar::Int,
        },
        &BuiltinCastLiteral::Float(i32::MIN as f64),
        NumericProfile::STANDARD,
    )
    .expect("exact i32 min should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(i64::from(i32::MIN)));
}

#[test]
fn float_to_int_rejects_one_above_i32_max() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Float,
            target: NumericScalar::Int,
        },
        &BuiltinCastLiteral::Float((i32::MAX as f64) + 1.0),
        NumericProfile::STANDARD,
    )
    .expect_err("one above i32 max should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn float_to_int_rejects_one_below_i32_min() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Float,
            target: NumericScalar::Int,
        },
        &BuiltinCastLiteral::Float((i32::MIN as f64) - 1.0),
        NumericProfile::STANDARD,
    )
    .expect_err("one below i32 min should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn float_to_int_accepts_wider_values_under_int64() {
    let int64_profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    };

    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Float,
            target: NumericScalar::Int,
        },
        &BuiltinCastLiteral::Float(3_000_000_000.0),
        int64_profile,
    )
    .expect("3e9 should fold under Int64");
    assert_eq!(result, BuiltinCastLiteral::Int(3_000_000_000));
}

#[test]
fn int_to_char_accepts_valid_unicode_scalars() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::IntToChar,
        &BuiltinCastLiteral::Int(0x41),
        NumericProfile::STANDARD,
    )
    .expect("'A' should fold");
    assert_eq!(result, BuiltinCastLiteral::Char('A'));
}

#[test]
fn int_to_char_rejects_negatives_with_invalid_codepoint_code() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::IntToChar,
        &BuiltinCastLiteral::Int(-1),
        NumericProfile::STANDARD,
    )
    .expect_err("negative codepoint should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntCastToCharInvalidCodepoint);
    assert!(error.code.is_implicit_failure());
    assert_eq!(
        builtin_cast_failure_codes(
            BuiltinCastPolicyId::IntToChar,
            BuiltinCastFallibility::Fallible,
        ),
        &[BuiltinErrorCode::IntCastToCharInvalidCodepoint]
    );
}

#[test]
fn int_to_char_rejects_surrogate_range_with_invalid_codepoint_code() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::IntToChar,
        &BuiltinCastLiteral::Int(0xD800),
        NumericProfile::STANDARD,
    )
    .expect_err("surrogate codepoint should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntCastToCharInvalidCodepoint);
}

#[test]
fn int_to_char_rejects_above_max_scalar_with_invalid_codepoint_code() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::IntToChar,
        &BuiltinCastLiteral::Int(0x110000),
        NumericProfile::STANDARD,
    )
    .expect_err("above max scalar should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntCastToCharInvalidCodepoint);
}

#[test]
fn char_to_int_returns_unicode_scalar_value() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::CharToInt,
        &BuiltinCastLiteral::Char('A'),
        NumericProfile::STANDARD,
    )
    .expect("Char -> Int should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(0x41));
}

#[test]
fn string_to_int_is_strict_base_10_with_optional_sign() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("-42".to_string()),
        NumericProfile::STANDARD,
    )
    .expect("signed integer should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(-42));

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("3.14".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("decimal text should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);
    assert!(error.code.is_implicit_failure());

    assert_eq!(
        builtin_cast_failure_codes(
            BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
            BuiltinCastFallibility::Fallible,
        ),
        &[
            BuiltinErrorCode::IntParseInvalidFormat,
            BuiltinErrorCode::IntParseOutOfRange,
        ]
    );

    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("1_000".to_string()),
        NumericProfile::STANDARD,
    )
    .expect("underscore-separated text should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(1000));

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("1__000".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("invalid underscore placement should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);
}

#[test]
fn string_to_int_rejects_surrounding_whitespace() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String(" 42 ".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("surrounding whitespace should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);
}

#[test]
fn string_to_int_rejects_unary_plus() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("+42".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("unary plus should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);
}

#[test]
fn string_to_int_rejects_exponent_forms() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("1e3".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("lowercase exponent text should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("1E3".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("uppercase exponent text should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);
}

#[test]
fn string_to_int_reports_overflow_as_out_of_range() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("2147483648".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("one above i32::MAX should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);
    assert!(error.code.is_implicit_failure());

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("-2147483649".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("one below i32::MIN should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);
}

#[test]
fn string_to_int_accepts_i32_max_boundary() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String(i32::MAX.to_string()),
        NumericProfile::STANDARD,
    )
    .expect("i32 max should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(i64::from(i32::MAX)));
}

#[test]
fn string_to_int_rejects_one_above_i32_max() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String((i64::from(i32::MAX) + 1).to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("one above i32 max should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);
}

#[test]
fn string_to_int_accepts_i32_min_boundary() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String(i32::MIN.to_string()),
        NumericProfile::STANDARD,
    )
    .expect("i32 min should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(i64::from(i32::MIN)));
}

#[test]
fn string_to_int_rejects_one_below_i32_min() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String((i64::from(i32::MIN) - 1).to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("one below i32 min should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);
}

#[test]
fn string_to_int_accepts_wider_values_under_int64() {
    let int64_profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    };

    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
        &BuiltinCastLiteral::String("3000000000".to_string()),
        int64_profile,
    )
    .expect("3e9 text should fold under Int64");
    assert_eq!(result, BuiltinCastLiteral::Int(3_000_000_000));
}

#[test]
fn string_to_float_rejects_nan_and_infinity_as_invalid_format() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float),
        &BuiltinCastLiteral::String("NaN".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("NaN text should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseInvalidFormat);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float),
        &BuiltinCastLiteral::String("Infinity".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("Infinity text should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseInvalidFormat);
}

#[test]
fn string_to_float_parses_ordinary_decimal_text() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float),
        &BuiltinCastLiteral::String("3.5e2".to_string()),
        NumericProfile::STANDARD,
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
            BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float),
            &BuiltinCastLiteral::String(source.to_string()),
            NumericProfile::STANDARD,
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
            BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float),
            &BuiltinCastLiteral::String(source.to_string()),
            NumericProfile::STANDARD,
        )
        .expect_err(source);
        assert_eq!(
            error.code,
            BuiltinErrorCode::FloatParseInvalidFormat,
            "{source}"
        );
    }

    let out_of_range_error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float),
        &BuiltinCastLiteral::String("1e10000".to_string()),
        NumericProfile::STANDARD,
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
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float),
        &BuiltinCastLiteral::String(" 1.0 ".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("surrounding whitespace should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseInvalidFormat);
}

#[test]
fn int_to_float_rounds_at_float32_precision() {
    let float32_profile = NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits32,
    };
    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Int,
        target: NumericScalar::Float,
    };

    // 16777217 is not representable in f32; single rounding gives 16777216.
    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Int(16_777_217),
        float32_profile,
    )
    .expect("Int -> Float should fold under Float32");
    assert_eq!(result, BuiltinCastLiteral::Float(16_777_216.0));

    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Int(16_777_217),
        NumericProfile::STANDARD,
    )
    .expect("Int -> Float should fold under Float64");
    assert_eq!(result, BuiltinCastLiteral::Float(16_777_217.0));
}

#[test]
fn string_to_bool_accepts_only_lowercase_true_and_false() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToBool,
        &BuiltinCastLiteral::String(" true ".to_string()),
        NumericProfile::STANDARD,
    )
    .expect("lowercase true should fold");
    assert_eq!(result, BuiltinCastLiteral::Bool(true));

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToBool,
        &BuiltinCastLiteral::String("TRUE".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("uppercase true should fail");
    assert_eq!(error.code, BuiltinErrorCode::StringParseBoolInvalidFormat);
}

#[test]
fn string_to_char_succeeds_only_for_single_scalar() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToChar,
        &BuiltinCastLiteral::String("A".to_string()),
        NumericProfile::STANDARD,
    )
    .expect("single char should fold");
    assert_eq!(result, BuiltinCastLiteral::Char('A'));

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToChar,
        &BuiltinCastLiteral::String("AB".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("multi-char string should fail");
    assert_eq!(error.code, BuiltinErrorCode::StringParseCharInvalidFormat);
}

#[test]
fn string_to_char_rejects_empty_with_invalid_format_code() {
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToChar,
        &BuiltinCastLiteral::String(String::new()),
        NumericProfile::STANDARD,
    )
    .expect_err("empty string should fail");
    assert_eq!(error.code, BuiltinErrorCode::StringParseCharInvalidFormat);
}

#[test]
fn int_to_string_uses_signed_base_10() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericToString(NumericScalar::Int),
        &BuiltinCastLiteral::Int(-42),
        NumericProfile::STANDARD,
    )
    .expect("Int -> String should fold");
    assert_eq!(result, BuiltinCastLiteral::String("-42".to_string()));
}

#[test]
fn float_to_string_uses_stable_decimal() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericToString(NumericScalar::Float),
        &BuiltinCastLiteral::Float(1.5),
        NumericProfile::STANDARD,
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
            BuiltinCastPolicyId::NumericToString(NumericScalar::Float),
            &BuiltinCastLiteral::Float(*value),
            NumericProfile::STANDARD,
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
fn float_to_string_uses_profile_precision_for_f32_exact_carrier() {
    let policy = BuiltinCastPolicyId::NumericToString(NumericScalar::Float);
    let carrier = BuiltinCastLiteral::Float(f64::from(0.1_f32));
    let float32_profile = NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits32,
    };

    let float32_result = apply_builtin_cast_policy(policy, &carrier, float32_profile)
        .expect("Float -> String should fold under Float32");
    assert_eq!(
        float32_result,
        BuiltinCastLiteral::String("0.1".to_string())
    );

    let standard_result = apply_builtin_cast_policy(policy, &carrier, NumericProfile::STANDARD)
        .expect("Float -> String should fold under the standard profile");
    assert_eq!(
        standard_result,
        BuiltinCastLiteral::String("0.10000000149011612".to_string())
    );
}

#[test]
fn bool_to_string_returns_true_or_false() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::BoolToString,
        &BuiltinCastLiteral::Bool(true),
        NumericProfile::STANDARD,
    )
    .expect("Bool -> String should fold");
    assert_eq!(result, BuiltinCastLiteral::String("true".to_string()));
}

#[test]
fn char_to_string_returns_one_character_string() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::CharToString,
        &BuiltinCastLiteral::Char('Z'),
        NumericProfile::STANDARD,
    )
    .expect("Char -> String should fold");
    assert_eq!(result, BuiltinCastLiteral::String("Z".to_string()));
}

#[test]
fn string_to_error_uses_text_as_message_and_default_code() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToError,
        &BuiltinCastLiteral::String("Missing number".to_string()),
        NumericProfile::STANDARD,
    )
    .expect("String -> Error should fold");

    assert_eq!(
        result,
        BuiltinCastLiteral::Error {
            message: "Missing number".to_string(),
            code: 0u32,
        }
    );
}

#[test]
fn error_to_string_returns_error_message_only() {
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::ErrorToString,
        &BuiltinCastLiteral::Error {
            message: "Missing number".to_string(),
            code: 200u32,
        },
        NumericProfile::STANDARD,
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
    assert!(BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int).is_const_foldable());
    assert!(BuiltinCastPolicyId::NumericToString(NumericScalar::Int).is_const_foldable());
}

#[test]
fn u64_to_f32_rounds_once_above_f64_double_rounding_midpoint() {
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    // One above the F32 midpoint rounds up directly; F64 drops the final +1 and makes it a tie.
    let value = (1_u64 << 60) + (1_u64 << 36) + 1;
    let source = BuiltinCastLiteral::Fixed(
        FixedScalarValue::unsigned(FixedScalar::U64, value).expect("the test value fits U64"),
    );
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::U64),
            target: NumericScalar::Fixed(FixedScalar::F32),
        },
        &source,
        NumericProfile::STANDARD,
    )
    .expect("U64 -> F32 should fold");

    // The exact upper neighbor is 2^60 + 2^37, independent of either cast path below.
    let direct_result =
        FixedScalarValue::binary_float(FixedScalar::F32, ((1_u64 << 60) + (1_u64 << 37)) as f64)
            .expect("the direct F32 result is finite");
    let double_rounded_result =
        FixedScalarValue::binary_float(FixedScalar::F32, f64::from((value as f64) as f32))
            .expect("the double-rounded F32 result is finite");

    assert_ne!(direct_result, double_rounded_result);
    assert_eq!(result, BuiltinCastLiteral::Fixed(direct_result));
    assert_ne!(result, BuiltinCastLiteral::Fixed(double_rounded_result));
}

#[test]
fn two_pow_53_plus_one_to_f64_rounds_to_two_pow_53() {
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let value = (1u64 << 53) + 1;
    let source = BuiltinCastLiteral::Fixed(
        FixedScalarValue::unsigned(FixedScalar::U64, value).expect("2^53+1 fits U64"),
    );
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::U64),
            target: NumericScalar::Fixed(FixedScalar::F64),
        },
        &source,
        NumericProfile::STANDARD,
    )
    .expect("2^53+1 -> F64 should fold");
    let expected = FixedScalarValue::binary_float(FixedScalar::F64, 9_007_199_254_740_992.0)
        .expect("2^53 is finite");
    assert_eq!(result, BuiltinCastLiteral::Fixed(expected));
}

#[test]
fn integer_to_integer_rejects_out_of_range_with_dedicated_code() {
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::I64),
            target: NumericScalar::Fixed(FixedScalar::U64),
        },
        &BuiltinCastLiteral::Fixed(
            FixedScalarValue::signed(FixedScalar::I64, i64::MIN).expect("I64::MIN fits I64"),
        ),
        NumericProfile::STANDARD,
    )
    .expect_err("I64::MIN -> U64 should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntCastOutOfRange);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::U64),
            target: NumericScalar::Fixed(FixedScalar::I64),
        },
        &BuiltinCastLiteral::Fixed(
            FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64::MAX fits U64"),
        ),
        NumericProfile::STANDARD,
    )
    .expect_err("U64::MAX -> I64 should fail");
    assert_eq!(error.code, BuiltinErrorCode::IntCastOutOfRange);
}

#[test]
fn float64_to_f32_rejects_non_finite_result() {
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::F64),
            target: NumericScalar::Fixed(FixedScalar::F32),
        },
        &BuiltinCastLiteral::Fixed(
            FixedScalarValue::binary_float(FixedScalar::F64, 1e39).expect("finite F64"),
        ),
        NumericProfile::STANDARD,
    )
    .expect_err("1e39 -> F32 should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastNonFinite);
}

#[test]
fn integer_to_f16_checks_finite_magnitude() {
    use moth_lexical::numeric::binary16::round_f64_to_f16;
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Int,
        target: NumericScalar::Fixed(FixedScalar::F16),
    };
    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Int(65_519),
        NumericProfile::STANDARD,
    )
    .expect("65519 -> F16 should fold");
    let expected = FixedScalarValue::binary_float(FixedScalar::F16, round_f64_to_f16(65_519.0))
        .expect("65519 rounds finite");
    assert_eq!(result, BuiltinCastLiteral::Fixed(expected));
    assert_eq!(
        expected.as_f64().expect("F16 reads as f64").to_bits(),
        65_504.0_f64.to_bits()
    );

    let error = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Int(65_520),
        NumericProfile::STANDARD,
    )
    .expect_err("65520 -> F16 should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastNonFinite);
}

#[test]
fn float_to_f16_rounds_once_and_rejects_non_finite() {
    use moth_lexical::numeric::binary16::round_f64_to_f16;
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Fixed(FixedScalar::F64),
        target: NumericScalar::Fixed(FixedScalar::F16),
    };
    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Fixed(
            FixedScalarValue::binary_float(FixedScalar::F64, 65_519.0).expect("finite F64"),
        ),
        NumericProfile::STANDARD,
    )
    .expect("65519.0 -> F16 should fold");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::binary_float(FixedScalar::F16, round_f64_to_f16(65_519.0))
                .expect("finite F16")
        )
    );

    let error = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Fixed(
            FixedScalarValue::binary_float(FixedScalar::F64, 65_520.0).expect("finite F64"),
        ),
        NumericProfile::STANDARD,
    )
    .expect_err("65520.0 -> F16 should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastNonFinite);
}

#[test]
fn float64_to_f16_rounds_directly_without_f32_intermediate() {
    use moth_lexical::numeric::binary16::round_f64_to_f16;
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    // Deterministic witness: 1.000488281251 sits just above the F16
    // midpoint between 1.0 and 1.0009765625. The hair (~1e-12) survives the
    // `f64` carrier but collapses in the `f32` intermediate, so direct
    // rounding goes up while F64 -> F32 -> F16 ties-to-even rounds down.
    let value = 1.000488281251_f64;
    let direct = round_f64_to_f16(value);
    let via_f32 = round_f64_to_f16(f64::from(value as f32));
    assert_ne!(
        direct.to_bits(),
        via_f32.to_bits(),
        "test needs a direct-vs-intermediate rounding witness"
    );
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::F64),
            target: NumericScalar::Fixed(FixedScalar::F16),
        },
        &BuiltinCastLiteral::Fixed(
            FixedScalarValue::binary_float(FixedScalar::F64, value).expect("finite F64"),
        ),
        NumericProfile::STANDARD,
    )
    .expect("witness -> F16 should fold");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::binary_float(FixedScalar::F16, direct).expect("finite F16")
        )
    );
}

#[test]
fn float_to_int_boundary_cases() {
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let i64_policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Fixed(FixedScalar::F64),
        target: NumericScalar::Fixed(FixedScalar::I64),
    };
    let result = apply_builtin_cast_policy(
        i64_policy,
        &BuiltinCastLiteral::Fixed(
            FixedScalarValue::binary_float(FixedScalar::F64, -9_223_372_036_854_775_808.0)
                .expect("finite F64"),
        ),
        NumericProfile::STANDARD,
    )
    .expect("-2^63 -> I64 should fold");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::signed(FixedScalar::I64, i64::MIN).expect("I64::MIN fits")
        )
    );

    let error = apply_builtin_cast_policy(
        i64_policy,
        &BuiltinCastLiteral::Fixed(
            FixedScalarValue::binary_float(FixedScalar::F64, 9_223_372_036_854_775_808.0)
                .expect("finite F64"),
        ),
        NumericProfile::STANDARD,
    )
    .expect_err("2^63 -> I64 should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Float,
            target: NumericScalar::Fixed(FixedScalar::U8),
        },
        &BuiltinCastLiteral::Float(f64::NAN),
        NumericProfile::STANDARD,
    )
    .expect_err("NaN -> U8 should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntInvalidValue);

    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Float,
            target: NumericScalar::Fixed(FixedScalar::U8),
        },
        &BuiltinCastLiteral::Float(-0.9),
        NumericProfile::STANDARD,
    )
    .expect("-0.9 -> U8 should fold");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::unsigned(FixedScalar::U8, 0).expect("0 fits U8")
        )
    );
}

#[test]
fn byte_u8_round_trip_preserves_value() {
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let byte = BuiltinCastLiteral::Fixed(
        FixedScalarValue::unsigned(FixedScalar::Byte, 255).expect("255 fits Byte"),
    );
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::ByteToU8,
        &byte,
        NumericProfile::STANDARD,
    )
    .expect("Byte 255 -> U8 should fold");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::unsigned(FixedScalar::U8, 255).expect("255 fits U8")
        )
    );

    let back = apply_builtin_cast_policy(
        BuiltinCastPolicyId::U8ToByte,
        &result,
        NumericProfile::STANDARD,
    )
    .expect("U8 255 -> Byte should fold");
    assert_eq!(back, byte);
}

#[test]
fn numeric_to_string_formats_each_domain_at_its_own_precision() {
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    // Integers use exact decimal text, including values above the `Int` range and the signed
    // minimum, which no `Int` or `Float` intermediate could carry.
    let cases = [
        (
            NumericScalar::Fixed(FixedScalar::U64),
            BuiltinCastLiteral::Fixed(
                FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("u64::MAX fits U64"),
            ),
            "18446744073709551615",
        ),
        (
            NumericScalar::Fixed(FixedScalar::I64),
            BuiltinCastLiteral::Fixed(
                FixedScalarValue::signed(FixedScalar::I64, i64::MIN).expect("i64::MIN fits I64"),
            ),
            "-9223372036854775808",
        ),
        (NumericScalar::Int, BuiltinCastLiteral::Int(-42), "-42"),
    ];

    for (scalar, source, expected) in cases {
        let result = apply_builtin_cast_policy(
            BuiltinCastPolicyId::NumericToString(scalar),
            &source,
            NumericProfile::STANDARD,
        )
        .unwrap_or_else(|error| panic!("{scalar:?} -> String should fold: {error:?}"));
        assert_eq!(result, BuiltinCastLiteral::String(expected.to_string()));
    }

    // A binary float formats at its own precision: `F32` prints the shortest f32 text and `F16`
    // prints the shortest text that reads back at binary16 precision.
    let float_cases = [
        (
            NumericScalar::Fixed(FixedScalar::F32),
            BuiltinCastLiteral::Fixed(
                FixedScalarValue::binary_float(FixedScalar::F32, f64::from(0.1_f32))
                    .expect("0.1f32 is F32-exact"),
            ),
            "0.1",
        ),
        (
            NumericScalar::Fixed(FixedScalar::F16),
            BuiltinCastLiteral::Fixed(
                FixedScalarValue::binary_float(FixedScalar::F16, 0.099_975_585_937_5)
                    .expect("nearest binary16 to 0.1"),
            ),
            "0.1",
        ),
        (NumericScalar::Float, BuiltinCastLiteral::Float(1.5), "1.5"),
    ];

    for (scalar, source, expected) in float_cases {
        let result = apply_builtin_cast_policy(
            BuiltinCastPolicyId::NumericToString(scalar),
            &source,
            NumericProfile::STANDARD,
        )
        .unwrap_or_else(|error| panic!("{scalar:?} -> String should fold: {error:?}"));
        assert_eq!(result, BuiltinCastLiteral::String(expected.to_string()));
    }
}

#[test]
fn string_to_numeric_parses_fixed_destinations_at_their_own_ranges() {
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let scalar = NumericScalar::Fixed(FixedScalar::U8);
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(scalar),
        &BuiltinCastLiteral::String("255".to_string()),
        NumericProfile::STANDARD,
    )
    .expect("255 fits U8");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(FixedScalarValue::unsigned(FixedScalar::U8, 255).unwrap())
    );

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(scalar),
        &BuiltinCastLiteral::String("256".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("256 does not fit U8");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(scalar),
        &BuiltinCastLiteral::String("-1".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("a negative spelling has no unsigned reading");
    assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(scalar),
        &BuiltinCastLiteral::String("abc".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("non-numeric text is a format failure");
    assert_eq!(error.code, BuiltinErrorCode::IntParseInvalidFormat);

    // A `U64` destination accepts its full range, which no `Int`-bounded parse could.
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::U64)),
        &BuiltinCastLiteral::String(u64::MAX.to_string()),
        NumericProfile::STANDARD,
    )
    .expect("u64::MAX fits U64");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).unwrap())
    );
}

#[test]
fn string_to_binary_float_rounds_at_the_destination_and_reports_its_own_failures() {
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let f16 = NumericScalar::Fixed(FixedScalar::F16);
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(f16),
        &BuiltinCastLiteral::String("0.1".to_string()),
        NumericProfile::STANDARD,
    )
    .expect("0.1 rounds to a finite binary16");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::binary_float(FixedScalar::F16, 0.099_975_585_937_5).unwrap()
        )
    );

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(f16),
        &BuiltinCastLiteral::String("70000".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("70000 is beyond the largest finite binary16");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseOutOfRange);

    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(f16),
        &BuiltinCastLiteral::String("1.5.5".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("malformed text is a format failure");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseInvalidFormat);

    // `NaN` spelling is rejected by the grammar for every runtime parse, matching the `Float` row.
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::F64)),
        &BuiltinCastLiteral::String("NaN".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("NaN is not Moth numeric text");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseInvalidFormat);
}

#[test]
fn integer_to_number_preserves_u64_max_and_scale() {
    let source = BuiltinCastLiteral::Fixed(
        FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("u64::MAX fits U64"),
    );
    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Fixed(FixedScalar::U64),
        target: NumericScalar::Number(NumberScale::new(2).expect("scale is valid")),
    };

    let result = apply_builtin_cast_policy(policy, &source, NumericProfile::STANDARD)
        .expect("every U64 value scales exactly into Dec2");

    assert_eq!(
        result,
        BuiltinCastLiteral::Number(number_value("18446744073709551615", 2))
    );
}

#[test]
fn number_to_integer_checks_u64_and_profile_int_boundaries() {
    let scale_zero = NumberScale::new(0).expect("zero is a valid Dec scale");
    let number_to_u64 = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Number(scale_zero),
        target: NumericScalar::Fixed(FixedScalar::U64),
    };
    let u64_max_source = BuiltinCastLiteral::Number(number_value("18446744073709551615", 0));
    let u64_max =
        apply_builtin_cast_policy(number_to_u64, &u64_max_source, NumericProfile::STANDARD)
            .expect("U64::MAX fits its exact destination");
    assert_eq!(
        u64_max,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("u64::MAX fits U64")
        )
    );

    let number_to_int = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Number(scale_zero),
        target: NumericScalar::Int,
    };
    let above_i32 = BuiltinCastLiteral::Number(number_value("2147483648", 0));
    let error = apply_builtin_cast_policy(number_to_int, &above_i32, NumericProfile::STANDARD)
        .expect_err("the standard Int profile is 32-bit");
    assert_eq!(error.code, BuiltinErrorCode::IntCastOutOfRange);

    let profile64 = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    };
    assert_eq!(
        apply_builtin_cast_policy(number_to_int, &above_i32, profile64)
            .expect("the 64-bit Int profile contains this value"),
        BuiltinCastLiteral::Int(2_147_483_648)
    );

    let i64_max = BuiltinCastLiteral::Number(number_value("9223372036854775807", 0));
    assert_eq!(
        apply_builtin_cast_policy(number_to_int, &i64_max, profile64)
            .expect("the 64-bit Int profile includes i64::MAX"),
        BuiltinCastLiteral::Int(i64::MAX)
    );
    let above_i64 = BuiltinCastLiteral::Number(number_value("9223372036854775808", 0));
    let error = apply_builtin_cast_policy(number_to_int, &above_i64, profile64)
        .expect_err("the 64-bit Int profile cannot contain i64::MAX + 1");
    assert_eq!(error.code, BuiltinErrorCode::IntCastOutOfRange);
}

#[test]
fn number_to_integer_rejects_fraction_and_values_beyond_i128_exactly() {
    let scale_two = NumberScale::new(2).expect("two is a valid Dec scale");
    let number_to_int = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Number(scale_two),
        target: NumericScalar::Int,
    };
    let fractional = BuiltinCastLiteral::Number(number_value("12.34", 2));
    let error = apply_builtin_cast_policy(number_to_int, &fractional, NumericProfile::STANDARD)
        .expect_err("a fractional Dec cannot become an integer");
    assert_eq!(error.code, BuiltinErrorCode::NumberCastInexact);

    let scale_zero = NumberScale::new(0).expect("zero is a valid Dec scale");
    let number_to_u64 = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Number(scale_zero),
        target: NumericScalar::Fixed(FixedScalar::U64),
    };
    let huge_exact =
        BuiltinCastLiteral::Number(number_value("1234567890123456789012345678901234567890", 0));
    let error = apply_builtin_cast_policy(number_to_u64, &huge_exact, NumericProfile::STANDARD)
        .expect_err("an exact Dec outside i128 cannot fit U64");
    assert_eq!(error.code, BuiltinErrorCode::IntCastOutOfRange);
}

#[test]
fn number_scale_casts_are_exact_and_report_inexact_narrowing() {
    let scale_three = NumberScale::new(3).expect("three is a valid Dec scale");
    let scale_one = NumberScale::new(1).expect("one is a valid Dec scale");
    let widen = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Number(scale_one),
        target: NumericScalar::Number(scale_three),
    };
    let source = BuiltinCastLiteral::Number(number_value("1.2", 1));
    assert_eq!(
        apply_builtin_cast_policy(widen, &source, NumericProfile::STANDARD)
            .expect("scale widening is exact"),
        BuiltinCastLiteral::Number(number_value("1.2", 3))
    );

    let narrow = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Number(NumberScale::new(2).expect("two is valid")),
        target: NumericScalar::Number(scale_one),
    };
    let exact_source = BuiltinCastLiteral::Number(number_value("12.30", 2));
    assert_eq!(
        apply_builtin_cast_policy(narrow, &exact_source, NumericProfile::STANDARD)
            .expect("removed fractional digits are zero"),
        BuiltinCastLiteral::Number(number_value("12.3", 1))
    );

    let fractional_source = BuiltinCastLiteral::Number(number_value("12.34", 2));
    let error = apply_builtin_cast_policy(narrow, &fractional_source, NumericProfile::STANDARD)
        .expect_err("narrowing nonzero fractional digits is fallible");
    assert_eq!(error.code, BuiltinErrorCode::NumberCastInexact);
}

#[test]
fn string_to_number_uses_exact_scale_and_number_error_codes() {
    let parse_to_zero = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Number(
        NumberScale::new(0).expect("zero is valid"),
    ));
    assert_eq!(
        apply_builtin_cast_policy(
            parse_to_zero,
            &BuiltinCastLiteral::String("1.25e2".to_string()),
            NumericProfile::STANDARD,
        )
        .expect("integral decimal/exponent text is exact at scale zero"),
        BuiltinCastLiteral::Number(number_value("125", 0))
    );

    let parse_to_two = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Number(
        NumberScale::new(2).expect("two is valid"),
    ));
    assert_eq!(
        apply_builtin_cast_policy(
            parse_to_two,
            &BuiltinCastLiteral::String("1.23e1".to_string()),
            NumericProfile::STANDARD,
        )
        .expect("exact decimal/exponent text materialises at Dec2"),
        BuiltinCastLiteral::Number(number_value("12.3", 2))
    );

    let inexact = apply_builtin_cast_policy(
        parse_to_two,
        &BuiltinCastLiteral::String("1.234".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("text with excess nonzero fractional digits is inexact");
    assert_eq!(inexact.code, BuiltinErrorCode::NumberParseInexactScale);

    let nonintegral_at_zero = apply_builtin_cast_policy(
        parse_to_zero,
        &BuiltinCastLiteral::String("1.2".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("decimal text at Dec0 must be exactly integral");
    assert_eq!(
        nonintegral_at_zero.code,
        BuiltinErrorCode::NumberParseInexactScale
    );

    let invalid = apply_builtin_cast_policy(
        parse_to_zero,
        &BuiltinCastLiteral::String("1E2".to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("uppercase exponent markers are rejected by the shared grammar");
    assert_eq!(invalid.code, BuiltinErrorCode::NumberParseInvalidFormat);

    let capacity_text = "1e999999999999999999999999999999999999999999999999999999";
    let capacity = apply_builtin_cast_policy(
        parse_to_zero,
        &BuiltinCastLiteral::String(capacity_text.to_string()),
        NumericProfile::STANDARD,
    )
    .expect_err("the Dec materializer rejects an exponent outside host-size capacity");
    assert_eq!(capacity.code, BuiltinErrorCode::NumberParseCapacity);

    assert_eq!(
        apply_builtin_cast_policy(
            parse_to_zero,
            &BuiltinCastLiteral::String(
                "0e999999999999999999999999999999999999999999999999999999".to_string(),
            ),
            NumericProfile::STANDARD,
        )
        .expect("zero fast-paths before huge exponent expansion"),
        BuiltinCastLiteral::Number(number_value("0", 0))
    );
}

#[test]
fn number_to_string_formats_values_beyond_i128_without_exponent_text() {
    let scale_three = NumberScale::new(3).expect("three is valid");
    let source = BuiltinCastLiteral::Number(number_value(
        "1234567890123456789012345678901234567890.000",
        3,
    ));
    let result = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericToString(NumericScalar::Number(scale_three)),
        &source,
        NumericProfile::STANDARD,
    )
    .expect("Dec formatting is infallible for arbitrary exact values");
    assert_eq!(
        result,
        BuiltinCastLiteral::String("1234567890123456789012345678901234567890".to_string())
    );
}

#[test]
fn float_to_uint_truncates_and_rejects_non_finite_negative_and_overflow() {
    // The Float -> Uint rows truncate toward zero; NaN/inf fail as
    // invalid values while negatives and 2^64 fail as out of range.
    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Float,
        target: NumericScalar::Uint,
    };

    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(1.9),
        NumericProfile::STANDARD,
    )
    .expect("1.9 should fold");
    assert_eq!(result, BuiltinCastLiteral::Uint(1));

    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = apply_builtin_cast_policy(
            policy,
            &BuiltinCastLiteral::Float(value),
            NumericProfile::STANDARD,
        )
        .expect_err("non-finite Float -> Uint should fail");
        assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntInvalidValue);
    }

    // Truncation toward zero folds `-0.5` to zero, exactly like `-0.9 -> U8`;
    // `-1.0` truncates to `-1`, which is out of the unsigned range.
    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(-0.5),
        NumericProfile::STANDARD,
    )
    .expect("-0.5 should fold");
    assert_eq!(result, BuiltinCastLiteral::Uint(0));

    for value in [-1.0, 4_294_967_296.0] {
        let error = apply_builtin_cast_policy(
            policy,
            &BuiltinCastLiteral::Float(value),
            NumericProfile::STANDARD,
        )
        .expect_err("negative or overflowing Float -> Uint should fail");
        assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
    }

    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(4_294_967_295.0),
        NumericProfile::STANDARD,
    )
    .expect("u32::MAX should fold");
    assert_eq!(result, BuiltinCastLiteral::Uint(u32::MAX as u64));

    let int64_profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    };
    let error = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::Float(18_446_744_073_709_551_616.0),
        int64_profile,
    )
    .expect_err("2^64 -> Uint64 should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn string_to_uint_accepts_boundaries_and_rejects_negative_zero() {
    // String -> Uint owns the unsigned text grammar, so even `-0`
    // is rejected rather than folding to zero.
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Uint);

    let result = apply_builtin_cast_policy(
        policy,
        &BuiltinCastLiteral::String(u32::MAX.to_string()),
        NumericProfile::STANDARD,
    )
    .expect("u32::MAX text should fold");
    assert_eq!(result, BuiltinCastLiteral::Uint(u32::MAX as u64));

    for text in ["-0", "-1", "4294967296"] {
        let error = apply_builtin_cast_policy(
            policy,
            &BuiltinCastLiteral::String(text.to_string()),
            NumericProfile::STANDARD,
        )
        .expect_err("signed or overflowing Uint text should fail");
        assert_eq!(error.code, BuiltinErrorCode::IntParseOutOfRange);
    }
}
