//! Binary16 rounding for `F16` literals.
//!
//! WHAT: rounds one `f64` magnitude to the nearest binary16 value with IEEE round-to-nearest,
//!       ties-to-even, and converts an authored decimal magnitude to binary16 without double
//!       rounding.
//! WHY:  `f16` is not stable on this toolchain, so the rounding is expressed with explicit bit and
//!       integer arithmetic; every entry point returns the binary16 value as an exactly
//!       representable `f64` because that is what `FixedScalarValue` stores.
//!
//! Rounding twice is the hazard this module owns. Parsing an authored decimal as `f64` and then
//! rounding that to binary16 rounds twice, and the two roundings can disagree: an authored value a
//! hair above a binary16 midpoint can land exactly on the midpoint inside `f64`, where ties-to-even
//! rounds it down. Only a parse that lands exactly on a binary16 midpoint can be affected: every
//! midpoint is exactly representable in `f64`, and correctly rounded parsing is monotonic, so an
//! authored value strictly on one side of a midpoint never parses to the other side.
//! [`decimal_to_f16`] therefore detects the midpoint case and compares the authored digits against
//! that midpoint exactly.
//!
//! Exclusions: numeric text grammar, token shape and range diagnostics stay in `numeric_text::parse`
//! and `compiler_messages`; `F32`/`F64` literal rounding stays with the standard library decimal
//! parsers.

use std::cmp::Ordering;

/// Significand fraction bits in an `f64`.
const F64_MANTISSA_BITS: u64 = 52;
const F64_MANTISSA_MASK: u64 = (1 << F64_MANTISSA_BITS) - 1;
const F64_EXPONENT_MASK: u64 = 0x7FF;
const F64_EXPONENT_BIAS: i32 = 1023;

/// Binary exponent of the smallest `f64` subnormal, the unit of a subnormal `f64` significand.
const F64_MIN_SUBNORMAL_EXPONENT: i32 = -1074;

/// Significand bits in a binary16 value, including the implicit leading bit.
const F16_SIGNIFICAND_BITS: i32 = 11;

/// Binary exponent of the largest binary16 normal value (2^15).
const F16_MAX_NORMAL_EXPONENT: i32 = 15;

/// The binary16 unit below the smallest normal value, shared by every smaller binade.
const F16_SUBNORMAL_ULP_EXPONENT: i32 = -24;

/// Round `value` to the nearest binary16 value, returned as an exactly representable `f64`.
///
/// WHAT: IEEE round-to-nearest, ties-to-even. A magnitude that rounds above binary16's largest
///       finite value becomes an infinity, an underflow keeps its sign and returns a signed zero,
///       and NaN and infinities pass through unchanged.
/// WHY:  `F32` and `F64` literals round through the standard library decimal parsers, so `F16`
///       needs its own single rounding step to satisfy the same literal contract.
pub(crate) fn round_f64_to_f16(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }

    let negative = value.is_sign_negative();
    let rounded = round_finite_magnitude_to_f16(value.abs()).value;

    if negative { -rounded } else { rounded }
}

/// The nearest binary16 magnitude for an authored unsigned decimal magnitude.
///
/// WHAT: parses the normalized magnitude text once as `f64`, rounds that to binary16, and re-checks
///       the rounding against the exact authored digits whenever the parse landed exactly on a
///       binary16 midpoint.
/// WHY:  an `F16` literal is the one destination whose value cannot come from a single standard
///       library parse, so the double-rounding guard lives with the rounding owner.
///
/// `normalized` is the token's unsigned, separator-free magnitude text. Returns `None` when the
/// rounded magnitude is not finite, which the caller reports as the ordinary non-finite float
/// failure.
pub(crate) fn decimal_to_f16(normalized: &str, negative: bool) -> Option<f64> {
    // The caller owns the sign, so the magnitude text must be unsigned. A signed spelling would
    // otherwise be rounded as a magnitude while the exact comparison below used the same signed
    // digits, so refuse it instead of guessing which sign wins.
    if normalized.starts_with('-') || normalized.starts_with('+') {
        return None;
    }

    // The grammar has already accepted this text, so a failed parse can only mean a magnitude
    // beyond `f64`'s range, which has no finite binary16 value either.
    let parsed: f64 = normalized.parse().ok()?;
    if !parsed.is_finite() {
        return None;
    }

    let rounding = round_finite_magnitude_to_f16(parsed);

    // A parsed magnitude that is exactly a midpoint keeps the ties-to-even result only when the
    // authored digits are exactly that midpoint; otherwise the authored value decides the side.
    let rounded = match rounding.midpoint {
        Some(midpoint) => {
            let exact =
                compare_decimal_magnitude(normalized, midpoint.significand, midpoint.exponent);
            match exact {
                Ordering::Less => midpoint.below,
                Ordering::Greater => midpoint.above,
                Ordering::Equal => rounding.value,
            }
        }
        None => rounding.value,
    };

    if !rounded.is_finite() {
        return None;
    }

    Some(if negative { -rounded } else { rounded })
}

