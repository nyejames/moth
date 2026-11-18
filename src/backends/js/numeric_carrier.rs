//! JavaScript numeric carrier and conversion policy.
//!
//! WHAT: maps numeric domains to JS carriers, then classifies casts and numeric operations into
//!       shared lowering decisions and demand-driven runtime-helper requirements.
//! WHY: representation and helper selection must not drift between generated calls and the
//!      prelude, especially when BigInt and Number carriers meet.
//!
//! Exclusions: frontend evidence owns semantic conversion fallibility, and the frontend
//! `numeric_failure_codes` table owns operation fallibility. The backend only maps those facts to
//! JavaScript representation. `Byte` is an octet value, and binary floats remain separate from
//! exact decimal `Dec`. Dec operations and exact casts use a shared BigInt coefficient family
//! with scales supplied by validated HIR or cast policies.

use crate::backends::js::runtime::NumericRuntimeHelperUsage;
use crate::compiler_frontend::builtins::casts::evidence::numeric_conversion_fallibility;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastFallibility;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, numeric_failure_codes,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::numeric::{
    FastStoreConversionClass, HirFastStoreConversion, HirNumericOp,
};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;

const MAX_EXACT_JS_INTEGER: i128 = 9_007_199_254_740_991;

/// The runtime carrier selected for one numeric domain.
///
/// WHAT: exact-range integers choose the exact JS Number carrier, wider integers the exact
///       BigInt carrier, and declared binary-float scalars the precision-preserving Number
///       carrier. Arbitrary-precision `Dec` rides a primitive BigInt coefficient with its
///       scale carried in typed HIR, so it selects a dedicated scaled variant instead of
///       pretending to have fixed integer bounds.
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
    ///       value and type, not in a guessed width or a fixed 64-bit bound pair.
    /// WHY: `Dec` has no language-level width, so borrowing fixed integer bounds would
    ///      silently reject legal scale-256 values and mislead later checks.
    ScaledInteger {
        scale: NumberScale,
    },
}

impl JsNumericCarrier {
    /// Maps one semantic scalar to its JavaScript representation.
    pub(crate) fn for_scalar(scalar: NumericScalar) -> Option<Self> {
        if let NumericScalar::Number(scale) = scalar {
            return Some(Self::ScaledInteger { scale });
        }

        if let Some((min, max)) = scalar.integer_range() {
            return Some(
                if min >= -MAX_EXACT_JS_INTEGER && max <= MAX_EXACT_JS_INTEGER {
                    Self::ExactInteger { min, max }
                } else {
                    Self::BigInteger { min, max }
                },
            );
        }

        scalar
            .binary_float_precision()
            .map(|precision| Self::BinaryFloat { precision })
    }

    /// Formats an already-materialised fixed scalar without changing its exact value.
    ///
    /// WHAT: integer carriers choose decimal Number or BigInt spelling, binary-float values retain
    ///       their exact f64 carrier bits (including signed zero), and Byte stays a Number octet.
    /// WHY: HIR literals must enter the same carrier used by later comparisons, casts and checked
    ///      operations instead of a separate literal carrier.
    pub(crate) fn fixed_literal(value: FixedScalarValue) -> Option<String> {
        let scalar = value.scalar();

        match scalar.class() {
            FixedScalarClass::SignedInteger => {
                let carrier = Self::for_scalar(NumericScalar::Fixed(scalar))?;
                Self::integer_literal(carrier, i128::from(value.as_i64()?))
            }
            FixedScalarClass::UnsignedInteger => {
                let carrier = Self::for_scalar(NumericScalar::Fixed(scalar))?;
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

    /// Formats one integer value in this carrier's JavaScript spelling.
    ///
    /// WHAT: exact-range carriers emit decimal Number literals, wider carriers exact BigInt
    ///       literals, and every non-integer carrier rejects the value.
    /// WHY: materialised literals must be spelled in the same carrier the runtime values use.
    fn integer_literal(self, value: i128) -> Option<String> {
        let (min, max) = self.integer_bounds()?;
        if value < min || value > max {
            return None;
        }

        match self {
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

    /// The fixed width and signedness of one integer carrier.
    ///
    /// WHAT: derives the semantic bit width from the carrier's exact bounds and reports whether
    ///       the domain is signed (its minimum is negative).
    /// WHY: fast wrapping arithmetic and fast compound write-back both reduce a value to the
    ///      *result domain's* width, never the JS carrier's width. One derivation keeps the two
    ///      from disagreeing about `I32` versus `U32` or a narrow `I8`.
    pub(crate) fn integer_width_and_signedness(self) -> Option<(u8, bool)> {
        let (minimum, maximum) = self.integer_bounds()?;
        let width = u8::try_from((maximum - minimum + 1).trailing_zeros()).ok()?;
        matches!(width, 8 | 16 | 32 | 64).then_some((width, minimum < 0))
    }

    /// Family stem used by checked numeric helper names.
    pub(crate) fn helper_family(self) -> &'static str {
        match self {
            Self::ExactInteger { .. } => "int",
            Self::BigInteger { .. } => "bigint",
            Self::BinaryFloat { precision } => checked_float_helper_family(precision),
            Self::ScaledInteger { .. } => "number",
        }
    }

    pub(crate) fn integer_bounds_js(self) -> Option<(String, String)> {
        let (min, max) = self.integer_bounds()?;
        Some(match self {
            Self::ExactInteger { .. } => (min.to_string(), max.to_string()),
            Self::BigInteger { .. } => (format!("{min}n"), format!("{max}n")),
            Self::BinaryFloat { .. } | Self::ScaledInteger { .. } => {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "bounded helper has no float or scaled carrier"
                )
            }
        })
    }
}

/// JavaScript conversion selected for one semantic numeric pair.
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
    ) -> Result<Self, InfrastructureFailure> {
        let carrier = |scalar: NumericScalar| {
            JsNumericCarrier::for_scalar(scalar).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::BackendLowering,
                    format!("JS backend has no numeric carrier for {scalar:?}"),
                )
            })
        };
        let source_carrier = carrier(source)?;
        let target_carrier = carrier(target)?;
        let fallible =
            numeric_conversion_fallibility(source, target) == BuiltinCastFallibility::Fallible;

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
                return Err(InfrastructureFailure::internal(
                    CompilerStage::BackendLowering,
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
            return Err(InfrastructureFailure::internal(
                CompilerStage::BackendLowering,
                format!(
                    "JS backend received unsupported Number conversion from {source} to {target}"
                ),
            ));
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
    ///       the checked helper would enforce. The caller stores the converted scalar directly
    ///       through the Invoke's success slot; no `{tag, value}` success-carrier shape remains.
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

fn number_conversion_policy_error(
    source: NumericScalar,
    target: NumericScalar,
) -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::BackendLowering,
        format!(
            "JS backend received inconsistent Number conversion evidence for {source} to {target}"
        ),
    )
}

