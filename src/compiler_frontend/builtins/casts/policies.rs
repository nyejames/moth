//! Pure builtin cast policy implementations.
//!
//! WHAT: implements every builtin cast policy as a pure function over a
//!      `BuiltinCastLiteral` input and the compilation boundary's
//!      `NumericProfile`. The helpers return either a folded
//!      `BuiltinCastLiteral` value or a `BuiltinCastError` that carries the
//!      stable `BuiltinErrorCode` so diagnostic and runtime layers can render
//!      the same code path.
//! WHY: the policy owner is the single source of truth for the actual rules.
//!      The constant folder and later backend phases can ask the policy owner
//!      for the same answer instead of duplicating per-cast ad hoc match logic.
//!      The boundary profile owns every `Int` range and `Float` rounding decision,
//!      so policies that materialise numbers take it as an explicit parameter.

use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason;
use crate::compiler_frontend::datatypes::fixed_scalar::{
    FixedScalar, FixedScalarClass, FixedScalarValue,
};
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::{BinaryFloatPrecision, NumericScalar};
use crate::compiler_frontend::numeric_text::format::format_finite_float;
use crate::compiler_frontend::numeric_text::parse::{
    parse_numeric_text_to_fixed_scalar, parse_numeric_text_to_float, parse_numeric_text_to_int,
};

/// A literal scalar value in policy space.
///
/// WHAT: policies operate on this narrow type so they do not depend on the
///      parser, AST, HIR, or runtime representation. Later phases will convert
///      their native expressions into this shape before calling the policy.
/// WHY: keeping policies pure and side-effect free allows sharing between the
///      constant folder and later backends without depending on `Expression`
///      or backend-specific types.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BuiltinCastLiteral {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Char(char),
    Error {
        message: String,
        code: i32,
    },
    /// One materialised fixed-width scalar or `Byte` value.
    Fixed(FixedScalarValue),
}

impl BuiltinCastLiteral {
    /// Returns the type tag for a literal, used by policy diagnostics.
    fn type_name(&self) -> &'static str {
        match self {
            BuiltinCastLiteral::Bool(_) => "Bool",
            BuiltinCastLiteral::Int(_) => "Int",
            BuiltinCastLiteral::Float(_) => "Float",
            BuiltinCastLiteral::String(_) => "String",
            BuiltinCastLiteral::Char(_) => "Char",
            BuiltinCastLiteral::Error { .. } => "Error",
            BuiltinCastLiteral::Fixed(value) => value.scalar().name(),
        }
    }
}

/// A single cast failure reported by a policy.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BuiltinCastError {
    pub(crate) code: BuiltinErrorCode,
    pub(crate) message: String,
}

impl BuiltinCastError {
    fn new(code: BuiltinErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Dispatches a builtin policy by id under the compilation boundary's numeric profile.
pub(crate) fn apply_builtin_cast_policy(
    policy: BuiltinCastPolicyId,
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    match policy {
        BuiltinCastPolicyId::NumericConversion {
            source: from,
            target,
        } => numeric_conversion(from, target, source, numeric_profile),
        BuiltinCastPolicyId::ByteToU8 => retag_octet(source, FixedScalar::Byte, FixedScalar::U8),
        BuiltinCastPolicyId::U8ToByte => retag_octet(source, FixedScalar::U8, FixedScalar::Byte),
        BuiltinCastPolicyId::NumericToString(scalar) => {
            numeric_to_string(scalar, source, numeric_profile)
        }
        BuiltinCastPolicyId::BoolToString => bool_to_string(source),
        BuiltinCastPolicyId::CharToString => char_to_string(source),
        BuiltinCastPolicyId::CharToInt => char_to_int(source),
        BuiltinCastPolicyId::StringToError => string_to_error(source),
        BuiltinCastPolicyId::ErrorToString => error_to_string(source),
        BuiltinCastPolicyId::IntToChar => int_to_char(source),
        BuiltinCastPolicyId::StringToNumeric(scalar) => {
            string_to_numeric(scalar, source, numeric_profile)
        }
        BuiltinCastPolicyId::StringToBool => string_to_bool(source),
        BuiltinCastPolicyId::StringToChar => string_to_char(source),
    }
}

// -----------------------------------------------------------
//  Numeric conversions
// -----------------------------------------------------------

/// One numeric source value in exact policy space.
#[derive(Clone, Copy)]
enum ExactNumericValue {
    /// Any integer domain, widened so `U64` and signed values compare exactly.
    Integer(i128),
    /// Any binary-float domain in its exact `f64` carrier.
    BinaryFloat(f64),
}

impl std::fmt::Display for ExactNumericValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExactNumericValue::Integer(value) => write!(formatter, "{value}"),
            ExactNumericValue::BinaryFloat(value) => write!(formatter, "{value}"),
        }
    }
}

