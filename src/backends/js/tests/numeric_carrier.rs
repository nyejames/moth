use crate::backends::js::numeric_carrier::{JsNumericCarrier, JsNumericConversion};
use crate::compiler_frontend::builtins::casts::evidence::numeric_scalars;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;

#[test]
fn carrier_selection_follows_the_exact_scalar_range() {
    // Every bounded identity carries its own width, so carrier selection reads only the scalar.
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::I64)),
        Some(JsNumericCarrier::BigInteger {
            min: i64::MIN as i128,
            max: i64::MAX as i128,
        })
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::F32)),
        Some(JsNumericCarrier::BinaryFloat {
            precision: BinaryFloatPrecision::Binary32,
        })
    );
    assert!(matches!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::U32)),
        Some(JsNumericCarrier::ExactInteger {
            min: 0,
            max: 4_294_967_295
        })
    ));
    assert!(matches!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::U64)),
        Some(JsNumericCarrier::BigInteger {
            min: 0,
            max: 18_446_744_073_709_551_615
        })
    ));
}

#[test]
fn fixed_literals_use_their_javascript_carrier_without_losing_bits() {
    for (value, expected) in [
        (
            FixedScalarValue::signed(FixedScalar::I8, i8::MIN.into()).unwrap(),
            "-128",
        ),
        (
            FixedScalarValue::unsigned(FixedScalar::U32, u32::MAX.into()).unwrap(),
            "4294967295",
        ),
        (
            FixedScalarValue::signed(FixedScalar::I64, i64::MIN).unwrap(),
            "-9223372036854775808n",
        ),
        (
            FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).unwrap(),
            "18446744073709551615n",
        ),
        (
            FixedScalarValue::binary_float(FixedScalar::F16, -0.0).unwrap(),
            "-0",
        ),
        (
            FixedScalarValue::binary_float(FixedScalar::F32, 1.5).unwrap(),
            "1.5",
        ),
        (
            FixedScalarValue::binary_float(FixedScalar::F64, 0.1).unwrap(),
            "0.1",
        ),
        (
            FixedScalarValue::unsigned(FixedScalar::Byte, u8::MAX.into()).unwrap(),
            "255",
        ),
    ] {
        assert_eq!(
            JsNumericCarrier::fixed_literal(value).as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn numeric_conversion_calls_only_required_helpers() {
    let scalars = numeric_scalars().collect::<Vec<_>>();

    for source in &scalars {
        for target in &scalars {
            let conversion = JsNumericConversion::classify(*source, *target)
                .expect("every canonical numeric pair has a JS conversion");
            let expression = conversion.expression("value", *source, *target);
            let required = conversion
                .required_helpers()
                .iter()
                .map(|helper| helper.function_name())
                .collect::<Vec<_>>();

            for called in expression
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .filter(|word| word.starts_with("__moth_"))
            {
                assert!(
                    required.contains(&called),
                    "{source:?} -> {target:?} calls {called}, which is not a required helper"
                );
            }
        }
    }
}

/// Verifies that each exact unsigned width selects the shared integer carrier and bounds:
/// `U32` rides exact `Number`, `U64` rides exact `BigInt`, with no unsigned-only arithmetic
/// helper family.
#[test]
fn uint_carriers_follow_the_exact_width_without_new_helpers() {
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::U32)),
        Some(JsNumericCarrier::ExactInteger {
            min: 0,
            max: 4_294_967_295
        })
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::U64)),
        Some(JsNumericCarrier::BigInteger {
            min: 0,
            max: 18_446_744_073_709_551_615
        })
    );

    // Both widths reuse the shared integer helper families and bounds.
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::U32))
            .expect("U32 has a carrier")
            .helper_family(),
        "int"
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::U64))
            .expect("U64 has a carrier")
            .helper_family(),
        "bigint"
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::U32))
            .expect("U32 has bounds")
            .integer_bounds_js(),
        Some(("0".to_owned(), "4294967295".to_owned()))
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Fixed(FixedScalar::U64))
            .expect("U64 has bounds")
            .integer_bounds_js(),
        Some(("0n".to_owned(), "18446744073709551615n".to_owned()))
    );

    // Adversarial boundary literals keep their exact carrier spelling.
    for (value, expected) in [
        (0u32, "0"),
        (2_147_483_647, "2147483647"),
        (2_147_483_648, "2147483648"),
        (4_294_967_295, "4294967295"),
    ] {
        let value = FixedScalarValue::unsigned(FixedScalar::U32, u64::from(value))
            .expect("U32 literal fits");
        assert_eq!(
            JsNumericCarrier::fixed_literal(value).as_deref(),
            Some(expected)
        );
    }
    for (value, expected) in [
        (0u64, "0n"),
        (9_223_372_036_854_775_807, "9223372036854775807n"),
        (9_223_372_036_854_775_808, "9223372036854775808n"),
        (18_446_744_073_709_551_615, "18446744073709551615n"),
    ] {
        let value = FixedScalarValue::unsigned(FixedScalar::U64, value).expect("U64 literal fits");
        assert_eq!(
            JsNumericCarrier::fixed_literal(value).as_deref(),
            Some(expected)
        );
    }
}

