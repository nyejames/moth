//! Pure builtin cast policy implementations.
//!
//! WHAT: implements every initial builtin evidence row from the cast plan as a
//!      pure function over a `BuiltinCastLiteral` input and the compilation boundary's
//!      `NumericProfile`. The helpers return
//!      either a folded `BuiltinCastLiteral` value or a `BuiltinCastError` that
//!      carries the stable `BuiltinErrorCode` so diagnostic and runtime layers
//!      can render the same code path.
//! WHY: the policy owner is the single source of truth for the actual rules.
//!      The constant folder and later backend phases can ask the policy owner
//!      for the same answer instead of duplicating per-cast ad hoc match logic.
//!      The boundary profile owns every `Int` range and `Float` rounding decision,
//!      so policies that materialise numbers take it as an explicit parameter.

use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::numeric_text::format::format_finite_float;
use crate::compiler_frontend::numeric_text::parse::{
    parse_numeric_text_to_float, parse_numeric_text_to_int,
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
    Error { message: String, code: i32 },
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
        BuiltinCastPolicyId::IntToFloat => int_to_float(source, numeric_profile),
        BuiltinCastPolicyId::IntToString => int_to_string(source),
        BuiltinCastPolicyId::FloatToString => float_to_string(source, numeric_profile),
        BuiltinCastPolicyId::BoolToString => bool_to_string(source),
        BuiltinCastPolicyId::CharToString => char_to_string(source),
        BuiltinCastPolicyId::CharToInt => char_to_int(source),
        BuiltinCastPolicyId::StringToError => string_to_error(source),
        BuiltinCastPolicyId::ErrorToString => error_to_string(source),
        BuiltinCastPolicyId::FloatToInt => float_to_int(source, numeric_profile),
        BuiltinCastPolicyId::IntToChar => int_to_char(source),
        BuiltinCastPolicyId::StringToInt => string_to_int(source, numeric_profile),
        BuiltinCastPolicyId::StringToFloat => string_to_float(source, numeric_profile),
        BuiltinCastPolicyId::StringToBool => string_to_bool(source),
        BuiltinCastPolicyId::StringToChar => string_to_char(source),
    }
}

// -----------------------------------------------------------
//  Infallible policies
// -----------------------------------------------------------

fn int_to_float(
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::Int(value) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "Int -> Float requires an Int source, found {}",
                source.type_name()
            ),
        ));
    };

    // Single direct rounding at the destination precision, never via an f64 intermediate.
    Ok(BuiltinCastLiteral::Float(
        numeric_profile.float_precision.round_int(*value),
    ))
}

fn int_to_string(source: &BuiltinCastLiteral) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::Int(value) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "Int -> String requires an Int source, found {}",
                source.type_name()
            ),
        ));
    };
    Ok(BuiltinCastLiteral::String(value.to_string()))
}

fn float_to_string(
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::Float(value) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "Float -> String requires a Float source, found {}",
                source.type_name()
            ),
        ));
    };

    // Moth `Float` is finite, so a non-finite value reaching the cast
    // policy is a defensive invariant failure rather than ordinary user input.
    let text = format_finite_float(*value, numeric_profile.float_precision).map_err(|error| {
        BuiltinCastError::new(
            BuiltinErrorCode::FloatFormatInvariant,
            format!("Float -> String formatting failed: {error}"),
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

fn float_to_int(
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::Float(value) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "Float -> Int requires a Float source, found {}",
                source.type_name()
            ),
        ));
    };

    if !value.is_finite() {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::FloatCastToIntInvalidValue,
            format!("Float -> Int source {value} is not finite"),
        ));
    }

    // Truncate toward zero, then require the result to fit the boundary Int width.
    let truncated = value.trunc();

    // Guard the f64-to-i64 narrowing below: an out-of-i64 truncated value would
    // saturate the `as` cast, so reject it in the f64 domain first. The bounds are
    // exact: `i64::MIN` is representable as f64, and `2^63` (one above `i64::MAX`)
    // is the first integral f64 that no longer fits.
    if truncated < (i64::MIN as f64) || truncated >= 9_223_372_036_854_775_808.0 {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::FloatCastToIntOutOfRange,
            format!("Float -> Int source {value} is out of Int range"),
        ));
    }

    // The guard above rules out saturation, so this narrowing cast is exact.
    let candidate = truncated as i64;
    if !numeric_profile.int_width.contains(candidate) {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::FloatCastToIntOutOfRange,
            format!("Float -> Int source {value} is out of Int range"),
        ));
    }

    Ok(BuiltinCastLiteral::Int(candidate))
}

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

fn string_to_int(
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::String(text) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "String -> Int requires a String source, found {}",
                source.type_name()
            ),
        ));
    };

    match parse_numeric_text_to_int(text, numeric_profile.int_width) {
        Ok(value) => Ok(BuiltinCastLiteral::Int(value)),
        Err(NumberLiteralErrorReason::OutsideIntRange) => Err(BuiltinCastError::new(
            BuiltinErrorCode::IntParseOutOfRange,
            format!("Cannot parse Int from {text:?}"),
        )),
        Err(_) => Err(BuiltinCastError::new(
            BuiltinErrorCode::IntParseInvalidFormat,
            format!("Cannot parse Int from {text:?}"),
        )),
    }
}

fn string_to_float(
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::String(text) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "String -> Float requires a String source, found {}",
                source.type_name()
            ),
        ));
    };

    match parse_numeric_text_to_float(text, numeric_profile.float_precision) {
        Ok(value) => Ok(BuiltinCastLiteral::Float(value)),
        Err(NumberLiteralErrorReason::NonFiniteFloat | NumberLiteralErrorReason::ParseOverflow) => {
            Err(BuiltinCastError::new(
                BuiltinErrorCode::FloatParseOutOfRange,
                format!("Cannot parse Float from {text:?}"),
            ))
        }
        Err(_) => Err(BuiltinCastError::new(
            BuiltinErrorCode::FloatParseInvalidFormat,
            format!("Cannot parse Float from {text:?}"),
        )),
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
