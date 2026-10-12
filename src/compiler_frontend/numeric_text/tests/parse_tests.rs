//! Numeric token and `StringTable` adapter tests.
//!
//! WHAT: verifies that compiler tokens resolve to the normalized spelling materializers consume.
//! WHY: AST and cast boundaries must share the parser's destination rules without owning its
//!      grammar.

use super::*;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::grammar::{NumericExponentSign, NumericLiteralKind, NumericLiteralSign};
use moth_lexical::numeric::parse::{NumberLiteralErrorReason, parse_numeric_literal};

fn signed_value(scalar: FixedScalar, value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(scalar, value).expect("test value fits the scalar")
}

#[test]
fn materialize_fixed_scalar_accepts_i32_boundary_values() {
    let mut string_table = StringTable::new();

    let max_token = whole_number_token(
        "2147483647",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(&max_token, max_token.sign, FixedScalar::I32, &string_table)
            .unwrap(),
        signed_value(FixedScalar::I32, i64::from(i32::MAX))
    );

    let min_token = whole_number_token(
        "2147483648",
        NumericLiteralSign::Negative,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(&min_token, min_token.sign, FixedScalar::I32, &string_table)
            .unwrap(),
        signed_value(FixedScalar::I32, i64::from(i32::MIN))
    );
}

#[test]
fn materialize_fixed_scalar_rejects_i32_out_of_range_values() {
    let mut string_table = StringTable::new();

    let too_large = whole_number_token(
        "2147483648",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(&too_large, too_large.sign, FixedScalar::I32, &string_table)
            .unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32)
    );

    let too_negative = whole_number_token(
        "2147483649",
        NumericLiteralSign::Negative,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(
            &too_negative,
            too_negative.sign,
            FixedScalar::I32,
            &string_table
        )
        .unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32)
    );
}

#[test]
fn materialize_fixed_scalar_with_sign_override_allows_negative_fallback_boundary() {
    let mut string_table = StringTable::new();

    let positive_token = whole_number_token(
        "2147483648",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(
            &positive_token,
            NumericLiteralSign::Negative,
            FixedScalar::I32,
            &string_table
        )
        .unwrap(),
        signed_value(FixedScalar::I32, i64::from(i32::MIN))
    );
}

#[test]
fn materialize_fixed_scalar_accepts_i64_boundaries() {
    let mut string_table = StringTable::new();

    let max_token = whole_number_token(
        "9223372036854775807",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(&max_token, max_token.sign, FixedScalar::I64, &string_table)
            .unwrap(),
        signed_value(FixedScalar::I64, i64::MAX)
    );

    let min_token = whole_number_token(
        "9223372036854775808",
        NumericLiteralSign::Negative,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(&min_token, min_token.sign, FixedScalar::I64, &string_table)
            .unwrap(),
        signed_value(FixedScalar::I64, i64::MIN)
    );
}

#[test]
fn materialize_fixed_scalar_rejects_i64_out_of_range_values() {
    let mut string_table = StringTable::new();

    // Positive 2^63 exceeds the I64 maximum.
    let too_large = whole_number_token(
        "9223372036854775808",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(&too_large, too_large.sign, FixedScalar::I64, &string_table)
            .unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I64)
    );

    // One below the I64 minimum.
    let too_negative = whole_number_token(
        "9223372036854775809",
        NumericLiteralSign::Negative,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(
            &too_negative,
            too_negative.sign,
            FixedScalar::I64,
            &string_table
        )
        .unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I64)
    );
}

#[test]
fn materialize_fixed_scalar_distinguishes_i32_and_i64_ranges() {
    let mut string_table = StringTable::new();

    // 2^31 fits I64 but not I32.
    let wider_token = whole_number_token(
        "2147483648",
        NumericLiteralSign::Positive,
        &mut string_table,
    );
    assert_eq!(
        materialize_fixed_scalar(
            &wider_token,
            wider_token.sign,
            FixedScalar::I64,
            &string_table
        )
        .unwrap(),
        signed_value(FixedScalar::I64, 2_147_483_648)
    );
    assert_eq!(
        materialize_fixed_scalar(
            &wider_token,
            wider_token.sign,
            FixedScalar::I32,
            &string_table
        )
        .unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32)
    );
}

