//! Single-rounding tests for the shared numeric profile widths.
//!
//! `FloatPrecision::round_int` rounds directly at the destination precision, never through an
//! `f64` intermediate under `Bits32`; this checks an integer where the two roundings disagree.

use crate::numeric::profile::FloatPrecision;

#[test]
fn float32_from_int_rounds_once_without_an_f64_intermediate() {
    // 9_007_199_791_611_905 is one above the midpoint between 2^53 and
    // 2^53 + 2^30 at f32 precision, so single rounding from the integer rounds
    // up to 2^53 + 2^30. Rounding through f64 first lands exactly on the
    // midpoint (9_007_199_791_611_904.0), which then ties to even: 2^53.
    assert_eq!(
        FloatPrecision::Bits32.round_int(9_007_199_791_611_905),
        9_007_200_328_482_816.0,
        "Float32 Int->Float must round directly at f32 precision"
    );
}
