//! Pure numeric parsing and fixed-width scalar materialization.
//!
//! WHAT: parses unsigned numeric text into structured normalized facts and applies shared
//!       width/precision rules to a caller-supplied destination.
//! WHY: numeric spellings and destination rounding must agree across compiler and data readers.
//!
//! Token storage, `StringTable` resolution, compiler diagnostics and arbitrary-precision `Dec`
//! values remain compiler-owned adapters.
use super::binary16::decimal_to_f16;
use super::decimal::NumberScale;
use super::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use super::grammar::{
    NumericExponentSign, NumericLiteralKind, NumericLiteralSign, is_digit_separator,
    is_exponent_marker, is_exponent_sign, is_numeric_digit,
};
use super::precision::BinaryFloatPrecision;
use super::profile::{FloatPrecision, IntWidth};
/// Structured reasons shared by numeric parsing and compiler diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberLiteralErrorReason {
    SeparatorNotBetweenDigits,
    MultipleDecimalPoints,
    DecimalPointNotAfterDigit,
    EndsWithSeparator,
    MissingFractionalDigits,
    UppercaseExponentMarker,
    MissingExponentDigits,
    InvalidExponentSignPlacement,
    InvalidSeparatorPlacement,
    OutsideIntRange,
    /// A literal is outside the inclusive range of the fixed scalar it initialises.
    OutsideFixedScalarRange(FixedScalar),
    /// A negative literal cannot initialise an unsigned fixed scalar or `Byte`.
    NegativeUnsignedLiteral(FixedScalar),
    /// A fixed binary float literal rounded to a non-finite value at its destination.
    NonFiniteFixedFloat(FixedScalar),
    NonFiniteFloat,
    /// A literal's exact decimal value does not fit the receiving `Dec` scale.
    ///
    /// WHAT: the literal's smallest exact scale exceeds the destination scale, so a checked
    ///       exact receiving boundary rejects it instead of rounding or trimming digits.
    /// WHY: the scale stays structured so the renderer names the receiving `Dec` identity exactly
    ///      like the fixed-scalar range reasons name their destination.
    InexactNumberScale(NumberScale),
    ParseOverflow,
}

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

/// Bound every digit counter before normalization reserves or accumulates text.
///
/// Each digit consumes at least one source byte, so this one check also bounds the sum of the
/// integer, fractional and exponent counts without a checked addition on every digit.
fn check_numeric_input_length(length: usize) -> Result<(), NumberLiteralErrorReason> {
    u32::try_from(length)
        .map(|_| ())
        .map_err(|_| NumberLiteralErrorReason::ParseOverflow)
}

