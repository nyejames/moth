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
    // materialising at `Float` and narrowing afterwards.
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
