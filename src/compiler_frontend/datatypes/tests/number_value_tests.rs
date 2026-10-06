//! Exact decimal-value boundary tests.
//!
//! WHAT: protects exact exponent materialization, immutable value handoff, and canonical
//!       arbitrary-precision formatting.
//! WHY: consumers need one exact decimal value without profile-sized or floating-point intermediates.

use crate::compiler_frontend::datatypes::number::{
    NumberArithmeticError, NumberIntegerConversionError, NumberMaterializationError, NumberValue,
};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::grammar::NumericLiteralSign;

fn materialize(
    normalized: &str,
    sign: NumericLiteralSign,
    scale: u16,
) -> Result<NumberValue, NumberMaterializationError> {
    let scale = NumberScale::new(scale).expect("test scale is within Number's supported range");
    NumberValue::from_normalized(normalized, sign, scale)
}

fn decimal(normalized: &str, sign: NumericLiteralSign, scale: u16) -> NumberValue {
    materialize(normalized, sign, scale).expect("test decimal must fit its Number scale")
}

#[test]
fn exact_materialization_reuses_decimal_scale_facts_for_exponents_and_zeroes() {
    let whole = materialize("123", NumericLiteralSign::Positive, 0).unwrap();
    assert_eq!(whole.coefficient().to_string(), "123");
    assert_eq!(whole.to_string(), "123");

    let fractional = materialize("1.20", NumericLiteralSign::Positive, 1).unwrap();
    assert_eq!(fractional.coefficient().to_string(), "12");
    assert_eq!(fractional.to_string(), "1.2");

    let positive_exponent = materialize("1.200e+2", NumericLiteralSign::Positive, 0).unwrap();
    assert_eq!(positive_exponent.coefficient().to_string(), "120");
    assert_eq!(positive_exponent.to_string(), "120");

    let negative_exponent = materialize("12300e-3", NumericLiteralSign::Positive, 1).unwrap();
    assert_eq!(negative_exponent.coefficient().to_string(), "123");
    assert_eq!(negative_exponent.to_string(), "12.3");

    let signed_fraction = materialize("0.00120", NumericLiteralSign::Negative, 5).unwrap();
    assert_eq!(signed_fraction.coefficient().to_string(), "-120");
    assert_eq!(signed_fraction.to_string(), "-0.0012");

    let zero = materialize("0.000e-999", NumericLiteralSign::Negative, 0).unwrap();
    assert_eq!(zero.coefficient().to_string(), "0");
    assert_eq!(zero.to_string(), "0");
}

#[test]
fn scale_256_and_arbitrary_coefficients_remain_exact() {
    let scale_256 = materialize("1e-256", NumericLiteralSign::Positive, 256).unwrap();
    assert_eq!(scale_256.coefficient().to_string(), "1");
    assert_eq!(scale_256.to_string(), format!("0.{}1", "0".repeat(255)));

    let digits = "9".repeat(200);
    let large = materialize(&digits, NumericLiteralSign::Positive, 0).unwrap();
    assert_eq!(large.coefficient().to_string(), digits);
    assert_eq!(large.to_string(), digits);
}

#[test]
fn huge_exponent_zero_short_circuits_and_nonzero_values_fail_early() {
    let exponent_digits = "9".repeat(2_000);
    let positive_exponent_zero = format!("0e+{exponent_digits}");
    let negative_exponent_zero = format!("0e-{exponent_digits}");

    let positive_zero =
        materialize(&positive_exponent_zero, NumericLiteralSign::Positive, 0).unwrap();
    let negative_zero =
        materialize(&negative_exponent_zero, NumericLiteralSign::Negative, 0).unwrap();
    assert_eq!(positive_zero.to_string(), "0");
    assert_eq!(negative_zero.to_string(), "0");

    let positive_huge_value = format!("1e+{exponent_digits}");
    assert_eq!(
        materialize(&positive_huge_value, NumericLiteralSign::Positive, 0),
        Err(NumberMaterializationError::Capacity)
    );

    let too_fine_value = format!("1e-{exponent_digits}");
    assert_eq!(
        materialize(&too_fine_value, NumericLiteralSign::Positive, 256),
        Err(NumberMaterializationError::InexactScale)
    );
}

#[test]
fn inexact_decimal_values_are_rejected_instead_of_rounded() {
    assert_eq!(
        materialize("1.239", NumericLiteralSign::Positive, 2),
        Err(NumberMaterializationError::InexactScale)
    );
    assert_eq!(
        materialize("1e-1", NumericLiteralSign::Negative, 0),
        Err(NumberMaterializationError::InexactScale)
    );
}

