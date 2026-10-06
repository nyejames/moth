//! Single-rounding tests for the shared numeric profile widths.
//!
//! `FloatPrecision::round_int` rounds directly at the destination precision, never through an
//! `f64` intermediate under `Bits32`; this checks an integer where the two roundings disagree.

use crate::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

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

#[test]
fn unsigned_max_value_follows_the_selected_int_width() {
    assert_eq!(IntWidth::Bits32.unsigned_max_value(), 4_294_967_295);
    assert_eq!(
        IntWidth::Bits64.unsigned_max_value(),
        18_446_744_073_709_551_615
    );
}

#[test]
fn float32_from_uint_rounds_once_without_an_f64_intermediate() {
    // 9_007_199_791_611_905 is one above the midpoint between 2^53 and
    // 2^53 + 2^30 at f32 precision, so single rounding from the integer rounds
    // up to 2^53 + 2^30 (bits 0x5a000001). Rounding through f64 first lands
    // exactly on the midpoint (9_007_199_791_611_904.0), which then ties to
    // even: 2^53 (bits 0x5a000000).
    let rounded = FloatPrecision::Bits32.round_uint(9_007_199_791_611_905);
    assert_eq!(
        rounded.to_bits(),
        9_007_200_328_482_816.0_f64.to_bits(),
        "Float32 Uint->Float must round directly at f32 precision"
    );
    assert_eq!(
        (rounded as f32).to_bits(),
        0x5a00_0001,
        "witness value must land on 0x5a000001, not the double-rounded 0x5a000000"
    );
}

#[test]
fn float64_from_uint_is_exact_where_representable() {
    assert_eq!(
        FloatPrecision::Bits64.round_uint(4_294_967_295),
        4_294_967_295.0
    );
    assert_eq!(
        FloatPrecision::Bits32.round_uint(0),
        0.0,
        "zero canonicalises through the direct path"
    );
}

#[test]
fn four_profile_fingerprint_encodings_are_unchanged() {
    assert_eq!(
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits64,
        }
        .fingerprint_byte(),
        0
    );
    assert_eq!(
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits32,
        }
        .fingerprint_byte(),
        1
    );
    assert_eq!(
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        }
        .fingerprint_byte(),
        2
    );
    assert_eq!(
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        }
        .fingerprint_byte(),
        3
    );
}