/// Emits a compile-time exact base-ten factor for a validated Number scale.
pub(crate) fn number_scale_factor_js(scale: u16) -> String {
    let mut factor = String::with_capacity(usize::from(scale) + 2);
    factor.push('1');
    factor.extend(std::iter::repeat_n('0', usize::from(scale)));
    factor.push('n');
    factor
}

/// The JavaScript lowering of one fast compound-assignment write-back conversion.
///
/// WHAT: owns the conversion class, its rendered expression and its runtime-helper demand.
/// WHY: demand tracking and statement lowering must agree on exactly which helpers a fast
///      write-back needs. Ordinary modules keep emitting no new code: every helper is demanded
///      only by a reachable fast conversion, and the expressions never use the checked cast
///      helpers that ordinary store conversions do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsFastStoreConversion {
    /// Integer write-back: keep the destination low bits and signedness.
    IntegerWrap {
        target: JsNumericCarrier,
        bits: u8,
        signed: bool,
    },
    /// Float-to-integer write-back: truncate toward zero and throw when out of range.
    FloatToInteger { target: JsNumericCarrier },
    /// Integer-to-binary-float write-back: round at the destination precision.
    IntegerToFloat {
        source: JsNumericCarrier,
        precision: BinaryFloatPrecision,
    },
    /// Binary-float rounding write-back.
    FloatToFloat { precision: BinaryFloatPrecision },
}