#[test]
fn exact_integer_conversion_distinguishes_fraction_from_i128_overflow() {
    let integral_fractional_scale = decimal("12.00", NumericLiteralSign::Positive, 2);
    assert_eq!(integral_fractional_scale.to_integer_exact(), Ok(12));

    let fraction = decimal("12.01", NumericLiteralSign::Positive, 2);
    assert_eq!(
        fraction.to_integer_exact(),
        Err(NumberIntegerConversionError::Inexact)
    );

    let out_of_range = decimal(
        "170141183460469231731687303715884105728",
        NumericLiteralSign::Positive,
        0,
    );
    assert_eq!(
        out_of_range.to_integer_exact(),
        Err(NumberIntegerConversionError::OutOfRange)
    );

    let maximum = NumberValue::from_integer(i128::MAX, NumberScale::new(256).unwrap());
    assert_eq!(maximum.to_integer_exact(), Ok(i128::MAX));
    assert_eq!(
        maximum
            .rescale(NumberScale::ZERO)
            .expect("scale-256 integer widening is exact")
            .to_integer_exact(),
        Ok(i128::MAX)
    );
}

#[test]
fn rescaling_is_exact_and_rejects_nonzero_removed_digits() {
    let exact = decimal("12.00", NumericLiteralSign::Positive, 2);
    let narrowed = exact
        .rescale(NumberScale::ZERO)
        .expect("zero fractional digits can be removed");
    assert_eq!(narrowed.coefficient().to_string(), "12");
    assert_eq!(narrowed.to_string(), "12");

    let widened = decimal("12.34", NumericLiteralSign::Positive, 2)
        .rescale(NumberScale::new(4).unwrap())
        .expect("scale widening preserves the exact value");
    assert_eq!(widened.coefficient().to_string(), "123400");
    assert_eq!(widened.to_string(), "12.34");

    assert_eq!(
        widened.rescale(NumberScale::new(1).unwrap()),
        Err(NumberMaterializationError::InexactScale)
    );

    let zero = decimal("0", NumericLiteralSign::Positive, 0);
    let zero_widened = zero
        .rescale(NumberScale::new(256).unwrap())
        .expect("zero remains exact when its scale widens");
    assert_eq!(zero_widened.to_string(), "0");
    assert_eq!(
        zero_widened
            .rescale(NumberScale::ZERO)
            .expect("zero remains exact when its scale narrows")
            .to_string(),
        "0"
    );
}

#[test]
fn arithmetic_rounds_signed_half_ties_to_even_at_the_result_scale() {
    let half = decimal("0.5", NumericLiteralSign::Positive, 1);
    let tenth = decimal("0.1", NumericLiteralSign::Positive, 1);
    let three_tenths = decimal("0.3", NumericLiteralSign::Positive, 1);

    assert_eq!(
        half.checked_binary(NumericOperator::Multiply, &tenth)
            .unwrap()
            .coefficient()
            .to_string(),
        "0"
    );
    assert_eq!(
        half.checked_binary(NumericOperator::Multiply, &three_tenths)
            .unwrap()
            .coefficient()
            .to_string(),
        "2"
    );

    let negative_half = decimal("0.5", NumericLiteralSign::Negative, 1);
    assert_eq!(
        negative_half
            .checked_binary(NumericOperator::Multiply, &tenth)
            .unwrap()
            .coefficient()
            .to_string(),
        "0"
    );
    assert_eq!(
        negative_half
            .checked_binary(NumericOperator::Multiply, &three_tenths)
            .unwrap()
            .coefficient()
            .to_string(),
        "-2"
    );

    let one = decimal("1.0", NumericLiteralSign::Positive, 1);
    let four_fifths = decimal("0.8", NumericLiteralSign::Positive, 1);
    let fourteen_tenths = decimal("1.4", NumericLiteralSign::Positive, 1);
    assert_eq!(
        one.checked_binary(NumericOperator::Divide, &four_fifths)
            .unwrap()
            .coefficient()
            .to_string(),
        "12"
    );
    assert_eq!(
        fourteen_tenths
            .checked_binary(NumericOperator::Divide, &four_fifths)
            .unwrap()
            .coefficient()
            .to_string(),
        "18"
    );
    assert_eq!(
        decimal("1.0", NumericLiteralSign::Negative, 1)
            .checked_binary(NumericOperator::Divide, &four_fifths)
            .unwrap()
            .coefficient()
            .to_string(),
        "-12"
    );

    let negative_four_fifths = decimal("0.8", NumericLiteralSign::Negative, 1);
    assert_eq!(
        one.checked_binary(NumericOperator::Divide, &negative_four_fifths)
            .unwrap()
            .coefficient()
            .to_string(),
        "-12"
    );
    assert_eq!(
        fourteen_tenths
            .checked_binary(NumericOperator::Divide, &negative_four_fifths)
            .unwrap()
            .coefficient()
            .to_string(),
        "-18"
    );
    assert_eq!(
        decimal("1.0", NumericLiteralSign::Negative, 1)
            .checked_binary(NumericOperator::Divide, &negative_four_fifths)
            .unwrap()
            .coefficient()
            .to_string(),
        "12"
    );
    assert_eq!(
        decimal("1.4", NumericLiteralSign::Negative, 1)
            .checked_binary(NumericOperator::Divide, &negative_four_fifths)
            .unwrap()
            .coefficient()
            .to_string(),
        "18"
    );
}

