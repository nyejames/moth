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
//!      The boundary profile owns every `Int`/`Uint` range and `Float` rounding decision,
//!      so policies that materialise numbers take it as an explicit parameter.

use std::fmt::{self, Display, Formatter};

use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::number::{
    NumberIntegerConversionError, NumberMaterializationError, NumberValue,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::numeric_text::parse::parse_numeric_text_to_number;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::format::format_finite_float;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;
use moth_lexical::numeric::parse::{
    parse_numeric_text_to_fixed_scalar, parse_numeric_text_to_float, parse_numeric_text_to_int,
    parse_numeric_text_to_uint,
};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::NumericProfile;

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
    /// One profile-sized unsigned integer value; follows the selected `Int` width.
    Uint(u64),
    Float(f64),
    String(String),
    Char(char),
    Error {
        message: String,
        code: u32,
    },
    /// One materialised fixed-width scalar or `Byte` value.
    Fixed(FixedScalarValue),
    /// One exact decimal `Dec` value at its own scale.
    ///
    /// WHY: exact scale/integer conversions and canonical formatting consume the retained
    ///      coefficient without a bounded integer or binary-float approximation.
    Number(NumberValue),
}

/// A formatted cast-literal type tag, without allocating for static or lazy `Dec` names.
#[derive(Clone, Copy)]
enum BuiltinCastTypeName {
    Static(&'static str),
    Number(NumberScale),
}

impl Display for BuiltinCastTypeName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static(name) => formatter.write_str(name),
            Self::Number(scale) => Display::fmt(scale, formatter),
        }
    }
}