/// Implements every numeric-to-numeric conversion.
///
/// WHAT: an integer keeps its exact value and must fit an integer target (`IntCastOutOfRange`).
///       A binary-float target rounds once, ties-to-even, directly from the source value and
///       fails only on a non-finite result (`FloatCastNonFinite`). A float source truncates
///       toward zero before an integer target's range check (`FloatCastToIntInvalidValue` for a
///       non-finite source, `FloatCastToIntOutOfRange` otherwise).
/// WHY: casts permit exactly this rounding and truncation, so one owner implements the rules
///      the evidence classifier promises.
fn numeric_conversion(
    source_scalar: NumericScalar,
    target_scalar: NumericScalar,
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let source_name = source_scalar.name();
    let target_name = target_scalar.name();

    let Some(value) = exact_numeric_value(source_scalar, source) else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "{source_name} -> {target_name} requires a {source_name} source, found {}",
                source.type_name()
            ),
        ));
    };

    let failure = |code: BuiltinErrorCode, reason: &str| {
        BuiltinCastError::new(
            code,
            format!("{source_name} -> {target_name} source {value} {reason}"),
        )
    };

    if let Some(precision) = target_scalar.binary_float_precision(numeric_profile) {
        let rounded = match value {
            ExactNumericValue::Integer(integer) => precision.round_integer(integer),
            ExactNumericValue::BinaryFloat(float) => precision.round(float),
        };

        return binary_float_literal(target_scalar, rounded).ok_or_else(|| {
            failure(
                BuiltinErrorCode::FloatCastNonFinite,
                "produced a non-finite value",
            )
        });
    }

    let (integer, out_of_range_code) = match value {
        ExactNumericValue::Integer(integer) => (integer, BuiltinErrorCode::IntCastOutOfRange),

        ExactNumericValue::BinaryFloat(float) => {
            if !float.is_finite() {
                return Err(failure(
                    BuiltinErrorCode::FloatCastToIntInvalidValue,
                    "is not finite",
                ));
            }

            // `as` saturates at the `i128` bounds, far outside every target range, so any
            // saturated value still fails the range check below.
            (
                float.trunc() as i128,
                BuiltinErrorCode::FloatCastToIntOutOfRange,
            )
        }
    };

    integer_literal(target_scalar, integer, numeric_profile)
        .ok_or_else(|| failure(out_of_range_code, &format!("is out of {target_name} range")))
}

/// Reads one numeric literal of `scalar` into exact policy space.
fn exact_numeric_value(
    scalar: NumericScalar,
    literal: &BuiltinCastLiteral,
) -> Option<ExactNumericValue> {
    match (scalar, literal) {
        (NumericScalar::Int, BuiltinCastLiteral::Int(value)) => {
            Some(ExactNumericValue::Integer(i128::from(*value)))
        }

        (NumericScalar::Float, BuiltinCastLiteral::Float(value)) => {
            Some(ExactNumericValue::BinaryFloat(*value))
        }

        (NumericScalar::Fixed(expected), BuiltinCastLiteral::Fixed(value))
            if value.scalar() == expected =>
        {
            match expected.class() {
                FixedScalarClass::SignedInteger => value
                    .as_i64()
                    .map(|v| ExactNumericValue::Integer(i128::from(v))),
                FixedScalarClass::UnsignedInteger => value
                    .as_u64()
                    .map(|v| ExactNumericValue::Integer(i128::from(v))),
                FixedScalarClass::BinaryFloat => value.as_f64().map(ExactNumericValue::BinaryFloat),
                FixedScalarClass::Octet => None,
            }
        }

        _ => None,
    }
}

