//! JavaScript numeric carrier and conversion policy.
//!
//! WHAT: maps numeric domains and profiles to JS carriers, then classifies casts into shared
//!       lowering decisions and demand-driven runtime-helper requirements.
//! WHY: representation and helper selection must not drift between generated calls and the
//!      prelude, especially when BigInt and Number carriers meet.
//!
//! Exclusions: frontend evidence owns semantic conversion fallibility. `Byte` is an octet value,
//! and binary floats remain separate from exact decimal `Dec`. Dec operations and exact casts
//! use a shared BigInt coefficient family with scales supplied by validated HIR or cast policies.

use crate::compiler_frontend::builtins::casts::evidence::numeric_conversion_fallibility;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastFallibility;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::NumericProfile;

const MAX_EXACT_JS_INTEGER: i128 = 9_007_199_254_740_991;

/// The runtime carrier selected for one numeric domain.
///
/// WHAT: exact-range integers choose the exact JS Number carrier, wider integers the exact
///       BigInt carrier, and declared binary-float scalars the precision-preserving Number
///       carrier. Arbitrary-precision `Dec` rides a primitive BigInt coefficient with its
///       scale carried in typed HIR, so it selects a dedicated scaled variant instead of
///       pretending to have profile-derived integer bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsNumericCarrier {
    ExactInteger {
        min: i128,
        max: i128,
    },
    BigInteger {
        min: i128,
        max: i128,
    },
    BinaryFloat {
        precision: BinaryFloatPrecision,
    },
    /// Arbitrary-precision decimal at one canonical scale.
    ///
    /// WHAT: stores the exact primitive BigInt coefficient carrier; the scale lives in the HIR
    ///       value and type, not in a profile guess or a fixed 64-bit bound pair.
    /// WHY: `Dec` has no language-level width, so borrowing fixed integer bounds would
    ///      silently reject legal scale-256 values and mislead later checks.
    ScaledInteger {
        scale: NumberScale,
    },
}