/// Rounding of one non-negative finite magnitude to binary16.
struct F16MagnitudeRounding {
    /// The ties-to-even result, exactly representable in `f64`.
    value: f64,
    /// `Some(..)` when the magnitude was exactly the midpoint between two adjacent binary16 values.
    midpoint: Option<F16Midpoint>,
}

/// The exact binary16 rounding midpoint an authored magnitude collapsed onto.
struct F16Midpoint {
    /// The binary16 magnitude just below the midpoint.
    below: f64,
    /// The binary16 magnitude just above the midpoint; infinity above the largest finite value.
    above: f64,
    /// The midpoint itself as `significand * 2^exponent`, with an odd significand.
    significand: u64,
    exponent: i32,
}

impl F16Midpoint {
    /// The midpoint between the quotient `truncated` and the next unit step above it.
    ///
    /// WHY: `truncated` is the rounding quotient below the midpoint, which fixes both neighbours and
    /// gives the midpoint as the odd multiple `(2 * truncated + 1) * 2^(unit_exponent - 1)`.
    fn new(truncated: u64, unit_exponent: i32) -> Self {
        Self {
            below: scaled_value(truncated, unit_exponent),
            above: scaled_value(truncated + 1, unit_exponent),
            significand: (truncated << 1) | 1,
            exponent: unit_exponent - 1,
        }
    }
}

/// Round one non-negative finite magnitude to its nearest binary16 magnitude.
fn round_finite_magnitude_to_f16(magnitude: f64) -> F16MagnitudeRounding {
    if magnitude == 0.0 {
        return F16MagnitudeRounding {
            value: 0.0,
            midpoint: None,
        };
    }

    let bits = magnitude.to_bits();
    let raw_exponent = ((bits >> F64_MANTISSA_BITS) & F64_EXPONENT_MASK) as i32;
    let fraction = bits & F64_MANTISSA_MASK;

    // A subnormal `f64` has no implicit leading bit, so both encodings reduce to
    // `significand * 2^exponent`.
    let (significand, exponent) = if raw_exponent == 0 {
        (fraction, F64_MIN_SUBNORMAL_EXPONENT)
    } else {
        (
            fraction | (1 << F64_MANTISSA_BITS),
            raw_exponent - F64_EXPONENT_BIAS - F64_MANTISSA_BITS as i32,
        )
    };

    // Binary16 keeps its full significand down to exponent -14; every binade below that shares the
    // smallest subnormal's 2^-24 unit, so one unit exponent covers both ranges.
    let leading_exponent = exponent + (63 - significand.leading_zeros() as i32);
    let unit_exponent =
        (leading_exponent - (F16_SIGNIFICAND_BITS - 1)).max(F16_SUBNORMAL_ULP_EXPONENT);
    let shift = unit_exponent - exponent;

    let mut quotient = 0;
    let mut midpoint = None;

    if shift <= 0 {
        quotient = significand << (-shift) as u32;
    } else if shift < 64 {
        let truncated = significand >> shift as u32;
        let remainder = significand & ((1 << shift as u32) - 1);
        let half = 1 << (shift as u32 - 1);

        // A remainder above half rounds up, below half rounds down, and exactly half resolves to the
        // even significand.
        let round_up = remainder > half || (remainder == half && truncated & 1 == 1);
        quotient = truncated + u64::from(round_up);

        if remainder == half {
            midpoint = Some(F16Midpoint::new(truncated, unit_exponent));
        }
    }
    // A larger shift can only leave an exact zero: the magnitude then sits far below the smallest
    // binary16 rounding midpoint, and half a unit cannot round it up.

    F16MagnitudeRounding {
        value: scaled_value(quotient, unit_exponent),
        midpoint,
    }
}

/// The binary16 magnitude for one rounded quotient of the destination unit.
///
/// WHY: a quotient of zero or of the carried `2^11` has no plain spelling as a scaled significand,
/// and any magnitude above binary16's largest exponent overflows to infinity instead.
fn scaled_value(quotient: u64, unit_exponent: i32) -> f64 {
    if quotient == 0 {
        return 0.0;
    }

    let value_exponent = unit_exponent + (63 - quotient.leading_zeros() as i32);
    if value_exponent > F16_MAX_NORMAL_EXPONENT {
        return f64::INFINITY;
    }

    scale_pow2(quotient, unit_exponent)
}

/// The exact `f64` for `significand * 2^exponent`.
///
/// WHY: every binary16 value is exactly representable in `f64`, so a rounded magnitude is rebuilt by
/// placing its significand bits directly instead of relying on float scaling.
fn scale_pow2(significand: u64, exponent: i32) -> f64 {
    let highest_bit = 63 - significand.leading_zeros();
    let mantissa = (significand << (F64_MANTISSA_BITS - highest_bit as u64)) & F64_MANTISSA_MASK;
    let biased_exponent = (exponent + highest_bit as i32 + F64_EXPONENT_BIAS) as u64;

    f64::from_bits((biased_exponent << F64_MANTISSA_BITS) | mantissa)
}

