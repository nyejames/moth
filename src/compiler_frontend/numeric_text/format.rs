//! Moth finite binary-float formatting contract.
//!
//! WHAT: converts a finite binary-float value (carried as `f64`) into the shortest
//!       round-trippable decimal string defined by the Moth language contract, at the
//!       caller's `BinaryFloatPrecision`.
//! WHY: AST constant folding, builtin casts, and compile-time template
//!      interpolation must all agree on binary-float stringification without relying on
//!      Rust, JavaScript, or host-native formatting quirks, for `Float` and the fixed
//!      binary floats alike.
//!
//! Contract:
//! - finite values only;
//! - shortest decimal that round-trips at the selected precision (`f32` shortest text for
//!   `Binary32`, `f64` shortest text for `Binary64`, and the shortest `F16` text for
//!   `Binary16`, which has no native shortest-decimal implementation on this toolchain);
//! - exponent form when `abs(value) >= 1e21` or `0 < abs(value) < 1e-6`;
//! - lowercase `e`;
//! - positive exponents include `+`;
//! - `-0.0` formats as `0`;
//! - omit trailing `.0`.

use std::fmt;

use crate::compiler_frontend::datatypes::numeric_scalar::BinaryFloatPrecision;
use crate::compiler_frontend::numeric_text::binary16::decimal_to_f16;

/// Threshold above which Moth always uses exponent notation.
const EXPONENT_THRESHOLD_HIGH: f64 = 1e21;

/// Threshold below which Moth always uses exponent notation (but not for zero).
const EXPONENT_THRESHOLD_LOW: f64 = 1e-6;

/// The only failure mode for the finite-Float formatter.
///
/// WHAT: the formatter promises to produce the Moth contract only for
///       finite inputs. Non-finite values are rejected so callers can decide
///       whether this is an internal invariant violation or a user-facing
///       diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatFormatError {
    NonFiniteFloat,
}

impl fmt::Display for FloatFormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FloatFormatError::NonFiniteFloat => formatter.write_str("Float value is not finite"),
        }
    }
}

impl std::error::Error for FloatFormatError {}

/// Format a finite binary-float value at one precision according to the Moth contract.
///
/// WHAT: `Binary64` formats the shortest `f64` representation, `Binary32` the shortest `f32`
///       representation (the `f64` carrier must already hold an `f32`-exact value), and
///       `Binary16` the shortest `F16` representation. The existing re-thresholding and exponent
///       formatting apply to all three.
/// WHY: the value's own precision owns the stringification, so `Float32` values such as
///      `f64::from(0.1f32)` format as `"0.1"` rather than the long `f64` expansion of that
///      carrier, and fixed `F16`/`F32` values format through the same contract as `Float`.
///
/// Returns [`FloatFormatError::NonFiniteFloat`] for `NaN` or infinities.
/// All finite values, including `-0.0`, produce a deterministic decimal string.
pub fn format_finite_float(
    value: f64,
    precision: BinaryFloatPrecision,
) -> Result<String, FloatFormatError> {
    if !value.is_finite() {
        return Err(FloatFormatError::NonFiniteFloat);
    }

    // Normalize negative zero to positive zero so the contract prints "0".
    let value = if value == 0.0 { 0.0 } else { value };
    if value == 0.0 {
        return Ok("0".to_string());
    }

    let shortest = shortest_decimal(value, precision);

    // Ryū and the binary16 search both return the shortest round-trippable decimal, but their
    // exponent thresholds and sign rules differ from Moth's. Parse the output into an exact
    // integer-mantissa / decimal-exponent form so we can re-render it with the correct thresholds
    // and decorations.
    let (is_negative, mantissa_digits, fractional_digit_count, scientific_exponent) =
        parse_ryu_output(&shortest)?;

    let decimal_exponent = scientific_exponent - fractional_digit_count;
    let digits = trim_leading_zeros(&mantissa_digits);
    if digits.is_empty() {
        return Ok("0".to_string());
    }

    let abs_value = value.abs();
    let use_exponent = abs_value >= EXPONENT_THRESHOLD_HIGH
        || (abs_value > 0.0 && abs_value < EXPONENT_THRESHOLD_LOW);

    let formatted = if use_exponent {
        format_exponent(digits, decimal_exponent)
    } else {
        format_fixed(digits, decimal_exponent)
    };

    if is_negative {
        Ok(format!("-{formatted}"))
    } else {
        Ok(formatted)
    }
}

