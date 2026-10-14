//! Tests for shared numeric spelling and destination parsing.

use super::*;
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
fn numeric_input_length_is_bounded_by_the_digit_count_capacity() {
    let boundary = u32::MAX as usize;
    assert_eq!(check_numeric_input_length(0), Ok(()));
    assert_eq!(check_numeric_input_length(boundary), Ok(()));

    // A 32-bit host cannot carry a larger input length at all.
    if let Some(too_long) = boundary.checked_add(1) {
        assert_eq!(
            check_numeric_input_length(too_long),
            Err(NumberLiteralErrorReason::ParseOverflow)
        );
        assert_eq!(
            check_numeric_input_length(usize::MAX),
            Err(NumberLiteralErrorReason::ParseOverflow)
        );
    }
}

#[test]
fn binary_float_carrier_rejects_values_not_exact_at_its_precision() {
    for scalar in [FixedScalar::F16, FixedScalar::F32] {
        for value in [0.1, -0.1, f64::MAX, f64::from_bits(1)] {
            assert_eq!(
                FixedScalarValue::binary_float(scalar, value),
                None,
                "{scalar:?}: {value:?}"
            );
        }
    }
}

#[test]
fn binary_float_carrier_preserves_exact_values_and_signed_zero() {
    let cases = [
        (FixedScalar::F16, 0.099_975_585_937_5),
        (FixedScalar::F16, 2f64.powi(-24)),
        (FixedScalar::F16, 65504.0),
        (FixedScalar::F32, f64::from(0.1f32)),
        (FixedScalar::F32, f64::from(f32::from_bits(1))),
        (FixedScalar::F32, f64::from(f32::MAX)),
        (FixedScalar::F64, f64::from_bits(1)),
        (FixedScalar::F64, f64::MAX),
    ];

    for (scalar, value) in cases {
        for signed in [value, -value, 0.0, -0.0] {
            let carrier = FixedScalarValue::binary_float(scalar, signed).unwrap();
            assert_eq!(carrier.scalar(), scalar);
            assert_eq!(carrier.as_f64().unwrap().to_bits(), signed.to_bits());
        }
    }

    for scalar in [FixedScalar::F16, FixedScalar::F32, FixedScalar::F64] {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(FixedScalarValue::binary_float(scalar, value), None);
        }
    }
    assert_eq!(FixedScalarValue::binary_float(FixedScalar::I32, 1.0), None);
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
fn materialize_normalized_signed_scalars_match_token_policy() {
    assert_eq!(
        materialize_normalized_fixed_scalar("2147483647", false, FixedScalar::I32)
            .unwrap()
            .as_i64(),
        Some(i64::from(i32::MAX))
    );
    assert_eq!(
        materialize_normalized_fixed_scalar("2147483648", true, FixedScalar::I32)
            .unwrap()
            .as_i64(),
        Some(i64::from(i32::MIN))
    );
    assert_eq!(
        materialize_normalized_fixed_scalar("9223372036854775808", true, FixedScalar::I64)
            .unwrap()
            .as_i64(),
        Some(i64::MIN)
    );
    assert_eq!(
        materialize_normalized_fixed_scalar("9223372036854775808", false, FixedScalar::I64)
            .unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I64)
    );
}

#[test]
fn parse_numeric_text_to_fixed_scalar_accepts_signed_i32_boundaries_and_separators() {
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("2_147_483_647", FixedScalar::I32)
            .unwrap()
            .as_i64(),
        Some(i64::from(i32::MAX))
    );
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("-2147483648", FixedScalar::I32)
            .unwrap()
            .as_i64(),
        Some(i64::from(i32::MIN))
    );
}

#[test]
fn parse_numeric_text_to_fixed_scalar_rejects_out_of_range_i32_values() {
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("2147483648", FixedScalar::I32).unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32)
    );
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("-2147483649", FixedScalar::I32).unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32)
    );
}