impl JsFastStoreConversion {
    /// Classifies one HIR fast write-back into its JavaScript lowering.
    pub(crate) fn classify(
        conversion: HirFastStoreConversion,
    ) -> Result<Self, InfrastructureFailure> {
        let class = conversion.classify().ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::BackendLowering,
                format!("JS backend received invalid fast store conversion {conversion}"),
            )
        })?;
        let carrier = |scalar: NumericScalar| {
            JsNumericCarrier::for_scalar(scalar).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::BackendLowering,
                    format!("JS backend has no numeric carrier for {scalar:?}"),
                )
            })
        };

        match class {
            FastStoreConversionClass::IntegerWrap => {
                let target = carrier(conversion.target)?;
                let (bits, signed) = target.integer_width_and_signedness().ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::BackendLowering,
                        format!(
                            "JS fast integer write-back target {target:?} has no fixed 8/16/32/64-bit width"
                        ),
                    )
                })?;
                Ok(Self::IntegerWrap {
                    target,
                    bits,
                    signed,
                })
            }
            FastStoreConversionClass::FloatToInteger => Ok(Self::FloatToInteger {
                target: carrier(conversion.target)?,
            }),
            FastStoreConversionClass::IntegerToFloat => Ok(Self::IntegerToFloat {
                source: carrier(conversion.source)?,
                precision: conversion.target.binary_float_precision().ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::BackendLowering,
                        "JS fast integer-to-float target has no precision",
                    )
                })?,
            }),
            FastStoreConversionClass::FloatToFloat => Ok(Self::FloatToFloat {
                precision: conversion.target.binary_float_precision().ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::BackendLowering,
                        "JS fast float write-back target has no precision",
                    )
                })?,
            }),
        }
    }

    /// Records the runtime helpers this conversion needs from the numeric prelude.
    pub(crate) fn require_helpers(self, usage: &mut NumericRuntimeHelperUsage) {
        match self {
            Self::IntegerWrap { .. } => usage.fast_integer_wrap = true,
            Self::FloatToInteger { .. } => usage.fast_float_to_integer = true,
            Self::IntegerToFloat { source, precision } => {
                usage.fast_float_conversion = true;
                if matches!(source, JsNumericCarrier::BigInteger { .. })
                    && precision != BinaryFloatPrecision::Binary64
                {
                    usage.fast_bigint_to_float = true;
                }
            }
            Self::FloatToFloat { .. } => usage.fast_float_conversion = true,
        }
    }

    /// Renders the conversion of the JavaScript expression `value`.
    ///
    /// Integer wraps produce a BigInt helper result; the Number carrier converts it back exactly
    /// because every Number-carried integer destination width is at most 32 bits.
    pub(crate) fn expression(self, value: &str) -> String {
        let bits = |precision: BinaryFloatPrecision| binary_float_precision_bits(precision);
        match self {
            Self::IntegerWrap {
                target,
                bits: width,
                signed,
            } => {
                let wrapped = format!("__moth_fast_wrap_integer({value}, {width}, {signed})");
                match target {
                    JsNumericCarrier::BigInteger { .. } => wrapped,
                    JsNumericCarrier::ExactInteger { .. } => format!("Number({wrapped})"),
                    JsNumericCarrier::BinaryFloat { .. }
                    | JsNumericCarrier::ScaledInteger { .. } => {
                        crate::compiler_bug!(
                            CompilerStage::BackendLowering,
                            "JS fast integer wrap target must be an integer carrier"
                        )
                    }
                }
            }
            Self::FloatToInteger { target } => {
                let (minimum, maximum) = target.integer_bounds_js().unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JS fast float-to-integer target must be an integer carrier"
                    )
                });
                format!("__moth_fast_float_to_integer({value}, {minimum}, {maximum})")
            }
            Self::IntegerToFloat { source, precision } => {
                let precision_bits = bits(precision);
                let numeric = match source {
                    JsNumericCarrier::BigInteger { .. }
                        if precision != BinaryFloatPrecision::Binary64 =>
                    {
                        format!("__moth_bigint_to_binary_float({value}, {precision_bits})")
                    }
                    JsNumericCarrier::BigInteger { .. } => format!("Number({value})"),
                    JsNumericCarrier::ExactInteger { .. } => value.to_owned(),
                    JsNumericCarrier::BinaryFloat { .. }
                    | JsNumericCarrier::ScaledInteger { .. } => {
                        crate::compiler_bug!(
                            CompilerStage::BackendLowering,
                            "JS fast integer-to-float source must be an integer carrier"
                        )
                    }
                };
                format!("__moth_fast_float_conversion({numeric}, {precision_bits})")
            }
            Self::FloatToFloat { precision } => {
                format!("__moth_fast_float_conversion({value}, {})", bits(precision))
            }
        }
    }
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

/// The JavaScript lowering of one fast scalar numeric operation.
///
/// WHAT: owns the operation's helper/expression shape, its rendered JavaScript and its
///       runtime-helper demand.
/// WHY: demand tracking and statement lowering must agree on exactly which fast helper a
///      reachable operation needs. Fast arithmetic reduces by the semantic result domain's width,
///      so the selection reads the carrier's fixed width and signedness instead of the JS carrier
///      type alone. Total operations render plain width-normalised expressions; trapping
///      operations render helpers that throw the canonical cause directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsFastNumericLowering {
    /// A Number-carried integer at most 32 bits wide.
    NumberInteger {
        operator: NumericOperator,
        bits: u8,
        signed: bool,
        minimum: i128,
    },
    /// A BigInt-carried 64-bit integer whose operation is exact after width normalisation.
    BigIntWrap {
        operator: NumericOperator,
        signed: bool,
    },
    /// A BigInt-carried 64-bit operation that owns a zero/minimum/exponent trap.
    BigIntHelper {
        operator: NumericOperator,
        signed: bool,
        minimum: i128,
    },
    /// A binary-float operation with per-operation rounding and a finite-result trap.
    BinaryFloat {
        operator: NumericOperator,
        precision: BinaryFloatPrecision,
    },
    /// Binary-float negation is total; it rounds in the resolved precision and never traps.
    BinaryFloatNegate { precision: BinaryFloatPrecision },
}

