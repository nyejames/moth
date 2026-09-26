//! Numeric text grammar tests.
//!
//! WHAT: validates the shared parser before tokens are materialized by AST or future casts.
//! WHY: separator, exponent, and normalization rules need one owner so tokenizer and string casts
//!      cannot drift into subtly different grammars.

use super::*;
use crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason;
use crate::compiler_frontend::datatypes::fixed_scalar::{FixedScalar, FixedScalarClass};
use crate::compiler_frontend::datatypes::numeric_profile::{FloatPrecision, IntWidth};
use crate::compiler_frontend::numeric_text::token::{
    NumericExponentSign, NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn parses_lowercase_exponent_metadata() {
    let parsed = parse_numeric_literal("1_200.50e-3").expect("literal should parse");

    assert_eq!(parsed.normalized_text, "1200.50e-3");
    assert_eq!(parsed.kind, NumericLiteralKind::Exponent);
    assert_eq!(parsed.digit_count, 7);
    assert_eq!(parsed.fractional_digit_count, 2);
    assert_eq!(parsed.exponent_digit_count, 1);
    assert_eq!(parsed.exponent_sign, NumericExponentSign::Negative);
}

#[test]
fn rejects_missing_exponent_digits() {
    for source in ["1e", "1e+", "1e-"] {
        let reason = parse_numeric_literal(source).expect_err("literal should be rejected");
        assert_eq!(
            reason,
            NumberLiteralErrorReason::MissingExponentDigits,
            "{source}"
        );
    }
}

#[test]
fn rejects_bad_separator_placement() {
    let cases = [
        ("_1", NumberLiteralErrorReason::InvalidSeparatorPlacement),
        ("1_", NumberLiteralErrorReason::EndsWithSeparator),
        ("1__0", NumberLiteralErrorReason::InvalidSeparatorPlacement),
        ("1_e2", NumberLiteralErrorReason::InvalidSeparatorPlacement),
        ("1e_2", NumberLiteralErrorReason::InvalidSeparatorPlacement),
        ("1e+_2", NumberLiteralErrorReason::InvalidSeparatorPlacement),
    ];

    for (source, expected_reason) in cases {
        let reason = parse_numeric_literal(source).expect_err("literal should be rejected");
        assert_eq!(reason, expected_reason, "{source}");
    }
}

#[test]
fn materialize_int_accepts_bits32_boundary_values() {
    let mut string_table = StringTable::new();

    let max_token = whole_number_token(
        "2147483647",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(&max_token, max_token.sign, IntWidth::Bits32, &string_table).unwrap(),
        i64::from(i32::MAX)
    );

    let min_token = whole_number_token(
        "2147483648",
        NumericLiteralSign::Negative,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(&min_token, min_token.sign, IntWidth::Bits32, &string_table).unwrap(),
        i64::from(i32::MIN)
    );
}

#[test]
fn materialize_int_rejects_bits32_out_of_range_values() {
    let mut string_table = StringTable::new();

    let too_large = whole_number_token(
        "2147483648",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(&too_large, too_large.sign, IntWidth::Bits32, &string_table).unwrap_err(),
        NumberLiteralErrorReason::OutsideIntRange
    );

    let too_negative = whole_number_token(
        "2147483649",
        NumericLiteralSign::Negative,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(
            &too_negative,
            too_negative.sign,
            IntWidth::Bits32,
            &string_table
        )
        .unwrap_err(),
        NumberLiteralErrorReason::OutsideIntRange
    );
}

#[test]
fn materialize_int_with_sign_override_allows_negative_fallback_boundary() {
    let mut string_table = StringTable::new();

    let positive_token = whole_number_token(
        "2147483648",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(
            &positive_token,
            NumericLiteralSign::Negative,
            IntWidth::Bits32,
            &string_table
        )
        .unwrap(),
        i64::from(i32::MIN)
    );
}

#[test]
fn materialize_int_accepts_bits64_boundaries() {
    let mut string_table = StringTable::new();

    let max_token = whole_number_token(
        "9223372036854775807",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(&max_token, max_token.sign, IntWidth::Bits64, &string_table).unwrap(),
        i64::MAX
    );

    let min_token = whole_number_token(
        "9223372036854775808",
        NumericLiteralSign::Negative,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(&min_token, min_token.sign, IntWidth::Bits64, &string_table).unwrap(),
        i64::MIN
    );
}

#[test]
fn materialize_int_rejects_bits64_out_of_range_values() {
    let mut string_table = StringTable::new();

    // Positive 2^63 exceeds the Bits64 maximum.
    let too_large = whole_number_token(
        "9223372036854775808",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(&too_large, too_large.sign, IntWidth::Bits64, &string_table).unwrap_err(),
        NumberLiteralErrorReason::OutsideIntRange
    );

    // One below the Bits64 minimum.
    let too_negative = whole_number_token(
        "9223372036854775809",
        NumericLiteralSign::Negative,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(
            &too_negative,
            too_negative.sign,
            IntWidth::Bits64,
            &string_table
        )
        .unwrap_err(),
        NumberLiteralErrorReason::OutsideIntRange
    );
}

#[test]
fn materialize_int_accepts_wider_values_under_bits64() {
    let mut string_table = StringTable::new();

    // 2^31 fits under Bits64 but not under Bits32.
    let wider_token = whole_number_token(
        "2147483648",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_int(
            &wider_token,
            wider_token.sign,
            IntWidth::Bits64,
            &string_table
        )
        .unwrap(),
        2_147_483_648_i64
    );
    assert_eq!(
        materialize_int(
            &wider_token,
            wider_token.sign,
            IntWidth::Bits32,
            &string_table
        )
        .unwrap_err(),
        NumberLiteralErrorReason::OutsideIntRange
    );
}

#[test]
fn materialize_normalized_int_matches_token_policy() {
    assert_eq!(
        materialize_normalized_int("2147483647", false, IntWidth::Bits32).unwrap(),
        i64::from(i32::MAX)
    );
    assert_eq!(
        materialize_normalized_int("2147483648", true, IntWidth::Bits32).unwrap(),
        i64::from(i32::MIN)
    );
    assert_eq!(
        materialize_normalized_int("9223372036854775808", true, IntWidth::Bits64).unwrap(),
        i64::MIN
    );
    assert_eq!(
        materialize_normalized_int("9223372036854775808", false, IntWidth::Bits64).unwrap_err(),
        NumberLiteralErrorReason::OutsideIntRange
    );
}

#[test]
fn parse_numeric_text_to_int_accepts_signed_boundaries_and_separators() {
    assert_eq!(
        parse_numeric_text_to_int("2_147_483_647", IntWidth::Bits32).unwrap(),
        i64::from(i32::MAX)
    );
    assert_eq!(
        parse_numeric_text_to_int("-2147483648", IntWidth::Bits32).unwrap(),
        i64::from(i32::MIN)
    );
}

#[test]
fn parse_numeric_text_to_int_rejects_out_of_range_values() {
    assert_eq!(
        parse_numeric_text_to_int("2147483648", IntWidth::Bits32).unwrap_err(),
        NumberLiteralErrorReason::OutsideIntRange
    );
    assert_eq!(
        parse_numeric_text_to_int("-2147483649", IntWidth::Bits32).unwrap_err(),
        NumberLiteralErrorReason::OutsideIntRange
    );
}

#[test]
fn parse_numeric_text_to_int_supports_bits64_boundaries() {
    assert_eq!(
        parse_numeric_text_to_int("9223372036854775807", IntWidth::Bits64).unwrap(),
        i64::MAX
    );
    assert_eq!(
        parse_numeric_text_to_int("-9223372036854775808", IntWidth::Bits64).unwrap(),
        i64::MIN
    );
    assert_eq!(
        parse_numeric_text_to_int("9223372036854775808", IntWidth::Bits64).unwrap_err(),
        NumberLiteralErrorReason::OutsideIntRange
    );
    // 2^31 is accepted under Bits64.
    assert_eq!(
        parse_numeric_text_to_int("2147483648", IntWidth::Bits64).unwrap(),
        2_147_483_648_i64
    );
}

#[test]
fn parse_numeric_text_to_int_rejects_non_whole_or_invalid_text() {
    for source in ["1.0", "1e3", "+1", " 1", "1 ", ""] {
        let reason = parse_numeric_text_to_int(source, IntWidth::Bits32).expect_err(source);
        assert_ne!(
            reason,
            NumberLiteralErrorReason::OutsideIntRange,
            "{source}"
        );
    }
}

#[test]
fn materialize_float_rejects_non_finite_values() {
    let mut string_table = StringTable::new();

    let token = NumericLiteralToken::new(
        NumericLiteralSign::Positive,
        string_table.intern("1e309"),
        string_table.intern("1e309"),
        NumericLiteralKind::Exponent,
        2,
        0,
        3,
        NumericExponentSign::Positive,
    );

    assert_eq!(
        materialize_float(&token, FloatPrecision::Bits64, &string_table).unwrap_err(),
        NumberLiteralErrorReason::NonFiniteFloat
    );
}

#[test]
fn materialize_float_applies_negative_sign() {
    let mut string_table = StringTable::new();

    let token = NumericLiteralToken::new(
        NumericLiteralSign::Negative,
        string_table.intern("-1.5e2"),
        string_table.intern("1.5e2"),
        NumericLiteralKind::Exponent,
        3,
        1,
        2,
        NumericExponentSign::Positive,
    );

    assert_eq!(
        materialize_float(&token, FloatPrecision::Bits64, &string_table).unwrap(),
        -150.0
    );
}

#[test]
fn materialize_float_rounds_directly_at_bits32() {
    let mut string_table = StringTable::new();

    // 0.1 is inexact in binary; Bits32 must equal the single f32 rounding.
    let text = "0.1";
    let token = NumericLiteralToken::new(
        NumericLiteralSign::Positive,
        string_table.intern(text),
        string_table.intern(text),
        NumericLiteralKind::DecimalPoint,
        2,
        1,
        0,
        NumericExponentSign::None,
    );

    assert_eq!(
        materialize_float(&token, FloatPrecision::Bits32, &string_table).unwrap(),
        f64::from(0.1f32)
    );
}

#[test]
fn materialize_float_rejects_bits32_overflow_as_non_finite() {
    let mut string_table = StringTable::new();

    // 3.5e38 fits in f64 but overflows f32.
    let text = "3.5e38";
    let token = NumericLiteralToken::new(
        NumericLiteralSign::Positive,
        string_table.intern(text),
        string_table.intern(text),
        NumericLiteralKind::Exponent,
        2,
        0,
        2,
        NumericExponentSign::None,
    );

    assert_eq!(
        materialize_float(&token, FloatPrecision::Bits32, &string_table).unwrap_err(),
        NumberLiteralErrorReason::NonFiniteFloat
    );
    assert!(
        materialize_float(&token, FloatPrecision::Bits64, &string_table)
            .unwrap()
            .is_finite()
    );
}

#[test]
fn materialize_normalized_float_preserves_bits32_subnormal_and_signed_zero() {
    // Smallest positive f32 subnormal survives the Bits32 path.
    let subnormal = materialize_normalized_float("1e-45", false, FloatPrecision::Bits32).unwrap();
    assert_eq!(subnormal, f64::from(1e-45f32));
    assert!(subnormal > 0.0);

    // Signed zero keeps its sign bit through negation.
    let negative_zero = materialize_normalized_float("0.0", true, FloatPrecision::Bits32).unwrap();
    assert!(negative_zero.is_sign_negative());
    assert_eq!(negative_zero, 0.0);
}

#[test]
fn parse_numeric_text_to_float_accepts_float_grammar_cases() {
    let cases = [
        ("1", 1.0),
        ("1.5", 1.5),
        ("-1.5", -1.5),
        ("1e6", 1e6),
        ("1e+21", 1e21),
        ("1e-6", 1e-6),
        ("1_000.5e-2", 1000.5e-2),
    ];

    for (source, expected) in cases {
        let result = parse_numeric_text_to_float(source, FloatPrecision::Bits64).expect(source);
        assert_eq!(result, expected, "{source}");
    }
}

#[test]
fn parse_numeric_text_to_float_rounds_directly_at_bits32() {
    assert_eq!(
        parse_numeric_text_to_float("0.1", FloatPrecision::Bits32).unwrap(),
        f64::from(0.1f32)
    );
    assert_eq!(
        parse_numeric_text_to_float("3.5e38", FloatPrecision::Bits32).unwrap_err(),
        NumberLiteralErrorReason::NonFiniteFloat
    );

    let negative_zero = parse_numeric_text_to_float("-0.0", FloatPrecision::Bits32).unwrap();
    assert!(negative_zero.is_sign_negative());
}

#[test]
fn parse_numeric_text_to_float_rejects_invalid_grammar_cases() {
    let invalid_cases = [
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

    for source in invalid_cases {
        let reason = parse_numeric_text_to_float(source, FloatPrecision::Bits64).expect_err(source);
        assert!(
            !matches!(reason, NumberLiteralErrorReason::NonFiniteFloat),
            "{source} should fail as invalid grammar, not as non-finite"
        );
    }
}

#[test]
fn parse_numeric_text_to_float_rejects_non_finite_materialization() {
    let reason = parse_numeric_text_to_float("1e10000", FloatPrecision::Bits64)
        .expect_err("should be non-finite");
    assert_eq!(reason, NumberLiteralErrorReason::NonFiniteFloat);
}

#[test]
fn literal_kind_initialises_matches_direct_receiving_boundaries() {
    for scalar in FixedScalar::ALL {
        for kind in [
            NumericLiteralKind::WholeNumber,
            NumericLiteralKind::DecimalPoint,
            NumericLiteralKind::Exponent,
        ] {
            let expected = kind == NumericLiteralKind::WholeNumber
                || scalar.class() == FixedScalarClass::BinaryFloat;

            assert_eq!(
                literal_kind_initialises(kind, scalar),
                expected,
                "{kind:?} into {}",
                scalar.name()
            );
        }
    }
}

#[test]
fn materialize_fixed_scalar_accepts_signed_endpoint_values() {
    let mut string_table = StringTable::new();

    // A signed width accepts its exact minimum from the negative magnitude, so the magnitude one
    // above its maximum must materialise without an `Int` intermediate rejecting it first.
    let minima = [
        (FixedScalar::I8, "128", -128_i64),
        (FixedScalar::I16, "32768", -32768),
        (FixedScalar::I32, "2147483648", i64::from(i32::MIN)),
        (FixedScalar::I64, "9223372036854775808", i64::MIN),
    ];

    for (scalar, magnitude, expected) in minima {
        let sign = NumericLiteralSign::Negative;
        let token = literal_token(magnitude, sign, &mut string_table);
        let value = materialize_fixed_scalar(&token, sign, scalar, &string_table)
            .unwrap_or_else(|reason| panic!("-{magnitude} into {}: {reason:?}", scalar.name()));

        assert_eq!(value.scalar(), scalar);
        assert_eq!(value.as_i64(), Some(expected), "-{magnitude}");
    }

    let maxima = [
        (FixedScalar::I8, "127", 127_i64),
        (FixedScalar::I16, "32767", 32767),
        (FixedScalar::I32, "2147483647", i64::from(i32::MAX)),
        (FixedScalar::I64, "9223372036854775807", i64::MAX),
    ];

    for (scalar, magnitude, expected) in maxima {
        let token = literal_token(magnitude, NumericLiteralSign::Positive, &mut string_table);

        assert_eq!(
            materialize_fixed_scalar(&token, token.sign, scalar, &string_table)
                .unwrap_or_else(|reason| panic!("{magnitude} into {}: {reason:?}", scalar.name()))
                .as_i64(),
            Some(expected),
            "{magnitude}"
        );
    }

    // A negative zero is a plain zero once the sign is applied.
    let negative_zero = literal_token("0", NumericLiteralSign::Negative, &mut string_table);

    assert_eq!(
        materialize_fixed_scalar(
            &negative_zero,
            negative_zero.sign,
            FixedScalar::I8,
            &string_table
        )
        .unwrap()
        .as_i64(),
        Some(0)
    );
}

#[test]
fn materialize_fixed_scalar_rejects_signed_out_of_range_values() {
    let mut string_table = StringTable::new();

    let cases = [
        (FixedScalar::I8, "128", NumericLiteralSign::Positive),
        (FixedScalar::I8, "129", NumericLiteralSign::Negative),
        (FixedScalar::I16, "32768", NumericLiteralSign::Positive),
        (FixedScalar::I16, "32769", NumericLiteralSign::Negative),
        (FixedScalar::I32, "2147483648", NumericLiteralSign::Positive),
        (FixedScalar::I32, "2147483649", NumericLiteralSign::Negative),
        (
            FixedScalar::I64,
            "9223372036854775808",
            NumericLiteralSign::Positive,
        ),
        (
            FixedScalar::I64,
            "9223372036854775809",
            NumericLiteralSign::Negative,
        ),
        (
            FixedScalar::I64,
            "99999999999999999999999",
            NumericLiteralSign::Positive,
        ),
    ];

    for (scalar, magnitude, sign) in cases {
        let token = literal_token(magnitude, sign, &mut string_table);
        let reason = materialize_fixed_scalar(&token, sign, scalar, &string_table).unwrap_err();

        assert_eq!(
            reason,
            NumberLiteralErrorReason::OutsideFixedScalarRange(scalar),
            "{sign:?}{magnitude} into {}",
            scalar.name()
        );
    }
}

#[test]
fn materialize_fixed_scalar_accepts_unsigned_endpoint_values() {
    let mut string_table = StringTable::new();

    let cases = [
        (FixedScalar::U8, "0", 0_u64),
        (FixedScalar::U8, "255", 255),
        (FixedScalar::U16, "65535", u64::from(u16::MAX)),
        (FixedScalar::U32, "4294967295", u64::from(u32::MAX)),
        (FixedScalar::U64, "18446744073709551615", u64::MAX),
        (FixedScalar::Byte, "0", 0),
        (FixedScalar::Byte, "255", 255),
    ];

    for (scalar, magnitude, expected) in cases {
        let token = literal_token(magnitude, NumericLiteralSign::Positive, &mut string_table);
        let value = materialize_fixed_scalar(&token, token.sign, scalar, &string_table)
            .unwrap_or_else(|reason| panic!("{magnitude} into {}: {reason:?}", scalar.name()));

        assert_eq!(value.scalar(), scalar);
        assert_eq!(value.as_u64(), Some(expected), "{magnitude}");
    }
}

#[test]
fn materialize_fixed_scalar_keeps_unsigned_values_above_f64_exact_range() {
    let mut string_table = StringTable::new();

    // 2^53 + 1 and 2^63 + 1 have no exact binary float spelling, so they prove the literal never
    // passed through an `f64` intermediate.
    for magnitude in [
        "9007199254740993",
        "9223372036854775809",
        "18000000000000000000",
    ] {
        let token = literal_token(magnitude, NumericLiteralSign::Positive, &mut string_table);
        let value = materialize_fixed_scalar(&token, token.sign, FixedScalar::U64, &string_table)
            .unwrap_or_else(|reason| panic!("{magnitude}: {reason:?}"));

        assert_eq!(
            value.as_u64(),
            Some(magnitude.parse::<u64>().unwrap()),
            "{magnitude}"
        );
    }
}

#[test]
fn materialize_fixed_scalar_rejects_unsigned_out_of_range_values() {
    let mut string_table = StringTable::new();

    let cases = [
        (FixedScalar::U8, "256"),
        (FixedScalar::U16, "65536"),
        (FixedScalar::U32, "4294967296"),
        (FixedScalar::U64, "18446744073709551616"),
        (FixedScalar::Byte, "256"),
    ];

    for (scalar, magnitude) in cases {
        let token = literal_token(magnitude, NumericLiteralSign::Positive, &mut string_table);
        let reason =
            materialize_fixed_scalar(&token, token.sign, scalar, &string_table).unwrap_err();

        assert_eq!(
            reason,
            NumberLiteralErrorReason::OutsideFixedScalarRange(scalar),
            "{magnitude} into {}",
            scalar.name()
        );
    }
}

#[test]
fn materialize_fixed_scalar_rejects_negative_values_for_unsigned_and_byte() {
    let mut string_table = StringTable::new();

    let unsigned_scalars = [
        FixedScalar::U8,
        FixedScalar::U16,
        FixedScalar::U32,
        FixedScalar::U64,
        FixedScalar::Byte,
    ];

    for scalar in unsigned_scalars {
        // `-0` is refused too: the sign belongs to the literal, so an unsigned destination never
        // accepts a negatively spelled literal.
        for magnitude in ["1", "0", "18446744073709551615"] {
            let sign = NumericLiteralSign::Negative;
            let token = literal_token(magnitude, sign, &mut string_table);
            let reason = materialize_fixed_scalar(&token, sign, scalar, &string_table).unwrap_err();

            assert_eq!(
                reason,
                NumberLiteralErrorReason::NegativeUnsignedLiteral(scalar),
                "-{magnitude} into {}",
                scalar.name()
            );
        }
    }
}

#[test]
fn materialize_fixed_scalar_rounds_binary_floats_at_their_destination() {
    let mut string_table = StringTable::new();

    // 0.1 is inexact in binary; each precision must equal its own single rounding.
    let decimal = literal_token("0.1", NumericLiteralSign::Positive, &mut string_table);

    assert_eq!(
        materialize_fixed_scalar(&decimal, decimal.sign, FixedScalar::F32, &string_table)
            .unwrap()
            .as_f64(),
        Some(f64::from(0.1f32))
    );
    assert_eq!(
        materialize_fixed_scalar(&decimal, decimal.sign, FixedScalar::F64, &string_table)
            .unwrap()
            .as_f64(),
        Some(0.1)
    );

    // A whole literal may initialise a binary float through the same rounding.
    let whole = literal_token("1", NumericLiteralSign::Positive, &mut string_table);

    assert_eq!(
        materialize_fixed_scalar(&whole, whole.sign, FixedScalar::F16, &string_table)
            .unwrap()
            .as_f64(),
        Some(1.0)
    );

    let cases = [
        ("0.5", 0.5),
        ("65504", 65504.0),
        ("65519.99", 65504.0),
        ("5.960464477539063e-8", 2f64.powi(-24)),
        ("1.0004882812500002", 1.0009765625),
        ("1.00048828125000000000001", 1.0009765625),
        // Exactly the midpoint between 1.0 and 1.0009765625, so ties-to-even keeps 1.0.
        ("1.00048828125", 1.0),
        ("1.00048828124999999999999", 1.0),
    ];

    for (text, expected) in cases {
        let token = literal_token(text, NumericLiteralSign::Positive, &mut string_table);
        let value = materialize_fixed_scalar(&token, token.sign, FixedScalar::F16, &string_table)
            .unwrap_or_else(|reason| panic!("{text} into F16: {reason:?}"));

        assert_eq!(value.scalar(), FixedScalar::F16);
        assert_eq!(value.as_f64(), Some(expected), "{text}");
    }

    for text in ["65520", "70000"] {
        let token = literal_token(text, NumericLiteralSign::Positive, &mut string_table);

        assert_eq!(
            materialize_fixed_scalar(&token, token.sign, FixedScalar::F16, &string_table)
                .unwrap_err(),
            NumberLiteralErrorReason::NonFiniteFixedFloat(FixedScalar::F16),
            "{text} rounds above the largest finite F16 value"
        );
    }

    // 1e-10 underflows to zero, and the sign survives the underflow.
    for sign in [NumericLiteralSign::Positive, NumericLiteralSign::Negative] {
        let token = literal_token("1e-10", sign, &mut string_table);
        let value =
            materialize_fixed_scalar(&token, sign, FixedScalar::F16, &string_table).unwrap();
        let rounded = value.as_f64().unwrap();

        assert_eq!(rounded, 0.0);
        assert_eq!(
            rounded.is_sign_negative(),
            sign == NumericLiteralSign::Negative
        );
    }

    // Signed zero keeps its sign bit at every binary float destination.
    let negative_zero = literal_token("0.0", NumericLiteralSign::Negative, &mut string_table);

    assert!(
        materialize_fixed_scalar(
            &negative_zero,
            negative_zero.sign,
            FixedScalar::F32,
            &string_table
        )
        .unwrap()
        .as_f64()
        .unwrap()
        .is_sign_negative()
    );
}
#[test]
fn materialize_fixed_scalar_rounds_f32_midpoint_from_authored_digits() {
    let mut string_table = StringTable::new();

    // 1 + 2^-24 is the F32 midpoint above 1.0 whose even neighbour is 1.0.
    // The nearest f64 to this decimal is exactly that midpoint, so a second
    // rounding through f64 would keep 1.0 while the single destination rounding
    // must follow the authored digits above the midpoint.
    let above = literal_token(
        "1.0000000596046449",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    let rounded = materialize_fixed_scalar(&above, above.sign, FixedScalar::F32, &string_table)
        .expect("digits above the midpoint should round up");

    assert_eq!(
        rounded.as_f64(),
        Some(f64::from(1.0f32) + f64::from(2f32.powi(-23))),
        "authored digits above the F32 midpoint must round up, not to even"
    );

    let midpoint = literal_token(
        "1.000000059604644775390625",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    let tied = materialize_fixed_scalar(&midpoint, midpoint.sign, FixedScalar::F32, &string_table)
        .expect("the exact midpoint should round to even");

    assert_eq!(tied.as_f64(), Some(1.0));
}

#[test]
fn materialize_fixed_scalar_rejects_f32_overflow_at_the_destination() {
    let mut string_table = StringTable::new();

    let largest = literal_token(
        "340282346638528859811704183484516925440",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    let rounded = materialize_fixed_scalar(&largest, largest.sign, FixedScalar::F32, &string_table)
        .expect("largest finite F32 should be accepted");

    assert!(rounded.as_f64().unwrap().is_finite());

    let overflow = literal_token(
        "340282357000000000000000000000000000000",
        NumericLiteralSign::Positive,
        &mut string_table,
    );

    assert_eq!(
        materialize_fixed_scalar(&overflow, overflow.sign, FixedScalar::F32, &string_table)
            .unwrap_err(),
        NumberLiteralErrorReason::NonFiniteFixedFloat(FixedScalar::F32),
        "smallest decimal rounding to F32 infinity should be rejected"
    );
}

fn whole_number_token(
    text: &str,
    sign: NumericLiteralSign,
    string_table: &mut StringTable,
) -> NumericLiteralToken {
    let normalized_text = string_table.intern(text);
    // For signed tokens the source_text includes the sign prefix.
    let source = match sign {
        NumericLiteralSign::Positive => text.to_owned(),
        NumericLiteralSign::Negative => format!("-{text}"),
    };
    let source_text = string_table.intern(&source);

    NumericLiteralToken::new(
        sign,
        source_text,
        normalized_text,
        NumericLiteralKind::WholeNumber,
        text.chars().filter(|c| c.is_ascii_digit()).count() as u32,
        0,
        0,
        NumericExponentSign::None,
    )
}

/// Build a literal token for any unsigned numeric spelling, including decimals and exponents.
///
/// WHY: fixed scalar materialisation must see the same lexical metadata the tokenizer produces, so
/// these tests take their kind and digit counts from the grammar owner instead of inventing them.
fn literal_token(
    text: &str,
    sign: NumericLiteralSign,
    string_table: &mut StringTable,
) -> NumericLiteralToken {
    let parsed = parse_numeric_literal(text)
        .unwrap_or_else(|reason| panic!("test literal '{text}' is invalid: {reason:?}"));
    let source = match sign {
        NumericLiteralSign::Positive => text.to_owned(),
        NumericLiteralSign::Negative => format!("-{text}"),
    };

    NumericLiteralToken::new(
        sign,
        string_table.intern(&source),
        string_table.intern(&parsed.normalized_text),
        parsed.kind,
        parsed.digit_count,
        parsed.fractional_digit_count,
        parsed.exponent_digit_count,
        parsed.exponent_sign,
    )
}