#[test]
fn parse_numeric_text_to_fixed_scalar_supports_i64_boundaries() {
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("9223372036854775807", FixedScalar::I64)
            .unwrap()
            .as_i64(),
        Some(i64::MAX)
    );
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("-9223372036854775808", FixedScalar::I64)
            .unwrap()
            .as_i64(),
        Some(i64::MIN)
    );
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("9223372036854775808", FixedScalar::I64).unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I64)
    );
    // 2^31 is accepted under I64.
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("2147483648", FixedScalar::I64)
            .unwrap()
            .as_i64(),
        Some(2_147_483_648_i64)
    );
}

#[test]
fn parse_numeric_text_to_fixed_scalar_rejects_non_whole_signed_text() {
    for source in ["1.0", "1e3", "+1", " 1", "1 ", ""] {
        let reason =
            parse_numeric_text_to_fixed_scalar(source, FixedScalar::I32).expect_err(source);
        assert_ne!(
            reason,
            NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32),
            "{source}"
        );
    }
}

#[test]
fn materialize_normalized_f32_preserves_subnormal_and_signed_zero() {
    // Smallest positive f32 subnormal survives the F32 path.
    let subnormal = materialize_normalized_fixed_scalar("1e-45", false, FixedScalar::F32)
        .unwrap()
        .as_f64()
        .unwrap();
    assert_eq!(subnormal, f64::from(1e-45f32));
    assert!(subnormal > 0.0);

    // Signed zero keeps its sign bit through negation.
    let negative_zero = materialize_normalized_fixed_scalar("0.0", true, FixedScalar::F32)
        .unwrap()
        .as_f64()
        .unwrap();
    assert!(negative_zero.is_sign_negative());
    assert_eq!(negative_zero, 0.0);
}

#[test]
fn parse_numeric_text_to_fixed_scalar_accepts_f64_grammar_cases() {
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
        let result = parse_numeric_text_to_fixed_scalar(source, FixedScalar::F64)
            .expect(source)
            .as_f64();
        assert_eq!(result, Some(expected), "{source}");
    }
}

#[test]
fn parse_numeric_text_to_fixed_scalar_rounds_directly_at_f32() {
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("0.1", FixedScalar::F32)
            .unwrap()
            .as_f64(),
        Some(f64::from(0.1f32))
    );
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("3.5e38", FixedScalar::F32).unwrap_err(),
        NumberLiteralErrorReason::NonFiniteFixedFloat(FixedScalar::F32)
    );

    let negative_zero = parse_numeric_text_to_fixed_scalar("-0.0", FixedScalar::F32)
        .unwrap()
        .as_f64()
        .unwrap();
    assert!(negative_zero.is_sign_negative());
}

#[test]
fn parse_numeric_text_to_fixed_scalar_rejects_invalid_f64_grammar_cases() {
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
        let reason =
            parse_numeric_text_to_fixed_scalar(source, FixedScalar::F64).expect_err(source);
        assert!(
            !matches!(reason, NumberLiteralErrorReason::NonFiniteFixedFloat(_)),
            "{source} should fail as invalid grammar, not as non-finite"
        );
    }
}