impl JsNumericCarrier {
    /// Maps a semantic scalar and selected profile to its JavaScript representation.
    pub(crate) fn for_scalar(scalar: NumericScalar, profile: NumericProfile) -> Option<Self> {
        if let NumericScalar::Number(scale) = scalar {
            return Some(Self::ScaledInteger { scale });
        }

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

    /// Formats one materialised `Number` value as its primitive BigInt JavaScript literal.
    ///
    /// WHAT: only the exact decimal sign and coefficient digits are emitted; the scale is not
    ///       part of the runtime value because typed HIR carries it.
    /// WHY: JS BigInt literals are exact, so the constant coefficient needs no conversion and no
    ///      per-scale literal family.
    pub(crate) fn number_literal(value: &NumberValue) -> String {
        format!("{}n", value.coefficient())
    }

    fn integer_literal(carrier: Self, value: i128) -> Option<String> {
        let (min, max) = carrier.integer_bounds()?;
        if value < min || value > max {
            return None;
        }

        match carrier {
            Self::ExactInteger { .. } => Some(value.to_string()),
            Self::BigInteger { .. } => Some(format!("{value}n")),
            Self::BinaryFloat { .. } | Self::ScaledInteger { .. } => None,
        }
    }

    pub(crate) fn integer_bounds(self) -> Option<(i128, i128)> {
        match self {
            Self::ExactInteger { min, max } | Self::BigInteger { min, max } => Some((min, max)),
            Self::BinaryFloat { .. } | Self::ScaledInteger { .. } => None,
        }
    }

    pub(crate) fn float_precision(self) -> Option<BinaryFloatPrecision> {
        match self {
            Self::BinaryFloat { precision } => Some(precision),
            Self::ExactInteger { .. } | Self::BigInteger { .. } | Self::ScaledInteger { .. } => {
                None
            }
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
            Self::ScaledInteger { .. } => Some("number"),
        }
    }

    pub(crate) fn integer_bounds_js(self) -> Option<(String, String)> {
        let (min, max) = self.integer_bounds()?;
        Some(match self {
            Self::ExactInteger { .. } => (min.to_string(), max.to_string()),
            Self::BigInteger { .. } => (format!("{min}n"), format!("{max}n")),
            Self::BinaryFloat { .. } | Self::ScaledInteger { .. } => {
                unreachable!("bounded helper has no float or scaled carrier")
            }
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
    /// Exact integer scaling into a Number coefficient.
    IntegerToNumber {
        target_scale: NumberScale,
    },
    /// Widening adds decimal zeroes without changing the represented value.
    NumberScaleWiden {
        source_scale: NumberScale,
        target_scale: NumberScale,
    },
    /// Narrowing succeeds only when every removed decimal digit is zero.
    CheckedNumberScaleNarrow {
        source_scale: NumberScale,
        target_scale: NumberScale,
    },
    /// Number-to-integer checks exact integrality and the concrete target's full bounds.
    CheckedNumberToInteger {
        source_scale: NumberScale,
        target: JsNumericCarrier,
    },
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

        match (source, target, source_carrier, target_carrier) {
            (NumericScalar::Number(source_scale), NumericScalar::Number(target_scale), _, _)
                if source_scale < target_scale =>
            {
                if fallible {
                    return Err(number_conversion_policy_error(source, target));
                }

                return Ok(Self::NumberScaleWiden {
                    source_scale,
                    target_scale,
                });
            }
            (NumericScalar::Number(source_scale), NumericScalar::Number(target_scale), _, _)
                if source_scale > target_scale =>
            {
                if !fallible {
                    return Err(number_conversion_policy_error(source, target));
                }

                return Ok(Self::CheckedNumberScaleNarrow {
                    source_scale,
                    target_scale,
                });
            }
            (NumericScalar::Number(_), NumericScalar::Number(_), _, _) => {
                return Err(CompilerError::compiler_error(
                    "JS backend received a same-scale Number cast policy",
                ));
            }
            (source, NumericScalar::Number(target_scale), _, _) if source.is_integer() => {
                if fallible {
                    return Err(number_conversion_policy_error(source, target));
                }

                if target_scale == NumberScale::ZERO {
                    return match source_carrier {
                        JsNumericCarrier::BigInteger { .. } => Ok(Self::Identity),
                        JsNumericCarrier::ExactInteger { .. } => Ok(Self::ToBigInt),
                        _ => Err(number_conversion_policy_error(source, target)),
                    };
                }

                return Ok(Self::IntegerToNumber { target_scale });
            }
            (NumericScalar::Number(source_scale), target, _, target_carrier)
                if target.is_integer() =>
            {
                if !fallible {
                    return Err(number_conversion_policy_error(source, target));
                }

                return Ok(Self::CheckedNumberToInteger {
                    source_scale,
                    target: target_carrier,
                });
            }
            _ => {}
        }
        if matches!(source, NumericScalar::Number(_)) || matches!(target, NumericScalar::Number(_))
        {
            return Err(CompilerError::compiler_error(format!(
                "JS backend received unsupported Number conversion from {source} to {target}"
            )));
        }

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
        // Direct ASCII-quoted canonical spellings, never Debug enum text.
        let names = || format!("\"{source}\", \"{target}\"");
        let bounded = |helper: JsNumericRuntimeHelper, target: JsNumericCarrier| {
            let (minimum, maximum) = target
                .integer_bounds_js()
                .expect("integer conversion targets use an integer carrier");
            format!(
                "{}({value}, {minimum}, {maximum}, {})",
                helper.function_name(),
                names(),
            )
        };
        let rounded = |helper: JsNumericRuntimeHelper, precision: BinaryFloatPrecision| {
            format!(
                "{}({value}, {}, {})",
                helper.function_name(),
                binary_float_precision_bits(precision),
                names(),
            )
        };

        match self {
            Self::Identity => value.to_owned(),
            Self::ToBigInt => format!("BigInt({value})"),
            Self::ToNumber => format!("Number({value})"),
            Self::NumberScaleWiden {
                source_scale,
                target_scale,
            } => format!(
                "{value} * {}",
                number_scale_factor_js(target_scale.get() - source_scale.get())
            ),
            Self::IntegerToNumber { target_scale } => format!(
                "BigInt({value}) * {}",
                number_scale_factor_js(target_scale.get())
            ),
            Self::CheckedNumberScaleNarrow {
                source_scale,
                target_scale,
            } => format!(
                "{}({value}, {}, {}, {})",
                JsNumericRuntimeHelper::CastNumberScale.function_name(),
                number_scale_factor_js(source_scale.get() - target_scale.get()),
                source_scale.get(),
                names(),
            ),
            Self::CheckedNumberToInteger {
                source_scale,
                target,
            } => {
                let (minimum, maximum) = target
                    .integer_bounds_js()
                    .expect("Number cast targets use an integer carrier");
                format!(
                    "{}({value}, {}, {}, {minimum}, {maximum}, {})",
                    JsNumericRuntimeHelper::CastNumberToInteger.function_name(),
                    number_scale_factor_js(source_scale.get()),
                    source_scale.get(),
                    names(),
                )
            }
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

    /// Returns the carrier-only conversion for a proven-safe checked integer narrowing.
    ///
    /// WHAT: proven narrowing performs exactly the original carrier conversion — identity within
    ///       one carrier, `BigInt`/`Number` across carriers — and omits only the range predicate
    ///       the checked helper would enforce. The caller keeps the `{tag, value}` success-carrier
    ///       shape the unchanged HIR requires.
    /// WHY: the proof table guarantees the source value is inside the target range, so the
    ///       conversion needs no runtime check, and it demands no runtime helpers.
    pub(crate) fn proven_safe_integer_narrowing(
        self,
        source_carrier: JsNumericCarrier,
    ) -> Option<Self> {
        let Self::CheckedIntegerToInteger { target } = self else {
            return None;
        };

        Some(match (source_carrier, target) {
            (JsNumericCarrier::ExactInteger { .. }, JsNumericCarrier::BigInteger { .. }) => {
                Self::ToBigInt
            }
            (JsNumericCarrier::BigInteger { .. }, JsNumericCarrier::ExactInteger { .. }) => {
                Self::ToNumber
            }
            (
                JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
                JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
            ) => Self::Identity,
            _ => return None,
        })
    }

    /// Runtime helpers in dependency-first order for deterministic prelude emission.
    pub(crate) fn required_helpers(self) -> &'static [JsNumericRuntimeHelper] {
        use JsNumericRuntimeHelper::*;

        match self {
            Self::CheckedIntegerToInteger { .. } => &[CastIntegerInRange, CastIntegerToInteger],
            Self::CheckedNumberScaleNarrow { .. } => &[FormatNumber, CastNumberScale],
            Self::CheckedNumberToInteger { .. } => &[FormatNumber, CastNumberToInteger],
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
            | Self::NumberScaleWiden { .. }
            | Self::IntegerToNumber { .. }
            | Self::RoundExactIntegerToFloat { .. } => &[],
        }
    }
}

fn number_conversion_policy_error(source: NumericScalar, target: NumericScalar) -> CompilerError {
    CompilerError::compiler_error(format!(
        "JS backend received inconsistent Number conversion evidence for {source} to {target}"
    ))
}

/// Emits a compile-time exact base-ten factor for a validated Number scale.
pub(crate) fn number_scale_factor_js(scale: u16) -> String {
    let mut factor = String::with_capacity(usize::from(scale) + 2);
    factor.push('1');
    factor.extend(std::iter::repeat_n('0', usize::from(scale)));
    factor.push('n');
    factor
}

/// A helper dependency selected by a `JsNumericConversion`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum JsNumericRuntimeHelper {
    CastIntegerInRange,
    CastIntegerToInteger,
    CastNumberScale,
    CastNumberToInteger,
    FormatNumber,
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
            Self::CastNumberScale => "__moth_cast_number_scale",
            Self::CastNumberToInteger => "__moth_cast_number_to_integer",
            Self::FormatNumber => "__moth_format_number",
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
