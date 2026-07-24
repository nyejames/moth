//! Numeric literal parsing and materialization.
//!
//! WHAT: parses an unsigned numeric literal text into a structured token payload and
//!       provides small materialization helpers for consumers that need `i32`/`f64` values.
//! WHY: the tokenizer and future string casts must share one grammar owner so separator,
//!      exponent, and sign rules stay consistent.

use crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason;
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

/// Materialize a whole-number token to a signed `i32`.
///
/// WHY: Moth Alpha `Int` is a signed 32-bit integer. Source numeric literals must
///      fit that range at materialization time, before they are widened into the
///      `ExpressionKind::Int(i32)` carrier used elsewhere.
///
/// Negative magnitudes one larger than `i32::MAX` are accepted for the exact
/// `-2147483648` boundary, matching normal signed-integer parsing rules.
pub fn materialize_i32(
    token: &NumericLiteralToken,
    string_table: &StringTable,
) -> Result<i32, NumberLiteralErrorReason> {
    materialize_i32_with_sign(token, token.sign, string_table)
}

/// Materialize a whole-number token with an explicit effective sign.
///
/// WHY: tokenizer-signed literals and parser-owned unary-negation fallbacks both need
///      the same signed-i32 boundary policy. Passing the effective sign keeps that
///      policy centralized without reconstructing token payloads.
pub(crate) fn materialize_i32_with_sign(
    token: &NumericLiteralToken,
    sign: NumericLiteralSign,
    string_table: &StringTable,
) -> Result<i32, NumberLiteralErrorReason> {
    let text = string_table.resolve(token.normalized_text);

    materialize_i32_text_with_sign(text, sign)
}

fn materialize_i32_text_with_sign(
    text: &str,
    sign: NumericLiteralSign,
) -> Result<i32, NumberLiteralErrorReason> {
    match sign {
        NumericLiteralSign::Positive => text
            .parse::<i32>()
            .map_err(|_| NumberLiteralErrorReason::OutsideIntRange),

        NumericLiteralSign::Negative => {
            let magnitude = text
                .parse::<u32>()
                .map_err(|_| NumberLiteralErrorReason::OutsideIntRange)?;

            if magnitude == 0 {
                return Ok(0);
            }

            let max_negative_magnitude = (i32::MAX as u32) + 1;
            if magnitude > max_negative_magnitude {
                return Err(NumberLiteralErrorReason::OutsideIntRange);
            }

            // The range check guarantees this is either a valid negative i32 or
            // exactly i32::MIN, which corresponds to magnitude == 2147483648.
            if magnitude == max_negative_magnitude {
                Ok(i32::MIN)
            } else {
                Ok(-(magnitude as i32))
            }
        }
    }
}

/// Parse signed numeric text into an `i32` using the Moth whole-number grammar.
///
/// WHAT: applies the shared numeric text grammar to an entire input string, including
///       an optional leading `-`, rejects non-whole-number forms, then materializes
///       the signed value through the same i32 boundary helper as source literals.
/// WHY: `String -> Int` casts must agree with source literal range and separator rules
///      without reimplementing sign/range policy in the cast subsystem.
pub fn parse_numeric_text_to_i32(source: &str) -> Result<i32, NumberLiteralErrorReason> {
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

    materialize_i32_text_with_sign(&parsed.normalized_text, sign)
}

/// Materialize a decimal or exponent token to a signed, finite `f64`.
pub fn materialize_f64(
    token: &NumericLiteralToken,
    string_table: &StringTable,
) -> Result<f64, NumberLiteralErrorReason> {
    let text = string_table.resolve(token.normalized_text);

    let value = text
        .parse::<f64>()
        .map_err(|_| NumberLiteralErrorReason::ParseOverflow)?;

    if !value.is_finite() {
        return Err(NumberLiteralErrorReason::NonFiniteFloat);
    }

    if token.sign == NumericLiteralSign::Negative {
        Ok(-value)
    } else {
        Ok(value)
    }
}

/// Parse a signed numeric text string into a finite `f64`.
///
/// WHAT: applies the shared Moth numeric text grammar to an entire input
///       string, including an optional leading `-`, then checks that the
///       resulting `f64` is finite.
/// WHY: `String -> Float` casts must agree with source numeric literals on
///      separator, exponent, sign, and whitespace rules without duplicating
///      grammar logic in the cast policy or backend runtime.
pub fn parse_numeric_text_to_f64(source: &str) -> Result<f64, NumberLiteralErrorReason> {
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

    let signed_text = if negative {
        format!("-{}", parsed.normalized_text)
    } else {
        parsed.normalized_text
    };

    let value = signed_text
        .parse::<f64>()
        .map_err(|_| NumberLiteralErrorReason::ParseOverflow)?;

    if !value.is_finite() {
        return Err(NumberLiteralErrorReason::NonFiniteFloat);
    }

    Ok(value)
}

#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod tests;