#[test]
fn materialize_fixed_scalar_rejects_non_finite_f64_spelling() {
    let mut string_table = StringTable::new();

    let token = literal_token("1e309", NumericLiteralSign::Positive, &mut string_table);

    assert_eq!(
        materialize_fixed_scalar(&token, token.sign, FixedScalar::F64, &string_table).unwrap_err(),
        NumberLiteralErrorReason::NonFiniteFixedFloat(FixedScalar::F64)
    );
}

#[test]
fn materialize_fixed_scalar_applies_negative_sign_to_binary_float() {
    let mut string_table = StringTable::new();

    let token = literal_token("1.5e2", NumericLiteralSign::Negative, &mut string_table);

    assert_eq!(
        materialize_fixed_scalar(&token, token.sign, FixedScalar::F64, &string_table)
            .unwrap()
            .as_f64(),
        Some(-150.0)
    );
}

#[test]
fn materialize_fixed_scalar_rejects_f32_overflow_while_f64_stays_finite() {
    let mut string_table = StringTable::new();

    // 3.5e38 fits in f64 but overflows f32, so the destination decides finiteness.
    let token = literal_token("3.5e38", NumericLiteralSign::Positive, &mut string_table);

    assert_eq!(
        materialize_fixed_scalar(&token, token.sign, FixedScalar::F32, &string_table).unwrap_err(),
        NumberLiteralErrorReason::NonFiniteFixedFloat(FixedScalar::F32)
    );
    assert!(
        materialize_fixed_scalar(&token, token.sign, FixedScalar::F64, &string_table)
            .unwrap()
            .as_f64()
            .unwrap()
            .is_finite()
    );
}

#[test]
fn materialize_fixed_scalar_accepts_signed_endpoint_values() {
    let mut string_table = StringTable::new();

    // A signed width accepts its exact minimum from the negative magnitude, so the magnitude one
    // above its maximum must materialise without a narrower signed intermediate rejecting it first.
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

#[test]
fn parse_numeric_text_to_number_accepts_exact_decimal_and_exponent_values() {
    let scale_zero = NumberScale::new(0).expect("zero is valid");
    let scale_two = NumberScale::new(2).expect("two is valid");

    let whole = parse_numeric_text_to_number("1.25e2", scale_zero)
        .expect("integral decimal/exponent spelling is exact at Dec0");
    assert_eq!(whole.to_string(), "125");
    assert_eq!(whole.scale(), scale_zero);

    let fractional = parse_numeric_text_to_number("-1.23e1", scale_two)
        .expect("negative exponent/decimal spelling is exact at Dec2");
    assert_eq!(fractional.to_string(), "-12.3");
    assert_eq!(fractional.scale(), scale_two);
}

#[test]
fn parse_numeric_text_to_number_reports_inexact_destination_scale() {
    let scale_two = NumberScale::new(2).expect("two is valid");
    assert_eq!(
        parse_numeric_text_to_number("1.234", scale_two),
        Err(NumberLiteralErrorReason::InexactNumberScale(scale_two))
    );
}

#[test]
fn parse_numeric_text_to_number_keeps_zero_huge_exponents_fast() {
    let scale_zero = NumberScale::new(0).expect("zero is valid");
    let parsed = parse_numeric_text_to_number(
        "0e999999999999999999999999999999999999999999999999999999",
        scale_zero,
    )
    .expect("zero does not expand its exponent");
    assert_eq!(parsed.to_string(), "0");
    assert_eq!(parsed.scale(), scale_zero);
}

#[test]
fn parse_numeric_text_to_number_reports_actual_host_capacity_exhaustion() {
    let scale_zero = NumberScale::new(0).expect("zero is valid");
    assert_eq!(
        parse_numeric_text_to_number(
            "1e999999999999999999999999999999999999999999999999999999",
            scale_zero,
        ),
        Err(NumberLiteralErrorReason::ParseOverflow)
    );
}

#[test]
fn parse_numeric_text_to_number_uses_the_complete_shared_grammar() {
    let scale_two = NumberScale::new(2).expect("two is valid");

    for invalid in ["", " 1", "1 ", "+1", "1E2", "1__0", "_1", "1_", "--1"] {
        assert!(
            parse_numeric_text_to_number(invalid, scale_two).is_err(),
            "{invalid:?} must be rejected by the numeric text grammar"
        );
    }
}