/// Materialises an exact integer in an integer domain, or `None` when it is out of range.
fn integer_literal(
    scalar: NumericScalar,
    value: i128,
    numeric_profile: NumericProfile,
) -> Option<BuiltinCastLiteral> {
    match scalar {
        NumericScalar::Int => i64::try_from(value)
            .ok()
            .filter(|value| numeric_profile.int_width.contains(*value))
            .map(BuiltinCastLiteral::Int),

        NumericScalar::Fixed(fixed) => match fixed.class() {
            FixedScalarClass::SignedInteger => i64::try_from(value)
                .ok()
                .and_then(|value| FixedScalarValue::signed(fixed, value)),
            FixedScalarClass::UnsignedInteger => u64::try_from(value)
                .ok()
                .and_then(|value| FixedScalarValue::unsigned(fixed, value)),
            FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => None,
        }
        .map(BuiltinCastLiteral::Fixed),

        NumericScalar::Float => None,
    }
}

/// Wraps a value already rounded to a binary-float domain, or `None` when it is non-finite.
fn binary_float_literal(scalar: NumericScalar, rounded: f64) -> Option<BuiltinCastLiteral> {
    match scalar {
        NumericScalar::Float => rounded
            .is_finite()
            .then_some(BuiltinCastLiteral::Float(rounded)),
        NumericScalar::Fixed(fixed) => {
            FixedScalarValue::binary_float(fixed, rounded).map(BuiltinCastLiteral::Fixed)
        }
        NumericScalar::Int => None,
    }
}

/// Re-tags one octet between `Byte` and `U8`, which share every value.
fn retag_octet(
    source: &BuiltinCastLiteral,
    from: FixedScalar,
    to: FixedScalar,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    match source {
        BuiltinCastLiteral::Fixed(value) if value.scalar() == from => value
            .as_u64()
            .and_then(|octet| FixedScalarValue::unsigned(to, octet))
            .map(BuiltinCastLiteral::Fixed),
        _ => None,
    }
    .ok_or_else(|| {
        BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "{} -> {} requires a {} source, found {}",
                from.name(),
                to.name(),
                from.name(),
                source.type_name()
            ),
        )
    })
}

// -----------------------------------------------------------
//  Infallible policies
// -----------------------------------------------------------

/// Implements every numeric-to-text conversion.
///
/// WHAT: integer domains format with Rust's decimal integer formatting, which is already the Moth
///       contract; binary-float domains format through the Moth finite-float formatter at their own
///       precision, so `Float` follows the profile while fixed widths follow theirs.
/// WHY: one policy owns the numeric text contract for every numeric domain, so `F32` and `Float`
///      under `Float32` cannot drift apart, and each domain formats without an `Int`/`Float`
///      intermediate that could lose a `U64` or round a wide fixed value.
fn numeric_to_string(
    scalar: NumericScalar,
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let Some(value) = exact_numeric_value(scalar, source) else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "{} -> String requires a {} source, found {}",
                scalar.name(),
                scalar.name(),
                source.type_name()
            ),
        ));
    };

    match value {
        ExactNumericValue::Integer(integer) => Ok(BuiltinCastLiteral::String(integer.to_string())),
        ExactNumericValue::BinaryFloat(float) => {
            // Every scalar that reads back as a binary float has a precision, so this arm only
            // guards the shared reader against a future domain that does not.
            let Some(precision) = scalar.binary_float_precision(numeric_profile) else {
                return Err(BuiltinCastError::new(
                    BuiltinErrorCode::Unsupported,
                    format!(
                        "{} -> String requires a numeric binary-float domain",
                        scalar.name()
                    ),
                ));
            };

            format_numeric_to_string(float, precision, scalar.name())
        }
    }
}

