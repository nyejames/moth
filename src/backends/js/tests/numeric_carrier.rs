use crate::backends::js::numeric_carrier::{JsNumericCarrier, JsNumericConversion};
use crate::compiler_frontend::builtins::casts::evidence::numeric_scalars;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

#[test]
fn carrier_selection_follows_profile_and_complete_scalar_range() {
    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };

    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Int, profile),
        Some(JsNumericCarrier::BigInteger {
            min: i64::MIN as i128,
            max: i64::MAX as i128,
        })
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Float, profile),
        Some(JsNumericCarrier::BinaryFloat {
            precision: BinaryFloatPrecision::Binary32,
        })
    );
    assert!(matches!(
        JsNumericCarrier::for_scalar(
            NumericScalar::Fixed(FixedScalar::U32),
            NumericProfile::STANDARD
        ),
        Some(JsNumericCarrier::ExactInteger {
            min: 0,
            max: 4_294_967_295
        })
    ));
    assert!(matches!(
        JsNumericCarrier::for_scalar(
            NumericScalar::Fixed(FixedScalar::U64),
            NumericProfile::STANDARD
        ),
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
            JsNumericCarrier::fixed_literal(value, NumericProfile::STANDARD).as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn numeric_conversion_calls_only_required_helpers_in_every_profile() {
    let profiles = [
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits64,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
    ];
    let scalars = numeric_scalars().collect::<Vec<_>>();

    for profile in profiles {
        for source in &scalars {
            for target in &scalars {
                let conversion = JsNumericConversion::classify(*source, *target, profile)
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
                        "{source:?} -> {target:?} under {profile} calls {called}, which is not a required helper"
                    );
                }
            }
        }
    }
}

/// Verifies that profile `Uint` selects the shared integer carriers and bounds:
/// `Uint32` rides exact `Number`, `Uint64` rides exact `BigInt`, with no
/// Uint-only arithmetic helper family.
#[test]
fn uint_carriers_follow_the_selected_int_width_without_new_helpers() {
    let profile32 = NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits64,
    };
    let profile64 = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    };

    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Uint, profile32),
        Some(JsNumericCarrier::ExactInteger {
            min: 0,
            max: 4_294_967_295
        })
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Uint, profile64),
        Some(JsNumericCarrier::BigInteger {
            min: 0,
            max: 18_446_744_073_709_551_615
        })
    );

    // Both widths reuse the shared integer helper families and bounds.
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Uint, profile32)
            .expect("Uint32 has a carrier")
            .helper_family(),
        Some("int")
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Uint, profile64)
            .expect("Uint64 has a carrier")
            .helper_family(),
        Some("bigint")
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Uint, profile32)
            .expect("Uint32 has bounds")
            .integer_bounds_js(),
        Some(("0".to_owned(), "4294967295".to_owned()))
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Uint, profile64)
            .expect("Uint64 has bounds")
            .integer_bounds_js(),
        Some(("0n".to_owned(), "18446744073709551615n".to_owned()))
    );

    // Adversarial boundary literals keep their exact carrier spelling.
    for (value, expected) in [
        (0, "0"),
        (2_147_483_647, "2147483647"),
        (2_147_483_648, "2147483648"),
        (4_294_967_295, "4294967295"),
    ] {
        assert_eq!(
            JsNumericCarrier::uint_literal(value, profile32).as_deref(),
            Some(expected)
        );
    }
    assert_eq!(
        JsNumericCarrier::uint_literal(4_294_967_296, profile32),
        None
    );
    for (value, expected) in [
        (0, "0n"),
        (9_223_372_036_854_775_807, "9223372036854775807n"),
        (9_223_372_036_854_775_808, "9223372036854775808n"),
        (18_446_744_073_709_551_615, "18446744073709551615n"),
    ] {
        assert_eq!(
            JsNumericCarrier::uint_literal(value, profile64).as_deref(),
            Some(expected)
        );
    }
}

/// Verifies that `Uint` conversions classify through the existing conversion
/// vocabulary: Uint32 rides `Number` rounding, Uint64 rides direct `BigInt`
/// rounding at the destination precision, and Uint/Int pairs stay checked.
#[test]
fn uint_conversions_classify_through_shared_integer_conversions() {
    let profiles = [
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits64,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
    ];

    for profile in profiles {
        let uint_to_float =
            JsNumericConversion::classify(NumericScalar::Uint, NumericScalar::Float, profile)
                .expect("Uint to Float classifies in every profile");
        let expected = match (profile.int_width, profile.float_precision.into()) {
            (IntWidth::Bits32, BinaryFloatPrecision::Binary32) => {
                JsNumericConversion::RoundExactIntegerToFloat {
                    precision: BinaryFloatPrecision::Binary32,
                }
            }
            (IntWidth::Bits32, _) => JsNumericConversion::RoundExactIntegerToFloat {
                precision: BinaryFloatPrecision::Binary64,
            },
            (IntWidth::Bits64, BinaryFloatPrecision::Binary32) => {
                JsNumericConversion::RoundBigIntegerToFloat {
                    precision: BinaryFloatPrecision::Binary32,
                }
            }
            (IntWidth::Bits64, _) => JsNumericConversion::ToNumber,
        };
        assert_eq!(uint_to_float, expected, "Uint to Float under {profile:?}");

        for (source, target) in [
            (NumericScalar::Uint, NumericScalar::Int),
            (NumericScalar::Int, NumericScalar::Uint),
        ] {
            assert!(
                matches!(
                    JsNumericConversion::classify(source, target, profile),
                    Ok(JsNumericConversion::CheckedIntegerToInteger { .. })
                ),
                "{source:?} to {target:?} stays a checked integer cast under {profile:?}"
            );
        }
    }

    // Uint64 converts to Float32 directly at destination precision, never
    // through a binary64 intermediate.
    let profile_float32 = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };
    let conversion =
        JsNumericConversion::classify(NumericScalar::Uint, NumericScalar::Float, profile_float32)
            .expect("Uint64 to Float32 classifies");
    assert_eq!(
        conversion.expression("value", NumericScalar::Uint, NumericScalar::Float),
        "__moth_bigint_to_binary_float(value, 32)"
    );
}