/// Returns the shortest decimal text that round-trips at one binary-float precision.
///
/// WHAT: `Binary64` and `Binary32` use Ryū's shortest round-trippable text for `f64` and `f32`;
///       `Binary16` searches its own digit budget because there is no native binary16 shortest
///       decimal implementation on this toolchain.
/// WHY: every precision needs the same "shortest text that reads back to the same bits" property,
///      so the precision choice is made once here instead of at each formatting call site.
fn shortest_decimal(value: f64, precision: BinaryFloatPrecision) -> String {
    let mut buffer = ryu::Buffer::new();
    match precision {
        // The carrier is documented to hold an f32-exact value under `Binary32`, so this
        // narrowing cast is a representation change, not a second rounding of the source text:
        // materialisation already rounded directly into `f32`.
        BinaryFloatPrecision::Binary32 => buffer.format_finite(value as f32).to_owned(),
        BinaryFloatPrecision::Binary64 => buffer.format_finite(value).to_owned(),
        BinaryFloatPrecision::Binary16 => shortest_binary16_decimal(value),
    }
}

/// Significant decimal digits that round-trip every finite binary16 value.
///
/// WHY: binary16 has 11 significand bits, so its maximum-digits-10 count is 5.
const BINARY16_SIGNIFICANT_DIGIT_BUDGET: usize = 5;

/// Returns the shortest decimal text that rounds back to one binary16-exact `f64` value.
///
/// WHAT: tries increasing significant-digit counts and returns the first candidate that reads back
///       to the same binary16 value.
/// WHY: this is the same shortest round-trip rule Ryū applies to `f32` and `f64`, so `F16`
///      prints like the other binary floats: `65504` prints as `65500`, just as the `f32`
///      value `123456792` prints as `123456790`.
fn shortest_binary16_decimal(value: f64) -> String {
    let magnitude = value.abs();
    let text = (1..=BINARY16_SIGNIFICANT_DIGIT_BUDGET)
        .find_map(|digits| round_tripping_binary16_decimal(magnitude, digits))
        // Unreachable: the budget round-trips every finite binary16 value. Printing the exact
        // carrier keeps the value correct if that invariant were ever broken.
        .unwrap_or_else(|| format!("{magnitude:e}"));

    if value.is_sign_negative() {
        format!("-{text}")
    } else {
        text
    }
}

/// Returns a decimal with `digits` significant digits that reads back to `magnitude` as binary16.
///
/// WHAT: tries the correctly rounded candidate first, then its two neighbours in the last digit,
///       and renders the winner as `<significand>e<exponent>` without trailing zeros.
/// WHY: at a power of two the binary16 rounding interval is asymmetric (the gap below is half the
///      gap above), so the nearest candidate can fall outside the interval while its neighbour on
///      the wider side lies inside it: `2^-6` prints as `0.01563`, not `0.015625`.
fn round_tripping_binary16_decimal(magnitude: f64, digits: usize) -> Option<String> {
    // Rust's exponential formatting is exact and correctly rounded, so the nearest candidate never
    // carries a second rounding of the value it was formatted from.
    let nearest = format!("{:.*e}", digits - 1, magnitude);
    let (mantissa, exponent) = nearest.split_once('e')?;
    let significand: u32 = mantissa.replace('.', "").parse().ok()?;
    let exponent = exponent.parse::<i32>().ok()? - (digits as i32 - 1);

    [significand, significand + 1, significand - 1]
        .into_iter()
        .map(|candidate| {
            let (mut candidate, mut exponent) = (candidate, exponent);
            while candidate != 0 && candidate % 10 == 0 {
                candidate /= 10;
                exponent += 1;
            }
            format!("{candidate}e{exponent}")
        })
        .find(|candidate| decimal_to_f16(candidate, false) == Some(magnitude))
}

