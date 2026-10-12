//! Single-rounding tests for integer-to-binary-float conversion at a destination precision.
//!
//! `BinaryFloatPrecision::round_integer` rounds directly at the destination precision, never
//! through an `f64` intermediate under `Binary32`; this checks an integer where the two roundings
//! disagree.

use crate::numeric::precision::BinaryFloatPrecision;

#[test]
fn float32_from_integer_rounds_once_without_an_f64_intermediate() {
    // 9_007_199_791_611_905 is one above the midpoint between 2^53 and
    // 2^53 + 2^30 at f32 precision, so single rounding from the integer rounds
    // up to 2^53 + 2^30. Rounding through f64 first lands exactly on the
    // midpoint (9_007_199_791_611_904.0), which then ties to even: 2^53.
    assert_eq!(
        BinaryFloatPrecision::Binary32.round_integer(9_007_199_791_611_905),
        9_007_200_328_482_816.0,
        "Binary32 integer conversion must round directly at f32 precision"
    );
}

#[test]
fn float32_witness_lands_on_the_direct_rounding_bits() {
    // The same witness at the bit level: the direct rounding is 2^53 + 2^30
    // (f32 bits 0x5a000001), not the double-rounded 2^53 (bits 0x5a000000).
    let rounded = BinaryFloatPrecision::Binary32.round_integer(9_007_199_791_611_905);
    assert_eq!(
        rounded.to_bits(),
        9_007_200_328_482_816.0_f64.to_bits(),
        "Binary32 integer conversion must round directly at f32 precision"
    );
    assert_eq!(
        (rounded as f32).to_bits(),
        0x5a00_0001,
        "witness value must land on 0x5a000001, not the double-rounded 0x5a000000"
    );
}

#[test]
fn float64_from_integer_is_exact_where_representable() {
    assert_eq!(
        BinaryFloatPrecision::Binary64.round_integer(4_294_967_295),
        4_294_967_295.0
    );
    assert_eq!(BinaryFloatPrecision::Binary32.round_integer(0), 0.0);
}