/// Compare an authored unsigned decimal magnitude against the exact binary value
/// `significand * 2^exponent`.
///
/// WHAT: reduces the authored text to its significant digits plus a power of ten, renders the binary
///       value as its exact terminating decimal, and compares the two digit by digit.
/// WHY:  this comparison is the double-rounding guard, so it must stay exact for arbitrarily long
///       authored digit runs; the authored digits are never folded into a fixed-width integer.
fn compare_decimal_magnitude(authored: &str, significand: u64, exponent: i32) -> Ordering {
    let (authored_digits, authored_scale) = decompose_decimal_magnitude(authored);
    if authored_digits.is_empty() {
        // Only a zero magnitude has no significant digits, and a rounding midpoint is never zero.
        return Ordering::Less;
    }

    let (exact_digits, exact_scale) = binary_decimal_expansion(significand, exponent);

    compare_scaled_decimals(&authored_digits, authored_scale, exact_digits, exact_scale)
}

/// Split an unsigned normalized decimal magnitude into its significant digits and the power of ten
/// those digits are scaled by.
///
/// WHY: exact comparison needs the authored digit sequence itself, so this performs no numeric
/// conversion beyond the exponent.
fn decompose_decimal_magnitude(text: &str) -> (String, i64) {
    let (mantissa, explicit_exponent) = match text.split_once('e') {
        Some((mantissa, exponent_text)) => (mantissa, parse_decimal_exponent(exponent_text)),
        None => (text, 0),
    };

    let (integer_digits, fractional_digits) = match mantissa.split_once('.') {
        Some((integer_digits, fractional_digits)) => (integer_digits, fractional_digits),
        None => (mantissa, ""),
    };

    let mut digits = String::with_capacity(integer_digits.len() + fractional_digits.len());
    digits.push_str(integer_digits);
    digits.push_str(fractional_digits);

    // Leading zeros only move the scale and trailing zeros fold into it, so every spelling of one
    // value compares digit for digit.
    let first_significant = digits
        .find(|character| character != '0')
        .unwrap_or(digits.len());
    digits.drain(..first_significant);

    let mut scale = explicit_exponent - fractional_digits.len() as i64;
    while digits.ends_with('0') {
        digits.pop();
        scale += 1;
    }

    (digits, scale)
}

/// The exponent written after `e` in a normalized literal, or `0` when the text carries none.
///
/// WHY: accumulation saturates instead of rejecting, because a caller only compares magnitudes that
/// already parsed as a finite `f64`; saturation keeps an absurd exponent from panicking the
/// comparison while leaving the comparison itself ordered.
fn parse_decimal_exponent(text: &str) -> i64 {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };

    let mut exponent: i64 = 0;
    for character in digits.bytes() {
        exponent = exponent
            .saturating_mul(10)
            .saturating_add(i64::from(character - b'0'));
    }

    if negative { -exponent } else { exponent }
}

/// The exact terminating decimal expansion of `significand * 2^exponent`.
///
/// WHAT: a non-negative exponent shifts the significand, while a negative one clears the denominator
///       with `5^-exponent` because `2^-n` is `5^n / 10^n`.
/// WHY: binary16 midpoints have an odd significand below `2^12` and an exponent of at least -25, so
///       the expansion always fits a `u128` exactly.
fn binary_decimal_expansion(significand: u64, exponent: i32) -> (u128, i64) {
    if exponent >= 0 {
        return ((significand as u128) << exponent as u32, 0);
    }

    let denominator = (-exponent) as u32;
    let numerator = (significand as u128) * 5u128.pow(denominator);

    (numerator, -i64::from(denominator))
}

/// Compare `authored_digits * 10^authored_scale` with `exact_digits * 10^exact_scale`.
///
/// WHAT: compares the decimal position of each most significant digit first, then the aligned digit
///       runs, padding the shorter run with zeros.
/// WHY:  equal positions mean both values start at the same decimal place, which is what makes a
///       digit-by-digit comparison exact without scaling either side.
fn compare_scaled_decimals(
    authored_digits: &str,
    authored_scale: i64,
    exact_digits: u128,
    exact_scale: i64,
) -> Ordering {
    let exact_text = exact_digits.to_string();
    let authored_position = authored_scale + authored_digits.len() as i64;
    let exact_position = exact_scale + exact_text.len() as i64;

    if authored_position != exact_position {
        return authored_position.cmp(&exact_position);
    }

    let compared_length = authored_digits.len().max(exact_text.len());
    for index in 0..compared_length {
        let authored_digit = authored_digits
            .as_bytes()
            .get(index)
            .copied()
            .unwrap_or(b'0');
        let exact_digit = exact_text.as_bytes().get(index).copied().unwrap_or(b'0');

        match authored_digit.cmp(&exact_digit) {
            Ordering::Equal => {}

            ordering => return ordering,
        }
    }

    Ordering::Equal
}

#[cfg(test)]
#[path = "tests/binary16_tests.rs"]
mod tests;