impl JsFastNumericLowering {
    /// Classifies one validated fast operation into its JavaScript lowering.
    ///
    /// Rejects ordinary operations, `Dec` domains and unsupported domain/operator pairs: a fast
    /// operation is always a fixed integer or binary float, and integer `/` already computes in
    /// `F64`, so an integer `Divide` can only be an internal contradiction.
    fn classify(op: HirNumericOp) -> Result<Self, InfrastructureFailure> {
        if !op.semantics.is_fast() {
            return Err(fast_lowering_error(format!(
                "JS backend received non-fast numeric operation {op} on the fast lowering path"
            )));
        }
        let carrier = JsNumericCarrier::for_scalar(op.domain).ok_or_else(|| {
            fast_lowering_error(format!(
                "JS backend has no numeric carrier for fast operation domain {:?}",
                op.domain
            ))
        })?;

        match carrier {
            JsNumericCarrier::ExactInteger { min, max } => {
                let (bits, signed) = carrier.integer_width_and_signedness().ok_or_else(|| {
                    fast_lowering_error("fast Number integer carrier has no fixed width".to_owned())
                })?;
                match op.operator {
                    NumericOperator::Add
                    | NumericOperator::Subtract
                    | NumericOperator::Multiply
                    | NumericOperator::IntegerDivide
                    | NumericOperator::Remainder
                    | NumericOperator::Power
                    | NumericOperator::Negate => Ok(Self::NumberInteger {
                        operator: op.operator,
                        bits,
                        signed,
                        minimum: min,
                    }),
                    NumericOperator::Divide => Err(fast_lowering_error(format!(
                        "fast integer division resolved to the Number integer carrier {min}..={max}"
                    ))),
                }
            }
            JsNumericCarrier::BigInteger { min, .. } => {
                let (_, signed) = carrier.integer_width_and_signedness().ok_or_else(|| {
                    fast_lowering_error("fast BigInt carrier has no fixed width".to_owned())
                })?;
                match op.operator {
                    NumericOperator::Add
                    | NumericOperator::Subtract
                    | NumericOperator::Multiply
                    | NumericOperator::Negate => Ok(Self::BigIntWrap {
                        operator: op.operator,
                        signed,
                    }),
                    NumericOperator::IntegerDivide
                    | NumericOperator::Remainder
                    | NumericOperator::Power => Ok(Self::BigIntHelper {
                        operator: op.operator,
                        signed,
                        minimum: min,
                    }),
                    NumericOperator::Divide => Err(fast_lowering_error(format!(
                        "fast integer division resolved to the BigInt carrier starting at {min}"
                    ))),
                }
            }
            JsNumericCarrier::BinaryFloat { precision } => match op.operator {
                NumericOperator::Add
                | NumericOperator::Subtract
                | NumericOperator::Multiply
                | NumericOperator::Divide
                | NumericOperator::Remainder
                | NumericOperator::Power => Ok(Self::BinaryFloat {
                    operator: op.operator,
                    precision,
                }),
                NumericOperator::Negate => Ok(Self::BinaryFloatNegate { precision }),
                NumericOperator::IntegerDivide => Err(fast_lowering_error(
                    "fast truncating integer division has no binary-float domain".to_owned(),
                )),
            },
            JsNumericCarrier::ScaledInteger { .. } => Err(fast_lowering_error(format!(
                "fast numeric operation {op} resolved to a Dec domain"
            ))),
        }
    }

    /// Records the runtime helpers this operation needs from the numeric prelude.
    ///
    /// WHY: the Number-carried division, remainder and power helpers call the width normaliser,
    ///      so a fast Number integer always demands that shared helper and only the trapping
    ///      helper its own operator uses. Fast BigInt add, subtract, multiply and negate inline
    ///      their `BigInt.asIntN`/`asUintN` normalisation and demand no runtime code.
    fn require_helpers(self, usage: &mut NumericRuntimeHelperUsage) {
        match self {
            Self::NumberInteger { operator, .. } => {
                usage.fast_number_integer_wrap = true;
                match operator {
                    NumericOperator::IntegerDivide => usage.fast_number_integer_division = true,
                    NumericOperator::Remainder => usage.fast_number_integer_remainder = true,
                    NumericOperator::Power => usage.fast_number_integer_power = true,
                    NumericOperator::Add
                    | NumericOperator::Subtract
                    | NumericOperator::Multiply
                    | NumericOperator::Negate => {}
                    NumericOperator::Divide => crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JS fast Number integer helper demand received a real-division domain"
                    ),
                }
            }
            // Width-normalised BigInt expressions use only `BigInt.asIntN`/`asUintN`.
            Self::BigIntWrap { .. } => {}
            Self::BigIntHelper { operator, .. } => match operator {
                NumericOperator::IntegerDivide => usage.fast_big_integer_division = true,
                NumericOperator::Remainder => usage.fast_big_integer_remainder = true,
                NumericOperator::Power => usage.fast_big_integer_power = true,
                NumericOperator::Add
                | NumericOperator::Subtract
                | NumericOperator::Multiply
                | NumericOperator::Divide
                | NumericOperator::Negate => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS fast BigInt helper demand received an operation with no trapping helper"
                ),
            },
            Self::BinaryFloat {
                operator,
                precision,
            } => {
                require_fast_float_helpers(usage, precision, operator);
            }
            Self::BinaryFloatNegate { .. } => {}
        }
    }

    /// Renders the operation over already-lowered operand expressions.
    fn expression(self, operands: &[String]) -> String {
        match self {
            Self::NumberInteger {
                operator,
                bits,
                signed,
                minimum,
            } => match (operator, operands) {
                (NumericOperator::Add, [left, right]) => {
                    format!("__moth_fast_int_wrap(({left} + {right}), {bits}, {signed})")
                }
                (NumericOperator::Subtract, [left, right]) => {
                    format!("__moth_fast_int_wrap(({left} - {right}), {bits}, {signed})")
                }
                (NumericOperator::Multiply, [left, right]) => {
                    format!("__moth_fast_int_wrap(Math.imul({left}, {right}), {bits}, {signed})")
                }
                (NumericOperator::Negate, [operand]) => {
                    format!("__moth_fast_int_wrap(-({operand}), {bits}, {signed})")
                }
                (NumericOperator::IntegerDivide, [left, right]) => {
                    format!("__moth_fast_int_div({left}, {right}, {bits}, {signed}, {minimum})")
                }
                (NumericOperator::Remainder, [left, right]) => {
                    format!("__moth_fast_int_mod({left}, {right}, {bits}, {signed})")
                }
                (NumericOperator::Power, [left, right]) => {
                    format!("__moth_fast_int_pow({left}, {right}, {bits}, {signed})")
                }
                _ => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS fast Number integer operation {operator:?} has unexpected operand shape"
                ),
            },
            Self::BigIntWrap { operator, signed } => {
                let normalization = if signed { "asIntN" } else { "asUintN" };
                match (operator, operands) {
                    (NumericOperator::Add, [left, right]) => {
                        format!("BigInt.{normalization}(64, ({left} + {right}))")
                    }
                    (NumericOperator::Subtract, [left, right]) => {
                        format!("BigInt.{normalization}(64, ({left} - {right}))")
                    }
                    (NumericOperator::Multiply, [left, right]) => {
                        format!("BigInt.{normalization}(64, ({left} * {right}))")
                    }
                    (NumericOperator::Negate, [operand]) => {
                        format!("BigInt.{normalization}(64, -({operand}))")
                    }
                    _ => crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JS fast BigInt wrap operation {operator:?} has unexpected operand shape"
                    ),
                }
            }
            Self::BigIntHelper {
                operator,
                signed,
                minimum,
            } => match (operator, operands) {
                (NumericOperator::IntegerDivide, [left, right]) => {
                    format!("__moth_fast_bigint_div({left}, {right}, {minimum}n)")
                }
                (NumericOperator::Remainder, [left, right]) => {
                    format!("__moth_fast_bigint_mod({left}, {right})")
                }
                (NumericOperator::Power, [left, right]) => {
                    format!("__moth_fast_bigint_pow({left}, {right}, {signed})")
                }
                _ => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS fast BigInt helper operation {operator:?} has unexpected operand shape"
                ),
            },
            Self::BinaryFloat {
                operator,
                precision,
            } => {
                let Some(operation) = binary_float_helper_operation(operator) else {
                    crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JS fast binary-float operation {operator:?} has no helper"
                    )
                };
                let family = fast_float_helper_family(precision);
                match operands {
                    [left, right] => format!("__moth_{family}_{operation}({left}, {right})"),
                    _ => crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JS fast binary-float operation {operator:?} has unexpected operand shape"
                    ),
                }
            }
            Self::BinaryFloatNegate { precision } => match operands {
                [operand] => binary_float_negate_expression(precision, operand),
                _ => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS fast binary-float negation has unexpected operand shape"
                ),
            },
        }
    }
}