#[test]
fn scale_zero_multiplication_and_power_remain_exact() {
    let two = decimal("2", NumericLiteralSign::Positive, 0);
    let three = decimal("3", NumericLiteralSign::Positive, 0);

    assert_eq!(
        two.checked_binary(NumericOperator::Multiply, &three)
            .unwrap()
            .coefficient()
            .to_string(),
        "6"
    );
    assert_eq!(
        two.checked_power(64).unwrap().coefficient().to_string(),
        "18446744073709551616"
    );
}

#[test]
fn exact_integral_powers_keep_scale_across_representative_scales() {
    for scale in [0_u16, 1, 4, 256] {
        let number_scale = NumberScale::new(scale).unwrap();
        for (base, expected) in [(2_i128, [1_i128, 2, 4, 8]), (-2_i128, [1_i128, -2, 4, -8])] {
            for (exponent, expected_magnitude) in expected.iter().enumerate() {
                let powered = NumberValue::from_integer(base, number_scale)
                    .checked_power(i64::try_from(exponent).expect("small test exponent fits i64"))
                    .unwrap();
                let expected_value = NumberValue::from_integer(*expected_magnitude, number_scale);
                assert_eq!(powered.scale(), number_scale);
                assert_eq!(
                    powered, expected_value,
                    "base {base} exponent {exponent} at scale {scale}"
                );
            }
        }
    }

    let scale_256 = NumberScale::new(256).unwrap();
    assert_eq!(
        NumberValue::from_integer(2, scale_256)
            .checked_power(64)
            .unwrap(),
        NumberValue::from_integer(18_446_744_073_709_551_616, scale_256)
    );

    let scale_four = NumberScale::new(4).unwrap();
    let beyond_machine = NumberValue::from_integer(2, NumberScale::ZERO)
        .checked_power(64)
        .unwrap()
        .rescale(scale_four)
        .expect("an integer rescales exactly to a wider Dec scale");
    let beyond_machine_squared = beyond_machine.checked_power(2).unwrap();
    assert_eq!(beyond_machine_squared.scale(), scale_four);
    assert_eq!(
        beyond_machine_squared.coefficient().to_string(),
        "3402823669209384634633746074317682114560000"
    );
    assert_eq!(
        beyond_machine.checked_power(-1),
        Err(NumberArithmeticError::InvalidExponent)
    );
}

#[test]
fn exact_power_rounds_once_and_handles_zero_unity_and_int64_exponents() {
    let three_halves = decimal("1.5", NumericLiteralSign::Positive, 1);
    let cube = three_halves.checked_power(3).unwrap();
    assert_eq!(cube.coefficient().to_string(), "34");
    assert_eq!(cube.to_string(), "3.4");

    let negative_cube = decimal("1.5", NumericLiteralSign::Negative, 1)
        .checked_power(3)
        .unwrap();
    assert_eq!(negative_cube.coefficient().to_string(), "-34");

    let scale_one = NumberScale::new(1).unwrap();
    let unity_at_scale_one = NumberValue::from_integer(1, scale_one);
    assert_eq!(
        unity_at_scale_one.checked_power(i64::MAX).unwrap(),
        unity_at_scale_one
    );
    assert_eq!(
        decimal("0.1", NumericLiteralSign::Positive, 1)
            .checked_power(i64::MAX)
            .unwrap()
            .coefficient()
            .to_string(),
        "0"
    );

    let scale_256 = NumberScale::new(256).unwrap();
    let unity = NumberValue::from_integer(1, scale_256);
    let negative_unity = NumberValue::from_integer(-1, scale_256);
    assert_eq!(unity.checked_power(i64::MAX).unwrap(), unity);
    assert_eq!(
        negative_unity.checked_power(i64::MAX).unwrap(),
        negative_unity
    );
    assert_eq!(negative_unity.checked_power(i64::MAX - 1).unwrap(), unity);

    let zero = decimal("0", NumericLiteralSign::Positive, 256);
    assert_eq!(zero.checked_power(i64::MAX).unwrap(), zero);
    assert_eq!(zero.checked_power(0).unwrap(), unity);
    assert_eq!(
        three_halves.checked_power(-1),
        Err(NumberArithmeticError::InvalidExponent)
    );
}