#[test]
fn parse_numeric_text_to_fixed_scalar_accepts_each_class() {
    // Whole-string fixed parsing is the `String -> fixed` policy's grammar, so it must accept the
    // same spellings a source literal at the destination would: separators for integers, an
    // optional leading minus, and decimal or exponent text only for binary floats.
    // Exact payloads pin both integer endpoints and destination-precision float rounding.
    let cases = [
        (
            FixedScalar::U8,
            "255",
            FixedScalarValue::unsigned(FixedScalar::U8, 255).expect("255 fits U8"),
        ),
        (
            FixedScalar::I8,
            "-128",
            FixedScalarValue::signed(FixedScalar::I8, -128).expect("-128 fits I8"),
        ),
        (
            FixedScalar::U64,
            "18446744073709551615",
            FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64::MAX fits U64"),
        ),
        (
            FixedScalar::I64,
            "-9223372036854775808",
            FixedScalarValue::signed(FixedScalar::I64, i64::MIN).expect("I64::MIN fits I64"),
        ),
        (
            FixedScalar::F16,
            "0.1",
            FixedScalarValue::binary_float(FixedScalar::F16, 0.099_975_585_937_5)
                .expect("0.1 rounds to a finite F16"),
        ),
        (
            FixedScalar::F32,
            "3.5e2",
            FixedScalarValue::binary_float(FixedScalar::F32, 350.0)
                .expect("350 is exactly representable as F32"),
        ),
    ];

    for (scalar, text, expected) in cases {
        let value = parse_numeric_text_to_fixed_scalar(text, scalar)
            .unwrap_or_else(|reason| panic!("{text:?} into {}: {reason:?}", scalar.name()));

        assert_eq!(value, expected, "{text}");
    }

    // Separators are part of the grammar, not decoration.
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("1_000", FixedScalar::U16)
            .unwrap()
            .as_u64(),
        Some(1000)
    );
}

#[test]
fn parse_numeric_text_to_fixed_scalar_rejects_out_of_range_and_wrong_families() {
    let cases = [
        // A whole number beyond the destination range.
        (
            "256",
            FixedScalar::U8,
            NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U8),
        ),
        (
            "128",
            FixedScalar::I8,
            NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I8),
        ),
        // A negative spelling has no unsigned or signed-zero reading.
        (
            "-1",
            FixedScalar::U8,
            NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::U8),
        ),
        (
            "-0",
            FixedScalar::U8,
            NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::U8),
        ),
        // Decimal and exponent spelling never reaches an integer destination.
        (
            "1.0",
            FixedScalar::U8,
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            "1e3",
            FixedScalar::U16,
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        // A magnitude at or above the binary16 overflow threshold is non-finite at F16.
        (
            "70000",
            FixedScalar::F16,
            NumberLiteralErrorReason::NonFiniteFixedFloat(FixedScalar::F16),
        ),
        // Sign and whitespace rules match the source grammar.
        (
            "+42",
            FixedScalar::U8,
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            " 42",
            FixedScalar::U8,
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            "42 ",
            FixedScalar::U8,
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            "",
            FixedScalar::U8,
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            "-",
            FixedScalar::I8,
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            "1__0",
            FixedScalar::U8,
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
    ];

    for (text, scalar, expected) in cases {
        let reason = parse_numeric_text_to_fixed_scalar(text, scalar).expect_err(&format!(
            "{text:?} into {} should be rejected",
            scalar.name()
        ));
        assert_eq!(reason, expected, "{text:?} into {}", scalar.name());
    }
}

#[test]
fn parse_numeric_text_to_fixed_scalar_rounds_binary_floats_at_the_destination() {
    // `0.1` has no exact binary16 form, so the parse must round once at `F16` instead of
    // materialising at `F64` and narrowing afterwards.
    let rounded = parse_numeric_text_to_fixed_scalar("0.1", FixedScalar::F16).unwrap();

    assert_eq!(rounded.as_f64(), Some(0.099_975_585_937_5));
    assert_ne!(rounded.as_f64(), Some(0.1));

    // A whole number that is exact at F32 and F64 stays exact in both.
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("1000000", FixedScalar::F32)
            .unwrap()
            .as_f64(),
        Some(1000000.0)
    );
}