impl BuiltinCastLiteral {
    /// Returns the type tag for a literal, used by policy diagnostics.
    fn type_name(&self) -> BuiltinCastTypeName {
        match self {
            BuiltinCastLiteral::Bool(_) => BuiltinCastTypeName::Static("Bool"),
            BuiltinCastLiteral::Int(_) => BuiltinCastTypeName::Static("Int"),
            BuiltinCastLiteral::Uint(_) => BuiltinCastTypeName::Static("Uint"),
            BuiltinCastLiteral::Float(_) => BuiltinCastTypeName::Static("Float"),
            BuiltinCastLiteral::String(_) => BuiltinCastTypeName::Static("String"),
            BuiltinCastLiteral::Char(_) => BuiltinCastTypeName::Static("Char"),
            BuiltinCastLiteral::Error { .. } => BuiltinCastTypeName::Static("Error"),
            BuiltinCastLiteral::Fixed(value) => BuiltinCastTypeName::Static(value.scalar().name()),
            BuiltinCastLiteral::Number(value) => BuiltinCastTypeName::Number(value.scale()),
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

/// Returns the runtime failure codes for a valid builtin cast policy selected as fallible.
///
/// AST failure witnesses use this policy-owned projection rather than matching cast ids in a
/// later stage. The caller supplies the fallibility selected by cast evidence, so numeric range
/// classification remains with the evidence owner.
pub(crate) fn builtin_cast_failure_codes(
    policy: BuiltinCastPolicyId,
    fallibility: BuiltinCastFallibility,
) -> &'static [BuiltinErrorCode] {
    if fallibility != BuiltinCastFallibility::Fallible {
        return &[];
    }

    match policy {
        BuiltinCastPolicyId::NumericConversion { source, target } => {
            numeric_conversion_failure_codes(source, target)
        }
        BuiltinCastPolicyId::StringToNumeric(target) => string_to_numeric_failure_codes(target),
        BuiltinCastPolicyId::IntToChar => &[BuiltinErrorCode::IntCastToCharInvalidCodepoint],
        BuiltinCastPolicyId::StringToBool => &[BuiltinErrorCode::StringParseBoolInvalidFormat],
        BuiltinCastPolicyId::StringToChar => &[BuiltinErrorCode::StringParseCharInvalidFormat],
        BuiltinCastPolicyId::ByteToU8
        | BuiltinCastPolicyId::U8ToByte
        | BuiltinCastPolicyId::NumericToString(_)
        | BuiltinCastPolicyId::BoolToString
        | BuiltinCastPolicyId::CharToString
        | BuiltinCastPolicyId::CharToInt
        | BuiltinCastPolicyId::StringToError
        | BuiltinCastPolicyId::ErrorToString => &[],
    }
}

fn numeric_conversion_failure_codes(
    source: NumericScalar,
    target: NumericScalar,
) -> &'static [BuiltinErrorCode] {
    if source == target {
        return &[];
    }

    match (source, target) {
        (NumericScalar::Number(_), NumericScalar::Number(_)) => {
            &[BuiltinErrorCode::NumberCastInexact]
        }
        (NumericScalar::Number(_), target) if target.is_integer() => &[
            BuiltinErrorCode::NumberCastInexact,
            BuiltinErrorCode::IntCastOutOfRange,
        ],
        (NumericScalar::Number(_), _) | (_, NumericScalar::Number(_)) => &[],
        (source, target)
            if (source.is_integer() || source.is_binary_float())
                && (target.is_integer() || target.is_binary_float()) =>
        {
            if target.is_binary_float() {
                &[BuiltinErrorCode::FloatCastNonFinite]
            } else if source.is_binary_float() && target.is_integer() {
                &[
                    BuiltinErrorCode::FloatCastToIntInvalidValue,
                    BuiltinErrorCode::FloatCastToIntOutOfRange,
                ]
            } else if source.is_integer() && target.is_integer() {
                &[BuiltinErrorCode::IntCastOutOfRange]
            } else {
                &[]
            }
        }
        _ => &[],
    }
}

fn string_to_numeric_failure_codes(target: NumericScalar) -> &'static [BuiltinErrorCode] {
    if matches!(target, NumericScalar::Number(_)) {
        &[
            BuiltinErrorCode::NumberParseInvalidFormat,
            BuiltinErrorCode::NumberParseInexactScale,
            BuiltinErrorCode::NumberParseCapacity,
        ]
    } else if target.is_binary_float() {
        &[
            BuiltinErrorCode::FloatParseInvalidFormat,
            BuiltinErrorCode::FloatParseOutOfRange,
        ]
    } else if target.is_integer() {
        &[
            BuiltinErrorCode::IntParseInvalidFormat,
            BuiltinErrorCode::IntParseOutOfRange,
        ]
    } else {
        &[]
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
/// WHAT: bounded numeric pairs retain their range, precision, rounding and truncation policy.
///       Dec pairs use exact `NumberValue` operations: integer sources and scale widening are
///       infallible; scale narrowing and Dec-to-integer are exact or fail.
/// WHY: the evidence classifier and this policy share one pair-level contract while keeping
///      arbitrary-precision values out of `ExactNumericValue`.
fn numeric_conversion(
    source_scalar: NumericScalar,
    target_scalar: NumericScalar,
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    if matches!(source_scalar, NumericScalar::Number(_))
        || matches!(target_scalar, NumericScalar::Number(_))
    {
        return number_numeric_conversion(source_scalar, target_scalar, source, numeric_profile);
    }

    let Some(value) = exact_numeric_value(source_scalar, source) else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "{source_scalar} -> {target_scalar} requires a {source_scalar} source, found {}",
                source.type_name()
            ),
        ));
    };

    let failure = |code: BuiltinErrorCode, reason: &str| {
        BuiltinCastError::new(
            code,
            format!("{source_scalar} -> {target_scalar} source {value} {reason}"),
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

    integer_literal(target_scalar, integer, numeric_profile).ok_or_else(|| {
        failure(
            out_of_range_code,
            &format!("is out of {target_scalar} range"),
        )
    })
}

/// Applies the exact conversion rows that carry a `BuiltinCastTarget::Number` endpoint.
fn number_numeric_conversion(
    source_scalar: NumericScalar,
    target_scalar: NumericScalar,
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    match (source_scalar, target_scalar) {
        (NumericScalar::Number(source_scale), NumericScalar::Number(target_scale)) => {
            let BuiltinCastLiteral::Number(value) = source else {
                return Err(unsupported_numeric_source(
                    source_scalar,
                    target_scalar,
                    source,
                ));
            };

            if value.scale() != source_scale {
                return Err(unsupported_numeric_source(
                    source_scalar,
                    target_scalar,
                    source,
                ));
            }

            value
                .rescale(target_scale)
                .map(BuiltinCastLiteral::Number)
                .map_err(|error| match error {
                    NumberMaterializationError::InexactScale => {
                        number_cast_inexact_error(source_scalar, target_scalar, value)
                    }
                    NumberMaterializationError::Capacity => BuiltinCastError::new(
                        BuiltinErrorCode::NumberParseCapacity,
                        error.to_string(),
                    ),
                })
        }

        (_, NumericScalar::Number(target_scale)) => {
            let Some(ExactNumericValue::Integer(integer)) =
                exact_numeric_value(source_scalar, source)
            else {
                return Err(unsupported_numeric_source(
                    source_scalar,
                    target_scalar,
                    source,
                ));
            };

            Ok(BuiltinCastLiteral::Number(NumberValue::from_integer(
                integer,
                target_scale,
            )))
        }

        (NumericScalar::Number(source_scale), target)
            if target.integer_range(numeric_profile).is_some() =>
        {
            let BuiltinCastLiteral::Number(value) = source else {
                return Err(unsupported_numeric_source(
                    source_scalar,
                    target_scalar,
                    source,
                ));
            };

            if value.scale() != source_scale {
                return Err(unsupported_numeric_source(
                    source_scalar,
                    target_scalar,
                    source,
                ));
            }

            let integer = match value.to_integer_exact() {
                Ok(integer) => integer,
                Err(NumberIntegerConversionError::Inexact) => {
                    return Err(number_cast_inexact_error(
                        source_scalar,
                        target_scalar,
                        value,
                    ));
                }
                Err(NumberIntegerConversionError::OutOfRange) => {
                    return Err(number_integer_out_of_range(
                        source_scalar,
                        target_scalar,
                        value,
                    ));
                }
            };

            integer_literal(target, integer, numeric_profile)
                .ok_or_else(|| number_integer_out_of_range(source_scalar, target_scalar, value))
        }

        _ => Err(unsupported_numeric_source(
            source_scalar,
            target_scalar,
            source,
        )),
    }
}

fn unsupported_numeric_source(
    source_scalar: NumericScalar,
    target_scalar: NumericScalar,
    source: &BuiltinCastLiteral,
) -> BuiltinCastError {
    BuiltinCastError::new(
        BuiltinErrorCode::Unsupported,
        format!(
            "{source_scalar} -> {target_scalar} requires a {source_scalar} source, found {}",
            source.type_name()
        ),
    )
}

fn number_cast_inexact_error(
    source_scalar: NumericScalar,
    target_scalar: NumericScalar,
    value: &NumberValue,
) -> BuiltinCastError {
    BuiltinCastError::new(
        BuiltinErrorCode::NumberCastInexact,
        format!(
            "{source_scalar} -> {target_scalar} source {value} is not exactly representable as {target_scalar}"
        ),
    )
}

fn number_integer_out_of_range(
    source_scalar: NumericScalar,
    target_scalar: NumericScalar,
    value: &NumberValue,
) -> BuiltinCastError {
    BuiltinCastError::new(
        BuiltinErrorCode::IntCastOutOfRange,
        format!(
            "{source_scalar} -> {target_scalar} source {value} is out of {target_scalar} range"
        ),
    )
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
        (NumericScalar::Uint, BuiltinCastLiteral::Uint(value)) => {
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
        NumericScalar::Uint => u64::try_from(value)
            .ok()
            .filter(|value| *value <= numeric_profile.int_width.unsigned_max_value())
            .map(BuiltinCastLiteral::Uint),

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

        NumericScalar::Float | NumericScalar::Number(_) => None,
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
        NumericScalar::Int | NumericScalar::Uint | NumericScalar::Number(_) => None,
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
    if let NumericScalar::Number(expected_scale) = scalar {
        return match source {
            BuiltinCastLiteral::Number(value) if value.scale() == expected_scale => {
                // `NumberValue` owns the canonical exact decimal text representation.
                Ok(BuiltinCastLiteral::String(value.to_string()))
            }
            _ => Err(BuiltinCastError::new(
                BuiltinErrorCode::Unsupported,
                format!(
                    "{scalar} -> String requires a {scalar} source, found {}",
                    source.type_name()
                ),
            )),
        };
    }

    let Some(value) = exact_numeric_value(scalar, source) else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "{scalar} -> String requires a {scalar} source, found {}",
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
                    format!("{scalar} -> String requires a numeric binary-float domain"),
                ));
            };

            format_numeric_to_string(float, precision, scalar)
        }
    }
}