/// Formats one finite binary-float value as Moth numeric text.
fn format_numeric_to_string(
    value: f64,
    precision: BinaryFloatPrecision,
    scalar_name: &str,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    // Moth binary floats are finite, so a non-finite value reaching the cast
    // policy is a defensive invariant failure rather than ordinary user input.
    let text = format_finite_float(value, precision).map_err(|error| {
        BuiltinCastError::new(
            BuiltinErrorCode::FloatFormatInvariant,
            format!("{scalar_name} -> String formatting failed: {error}"),
        )
    })?;

    Ok(BuiltinCastLiteral::String(text))
}

fn bool_to_string(source: &BuiltinCastLiteral) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::Bool(value) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "Bool -> String requires a Bool source, found {}",
                source.type_name()
            ),
        ));
    };
    Ok(BuiltinCastLiteral::String(value.to_string()))
}

fn char_to_string(source: &BuiltinCastLiteral) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::Char(value) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "Char -> String requires a Char source, found {}",
                source.type_name()
            ),
        ));
    };
    Ok(BuiltinCastLiteral::String(value.to_string()))
}

fn char_to_int(source: &BuiltinCastLiteral) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::Char(value) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "Char -> Int requires a Char source, found {}",
                source.type_name()
            ),
        ));
    };
    Ok(BuiltinCastLiteral::Int(*value as i64))
}

fn string_to_error(source: &BuiltinCastLiteral) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::String(text) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "String -> Error requires a String source, found {}",
                source.type_name()
            ),
        ));
    };
    Ok(BuiltinCastLiteral::Error {
        message: text.to_owned(),
        code: 0,
    })
}

fn error_to_string(source: &BuiltinCastLiteral) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::Error { message, .. } = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "Error -> String policy requires an Error source, found {}",
                source.type_name()
            ),
        ));
    };
    Ok(BuiltinCastLiteral::String(message.to_owned()))
}

// -----------------------------------------------------------
//  Fallible policies
// -----------------------------------------------------------

fn int_to_char(source: &BuiltinCastLiteral) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::Int(value) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "Int -> Char requires an Int source, found {}",
                source.type_name()
            ),
        ));
    };

    if *value < 0 {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::IntCastToCharInvalidCodepoint,
            format!("Int -> Char source {value} is negative"),
        ));
    }

    if (0xD800_i64..=0xDFFF_i64).contains(value) {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::IntCastToCharInvalidCodepoint,
            format!("Int -> Char source {value} falls in the surrogate range"),
        ));
    }

    if *value > 0x10FFFF_i64 {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::IntCastToCharInvalidCodepoint,
            format!("Int -> Char source {value} exceeds the maximum Unicode scalar"),
        ));
    }

    // The range checks above leave exactly the valid scalar range, so this checked
    // conversion documents the i64-to-u32 narrowing rather than handling new failures.
    let codepoint = u32::try_from(*value).map_err(|_| {
        BuiltinCastError::new(
            BuiltinErrorCode::IntCastToCharInvalidCodepoint,
            format!("Int -> Char source {value} is not a valid Unicode scalar"),
        )
    })?;
    let scalar = char::from_u32(codepoint).ok_or_else(|| {
        BuiltinCastError::new(
            BuiltinErrorCode::IntCastToCharInvalidCodepoint,
            format!("Int -> Char source {value} is not a valid Unicode scalar"),
        )
    })?;

    Ok(BuiltinCastLiteral::Char(scalar))
}