/// The helper suffix for one binary-float arithmetic operator.
///
/// WHAT: the shared `add`/`sub`/`mul`/`div`/`mod`/`pow` spelling, or `None` for the operators with
///       no binary-float helper.
/// WHY: the fast and checked helper families differ only in their stems and failure delivery, so
///      one suffix map keeps the two call sites from spelling the same operation differently.
fn binary_float_helper_operation(operator: NumericOperator) -> Option<&'static str> {
    match operator {
        NumericOperator::Add => Some("add"),
        NumericOperator::Subtract => Some("sub"),
        NumericOperator::Multiply => Some("mul"),
        NumericOperator::Divide => Some("div"),
        NumericOperator::Remainder => Some("mod"),
        NumericOperator::Power => Some("pow"),
        NumericOperator::IntegerDivide | NumericOperator::Negate => None,
    }
}

/// The JavaScript expression for total binary-float negation at one precision.
///
/// WHAT: rounds `-operand` at the resolved precision, which for F16/F32 is also the value's own
///       carrier precision and for F64 needs no rounding step.
/// WHY: ordinary total negation and fast arithmetic negation share this expression so both
///      deliveries keep identical rounding and signed-zero behaviour; negation is the one binary
///      float operation with no failure edge, and only its delivery differed before this owner.
fn binary_float_negate_expression(precision: BinaryFloatPrecision, operand: &str) -> String {
    match precision {
        BinaryFloatPrecision::Binary16 => format!("Math.f16round(-({operand}))"),
        BinaryFloatPrecision::Binary32 => format!("Math.fround(-({operand}))"),
        BinaryFloatPrecision::Binary64 => format!("(-({operand}))"),
    }
}

/// The JavaScript lowering selected for one ordinary or fast numeric operation.
///
/// WHAT: classifies an operation once as fast arithmetic, a total ordinary scalar, a proven
///       integer scalar or a checked recoverable-carrier helper call, then owns its rendered
///       expression and runtime-helper demand.
/// WHY: statement lowering and the demand scan must reach the same decision. A second
///      operator/carrier/proof test in either place can select a helper the other omits, or
///      demand a family an operation never calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsNumericOperationLowering {
    /// `$fast_math` arithmetic with the fast wrap/trap helpers.
    Fast(JsFastNumericLowering),
    /// An ordinary operation whose shared failure classification is empty, so it returns a
    /// scalar and needs no carrier unwrap or trap helper.
    Scalar(JsScalarNumericLowering),
    /// An ordinary checked integer operation discharged by the sparse proof table.
    ProvenInteger(JsProvenIntegerLowering),
    /// An ordinary operation that retains the recoverable-carrier helper family.
    Checked(JsCheckedNumericLowering),
}