/// Formats one finite binary-float value as Moth numeric text.
fn format_numeric_to_string(
    value: f64,
    precision: BinaryFloatPrecision,
    scalar: NumericScalar,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    // Moth binary floats are finite, so a non-finite value reaching the cast
    // policy is a defensive invariant failure rather than ordinary user input.
    let text = format_finite_float(value, precision).map_err(|error| {
        BuiltinCastError::new(
            BuiltinErrorCode::FloatFormatInvariant,
            format!("{scalar} -> String formatting failed: {error}"),
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
/// WHAT: parses the complete text at the destination domain: `Int`, `Uint` and `Float` follow
///       the profile, fixed integers materialise inside their own range, fixed binary floats
///       round once, and Dec uses exact scale materialisation after the shared whole-input
///       grammar.
/// WHY: the destination domain owns the accepted range, precision or exact Dec scale, so no
///      parser routes through a different numeric type.
fn string_to_numeric(
    scalar: NumericScalar,
    source: &BuiltinCastLiteral,
    numeric_profile: NumericProfile,
) -> Result<BuiltinCastLiteral, BuiltinCastError> {
    let BuiltinCastLiteral::String(text) = source else {
        return Err(BuiltinCastError::new(
            BuiltinErrorCode::Unsupported,
            format!(
                "String -> {scalar} requires a String source, found {}",
                source.type_name()
            ),
        ));
    };

    let parsed = match scalar {
        NumericScalar::Int => {
            parse_numeric_text_to_int(text, numeric_profile.int_width).map(BuiltinCastLiteral::Int)
        }
        NumericScalar::Uint => parse_numeric_text_to_uint(text, numeric_profile.int_width)
            .map(BuiltinCastLiteral::Uint),
        NumericScalar::Float => parse_numeric_text_to_float(text, numeric_profile.float_precision)
            .map(BuiltinCastLiteral::Float),
        NumericScalar::Fixed(fixed) => {
            parse_numeric_text_to_fixed_scalar(text, fixed).map(BuiltinCastLiteral::Fixed)
        }
        NumericScalar::Number(scale) => {
            parse_numeric_text_to_number(text, scale).map(BuiltinCastLiteral::Number)
        }
    };

    parsed.map_err(|reason| numeric_parse_error(scalar, reason, text))
}

/// Maps one numeric text parse failure onto the destination domain's parse error code.
///
/// Dec parsing keeps the grammar, scale and actual host-capacity failures distinct; bounded
/// integer and binary-float parsing retains its existing error families.
fn numeric_parse_error(
    scalar: NumericScalar,
    reason: NumberLiteralErrorReason,
    text: &str,
) -> BuiltinCastError {
    let message = format!("Cannot parse {scalar} from {text:?}");

    if matches!(scalar, NumericScalar::Number(_)) {
        let code = match reason {
            NumberLiteralErrorReason::InexactNumberScale(_) => {
                BuiltinErrorCode::NumberParseInexactScale
            }
            NumberLiteralErrorReason::ParseOverflow => BuiltinErrorCode::NumberParseCapacity,
            _ => BuiltinErrorCode::NumberParseInvalidFormat,
        };
        return BuiltinCastError::new(code, message);
    }

    // The destination's class, not the boundary profile, chooses the error family: `Float` and the
    // fixed binary floats report float parse codes, and every integer destination, including `Int`,
    // reports integer parse codes.
    let is_binary_float = match scalar {
        NumericScalar::Int | NumericScalar::Uint => false,
        NumericScalar::Number(_) => unreachable!("Number parse failures are classified above"),
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
            | NumberLiteralErrorReason::OutsideUintRange(_)
            | NumberLiteralErrorReason::NegativeUintLiteral(_)
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
