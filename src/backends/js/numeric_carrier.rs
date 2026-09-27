//! JavaScript numeric carrier and conversion policy.
//!
//! WHAT: maps numeric domains and profiles to JS carriers, then classifies casts into shared
//!       lowering decisions and demand-driven runtime-helper requirements.
//! WHY: representation and helper selection must not drift between generated calls and the
//!      prelude, especially when BigInt and Number carriers meet.
//!
//! Exclusions: frontend evidence owns semantic conversion fallibility. `Byte` is an octet value,
//! not a numeric domain, though its literal uses the JS Number carrier.

use crate::compiler_frontend::builtins::casts::evidence::numeric_conversion_fallibility;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastFallibility;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::fixed_scalar::{FixedScalarClass, FixedScalarValue};
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::{BinaryFloatPrecision, NumericScalar};

const MAX_EXACT_JS_INTEGER: i128 = 9_007_199_254_740_991;

/// The runtime carrier selected for one numeric domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsNumericCarrier {
    ExactInteger { min: i128, max: i128 },
    BigInteger { min: i128, max: i128 },
    BinaryFloat { precision: BinaryFloatPrecision },
}

impl JsNumericCarrier {
    /// Maps a semantic scalar and selected profile to its JavaScript representation.
    pub(crate) fn for_scalar(scalar: NumericScalar, profile: NumericProfile) -> Option<Self> {
        if let Some((min, max)) = scalar.integer_range(profile) {
            return Some(
                if min >= -MAX_EXACT_JS_INTEGER && max <= MAX_EXACT_JS_INTEGER {
                    Self::ExactInteger { min, max }
                } else {
                    Self::BigInteger { min, max }
                },
            );
        }

        scalar
            .binary_float_precision(profile)
            .map(|precision| Self::BinaryFloat { precision })
    }

    /// Formats a profile-selected Moth `Int` literal for JavaScript.
    pub(crate) fn int_literal(value: i64, profile: NumericProfile) -> Option<String> {
        let carrier = Self::for_scalar(NumericScalar::Int, profile)?;
        Self::integer_literal(carrier, i128::from(value))
    }

    /// Formats an already-materialised fixed scalar without changing its exact value.
    ///
    /// WHAT: integer carriers choose decimal Number or BigInt spelling, binary-float values retain
    ///       their exact f64 carrier bits (including signed zero), and Byte stays a Number octet.
    /// WHY: HIR literals must enter the same carrier used by later comparisons, casts and checked
    ///      operations instead of passing through profile Int or Float.
    pub(crate) fn fixed_literal(
        value: FixedScalarValue,
        profile: NumericProfile,
    ) -> Option<String> {
        let scalar = value.scalar();

        match scalar.class() {
            FixedScalarClass::SignedInteger => {
                let carrier = Self::for_scalar(NumericScalar::Fixed(scalar), profile)?;
                Self::integer_literal(carrier, i128::from(value.as_i64()?))
            }
            FixedScalarClass::UnsignedInteger => {
                let carrier = Self::for_scalar(NumericScalar::Fixed(scalar), profile)?;
                Self::integer_literal(carrier, i128::from(value.as_u64()?))
            }
            FixedScalarClass::BinaryFloat => {
                let number = value.as_f64()?;
                number.is_finite().then(|| number.to_string())
            }
            FixedScalarClass::Octet => {
                let value = value.as_u64()?;
                (value <= u64::from(u8::MAX)).then(|| value.to_string())
            }
        }
    }

    fn integer_literal(carrier: Self, value: i128) -> Option<String> {
        let (min, max) = carrier.integer_bounds()?;
        if value < min || value > max {
            return None;
        }

        match carrier {
            Self::ExactInteger { .. } => Some(value.to_string()),
            Self::BigInteger { .. } => Some(format!("{value}n")),
            Self::BinaryFloat { .. } => None,
        }
    }

    pub(crate) fn integer_bounds(self) -> Option<(i128, i128)> {
        match self {
            Self::ExactInteger { min, max } | Self::BigInteger { min, max } => Some((min, max)),
            Self::BinaryFloat { .. } => None,
        }
    }

    pub(crate) fn float_precision(self) -> Option<BinaryFloatPrecision> {
        match self {
            Self::BinaryFloat { precision } => Some(precision),
            Self::ExactInteger { .. } | Self::BigInteger { .. } => None,
        }
    }