/// Implements every text-to-numeric conversion.
///
/// WHAT: parses the complete text at the destination domain: `Int` and `Float` follow the profile,
///       fixed integers materialise inside their own range, and fixed binary floats round once at
///       their own precision. Integer destinations report out-of-range text as
///       `IntParseOutOfRange` and every other failure as `IntParseInvalidFormat`; binary-float
///       destinations report overflow and non-finite results as `FloatParseOutOfRange` and every
///       other failure as `FloatParseInvalidFormat`.
/// WHY: the destination domain owns the accepted range and rounding, so `U64` accepts exactly the
///      `u64` range and `F16` rounds exactly once rather than through `Float`.
fn string_to_numeric(
    scalar: NumericScalar,
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::String(text) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "String -> {} requires a String source, found {}",
                scalar.name(),
                source.type_name()
            ),
        ));
    };

    let parsed = match scalar {
        NumericScalar::Int => {
            parse_numeric_text_to_int(text, numeric_profile.int_width).map(BuiltinCastLiteral::Int)
        }
        NumericScalar::Float => parse_numeric_text_to_float(text, numeric_profile.float_precision)
            .map(BuiltinCastLiteral::Float),
        NumericScalar::Fixed(fixed) => {
            parse_numeric_text_to_fixed_scalar(text, fixed).map(BuiltinCastLiteral::Fixed)
        }
    };

    parsed.map_err(|reason| numeric_parse_error(scalar, reason, text))
}

/// Maps one numeric text parse failure onto the destination domain's parse error code.
///
/// WHAT: integer destinations collapse out-of-range reasons into `IntParseOutOfRange` and every
///       other reason into `IntParseInvalidFormat`; binary-float destinations collapse non-finite
///       and overflow reasons into `FloatParseOutOfRange` and every other reason into
///       `FloatParseInvalidFormat`.
/// WHY: the parse grammar reports the same failure in more than one place (a negative spelling for
///      an unsigned destination, a whole part beyond the destination's precision), so one mapping
///      keeps the integer destinations and the binary-float destinations reporting consistent
///      codes.
fn numeric_parse_error(
    scalar: NumericScalar,
    reason: NumberLiteralErrorReason,
    text: &str,
) -> BuiltinCastError {
    let message = format!("Cannot parse {} from {text:?}", scalar.name());

    // The destination's class, not the boundary profile, chooses the error family: `Float` and the
    // fixed binary floats report float parse codes, and every integer destination, including `Int`,
    // reports integer parse codes.
    let is_binary_float = match scalar {
        NumericScalar::Int => false,
        NumericScalar::Float => true,
        NumericScalar::Fixed(fixed) => fixed.class() == FixedScalarClass::BinaryFloat,
    };

    if is_binary_float {
        match reason {
            NumberLiteralErrorReason::NonFiniteFloat
            | NumberLiteralErrorReason::NonFiniteFixedFloat(_)
            | NumberLiteralErrorReason::ParseOverflow => {
                BuiltinCastError::new(BuiltinErrorCode::FloatParseOutOfRange, message)
            }
            _ => BuiltinCastError::new(BuiltinErrorCode::FloatParseInvalidFormat, message),
        }
    } else {
        match reason {
            NumberLiteralErrorReason::OutsideIntRange
            | NumberLiteralErrorReason::OutsideFixedScalarRange(_)
            | NumberLiteralErrorReason::NegativeUnsignedLiteral(_)
            | NumberLiteralErrorReason::ParseOverflow => {
                BuiltinCastError::new(BuiltinErrorCode::IntParseOutOfRange, message)
            }
            _ => BuiltinCastError::new(BuiltinErrorCode::IntParseInvalidFormat, message),
        }
    }
}

fn string_to_bool(source: &BuiltinCastLiteral) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::String(text) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "String -> Bool requires a String source, found {}",
                source.type_name()
            ),
        ));
    };

    let trimmed = text.trim();
    match trimmed {
        "true" => Ok(BuiltinCastLiteral::Bool(true)),
        "false" => Ok(BuiltinCastLiteral::Bool(false)),
        _ => Err(BuiltinCastError::new(
            BuiltinErrorCode::StringParseBoolInvalidFormat,
            format!("Cannot parse Bool from {trimmed:?}"),
        )),
    }
}

fn string_to_char(source: &BuiltinCastLiteral) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::String(text) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "String -> Char requires a String source, found {}",
                source.type_name()
            ),
        ));
    };

    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::StringParseCharInvalidFormat,
            "String -> Char text is empty",
        ));
    };

    if chars.next().is_some() {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::StringParseCharInvalidFormat,
            "String -> Char text contains more than one Unicode scalar",
        ));
    }

    Ok(BuiltinCastLiteral::Char(first))
}