/// Splits a formatted decimal string into sign, digit string, fractional digit count,
/// and scientific exponent.
///
/// `source` is expected to be a finite decimal in one of these forms:
/// - fixed: `123`, `1.5`, `0.000001`
/// - scientific: `1e-7`, `1.23e+21`, `-1.5e6`
fn parse_ryu_output(source: &str) -> Result<(bool, String, i32, i32), FloatFormatError> {
    let (signless, is_negative) = if let Some(rest) = source.strip_prefix('-') {
        (rest, true)
    } else {
        (source, false)
    };

    let (mantissa_part, exponent_part) = if let Some(position) = signless.find(['e', 'E']) {
        signless.split_at(position)
    } else {
        (signless, "")
    };

    if mantissa_part.is_empty() {
        return Err(FloatFormatError::NonFiniteFloat);
    }

    let (integer_part, fractional_part) = if let Some(position) = mantissa_part.find('.') {
        mantissa_part.split_at(position)
    } else {
        (mantissa_part, "")
    };

    // Concatenate integer and fractional digits so the mantissa becomes one
    // clean digit string. The decimal point's position is captured by the
    // fractional-digit count.
    let mut mantissa_digits = String::with_capacity(mantissa_part.len());
    mantissa_digits.push_str(integer_part);
    // Skip the '.' itself, if there was one.
    if fractional_part.len() > 1 {
        mantissa_digits.push_str(&fractional_part[1..]);
    }

    let fractional_digit_count = if fractional_part.is_empty() {
        0
    } else {
        // `fractional_part` includes the leading '.', so subtract one.
        (fractional_part.len() - 1) as i32
    };

    let scientific_exponent = if exponent_part.is_empty() {
        0
    } else {
        // Skip the 'e' or 'E'.
        exponent_part[1..]
            .parse::<i32>()
            .map_err(|_| FloatFormatError::NonFiniteFloat)?
    };

    Ok((
        is_negative,
        mantissa_digits,
        fractional_digit_count,
        scientific_exponent,
    ))
}

fn trim_leading_zeros(source: &str) -> &str {
    let trimmed = source.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

/// Render the value in fixed-point notation, dropping a trailing `.0`.
fn format_fixed(digits: &str, decimal_exponent: i32) -> String {
    if decimal_exponent >= 0 {
        let zeros = decimal_exponent as usize;
        let mut result = String::with_capacity(digits.len() + zeros);
        result.push_str(digits);
        result.extend(std::iter::repeat_n('0', zeros));
        return result;
    }

    let zeros = (-decimal_exponent) as usize;

    if digits.len() > zeros {
        let split = digits.len() - zeros;
        let integer_part = &digits[..split];
        let fractional_part = &digits[split..];
        let trimmed_fraction = fractional_part.trim_end_matches('0');

        if trimmed_fraction.is_empty() {
            integer_part.to_string()
        } else {
            format!("{integer_part}.{trimmed_fraction}")
        }
    } else {
        let leading_zeros = zeros - digits.len();
        let fractional_part = format!("{:0>width$}", digits, width = leading_zeros + digits.len());
        let trimmed_fraction = fractional_part.trim_end_matches('0');

        if trimmed_fraction.is_empty() {
            "0".to_string()
        } else {
            format!("0.{trimmed_fraction}")
        }
    }
}

/// Render the value in scientific notation with one digit before the decimal
/// point, lowercase `e`, and an explicit `+` for positive exponents.
fn format_exponent(digits: &str, decimal_exponent: i32) -> String {
    let digit_count = digits.len() as i32;
    let scientific_exponent = decimal_exponent + (digit_count - 1);

    let first_digit = &digits[..1];
    let rest = &digits[1..];
    let trimmed_rest = rest.trim_end_matches('0');

    let mantissa = if trimmed_rest.is_empty() {
        first_digit.to_string()
    } else {
        format!("{first_digit}.{trimmed_rest}")
    };

    let exponent_sign = if scientific_exponent >= 0 { "+" } else { "" };
    format!("{mantissa}e{exponent_sign}{scientific_exponent}")
}

#[cfg(test)]
#[path = "tests/format_tests.rs"]
mod tests;
