//! Moth finite `Float` formatting contract.
//!
//! WHAT: converts a finite `f64` value into the shortest round-trippable decimal
//!       string defined by the Moth language contract.
//! WHY: AST constant folding, builtin casts, and compile-time template
//!      interpolation must all agree on Float stringification without relying on
//!      Rust, JavaScript, or host-native formatting quirks.
//!
//! Contract:
//! - finite `f64` only;
//! - shortest round-trippable decimal;
//! - exponent form when `abs(value) >= 1e21` or `0 < abs(value) < 1e-6`;
//! - lowercase `e`;
//! - positive exponents include `+`;
//! - `-0.0` formats as `0`;
//! - omit trailing `.0`.

use std::fmt;

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

/// Format a finite `f64` according to the Moth Float contract.
///
/// Returns [`FloatFormatError::NonFiniteFloat`] for `NaN` or infinities.
/// All finite values, including `-0.0`, produce a deterministic decimal string.
pub fn format_finite_float(value: f64) -> Result<String, FloatFormatError> {
    if !value.is_finite() {
        return Err(FloatFormatError::NonFiniteFloat);
    }

    // Normalize negative zero to positive zero so the contract prints "0".
    let value = if value == 0.0 { 0.0 } else { value };
    if value == 0.0 {
        return Ok("0".to_string());
    }

    let mut buffer = ryu::Buffer::new();
    let ryu_output = buffer.format_finite(value);

    // Ryū returns the shortest round-trippable decimal, but its exponent
    // thresholds and sign rules differ from Moth's. Parse its output into
    // an exact integer-mantissa / decimal-exponent form so we can re-render it
    // with the correct thresholds and decorations.
    let (is_negative, mantissa_digits, fractional_digit_count, scientific_exponent) =
        parse_ryu_output(ryu_output)?;

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

/// Splits a Ryū output string into sign, digit string, fractional digit count,
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