/// Parse an unsigned numeric literal from the start of `source`.
///
/// WHY: the caller decides where the literal ends, so this function validates the
///      whole provided text rather than scanning a larger buffer.
///
/// `source` must start with a digit. Leading signs are handled by callers because
/// Moth tokenizes `-` as a separate operator in this phase.
///
/// Inputs whose byte length exceeds the `u32` digit-count capacity return
/// [`NumberLiteralErrorReason::ParseOverflow`] before normalization allocates.
pub fn parse_numeric_literal(
    source: &str,
) -> Result<ParsedNumericLiteral, NumberLiteralErrorReason> {
    check_numeric_input_length(source.len())?;

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
///
/// # Input contract
///
/// `normalized` must be the unsigned, separator-free `normalized_text` produced by
/// [`parse_numeric_literal`] with [`NumericLiteralKind::WholeNumber`]. Retained parser facts may
/// provide the same text and kind. This low-level operation does not validate the grammar again;
/// use [`parse_numeric_text_to_int`] for unchecked text.
pub fn materialize_normalized_int(
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
pub fn parse_numeric_text_to_int(
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

/// Materialize already validated normalized numeric text to a finite `Float` at one precision.
///
/// WHAT: parses the unsigned normalized text once at the destination precision, then
///       applies the token sign by negation so no signed intermediate string is allocated.
/// WHY: MON `Float` retains normalized facts from its single parse; rebuilding a signed
///      string would allocate a duplicate scratch buffer.
///
/// # Input contract
///
/// `normalized` must be the unsigned, separator-free `normalized_text` produced by
/// [`parse_numeric_literal`], or equivalent retained parser facts. This low-level operation does
/// not validate the grammar again; use [`parse_numeric_text_to_float`] for unchecked text.
pub fn materialize_normalized_float(
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
pub fn parse_numeric_text_to_float(
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

/// Parse signed numeric text into one fixed scalar using the Moth whole-string grammar.
///
/// WHAT: applies the shared numeric text grammar to an entire input string, including an optional
///       leading `-`, then materialises the value directly at the destination scalar. Integer
///       destinations accept whole-number spelling only and check their own exact range, unsigned
///       destinations reject every negative spelling including `-0`, and binary-float destinations
///       accept a whole number or a decimal/exponent and round once at their own precision.
/// WHY: `String -> U8` and the other fixed text conversions must agree with source literals on
///      separator, sign, and whitespace rules, and must stay exact above `2^53` rather than
///      passing through an `Int` or `Float` intermediate.
pub fn parse_numeric_text_to_fixed_scalar(
    source: &str,
    scalar: FixedScalar,
) -> Result<FixedScalarValue, NumberLiteralErrorReason> {
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

    match scalar.class() {
        // An integer destination requires whole-number text, so a decimal point or exponent is a
        // spelling failure rather than a range failure.
        FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger => {
            if parsed.kind != NumericLiteralKind::WholeNumber {
                return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
            }

            if scalar.class() == FixedScalarClass::SignedInteger {
                materialize_signed_fixed_scalar(&parsed.normalized_text, sign, scalar)
            } else {
                materialize_unsigned_fixed_scalar(&parsed.normalized_text, sign, scalar)
            }
        }

        FixedScalarClass::BinaryFloat => {
            materialize_binary_float_fixed_scalar(&parsed.normalized_text, sign, scalar)
        }

        // `Byte` is outside the numeric vocabulary, so no text conversion reaches it; the format
        // reason keeps this routine total without a panic path.
        FixedScalarClass::Octet => Err(NumberLiteralErrorReason::InvalidSeparatorPlacement),
    }
}

// -----------------------------------------------------------
//  Fixed Scalar Materialization
// -----------------------------------------------------------

/// True when a literal of `kind` may initialise `scalar` at a direct receiving boundary.
///
/// WHAT: whole-number literals initialise every fixed scalar; decimal-point and exponent literals
///       initialise only binary floats.
/// WHY:  destination-aware materialisation must not turn integral decimal spelling such as `1.0`
///       into an integer or `Byte`, while a whole literal may still initialise a binary float.
///
/// When this returns `false` the caller keeps the default `Float` materialisation and the receiving
/// boundary reports the ordinary type mismatch.
pub fn literal_kind_initialises(kind: NumericLiteralKind, scalar: FixedScalar) -> bool {
    match kind {
        NumericLiteralKind::WholeNumber => true,

        NumericLiteralKind::DecimalPoint | NumericLiteralKind::Exponent => {
            scalar.class() == FixedScalarClass::BinaryFloat
        }
    }
}

/// Materialise already validated normalized text at one fixed scalar destination.
///
/// WHAT: signed widths use their exact range, unsigned widths and `Byte` reject every negative
///       spelling and enforce their own maximum, and binary floats round once at their precision.
/// WHY: each literal-only consumer can share this destination policy without compiler token or
///      `StringTable` storage, or the need for a second numeric parser.
///
/// # Input contract
///
/// `normalized` must be the unsigned, separator-free `normalized_text` produced by
/// [`parse_numeric_literal`], or equivalent retained parser facts. The caller must check the
/// parsed kind with [`literal_kind_initialises`] before selecting `scalar`. This low-level
/// operation does not validate the grammar again; [`parse_numeric_text_to_fixed_scalar`] owns
/// whole-string numeric casts, which deliberately exclude `Byte`.
pub fn materialize_normalized_fixed_scalar(
    normalized: &str,
    negative: bool,
    scalar: FixedScalar,
) -> Result<FixedScalarValue, NumberLiteralErrorReason> {
    let sign = if negative {
        NumericLiteralSign::Negative
    } else {
        NumericLiteralSign::Positive
    };

    match scalar.class() {
        FixedScalarClass::SignedInteger => {
            materialize_signed_fixed_scalar(normalized, sign, scalar)
        }

        FixedScalarClass::UnsignedInteger | FixedScalarClass::Octet => {
            materialize_unsigned_fixed_scalar(normalized, sign, scalar)
        }

        FixedScalarClass::BinaryFloat => {
            materialize_binary_float_fixed_scalar(normalized, sign, scalar)
        }
    }
}

/// Materialise a whole-number magnitude into one signed fixed scalar.
///
/// WHAT: positive magnitudes parse as `i64`, and negative magnitudes parse as `u64` and negate by
///       wrapping so the exact minimum (whose magnitude exceeds the maximum by one) is accepted
///       without overflow; the constructor then applies the scalar's own inclusive range.
/// WHY:  `I8`..`I64` literals must not be range-checked against the profile `Int` width first, and
///       `-0` must materialise as a plain zero.
fn materialize_signed_fixed_scalar(
    text: &str,
    sign: NumericLiteralSign,
    scalar: FixedScalar,
) -> Result<FixedScalarValue, NumberLiteralErrorReason> {
    let value = match sign {
        NumericLiteralSign::Positive => text
            .parse::<i64>()
            .map_err(|_| NumberLiteralErrorReason::OutsideFixedScalarRange(scalar))?,

        NumericLiteralSign::Negative => {
            let magnitude = text
                .parse::<u64>()
                .map_err(|_| NumberLiteralErrorReason::OutsideFixedScalarRange(scalar))?;

            // Wrapping negation is exact for the one magnitude with no positive counterpart.
            // Larger magnitudes cannot be negated into any signed fixed scalar at all.
            if magnitude > i64::MIN.unsigned_abs() {
                return Err(NumberLiteralErrorReason::OutsideFixedScalarRange(scalar));
            }

            (magnitude as i64).wrapping_neg()
        }
    };

    FixedScalarValue::signed(scalar, value)
        .ok_or(NumberLiteralErrorReason::OutsideFixedScalarRange(scalar))
}

/// Materialise a whole-number magnitude into one unsigned fixed scalar or `Byte`.
///
/// WHAT: any literal carrying a negative sign is rejected, including `-0`; accepted magnitudes parse
///       as `u64` and range-check against the scalar's own inclusive maximum.
/// WHY:  negative values cannot initialise `U*` or `Byte`, and parsing as `u64` keeps `U64` values
///       above `2^53` exact instead of narrowing them through a binary float.
fn materialize_unsigned_fixed_scalar(
    text: &str,
    sign: NumericLiteralSign,
    scalar: FixedScalar,
) -> Result<FixedScalarValue, NumberLiteralErrorReason> {
    // The sign is part of the literal spelling, so `-0` is refused like any other negative literal
    // rather than being normalised away.
    if sign == NumericLiteralSign::Negative {
        return Err(NumberLiteralErrorReason::NegativeUnsignedLiteral(scalar));
    }

    let value = text
        .parse::<u64>()
        .map_err(|_| NumberLiteralErrorReason::OutsideFixedScalarRange(scalar))?;

    FixedScalarValue::unsigned(scalar, value)
        .ok_or(NumberLiteralErrorReason::OutsideFixedScalarRange(scalar))
}

/// Materialise a literal magnitude into one binary float scalar.
///
/// WHAT: `F16` rounds through the binary16 owner, which owns its double-rounding guard, while `F32`
///       and `F64` parse once at their own precision through the shared profile-precision parser.
/// WHY:  each binary float literal must round exactly once, at its destination precision, so no
///       value is produced through a wider intermediate; a non-finite rounded result is rejected
///       instead of entering the `FixedScalarValue` carrier.
fn materialize_binary_float_fixed_scalar(
    text: &str,
    sign: NumericLiteralSign,
    scalar: FixedScalar,
) -> Result<FixedScalarValue, NumberLiteralErrorReason> {
    let negative = sign == NumericLiteralSign::Negative;

    let precision = scalar
        .binary_float_precision()
        .ok_or(NumberLiteralErrorReason::NonFiniteFixedFloat(scalar))?;
    let magnitude = match precision {
        BinaryFloatPrecision::Binary16 => decimal_to_f16(text, negative)
            .ok_or(NumberLiteralErrorReason::NonFiniteFixedFloat(scalar)),
        BinaryFloatPrecision::Binary32 => {
            materialize_float_text_with_sign(text, negative, FloatPrecision::Bits32)
        }
        BinaryFloatPrecision::Binary64 => {
            materialize_float_text_with_sign(text, negative, FloatPrecision::Bits64)
        }
    }
    .map_err(|reason| {
        if reason == NumberLiteralErrorReason::NonFiniteFloat {
            NumberLiteralErrorReason::NonFiniteFixedFloat(scalar)
        } else {
            reason
        }
    })?;

    FixedScalarValue::binary_float(scalar, magnitude)
        .ok_or(NumberLiteralErrorReason::NonFiniteFixedFloat(scalar))
}

#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod tests;
