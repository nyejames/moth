//! Numeric text grammar tests.
//!
//! WHAT: validates the shared parser before tokens are materialized by AST or future casts.
//! WHY: separator, exponent, and normalization rules need one owner so tokenizer and string casts
//!      cannot drift into subtly different grammars.

use super::*;
use crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason;
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