/// Verifies that unsigned conversions classify through the existing conversion
/// vocabulary: U32 rides `Number` rounding, U64 rides direct `BigInt` rounding at
/// the destination precision, and unsigned/signed pairs stay checked.
#[test]
fn uint_conversions_classify_through_shared_integer_conversions() {
    assert_eq!(
        JsNumericConversion::classify(
            NumericScalar::Fixed(FixedScalar::U32),
            NumericScalar::Fixed(FixedScalar::F64),
        )
        .expect("U32 to F64 classifies"),
        JsNumericConversion::RoundExactIntegerToFloat {
            precision: BinaryFloatPrecision::Binary64,
        }
    );
    assert_eq!(
        JsNumericConversion::classify(
            NumericScalar::Fixed(FixedScalar::U32),
            NumericScalar::Fixed(FixedScalar::F32),
        )
        .expect("U32 to F32 classifies"),
        JsNumericConversion::RoundExactIntegerToFloat {
            precision: BinaryFloatPrecision::Binary32,
        }
    );
    assert_eq!(
        JsNumericConversion::classify(
            NumericScalar::Fixed(FixedScalar::U64),
            NumericScalar::Fixed(FixedScalar::F64),
        )
        .expect("U64 to F64 classifies"),
        JsNumericConversion::ToNumber
    );
    assert_eq!(
        JsNumericConversion::classify(
            NumericScalar::Fixed(FixedScalar::U64),
            NumericScalar::Fixed(FixedScalar::F32),
        )
        .expect("U64 to F32 classifies"),
        JsNumericConversion::RoundBigIntegerToFloat {
            precision: BinaryFloatPrecision::Binary32,
        }
    );
    assert_eq!(
        JsNumericConversion::classify(
            NumericScalar::Fixed(FixedScalar::U32),
            NumericScalar::Fixed(FixedScalar::U64),
        )
        .expect("the wider unsigned target is an infallible carrier change"),
        JsNumericConversion::ToBigInt
    );

    for (source, target) in [
        (
            NumericScalar::Fixed(FixedScalar::U64),
            NumericScalar::Fixed(FixedScalar::I64),
        ),
        (
            NumericScalar::Fixed(FixedScalar::I64),
            NumericScalar::Fixed(FixedScalar::U64),
        ),
        (
            NumericScalar::Fixed(FixedScalar::U64),
            NumericScalar::Fixed(FixedScalar::U32),
        ),
    ] {
        assert!(
            matches!(
                JsNumericConversion::classify(source, target),
                Ok(JsNumericConversion::CheckedIntegerToInteger { .. })
            ),
            "{source:?} to {target:?} stays a checked integer cast"
        );
    }

    // U64 converts to F32 directly at destination precision, never
    // through a binary64 intermediate.
    let conversion = JsNumericConversion::classify(
        NumericScalar::Fixed(FixedScalar::U64),
        NumericScalar::Fixed(FixedScalar::F32),
    )
    .expect("U64 to F32 classifies");
    assert_eq!(
        conversion.expression(
            "value",
            NumericScalar::Fixed(FixedScalar::U64),
            NumericScalar::Fixed(FixedScalar::F32)
        ),
        "__moth_bigint_to_binary_float(value, 32)"
    );
}