impl JsNumericOperationLowering {
    /// Classifies one numeric operation, consulting the proof table lazily.
    ///
    /// WHAT: fast arithmetic wins first, then a proof-backed integer expression, then the shared
    ///       empty-failure-set scalar path, and finally the checked helper family.
    /// WHY: `numeric_failure_codes` remains the only semantic fallibility classifier, and the
    ///      proof table remains the only optional check-elision source. The closure keeps the
    ///      sparse proof lookup off every fast, float and Dec path, which it can never prove.
    pub(crate) fn classify(
        op: HirNumericOp,
        proven_integer: impl FnOnce() -> bool,
    ) -> Result<Self, InfrastructureFailure> {
        if op.semantics.is_fast() {
            return Ok(Self::Fast(JsFastNumericLowering::classify(op)?));
        }

        let carrier = JsNumericCarrier::for_scalar(op.domain).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::BackendLowering,
                format!("JS backend has no numeric carrier for {:?}", op.domain),
            )
        })?;

        if let Some(proven) = JsProvenIntegerLowering::classify(op, carrier, proven_integer) {
            return Ok(Self::ProvenInteger(proven));
        }

        if numeric_failure_codes(op.operator, op.domain).is_empty() {
            return Ok(Self::Scalar(JsScalarNumericLowering::classify(
                op, carrier,
            )?));
        }

        Ok(Self::Checked(JsCheckedNumericLowering {
            carrier,
            operator: op.operator,
        }))
    }

    /// Renders the operation over already-lowered operand expressions.
    ///
    /// Operands are consumed so the checked renderer can append its scale factor and bounds
    /// without cloning every expression string.
    pub(crate) fn expression(self, operands: Vec<String>) -> String {
        match self {
            Self::Fast(lowering) => lowering.expression(&operands),
            Self::Scalar(lowering) => lowering.expression(&operands),
            Self::ProvenInteger(lowering) => lowering.expression(&operands),
            Self::Checked(lowering) => lowering.expression(operands),
        }
    }

    /// Whether the rendered expression is a scalar value rather than a `{ tag, value }` carrier.
    pub(crate) fn is_scalar(self) -> bool {
        !matches!(self, Self::Checked(_))
    }

    /// Records the runtime helpers this operation needs from the numeric prelude.
    ///
    /// Delivery is not part of helper demand: the demand scan adds the trap demand for a
    /// statement that consumes a checked carrier, mirroring `emit_numeric_carrier_assignment`.
    pub(crate) fn require_helpers(self, usage: &mut NumericRuntimeHelperUsage) {
        match self {
            Self::Fast(lowering) => lowering.require_helpers(usage),
            Self::Scalar(lowering) => lowering.require_helpers(usage),
            Self::ProvenInteger(_) => {}
            Self::Checked(lowering) => lowering.require_helpers(usage),
        }
    }
}

/// The scalar ordinary lowering for an operation with no recoverable failure.
///
/// WHAT: exact `Dec` coefficient arithmetic and binary-float arithmetic at the resolved precision.
/// WHY: the shared failure classifier says these operations cannot fail, so a carrier would only
///      add allocation and an unwrap. Positive-scale Dec multiplication still rounds once through
///      the same half-even kernel the fallible Dec family uses, so values cannot diverge between
///      paths; scale-zero multiplication is exact coefficient multiplication and skips the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsScalarNumericLowering {
    /// Add, subtract, negate or multiply BigInt coefficients at one resolved `Dec` scale.
    Decimal {
        operator: NumericOperator,
        scale: NumberScale,
    },
    /// Binary-float arithmetic at the resolved precision.
    BinaryFloat {
        operator: NumericOperator,
        precision: BinaryFloatPrecision,
    },
}