#[test]
fn parse_numeric_text_to_fixed_scalar_rejects_non_finite_f64() {
    let reason = parse_numeric_text_to_fixed_scalar("1e10000", FixedScalar::F64)
        .expect_err("should be non-finite");
    assert_eq!(
        reason,
        NumberLiteralErrorReason::NonFiniteFixedFloat(FixedScalar::F64)
    );
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
fn materialize_normalized_unsigned_accepts_both_unsigned_maxima() {
    assert_eq!(
        materialize_normalized_fixed_scalar("4294967295", false, FixedScalar::U32)
            .unwrap()
            .as_u64(),
        Some(u64::from(u32::MAX))
    );
    assert_eq!(
        materialize_normalized_fixed_scalar("18446744073709551615", false, FixedScalar::U64)
            .unwrap()
            .as_u64(),
        Some(u64::MAX)
    );
    // Zero materialises without a sign.
    assert_eq!(
        materialize_normalized_fixed_scalar("0", false, FixedScalar::U32)
            .unwrap()
            .as_u64(),
        Some(0)
    );
}

#[test]
fn materialize_normalized_unsigned_rejects_maxima_plus_one_as_text() {
    assert_eq!(
        materialize_normalized_fixed_scalar("4294967296", false, FixedScalar::U32).unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U32)
    );
    assert_eq!(
        materialize_normalized_fixed_scalar("18446744073709551616", false, FixedScalar::U64)
            .unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U64)
    );
    // A 32-bit maximum still fits under U64.
    assert_eq!(
        materialize_normalized_fixed_scalar("4294967295", false, FixedScalar::U64)
            .unwrap()
            .as_u64(),
        Some(u64::from(u32::MAX))
    );
}

#[test]
fn materialize_normalized_unsigned_rejects_negative_spellings_including_minus_zero() {
    for normalized in ["1", "0", "00", "4294967295"] {
        assert_eq!(
            materialize_normalized_fixed_scalar(normalized, true, FixedScalar::U32).unwrap_err(),
            NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::U32),
            "{normalized}"
        );
    }
    assert_eq!(
        materialize_normalized_fixed_scalar("0", true, FixedScalar::U64).unwrap_err(),
        NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::U64)
    );
}

#[test]
fn parse_numeric_text_to_fixed_scalar_accepts_unsigned_boundaries_and_separators() {
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("4_294_967_295", FixedScalar::U32)
            .unwrap()
            .as_u64(),
        Some(u64::from(u32::MAX))
    );
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("18_446_744_073_709_551_615", FixedScalar::U64)
            .unwrap()
            .as_u64(),
        Some(u64::MAX)
    );
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("9_223_372_036_854_775_808", FixedScalar::U64)
            .unwrap()
            .as_u64(),
        Some(i64::MAX as u64 + 1),
        "values above i64::MAX stay exact instead of narrowing through a signed type"
    );
}

#[test]
fn parse_numeric_text_to_fixed_scalar_rejects_unsigned_out_of_range_values() {
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("4294967296", FixedScalar::U32).unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U32)
    );
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("18446744073709551616", FixedScalar::U64).unwrap_err(),
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U64)
    );
    // 2^31 is accepted under U64.
    assert_eq!(
        parse_numeric_text_to_fixed_scalar("2147483648", FixedScalar::U64)
            .unwrap()
            .as_u64(),
        Some(2_147_483_648_u64)
    );
}

#[test]
fn parse_numeric_text_to_fixed_scalar_rejects_negative_and_non_whole_unsigned_text() {
    for source in ["-1", "-0", "-4294967295"] {
        assert_eq!(
            parse_numeric_text_to_fixed_scalar(source, FixedScalar::U32).unwrap_err(),
            NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::U32),
            "{source}"
        );
    }
    for source in ["1.0", "1e3", "+1", " 1", "1 ", ""] {
        let reason =
            parse_numeric_text_to_fixed_scalar(source, FixedScalar::U32).expect_err(source);
        assert!(
            !matches!(
                reason,
                NumberLiteralErrorReason::OutsideFixedScalarRange(_)
                    | NumberLiteralErrorReason::NegativeUnsignedLiteral(_)
            ),
            "{source} should fail as invalid grammar, not as a range error"
        );
    }
}