#[test]
fn power_shortcut_edges_zero_tiny_powers_and_reject_negative_exponents() {
    // A coefficient of +/-1 at a positive scale is at most a tenth in magnitude, so its
    // square and every higher power round to zero at that scale. Exponent one keeps input.
    for (scale, tiny_text) in [(1_u16, "0.1"), (4_u16, "0.0001")] {
        let number_scale = NumberScale::new(scale).unwrap();
        let zero = NumberValue::from_integer(0, number_scale);
        let unity = NumberValue::from_integer(1, number_scale);
        for sign in [NumericLiteralSign::Positive, NumericLiteralSign::Negative] {
            let tiny = decimal(tiny_text, sign, scale);
            let expected_coefficient = match sign {
                NumericLiteralSign::Positive => "1",
                NumericLiteralSign::Negative => "-1",
            };
            assert_eq!(
                tiny.coefficient().to_string(),
                expected_coefficient,
                "test setup keeps coefficient one at scale {scale}"
            );
            assert_eq!(
                tiny.checked_power(1).unwrap(),
                tiny,
                "coefficient one keeps its input for exponent one at scale {scale}"
            );
            for exponent in [2_i64, 3, i64::MAX] {
                assert_eq!(
                    tiny.checked_power(exponent).unwrap(),
                    zero,
                    "coefficient one rounds to zero for exponent {exponent} at scale {scale}"
                );
            }
        }

        // Zero to the zeroth power keeps the existing unity contract at each scale.
        assert_eq!(
            zero.checked_power(0).unwrap(),
            unity,
            "zero to the zeroth power is the scale unity at scale {scale}"
        );
    }

    // Negative exponents are rejected before any power shortcut fires.
    let scale_one = NumberScale::new(1).unwrap();
    let negative_exponent_cases: [(&str, NumberValue); 7] = [
        ("zero", decimal("0", NumericLiteralSign::Positive, 1)),
        ("positive unity", NumberValue::from_integer(1, scale_one)),
        ("negative unity", NumberValue::from_integer(-1, scale_one)),
        ("tiny one", decimal("0.1", NumericLiteralSign::Positive, 1)),
        (
            "tiny negative one",
            decimal("0.1", NumericLiteralSign::Negative, 1),
        ),
        ("integral", NumberValue::from_integer(2, scale_one)),
        (
            "fractional",
            decimal("1.5", NumericLiteralSign::Positive, 1),
        ),
    ];
    for (name, base) in negative_exponent_cases {
        assert_eq!(
            base.checked_power(-1),
            Err(NumberArithmeticError::InvalidExponent),
            "{name} rejects a negative exponent"
        );
    }
}

#[test]
fn number_remainder_is_signed_and_large_coefficients_stay_exact() {
    let negative = decimal("1.4", NumericLiteralSign::Negative, 1);
    let half = decimal("0.5", NumericLiteralSign::Positive, 1);
    assert_eq!(
        negative
            .checked_binary(NumericOperator::Remainder, &half)
            .unwrap()
            .coefficient()
            .to_string(),
        "-4"
    );

    let large = decimal(
        "12345678901234567890123456789012345678901234567890",
        NumericLiteralSign::Positive,
        0,
    );
    let sum = large.checked_binary(NumericOperator::Add, &large).unwrap();
    assert_eq!(
        sum.coefficient().to_string(),
        "24691357802469135780246913578024691357802469135780"
    );
}

#[test]
fn invalid_division_remainder_and_operator_scales_are_distinguished() {
    let scale_zero = decimal("4", NumericLiteralSign::Positive, 0);
    let zero = decimal("0", NumericLiteralSign::Positive, 0);
    assert_eq!(
        scale_zero.checked_binary(NumericOperator::Divide, &zero),
        Err(NumberArithmeticError::InvalidOperation)
    );
    assert_eq!(
        scale_zero.checked_binary(NumericOperator::Remainder, &zero),
        Err(NumberArithmeticError::DivideByZero)
    );

    let scale_one = decimal("4.0", NumericLiteralSign::Positive, 1);
    let scale_one_zero = decimal("0", NumericLiteralSign::Positive, 1);
    assert_eq!(
        scale_one.checked_binary(NumericOperator::Divide, &scale_one_zero),
        Err(NumberArithmeticError::DivideByZero)
    );
    assert_eq!(
        scale_one.checked_binary(NumericOperator::IntegerDivide, &scale_one),
        Err(NumberArithmeticError::InvalidOperation)
    );
    assert_eq!(
        scale_zero.checked_binary(NumericOperator::IntegerDivide, &scale_one),
        Err(NumberArithmeticError::InvalidOperation)
    );

    assert_eq!(
        decimal("7", NumericLiteralSign::Negative, 0)
            .checked_binary(
                NumericOperator::IntegerDivide,
                &decimal("3", NumericLiteralSign::Positive, 0)
            )
            .unwrap()
            .coefficient()
            .to_string(),
        "-2"
    );
}
