//! Binary16 rounding tests.
//!
//! WHAT: protects the single-rounding conversion `F16` literals depend on, including its
//!       ties-to-even boundaries and the double-rounding guard.
//! WHY:  an `F16` literal has no standard-library parse, so its rounding contract is only visible
//!       through these invariants.

use super::*;

#[test]
fn round_f64_to_f16_keeps_exactly_representable_values() {
    let cases = [
        0.0,
        0.5,
        1.0,
        1.5,
        2048.0,
        65504.0,
        2f64.powi(-24), // Smallest positive binary16 subnormal.
        2f64.powi(-14), // Smallest normal binary16 value.
    ];

    for value in cases {
        assert_eq!(
            round_f64_to_f16(value),
            value,
            "{value} is representable and must round to itself"
        );
        assert_eq!(
            round_f64_to_f16(-value),
            -value,
            "-{value} must round to itself"
        );
    }
}

#[test]
fn round_f64_to_f16_resolves_normal_boundary_ties_to_even() {
    // Neighbours of 1.0 are spaced 2^-10 apart, so 1 + 2^-11 is the midpoint.
    let midpoint = 1.0 + 2f64.powi(-11);

    assert_eq!(round_f64_to_f16(midpoint), 1.0);
    assert_eq!(
        round_f64_to_f16(midpoint + 2f64.powi(-52)),
        1.0 + 2f64.powi(-10)
    );
    assert_eq!(
        round_f64_to_f16(midpoint - 2f64.powi(-52)),
        1.0,
        "an f64 step below the midpoint still rounds to the even neighbour"
    );

    // Neighbours of 2.0 are spaced 2^-9 apart, so 2.0 + 2^-10 is the midpoint whose even neighbour
    // is 2.0; one f64 step above it rounds up to 2.0 + 2^-9.
    let up_midpoint = 2.0 + 2f64.powi(-10) + 2f64.powi(-11);
    assert_eq!(round_f64_to_f16(up_midpoint), 2.0 + 2f64.powi(-9));
}

#[test]
fn round_f64_to_f16_resolves_subnormal_boundary_ties_to_even() {
    // Below 2^-14 every binary16 value is a multiple of 2^-24, so 2^-25 is the midpoint above zero
    // and 3 * 2^-25 is the midpoint whose even neighbour is 2^-23.
    assert_eq!(round_f64_to_f16(2f64.powi(-25)), 0.0);
    assert_eq!(round_f64_to_f16(3.0 * 2f64.powi(-25)), 2f64.powi(-23));
    assert_eq!(
        round_f64_to_f16(2f64.powi(-25) - 2f64.powi(-78)),
        0.0,
        "the largest magnitude below the first midpoint still rounds to zero"
    );
    assert_eq!(
        round_f64_to_f16(2f64.powi(-24) - 2f64.powi(-77)),
        2f64.powi(-24),
        "the smallest subnormal absorbs everything just below it"
    );
    assert_eq!(round_f64_to_f16(f64::from_bits(1)), 0.0);

    // A magnitude between the top subnormal and the smallest normal keeps the 2^-24 unit.
    assert_eq!(
        round_f64_to_f16(1023.5 * 2f64.powi(-24)),
        1024.0 * 2f64.powi(-24)
    );
}

#[test]
fn round_f64_to_f16_overflows_to_infinity_at_the_midpoint_above_maximum() {
    // 65504 is the largest finite value and 65520 is the midpoint to the next binade, whose even
    // neighbour is the 2^16 overflow.
    assert_eq!(round_f64_to_f16(65519.99), 65504.0);
    assert_eq!(round_f64_to_f16(65520.0), f64::INFINITY);
    assert_eq!(round_f64_to_f16(65520.0 - 2f64.powi(-30)), 65504.0);
    assert_eq!(round_f64_to_f16(-65520.0), f64::NEG_INFINITY);
    assert_eq!(round_f64_to_f16(70000.0), f64::INFINITY);
    assert_eq!(round_f64_to_f16(f64::MAX), f64::INFINITY);
}

#[test]
fn round_f64_to_f16_preserves_sign_and_non_finite_inputs() {
    assert!(round_f64_to_f16(-0.0).is_sign_negative());
    assert!(round_f64_to_f16(-2f64.powi(-40)).is_sign_negative());
    assert_eq!(round_f64_to_f16(-2f64.powi(-40)), 0.0);

    assert!(round_f64_to_f16(f64::NAN).is_nan());
    assert_eq!(round_f64_to_f16(f64::INFINITY), f64::INFINITY);
    assert_eq!(round_f64_to_f16(f64::NEG_INFINITY), f64::NEG_INFINITY);
}

#[test]
fn decimal_to_f16_rounds_authored_decimals_once() {
    assert_eq!(decimal_to_f16("0.5", false), Some(0.5));
    assert_eq!(decimal_to_f16("65504", false), Some(65504.0));
    assert_eq!(decimal_to_f16("65519.99", false), Some(65504.0));
    assert_eq!(decimal_to_f16("65520", false), None);
    assert_eq!(decimal_to_f16("70000", false), None);
    assert_eq!(
        decimal_to_f16("5.960464477539063e-8", false),
        Some(2f64.powi(-24))
    );
    assert_eq!(decimal_to_f16("1e-10", false), Some(0.0));
    assert_eq!(decimal_to_f16("1.0e400", false), None);
}

#[test]
fn decimal_to_f16_compares_double_rounding_guard_against_authored_digits() {
    // The exact midpoint between 1.0 and 1.0009765625, written out: ties-to-even keeps 1.0.
    assert_eq!(decimal_to_f16("1.00048828125", false), Some(1.0));

    // Authored digits that only round to that midpoint inside `f64` must still follow the authored
    // value instead of the `f64` tie, on both sides of the midpoint.
    assert_eq!(
        decimal_to_f16("1.00048828125000000000001", false),
        Some(1.0009765625)
    );
    assert_eq!(
        decimal_to_f16("1.00048828124999999999999", false),
        Some(1.0)
    );
    assert_eq!(
        decimal_to_f16("1.00048828125000000000001", true),
        Some(-1.0009765625)
    );

    // The overflow threshold is a midpoint too, so digits a hair above it must overflow.
    assert_eq!(
        decimal_to_f16("65519.99999999999999999", false),
        Some(65504.0)
    );
    assert_eq!(decimal_to_f16("65520.00000000000001", false), None);
}
