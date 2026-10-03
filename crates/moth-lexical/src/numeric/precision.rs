//! Shared binary-float precision and rounding facts.
//!
//! WHAT: owns the IEEE binary16/32/64 precision order, conversion from boundary `Float` precision,
//!       single-rounding helpers, and the binary16 integer-overflow midpoint.
//! WHY: parsing, formatting and compiler consumers must use one destination-rounding contract.
//!
//! Arithmetic policy and numeric type classification remain in the compiler.

use super::binary16::round_f64_to_f16;
use super::profile::FloatPrecision;

/// IEEE binary-float precision, ordered narrowest to widest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BinaryFloatPrecision {
    Binary16,
    Binary32,
    Binary64,
}

impl From<FloatPrecision> for BinaryFloatPrecision {
    /// Maps the profile-selected `Float` precision onto its binary-float precision.
    fn from(precision: FloatPrecision) -> Self {
        match precision {
            FloatPrecision::Bits32 => BinaryFloatPrecision::Binary32,
            FloatPrecision::Bits64 => BinaryFloatPrecision::Binary64,
        }
    }
}

impl BinaryFloatPrecision {
    /// Rounds a finite `f64` once to this precision, ties-to-even.
    ///
    /// A magnitude beyond the precision's finite range becomes a signed infinity for the caller to
    /// reject. Every binary-float value is exact in `f64`, so rounding directly from this carrier is
    /// the single rounding step (`F64 -> F16` never passes through `F32`).
    pub fn round(self, value: f64) -> f64 {
        match self {
            BinaryFloatPrecision::Binary16 => round_f64_to_f16(value),
            BinaryFloatPrecision::Binary32 => f64::from(value as f32),
            BinaryFloatPrecision::Binary64 => value,
        }
    }

    /// Rounds an integer once to this precision, ties-to-even.
    ///
    /// `Binary32`/`Binary64` convert directly from the integer. `Binary16` rounds the exact `f64`
    /// form for magnitudes below its overflow threshold and returns a signed infinity otherwise.
    pub fn round_integer(self, value: i128) -> f64 {
        match self {
            BinaryFloatPrecision::Binary16 => {
                // Every magnitude below the threshold is exact in `f64`, so the only rounding is
                // the binary16 one. Anything at or above it rounds past the largest finite value.
                if value.unsigned_abs() < BINARY16_OVERFLOW_THRESHOLD {
                    round_f64_to_f16(value as f64)
                } else {
                    f64::INFINITY.copysign(value as f64)
                }
            }
            BinaryFloatPrecision::Binary32 => f64::from(value as f32),
            BinaryFloatPrecision::Binary64 => value as f64,
        }
    }
}

/// Smallest integer magnitude that rounds to infinity in binary16 (the midpoint above `65504`).
const BINARY16_OVERFLOW_THRESHOLD: u128 = 65_520;
