//! Tests for the Moth finite `Float` formatting contract.

use crate::numeric::binary16::decimal_to_f16;
use crate::numeric::format::{FloatFormatError, format_finite_float};
use crate::numeric::precision::BinaryFloatPrecision;

#[test]
fn one_point_zero_formats_without_decimal() {
    assert_eq!(
        format_finite_float(1.0, BinaryFloatPrecision::Binary64).unwrap(),
        "1"
    );
}

#[test]
fn one_point_five_keeps_fractional_part() {
    assert_eq!(
        format_finite_float(1.5, BinaryFloatPrecision::Binary64).unwrap(),
        "1.5"
    );
}

#[test]
fn one_millionth_uses_fixed_form() {
    assert_eq!(
        format_finite_float(0.000001, BinaryFloatPrecision::Binary64).unwrap(),
        "0.000001"
    );
}

#[test]
fn one_ten_millionth_uses_exponent_form() {
    assert_eq!(
        format_finite_float(0.0000001, BinaryFloatPrecision::Binary64).unwrap(),
        "1e-7"
    );
}

#[test]
fn one_e_twenty_one_uses_positive_signed_exponent() {
    assert_eq!(
        format_finite_float(1e21, BinaryFloatPrecision::Binary64).unwrap(),
        "1e+21"
    );
}

#[test]
fn one_e_twenty_stays_fixed_below_upper_threshold() {
    assert_eq!(
        format_finite_float(1e20, BinaryFloatPrecision::Binary64).unwrap(),
        "100000000000000000000"
    );
}

#[test]
fn negative_small_values_keep_sign_in_exponent_form() {
    assert_eq!(
        format_finite_float(-0.0000001, BinaryFloatPrecision::Binary64).unwrap(),
        "-1e-7"
    );
}

#[test]
fn negative_zero_formats_as_zero() {
    assert_eq!(
        format_finite_float(-0.0, BinaryFloatPrecision::Binary64).unwrap(),
        "0"
    );
}

#[test]
fn nan_is_rejected() {
    assert_eq!(
        format_finite_float(f64::NAN, BinaryFloatPrecision::Binary64).unwrap_err(),
        FloatFormatError::NonFiniteFloat
    );
}

#[test]
fn positive_infinity_is_rejected() {
    assert_eq!(
        format_finite_float(f64::INFINITY, BinaryFloatPrecision::Binary64).unwrap_err(),
        FloatFormatError::NonFiniteFloat
    );
}

#[test]
fn negative_infinity_is_rejected() {
    assert_eq!(
        format_finite_float(f64::NEG_INFINITY, BinaryFloatPrecision::Binary64).unwrap_err(),
        FloatFormatError::NonFiniteFloat
    );
}

#[test]
fn negative_values_keep_leading_minus() {
    assert_eq!(
        format_finite_float(-1.5, BinaryFloatPrecision::Binary64).unwrap(),
        "-1.5"
    );
    assert_eq!(
        format_finite_float(-1e21, BinaryFloatPrecision::Binary64).unwrap(),
        "-1e+21"
    );
}

#[test]
fn representative_values_round_trip() {
    let values = [
        1.0,
        1.5,
        0.000001,
        0.0000001,
        1e20,
        1e21,
        -0.0000001,
        std::f64::consts::PI,
    ];

    for value in values {
        let formatted = format_finite_float(value, BinaryFloatPrecision::Binary64).unwrap();
        let parsed = formatted.parse::<f64>().unwrap();
        assert_eq!(parsed, value, "formatted text {formatted:?}");
    }
}

#[test]
fn bits32_formats_shortest_f32_text() {
    // The carrier holds the single f32 rounding of 0.1; Bits32 prints its shortest text.
    assert_eq!(
        format_finite_float(f64::from(0.1f32), BinaryFloatPrecision::Binary32).unwrap(),
        "0.1"
    );
}

#[test]
fn bits64_keeps_long_f64_expansion_of_f32_value() {
    // Same carrier under Bits64 prints the exact f64 expansion, which differs from "0.1".
    let bits64_text =
        format_finite_float(f64::from(0.1f32), BinaryFloatPrecision::Binary64).unwrap();
    assert_ne!(bits64_text, "0.1");
    assert_eq!(bits64_text.parse::<f64>().unwrap(), f64::from(0.1f32));
}