impl JsScalarNumericLowering {
    /// Selects the scalar family from the operation's resolved carrier.
    ///
    /// WHAT: `Dec` and binary float own a scalar renderer; an integer carrier cannot be total
    ///       here because every integer arithmetic failure set is non-empty.
    /// WHY: `JsNumericOperationLowering::classify` has already gated this path on the shared empty
    ///      failure set, so repeating the per-operator Dec/float sets here would be a second
    ///      fallibility authority. The renderer keeps its `compiler_bug!` for a total operator it
    ///      cannot spell, which the current classifier never selects.
    fn classify(
        op: HirNumericOp,
        carrier: JsNumericCarrier,
    ) -> Result<Self, InfrastructureFailure> {
        match carrier {
            JsNumericCarrier::ScaledInteger { scale } => Ok(Self::Decimal {
                operator: op.operator,
                scale,
            }),
            JsNumericCarrier::BinaryFloat { precision } => Ok(Self::BinaryFloat {
                operator: op.operator,
                precision,
            }),
            JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. } => {
                Err(total_lowering_error(op))
            }
        }
    }

    /// Renders the scalar expression over already-lowered operands.
    fn expression(self, operands: &[String]) -> String {
        match self {
            Self::Decimal { operator, scale } => match (operator, operands) {
                (NumericOperator::Add, [left, right]) => format!("({left} + {right})"),
                (NumericOperator::Subtract, [left, right]) => format!("({left} - {right})"),
                (NumericOperator::Negate, [operand]) => format!("(-({operand}))"),
                (NumericOperator::Multiply, [left, right]) if scale.get() == 0 => {
                    format!("({left} * {right})")
                }
                (NumericOperator::Multiply, [left, right]) => format!(
                    "__moth_number_round_half_even(({left} * {right}), {})",
                    number_scale_factor_js(scale.get())
                ),
                _ => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS scalar Dec operation {operator:?} has unexpected operand shape"
                ),
            },
            Self::BinaryFloat {
                operator,
                precision,
            } => match (operator, operands) {
                (NumericOperator::Negate, [operand]) => {
                    binary_float_negate_expression(precision, operand)
                }
                _ => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS scalar binary-float operation {operator:?} has unexpected operand shape"
                ),
            },
        }
    }

    /// Records the runtime helpers this scalar expression needs.
    fn require_helpers(self, usage: &mut NumericRuntimeHelperUsage) {
        if matches!(
            self,
            Self::Decimal {
                operator: NumericOperator::Multiply,
                scale,
            } if scale.get() > 0
        ) {
            usage.number_round_half_even = true;
        }
    }
}

/// The native-carrier expression for one proof-discharged integer operation.
///
/// WHAT: exact Number or BigInt arithmetic with the same `-0` normalisation the checked helpers
///       apply at their success boundary.
/// WHY: the proof table says the checked range predicate cannot fail, so only the helper envelope
///      and predicate are removed; values, signedness and BigInt range stay byte-identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JsProvenIntegerLowering {
    carrier: JsNumericCarrier,
    operator: NumericOperator,
}

impl JsProvenIntegerLowering {
    /// Selects the proof-backed expression only for the operator/carrier pairs it can render.
    ///
    /// Returns `None` for `Divide`, `Power`, binary floats and Dec, which the proof table never
    /// proves. The proof lookup runs only after the operator/carrier gate passes, so fast and
    /// non-integer operations never touch the sparse proof table. The shared classifier then
    /// chooses the total scalar lowering or the checked helper from the frontend failure
    /// classification.
    fn classify(
        op: HirNumericOp,
        carrier: JsNumericCarrier,
        proven_integer: impl FnOnce() -> bool,
    ) -> Option<Self> {
        let integer_carrier = matches!(
            carrier,
            JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. }
        );
        let supported_operator = matches!(
            op.operator,
            NumericOperator::Add
                | NumericOperator::Subtract
                | NumericOperator::Multiply
                | NumericOperator::IntegerDivide
                | NumericOperator::Remainder
                | NumericOperator::Negate
        );
        (integer_carrier && supported_operator && proven_integer()).then_some(Self {
            carrier,
            operator: op.operator,
        })
    }

    /// Renders the proven operation over already-lowered operands.
    fn expression(self, operands: &[String]) -> String {
        match (self.carrier, self.operator, operands) {
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Add, [left, right]) => {
                format!("({left} + {right})")
            }
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Subtract, [left, right]) => {
                format!("({left} - {right})")
            }
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Multiply, [left, right]) => {
                format!("({left} * {right} + 0)")
            }
            (
                JsNumericCarrier::ExactInteger { .. },
                NumericOperator::IntegerDivide,
                [left, right],
            ) => format!("(Math.trunc({left} / {right}) + 0)"),
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Remainder, [left, right]) => {
                format!("({left} % {right} + 0)")
            }
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Negate, [left]) => {
                format!("(-({left}) + 0)")
            }
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Add, [left, right]) => {
                format!("({left} + {right})")
            }
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Subtract, [left, right]) => {
                format!("({left} - {right})")
            }
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Multiply, [left, right]) => {
                format!("({left} * {right})")
            }
            (
                JsNumericCarrier::BigInteger { .. },
                NumericOperator::IntegerDivide,
                [left, right],
            ) => format!("({left} / {right})"),
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Remainder, [left, right]) => {
                format!("({left} % {right})")
            }
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Negate, [left]) => {
                format!("(-({left}))")
            }
            _ => crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS proven integer operation {:?} has no renderable carrier/operator/operand shape",
                self.operator
            ),
        }
    }
}

/// The checked helper call retained by an ordinary operation with a recoverable failure edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JsCheckedNumericLowering {
    carrier: JsNumericCarrier,
    operator: NumericOperator,
}

impl JsCheckedNumericLowering {
    /// Renders the helper call, including the compile-time scale factor and semantic bounds.
    fn expression(self, mut arguments: Vec<String>) -> String {
        if let JsNumericCarrier::ScaledInteger { scale } = self.carrier
            && matches!(
                self.operator,
                NumericOperator::Divide | NumericOperator::Power
            )
        {
            arguments.push(number_scale_factor_js(scale.get()));
        }
        if let Some((minimum, maximum)) = self.carrier.integer_bounds_js() {
            arguments.push(minimum);
            arguments.push(maximum);
        }

        format!(
            "__moth_{}_{}({})",
            self.carrier.helper_family(),
            self.helper_operation(),
            arguments.join(", ")
        )
    }

