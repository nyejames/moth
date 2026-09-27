//! JavaScript numeric representation policy.
//!
//! WHAT: maps each semantic numeric domain and compilation profile to the exact JS carrier used by
//!       literals, checked operators, casts and comparisons.
//! WHY: representation decisions must share one boundary owner so `Int64` never slips through a
//!      Number-only path and future fixed-width lowering can reuse the same carrier families.
//!
//! Exclusions: this only describes scalar representation. Fixed-width runtime values remain gated
//! before JS lowering in this slice.

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
        let (min, max) = carrier.integer_bounds()?;
        let value = i128::from(value);

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

#[cfg(test)]
mod tests {
    use super::JsNumericCarrier;
    use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
    use crate::compiler_frontend::datatypes::numeric_profile::{
        FloatPrecision, IntWidth, NumericProfile,
    };
    use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;

    #[test]
    fn carrier_selection_follows_profile_and_complete_scalar_range() {
        let profile = NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        };

        assert_eq!(
            JsNumericCarrier::for_scalar(NumericScalar::Int, profile),
            Some(JsNumericCarrier::BigInteger {
                min: i64::MIN as i128,
                max: i64::MAX as i128,
            })
        );
        assert_eq!(
            JsNumericCarrier::for_scalar(NumericScalar::Float, profile),
            Some(JsNumericCarrier::BinaryFloat {
                precision: crate::compiler_frontend::datatypes::numeric_scalar::BinaryFloatPrecision::Binary32,
            })
        );
        assert!(matches!(
            JsNumericCarrier::for_scalar(
                NumericScalar::Fixed(FixedScalar::U32),
                NumericProfile::STANDARD
            ),
            Some(JsNumericCarrier::ExactInteger {
                min: 0,
                max: 4_294_967_295
            })
        ));
        assert!(matches!(
            JsNumericCarrier::for_scalar(
                NumericScalar::Fixed(FixedScalar::U64),
                NumericProfile::STANDARD
            ),
            Some(JsNumericCarrier::BigInteger {
                min: 0,
                max: 18_446_744_073_709_551_615
            })
        ));
    }
}