#[test]
fn binary16_formats_the_shortest_round_tripping_text() {
    // 65504 is the largest finite binary16. "65500" is the shortest text that rounds back to it,
    // matching the shortest round-trip rule Ryū applies to f32 and f64.
    assert_eq!(
        format_finite_float(65504.0, BinaryFloatPrecision::Binary16).unwrap(),
        "65500"
    );

    // The smallest subnormal has no short exact decimal, so its shortest round-tripping text wins
    // and renders in exponent form under the Moth thresholds.
    let subnormal = format_finite_float(5.960_464_477_539_063e-8, BinaryFloatPrecision::Binary16)
        .expect("subnormal binary16 has a finite decimal");
    assert_eq!(subnormal, "6e-8");

    // At a power of two the gap below is half the gap above, so the nearest four-digit decimal
    // (0.01562) falls outside the rounding interval while its upper neighbour (0.01563) lies inside.
    assert_eq!(
        format_finite_float(2f64.powi(-6), BinaryFloatPrecision::Binary16).unwrap(),
        "0.01563"
    );
}

#[test]
fn binary16_text_round_trips_for_every_finite_value() {
    for bits in 1_u32..0x7C00 {
        let exponent = (bits >> 10) as i32;
        let fraction = f64::from(bits & 0x3FF);
        let value = if exponent == 0 {
            fraction * 2f64.powi(-24)
        } else {
            (1024.0 + fraction) * 2f64.powi(exponent - 25)
        };

        let text = format_finite_float(value, BinaryFloatPrecision::Binary16).unwrap();
        assert_eq!(
            decimal_to_f16(&text, false),
            Some(value),
            "binary16 bits {bits:#06x} formatted as {text:?}"
        );
    }
}

#[test]
fn binary16_formats_shortest_text_for_a_rounded_value() {
    // The nearest binary16 to 0.1 is 0.0999755859375; its shortest round-trippable text is "0.1",
    // matching the binary32 case rather than printing the full expansion.
    assert_eq!(
        format_finite_float(0.099_975_585_937_5, BinaryFloatPrecision::Binary16).unwrap(),
        "0.1"
    );

    // Negative zero and ordinary values follow the shared contract.
    assert_eq!(
        format_finite_float(-0.0, BinaryFloatPrecision::Binary16).unwrap(),
        "0"
    );
    assert_eq!(
        format_finite_float(-1.5, BinaryFloatPrecision::Binary16).unwrap(),
        "-1.5"
    );
}

#[test]
fn rejects_carriers_not_exact_at_the_formatting_precision() {
    for precision in [
        BinaryFloatPrecision::Binary16,
        BinaryFloatPrecision::Binary32,
    ] {
        for value in [0.1, -0.1, f64::MAX, f64::from_bits(1)] {
            assert_eq!(
                format_finite_float(value, precision),
                Err(FloatFormatError::InexactPrecision(precision)),
                "{precision:?}: {value:?}"
            );
        }
    }
}

#[test]
fn exact_carriers_format_at_each_precision_without_losing_subnormals() {
    let cases = [
        (BinaryFloatPrecision::Binary16, 2f64.powi(-24), "6e-8"),
        (
            BinaryFloatPrecision::Binary32,
            f64::from(f32::from_bits(1)),
            "1e-45",
        ),
        (BinaryFloatPrecision::Binary64, f64::from_bits(1), "5e-324"),
    ];

    for (precision, value, expected) in cases {
        assert_eq!(format_finite_float(value, precision).unwrap(), expected);
        assert_eq!(
            format_finite_float(-value, precision).unwrap(),
            format!("-{expected}")
        );
        for zero in [0.0, -0.0] {
            assert_eq!(format_finite_float(zero, precision).unwrap(), "0");
        }
        for non_finite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                format_finite_float(non_finite, precision),
                Err(FloatFormatError::NonFiniteFloat)
            );
        }
    }
}
