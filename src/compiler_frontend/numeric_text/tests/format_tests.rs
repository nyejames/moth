//! Tests for the Moth finite `Float` formatting contract.

use crate::compiler_frontend::numeric_text::format::{FloatFormatError, format_finite_float};

#[test]
fn one_point_zero_formats_without_decimal() {
    assert_eq!(format_finite_float(1.0).unwrap(), "1");
}

#[test]
fn one_point_five_keeps_fractional_part() {
    assert_eq!(format_finite_float(1.5).unwrap(), "1.5");
}

#[test]
fn one_millionth_uses_fixed_form() {
    assert_eq!(format_finite_float(0.000001).unwrap(), "0.000001");
}

#[test]
fn one_ten_millionth_uses_exponent_form() {
    assert_eq!(format_finite_float(0.0000001).unwrap(), "1e-7");
}

#[test]
fn one_e_twenty_one_uses_positive_signed_exponent() {
    assert_eq!(format_finite_float(1e21).unwrap(), "1e+21");
}

#[test]
fn one_e_twenty_stays_fixed_below_upper_threshold() {
    assert_eq!(format_finite_float(1e20).unwrap(), "100000000000000000000");
}

#[test]
fn negative_small_values_keep_sign_in_exponent_form() {
    assert_eq!(format_finite_float(-0.0000001).unwrap(), "-1e-7");
}

#[test]
fn negative_zero_formats_as_zero() {
    assert_eq!(format_finite_float(-0.0).unwrap(), "0");
}

#[test]
fn nan_is_rejected() {
    assert_eq!(
        format_finite_float(f64::NAN).unwrap_err(),
        FloatFormatError::NonFiniteFloat
    );
}

#[test]
fn positive_infinity_is_rejected() {
    assert_eq!(
        format_finite_float(f64::INFINITY).unwrap_err(),
        FloatFormatError::NonFiniteFloat
    );
}

#[test]
fn negative_infinity_is_rejected() {
    assert_eq!(
        format_finite_float(f64::NEG_INFINITY).unwrap_err(),
        FloatFormatError::NonFiniteFloat
    );
}

#[test]
fn negative_values_keep_leading_minus() {
    assert_eq!(format_finite_float(-1.5).unwrap(), "-1.5");
    assert_eq!(format_finite_float(-1e21).unwrap(), "-1e+21");
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
        let formatted = format_finite_float(value).unwrap();
        let parsed = formatted.parse::<f64>().unwrap();
        assert_eq!(parsed, value, "formatted text {formatted:?}");
    }
}
