//! Numeric literal parsing and materialization.
//!
//! WHAT: parses an unsigned numeric literal text into a structured token payload and
//!       provides small materialization helpers that type numbers under a caller-supplied
//!       `IntWidth` or `FloatPrecision`.
//! WHY: the tokenizer and string casts must share one grammar owner so separator,
//!      exponent, and sign rules stay consistent, while the compilation boundary profile
//!      owns every range and rounding decision.

use crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason;
use crate::compiler_frontend::datatypes::numeric_profile::{FloatPrecision, IntWidth};
use crate::compiler_frontend::numeric_text::grammar::{
    is_digit_separator, is_exponent_marker, is_exponent_sign, is_numeric_digit,
};
use crate::compiler_frontend::numeric_text::token::{
    NumericExponentSign, NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Result of parsing a numeric literal from its source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedNumericLiteral {
    pub normalized_text: String,
    pub kind: NumericLiteralKind,
    pub digit_count: u32,
    pub fractional_digit_count: u32,
    pub exponent_digit_count: u32,
    pub exponent_sign: NumericExponentSign,
}

/// Parse an unsigned numeric literal from the start of `source`.
///
/// WHY: the caller decides where the literal ends, so this function validates the
///      whole provided text rather than scanning a larger buffer.
///
/// `source` must start with a digit. Leading signs are handled by callers because
/// Moth tokenizes `-` as a separate operator in this phase.
pub fn parse_numeric_literal(
    source: &str,
) -> Result<ParsedNumericLiteral, NumberLiteralErrorReason> {
    let mut characters = source.chars().peekable();

    let mut normalized_text = String::with_capacity(source.len());
    let mut integer_digits: u32 = 0;
    let mut fractional_digits: u32 = 0;
    let mut exponent_digits: u32 = 0;

    let mut last_was_digit = false;
    let mut last_was_separator = false;

    // ------------------
    //  Integer part
    // ------------------
    while let Some(&character) = characters.peek() {
        if is_numeric_digit(character) {
            normalized_text.push(character);
            integer_digits += 1;
            last_was_digit = true;
            last_was_separator = false;
            characters.next();
        } else if is_digit_separator(character) {
            if !last_was_digit {
                return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
            }
            if last_was_separator {
                return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
            }
            last_was_digit = false;
            last_was_separator = true;
            characters.next();
        } else {
            break;
        }
    }

    if integer_digits == 0 {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    }

    if last_was_separator {
        if characters.peek().is_some() {
            return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
        }
        return Err(NumberLiteralErrorReason::EndsWithSeparator);
    }

    // ------------------
    //  Fractional part
    // ------------------
    let mut has_decimal_point = false;

    if characters.peek() == Some(&'.') {
        if !last_was_digit {
            return Err(NumberLiteralErrorReason::DecimalPointNotAfterDigit);
        }

        has_decimal_point = true;
        normalized_text.push('.');
        last_was_digit = false;
        last_was_separator = false;
        characters.next();

        let mut saw_fractional_digit = false;

        while let Some(&character) = characters.peek() {
            if is_numeric_digit(character) {
                normalized_text.push(character);
                fractional_digits += 1;
                saw_fractional_digit = true;
                last_was_digit = true;
                last_was_separator = false;
                characters.next();
            } else if is_digit_separator(character) {
                if !last_was_digit {
                    return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
                }
                if last_was_separator {
                    return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
                }
                last_was_digit = false;
                last_was_separator = true;
                characters.next();
            } else {
                break;
            }
        }

        if last_was_separator {
            if characters.peek().is_some() {
                return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
            }
            return Err(NumberLiteralErrorReason::EndsWithSeparator);
        }

        if !saw_fractional_digit {
            return Err(NumberLiteralErrorReason::MissingFractionalDigits);
        }
    }

    // ------------------
    //  Exponent part
    // ------------------
    let mut exponent_sign = NumericExponentSign::None;

    if let Some(&character) = characters.peek()
        && is_exponent_marker(character)
    {
        if character == 'E' {
            return Err(NumberLiteralErrorReason::UppercaseExponentMarker);
        }

        normalized_text.push('e');
        last_was_digit = false;
        last_was_separator = false;
        characters.next();

        if let Some(&sign_character) = characters.peek()
            && is_exponent_sign(sign_character)
        {
            exponent_sign = if sign_character == '+' {
                NumericExponentSign::Positive
            } else {
                NumericExponentSign::Negative
            };
            normalized_text.push(sign_character);
            characters.next();
        }

        let mut saw_exponent_digit = false;

        while let Some(&character) = characters.peek() {
            if is_numeric_digit(character) {
                normalized_text.push(character);
                exponent_digits += 1;
                saw_exponent_digit = true;
                last_was_digit = true;
                last_was_separator = false;
                characters.next();
            } else if is_digit_separator(character) {
                if !last_was_digit {
                    return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
                }
                if last_was_separator {
                    return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
                }
                last_was_digit = false;
                last_was_separator = true;
                characters.next();
            } else {
                break;
            }
        }

        if last_was_separator {
            if characters.peek().is_some() {
                return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
            }
            return Err(NumberLiteralErrorReason::EndsWithSeparator);
        }

        if !saw_exponent_digit {
            return Err(NumberLiteralErrorReason::MissingExponentDigits);
        }
    }

    if characters.peek().is_some() {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    }

    let kind = if exponent_digits > 0 {
        NumericLiteralKind::Exponent
    } else if has_decimal_point {
        NumericLiteralKind::DecimalPoint
    } else {
        NumericLiteralKind::WholeNumber
    };

    Ok(ParsedNumericLiteral {
        normalized_text,
        kind,
        digit_count: integer_digits + fractional_digits + exponent_digits,
        fractional_digit_count: fractional_digits,
        exponent_digit_count: exponent_digits,
        exponent_sign,
    })
}

/// Materialize a whole-number token to a signed `Int` at the boundary width.
///
/// WHY: source numeric literals must fit the compilation boundary's `Int` width at
///      materialization time, before they enter the `ExpressionKind::Int(i64)` carrier.
///      Callers with no sign override pass `token.sign`.
///
/// Negative magnitudes one larger than the width maximum are accepted for the exact
/// minimum boundary (such as `-2147483648` under `Bits32`), matching normal
/// signed-integer parsing rules.
pub(crate) fn materialize_int(
    token: &NumericLiteralToken,
    sign: NumericLiteralSign,
    width: IntWidth,
    string_table: &StringTable,
) -> Result<i64, NumberLiteralErrorReason> {
    let text = string_table.resolve(token.normalized_text);

    materialize_int_text_with_sign(text, sign, width)
}

/// Shared signed whole-number materialization for one `Int` width.
///
/// WHY: source literals, normalized MON facts and `String -> Int` casts share one
///      boundary policy. Positive magnitudes parse directly as `i64` and are
///      range-checked; negative magnitudes parse as `u64` so the exact minimum
///      (whose magnitude exceeds the maximum by one) is accepted without overflow.
fn materialize_int_text_with_sign(
    text: &str,
    sign: NumericLiteralSign,
    width: IntWidth,
) -> Result<i64, NumberLiteralErrorReason> {
    match sign {
        NumericLiteralSign::Positive => {
            let value = text
                .parse::<i64>()
                .map_err(|_| NumberLiteralErrorReason::OutsideIntRange)?;

            if width.contains(value) {
                Ok(value)
            } else {
                Err(NumberLiteralErrorReason::OutsideIntRange)
            }
        }

        NumericLiteralSign::Negative => {
            let magnitude = text
                .parse::<u64>()
                .map_err(|_| NumberLiteralErrorReason::OutsideIntRange)?;

            if magnitude == 0 {
                return Ok(0);
            }

            // The range check below guarantees the negation cannot overflow: the
            // accepted magnitudes are exactly `1..=max+1`, and `max+1` maps to the
            // width minimum.
            let max_magnitude = (width.max_value() as u64) + 1;
            if magnitude > max_magnitude {
                return Err(NumberLiteralErrorReason::OutsideIntRange);
            }

            if magnitude == max_magnitude {
                Ok(width.min_value())
            } else {
                Ok(-(magnitude as i64))
            }
        }
    }
}

/// Materialize already validated normalized whole-number text at one `Int` width.
///
/// WHAT: reuses the shared signed-width boundary policy without reparsing the literal
///       grammar. MON has already validated separators, kind and digit counts through
///       `parse_numeric_literal`.
/// WHY: MON `Int` retains normalized facts from its single parse; reparsing the original
///      text would normalize the same literal twice.
pub(crate) fn materialize_normalized_int(
    normalized: &str,
    negative: bool,
    width: IntWidth,
) -> Result<i64, NumberLiteralErrorReason> {
    let sign = if negative {
        NumericLiteralSign::Negative
    } else {
        NumericLiteralSign::Positive
    };
    materialize_int_text_with_sign(normalized, sign, width)
}

/// Shared float materialization for one `Float` precision.
///
/// WHAT: parses the unsigned normalized text directly at the destination precision
///       (`parse::<f32>` under `Bits32`, `parse::<f64>` under `Bits64`), then applies
///       the token sign by negation so no signed intermediate string is allocated.
/// WHY: binary float literals round directly to their destination with round-to-nearest,
///      ties-to-even. Parsing at the destination precision avoids double rounding through
///      an `f64` intermediate; a non-finite rounded result is rejected with the existing
///      reasons. Signed zero and subnormals are preserved because the parse handles them.
fn materialize_float_text_with_sign(
    normalized: &str,
    negative: bool,
    precision: FloatPrecision,
) -> Result<f64, NumberLiteralErrorReason> {
    let magnitude = match precision {
        FloatPrecision::Bits64 => normalized
            .parse::<f64>()
            .map_err(|_| NumberLiteralErrorReason::ParseOverflow)?,
        FloatPrecision::Bits32 => {
            let rounded = normalized
                .parse::<f32>()
                .map_err(|_| NumberLiteralErrorReason::ParseOverflow)?;
            f64::from(rounded)
        }
    };

    if !magnitude.is_finite() {
        return Err(NumberLiteralErrorReason::NonFiniteFloat);
    }

    Ok(if negative { -magnitude } else { magnitude })
}

/// Parse signed numeric text into an `Int` at one width using the Moth whole-number grammar.
///
/// WHAT: applies the shared numeric text grammar to an entire input string, including
///       an optional leading `-`, rejects non-whole-number forms, then materializes
///       the signed value through the same width boundary helper as source literals.
/// WHY: `String -> Int` casts must agree with source literal range and separator rules
///      without reimplementing sign/range policy in the cast subsystem.
pub(crate) fn parse_numeric_text_to_int(
    source: &str,
    width: IntWidth,
) -> Result<i64, NumberLiteralErrorReason> {
    if source.is_empty() {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    }

    let (sign, unsigned) = if let Some(rest) = source.strip_prefix('-') {
        (NumericLiteralSign::Negative, rest)
    } else if source.starts_with('+') {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    } else {
        (NumericLiteralSign::Positive, source)
    };

    if unsigned.is_empty() {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    }

    let parsed = parse_numeric_literal(unsigned)?;
    if parsed.kind != NumericLiteralKind::WholeNumber {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    }

    materialize_int_text_with_sign(&parsed.normalized_text, sign, width)
}

/// Materialize a decimal or exponent token to a signed, finite `Float` at one precision.
///
/// WHY: source float literals must round directly at the compilation boundary's `Float`
///      precision, before they enter the `f64` carrier used elsewhere.
pub(crate) fn materialize_float(
    token: &NumericLiteralToken,
    precision: FloatPrecision,
    string_table: &StringTable,
) -> Result<f64, NumberLiteralErrorReason> {
    let text = string_table.resolve(token.normalized_text);

    materialize_float_text_with_sign(text, token.sign == NumericLiteralSign::Negative, precision)
}

/// Materialize already validated normalized numeric text to a finite `Float` at one precision.
///
/// WHAT: parses the unsigned normalized text once at the destination precision, then
///       applies the token sign by negation so no signed intermediate string is allocated.
/// WHY: MON `Float` retains normalized facts from its single parse; rebuilding a signed
///      string would allocate a duplicate scratch buffer.
pub(crate) fn materialize_normalized_float(
    normalized: &str,
    negative: bool,
    precision: FloatPrecision,
) -> Result<f64, NumberLiteralErrorReason> {
    materialize_float_text_with_sign(normalized, negative, precision)
}

/// Parse a signed numeric text string into a finite `Float` at one precision.
///
/// WHAT: applies the shared Moth numeric text grammar to an entire input
///       string, including an optional leading `-`, parses directly at the destination
///       precision, then checks that the resulting `f64` is finite.
/// WHY: `String -> Float` casts must agree with source numeric literals on
///      separator, exponent, sign, and whitespace rules without duplicating
///      grammar logic in the cast policy or backend runtime.
pub(crate) fn parse_numeric_text_to_float(
    source: &str,
    precision: FloatPrecision,
) -> Result<f64, NumberLiteralErrorReason> {
    if source.is_empty() {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    }

    let (negative, unsigned) = if let Some(rest) = source.strip_prefix('-') {
        (true, rest)
    } else if source.starts_with('+') {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    } else {
        (false, source)
    };

    if unsigned.is_empty() {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    }

    let parsed = parse_numeric_literal(unsigned)?;

    materialize_float_text_with_sign(&parsed.normalized_text, negative, precision)
}

#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod tests;