    /// Family stem used by checked numeric helper names.
    pub(crate) fn helper_family(self) -> Option<&'static str> {
        match self {
            Self::ExactInteger { .. } => Some("int"),
            Self::BigInteger { .. } => Some("bigint"),
            Self::BinaryFloat {
                precision: BinaryFloatPrecision::Binary16,
            } => None,
            Self::BinaryFloat {
                precision: BinaryFloatPrecision::Binary32,
            } => Some("float32"),
            Self::BinaryFloat {
                precision: BinaryFloatPrecision::Binary64,
            } => Some("float"),
        }
    }

    pub(crate) fn integer_bounds_js(self) -> Option<(String, String)> {
        let (min, max) = self.integer_bounds()?;
        Some(match self {
            Self::ExactInteger { .. } => (min.to_string(), max.to_string()),
            Self::BigInteger { .. } => (format!("{min}n"), format!("{max}n")),
            Self::BinaryFloat { .. } => unreachable!("float carrier has no integer range"),
        })
    }
}

/// JavaScript conversion selected for one semantic numeric pair and profile.
///
/// WHAT: owns carrier changes, fallibility, range bounds and binary-float rounding decisions, the
///       rendered JS expression and the runtime helpers that expression needs.
/// WHY: lowering must never reference a helper that the runtime prelude did not select, so the
///      call and its helper requirements are decided in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsNumericConversion {
    Identity,
    ToBigInt,
    ToNumber,
    /// `target` is always an integer carrier.
    CheckedIntegerToInteger {
        target: JsNumericCarrier,
    },
    RoundExactIntegerToFloat {
        precision: BinaryFloatPrecision,
    },
    CheckedExactIntegerToFloat {
        precision: BinaryFloatPrecision,
    },
    /// Always narrower than binary64; `Number(bigint)` is already correctly rounded.
    RoundBigIntegerToFloat {
        precision: BinaryFloatPrecision,
    },
    CheckedBigIntegerToFloat {
        precision: BinaryFloatPrecision,
    },
    /// `target` is always an integer carrier.
    FloatToInteger {
        target: JsNumericCarrier,
    },
    CheckedFloatNarrowing {
        precision: BinaryFloatPrecision,
    },
}