    /// Records the checked family and its optional members.
    fn require_helpers(self, usage: &mut NumericRuntimeHelperUsage) {
        match self.carrier {
            JsNumericCarrier::ExactInteger { .. } => usage.number_integer_ops = true,
            JsNumericCarrier::ScaledInteger { .. } => usage.number_decimal_ops = true,
            JsNumericCarrier::BigInteger { .. } => usage.big_integer_ops = true,
            JsNumericCarrier::BinaryFloat { precision } => {
                let demands_power = self.operator == NumericOperator::Power;
                match precision {
                    BinaryFloatPrecision::Binary16 => {
                        usage.binary16_ops = true;
                        if demands_power {
                            usage.binary16_power = true;
                        }
                    }
                    BinaryFloatPrecision::Binary32 => {
                        usage.binary32_ops = true;
                        if demands_power {
                            usage.binary32_power = true;
                        }
                    }
                    BinaryFloatPrecision::Binary64 => {
                        usage.binary64_ops = true;
                        if demands_power {
                            usage.binary64_power = true;
                        }
                    }
                }
            }
        }
    }

    /// The operation suffix of the checked helper name.
    ///
    /// Total ordinary operations never reach this path; their absence here keeps the emitted
    /// prelude free of helper definitions nothing can call.
    fn helper_operation(self) -> &'static str {
        match (self.carrier, self.operator) {
            (
                JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
                NumericOperator::Add,
            ) => "add",
            (
                JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
                NumericOperator::Subtract,
            ) => "sub",
            (
                JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
                NumericOperator::Multiply,
            ) => "mul",
            (
                JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
                NumericOperator::IntegerDivide,
            ) => "div",
            (
                JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
                NumericOperator::Remainder,
            ) => "mod",
            (
                JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
                NumericOperator::Power,
            ) => "pow",
            (
                JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
                NumericOperator::Negate,
            ) => "neg",
            (JsNumericCarrier::ScaledInteger { scale }, NumericOperator::Divide)
                if scale.get() > 0 =>
            {
                "div"
            }
            (JsNumericCarrier::ScaledInteger { scale }, NumericOperator::IntegerDivide)
                if scale.get() == 0 =>
            {
                "idiv"
            }
            (JsNumericCarrier::ScaledInteger { .. }, NumericOperator::Remainder) => "mod",
            (JsNumericCarrier::ScaledInteger { .. }, NumericOperator::Power) => "pow",
            (JsNumericCarrier::BinaryFloat { .. }, operator) => {
                let Some(operation) = binary_float_helper_operation(operator) else {
                    crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JS checked binary-float operation {operator:?} has no helper"
                    )
                };
                operation
            }
            _ => crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS ordinary operation {:?} reached the checked helper path despite total semantics",
                self.operator
            ),
        }
    }
}

fn total_lowering_error(op: HirNumericOp) -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::BackendLowering,
        format!(
            "JS backend classified {op} as total but has no scalar lowering for domain {:?}",
            op.domain
        ),
    )
}

fn require_fast_float_helpers(
    usage: &mut NumericRuntimeHelperUsage,
    precision: BinaryFloatPrecision,
    operator: NumericOperator,
) {
    let demands_power = operator == NumericOperator::Power;
    match precision {
        BinaryFloatPrecision::Binary16 => {
            usage.fast_binary16_ops = true;
            if demands_power {
                usage.fast_binary16_power = true;
            }
        }
        BinaryFloatPrecision::Binary32 => {
            usage.fast_binary32_ops = true;
            if demands_power {
                usage.fast_binary32_power = true;
            }
        }
        BinaryFloatPrecision::Binary64 => {
            usage.fast_binary64_ops = true;
            if demands_power {
                usage.fast_binary64_power = true;
            }
        }
    }
}

/// Helper family stem for checked binary-float helpers.
///
/// WHAT: the `__moth_<stem>_<operation>` prefix for one precision's recoverable-carrier family.
/// WHY: the lowering that calls a helper and the runtime emitter that defines it must state the
///      same name; one function owns all three checked stems.
pub(crate) fn checked_float_helper_family(precision: BinaryFloatPrecision) -> &'static str {
    match precision {
        BinaryFloatPrecision::Binary16 => "float16",
        BinaryFloatPrecision::Binary32 => "float32",
        BinaryFloatPrecision::Binary64 => "float",
    }
}

/// Helper family stem for fast binary-float helpers.
///
/// WHAT: the `__moth_<stem>_<operation>` prefix for one precision's fatal-delivery family.
/// WHY: fast lowering and the runtime emitter must agree on the same three stems, and fast helpers
///      never share a name with the checked family.
pub(crate) fn fast_float_helper_family(precision: BinaryFloatPrecision) -> &'static str {
    match precision {
        BinaryFloatPrecision::Binary16 => "fast_float16",
        BinaryFloatPrecision::Binary32 => "fast_float32",
        BinaryFloatPrecision::Binary64 => "fast_float",
    }
}

fn fast_lowering_error(message: String) -> InfrastructureFailure {
    InfrastructureFailure::internal(CompilerStage::BackendLowering, message)
}