impl JsNumericConversion {
    /// Classifies one numeric conversion using the shared frontend evidence and JS carriers.
    pub(crate) fn classify(
        source: NumericScalar,
        target: NumericScalar,
        profile: NumericProfile,
    ) -> Result<Self, CompilerError> {
        let carrier = |scalar: NumericScalar| {
            JsNumericCarrier::for_scalar(scalar, profile).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "JS backend has no numeric carrier for {scalar:?}"
                ))
            })
        };
        let source_carrier = carrier(source)?;
        let target_carrier = carrier(target)?;
        let fallible = numeric_conversion_fallibility(source, target, profile)
            == BuiltinCastFallibility::Fallible;

        Ok(match (source_carrier, target_carrier) {
            (JsNumericCarrier::BinaryFloat { .. }, JsNumericCarrier::BinaryFloat { precision }) => {
                if fallible {
                    Self::CheckedFloatNarrowing { precision }
                } else {
                    Self::Identity
                }
            }
            (JsNumericCarrier::BinaryFloat { .. }, target) => Self::FloatToInteger { target },
            (
                JsNumericCarrier::ExactInteger { .. },
                JsNumericCarrier::BinaryFloat { precision },
            ) => {
                if fallible {
                    Self::CheckedExactIntegerToFloat { precision }
                } else {
                    Self::RoundExactIntegerToFloat { precision }
                }
            }
            (JsNumericCarrier::BigInteger { .. }, JsNumericCarrier::BinaryFloat { precision }) => {
                if fallible {
                    Self::CheckedBigIntegerToFloat { precision }
                } else if precision == BinaryFloatPrecision::Binary64 {
                    Self::ToNumber
                } else {
                    Self::RoundBigIntegerToFloat { precision }
                }
            }
            (_, target) if fallible => Self::CheckedIntegerToInteger { target },
            (JsNumericCarrier::ExactInteger { .. }, JsNumericCarrier::BigInteger { .. }) => {
                Self::ToBigInt
            }
            (JsNumericCarrier::BigInteger { .. }, JsNumericCarrier::ExactInteger { .. }) => {
                Self::ToNumber
            }
            _ => Self::Identity,
        })
    }

    /// Renders the conversion of the JS expression `value` from `source` to `target`.
    ///
    /// Checked conversions evaluate to the `{ tag, value }` result carrier; every other
    /// conversion evaluates to the converted value.
    pub(crate) fn expression(
        self,
        value: &str,
        source: NumericScalar,
        target: NumericScalar,
    ) -> String {
        let names = || format!("{:?}, {:?}", source.name(), target.name());
        let bounded = |helper: JsNumericRuntimeHelper, target: JsNumericCarrier| {
            let (minimum, maximum) = target
                .integer_bounds_js()
                .expect("integer conversion targets use an integer carrier");
            format!(
                "{}({value}, {minimum}, {maximum}, {})",
                helper.function_name(),
                names()
            )
        };
        let rounded = |helper: JsNumericRuntimeHelper, precision: BinaryFloatPrecision| {
            format!(
                "{}({value}, {}, {})",
                helper.function_name(),
                binary_float_precision_bits(precision),
                names()
            )
        };

        match self {
            Self::Identity => value.to_owned(),
            Self::ToBigInt => format!("BigInt({value})"),
            Self::ToNumber => format!("Number({value})"),
            Self::CheckedIntegerToInteger { target } => {
                bounded(JsNumericRuntimeHelper::CastIntegerToInteger, target)
            }
            Self::FloatToInteger { target } => {
                bounded(JsNumericRuntimeHelper::CastFloatToInteger, target)
            }
            Self::RoundExactIntegerToFloat { precision } => match precision {
                BinaryFloatPrecision::Binary16 => format!("Math.f16round({value})"),
                BinaryFloatPrecision::Binary32 => format!("Math.fround({value})"),
                BinaryFloatPrecision::Binary64 => value.to_owned(),
            },
            Self::RoundBigIntegerToFloat { precision } => format!(
                "{}({value}, {})",
                JsNumericRuntimeHelper::BigIntToBinaryFloat.function_name(),
                binary_float_precision_bits(precision)
            ),
            Self::CheckedExactIntegerToFloat { precision }
            | Self::CheckedBigIntegerToFloat { precision } => {
                rounded(JsNumericRuntimeHelper::CastIntegerToFloat, precision)
            }
            Self::CheckedFloatNarrowing { precision } => {
                rounded(JsNumericRuntimeHelper::CastFloatToFloat, precision)
            }
        }
    }

    /// Runtime helpers in dependency-first order for deterministic prelude emission.
    pub(crate) fn required_helpers(self) -> &'static [JsNumericRuntimeHelper] {
        use JsNumericRuntimeHelper::*;

        match self {
            Self::CheckedIntegerToInteger { .. } => &[CastIntegerInRange, CastIntegerToInteger],
            Self::CheckedExactIntegerToFloat { .. } => &[CastIntegerToFloat],
            Self::CheckedBigIntegerToFloat { .. } => &[BigIntToBinaryFloat, CastIntegerToFloat],
            Self::RoundBigIntegerToFloat { .. } => &[BigIntToBinaryFloat],
            Self::FloatToInteger { .. } => {
                &[CastIntegerInRange, NumericValueDisplay, CastFloatToInteger]
            }
            Self::CheckedFloatNarrowing { .. } => &[NumericValueDisplay, CastFloatToFloat],
            Self::Identity
            | Self::ToBigInt
            | Self::ToNumber
            | Self::RoundExactIntegerToFloat { .. } => &[],
        }
    }
}

/// A helper dependency selected by a `JsNumericConversion`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum JsNumericRuntimeHelper {
    CastIntegerInRange,
    CastIntegerToInteger,
    BigIntToBinaryFloat,
    CastIntegerToFloat,
    NumericValueDisplay,
    CastFloatToInteger,
    CastFloatToFloat,
}

impl JsNumericRuntimeHelper {
    pub(crate) fn function_name(self) -> &'static str {
        match self {
            Self::CastIntegerInRange => "__moth_cast_integer_in_range",
            Self::CastIntegerToInteger => "__moth_cast_integer_to_integer",
            Self::BigIntToBinaryFloat => "__moth_bigint_to_binary_float",
            Self::CastIntegerToFloat => "__moth_cast_integer_to_float",
            Self::NumericValueDisplay => "__moth_numeric_value_display",
            Self::CastFloatToInteger => "__moth_cast_float_to_int",
            Self::CastFloatToFloat => "__moth_cast_float_to_float",
        }
    }
}

/// The source-language bit count for a binary-float precision in generated JavaScript.
pub(crate) fn binary_float_precision_bits(precision: BinaryFloatPrecision) -> u8 {
    match precision {
        BinaryFloatPrecision::Binary16 => 16,
        BinaryFloatPrecision::Binary32 => 32,
        BinaryFloatPrecision::Binary64 => 64,
    }
}
