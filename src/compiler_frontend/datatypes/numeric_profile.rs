//! Compiler-owned numeric profile.
//!
//! WHAT: the exact `Int` width and `Float` precision one compilation boundary types
//!       numbers under, selected independently from 32 or 64 bits.
//! WHY:  literal materialisation, folding, diagnostics and ABI facts depend on these widths,
//!       so every compiler service that types numbers reads one shared boundary input instead
//!       of choosing its own representation.
//!
//! Exclusions: not a build profile, not a builder global, not a `#Config` contract, and no
//! source, directive or CLI spelling selects it. The selected builder settles it before
//! command-input materialisation and config compilation, and lowerers consume it.

/// Width of the boundary `Int` type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IntWidth {
    Bits32,
    Bits64,
}

impl IntWidth {
    /// Smallest `Int` value representable at this width.
    ///
    /// WHY: literal materialisation, folding and cast policies range-check one
    ///      shared boundary instead of hard-coding `i32` limits per call site.
    pub fn min_value(self) -> i64 {
        match self {
            IntWidth::Bits32 => i32::MIN as i64,
            IntWidth::Bits64 => i64::MIN,
        }
    }

    /// Largest `Int` value representable at this width.
    ///
    /// WHY: same single-owner range as `min_value`; callers derive every
    ///      overflow diagnostic from this pair.
    pub fn max_value(self) -> i64 {
        match self {
            IntWidth::Bits32 => i32::MAX as i64,
            IntWidth::Bits64 => i64::MAX,
        }
    }

    /// True when `value` fits in this width.
    ///
    /// WHY: folded and cast `Int` results are valid only inside the boundary
    ///      width, so policies ask the width rather than comparing ad hoc bounds.
    pub fn contains(self, value: i64) -> bool {
        value >= self.min_value() && value <= self.max_value()
    }
}

/// Precision of the boundary `Float` type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FloatPrecision {
    Bits32,
    Bits64,
}

impl FloatPrecision {
    /// Round an `f64` carrier to this precision.
    ///
    /// WHY: `Float` values stay `f64` carriers holding a value exactly
    ///      representable at the profile precision. Bits32 rounds once with
    ///      round-to-nearest-even via `f32`; Bits64 is the identity. Callers
    ///      check finiteness afterwards.
    pub fn round(self, value: f64) -> f64 {
        match self {
            FloatPrecision::Bits64 => value,
            FloatPrecision::Bits32 => f64::from(value as f32),
        }
    }

    /// Convert an `Int` value to this precision with a single rounding.
    ///
    /// WHY: `Int -> Float` conversion rounds directly at the destination
    ///      precision (never via an `f64` intermediate under Bits32) so
    ///      folding and casts agree on one rounding rule.
    pub fn round_int(self, value: i64) -> f64 {
        match self {
            FloatPrecision::Bits64 => value as f64,
            FloatPrecision::Bits32 => f64::from(value as f32),
        }
    }
}

/// Compiler-owned numeric widths for one compilation boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NumericProfile {
    pub int_width: IntWidth,
    pub float_precision: FloatPrecision,
}

impl NumericProfile {
    /// Default boundary widths: `Int32` with `Float64`.
    pub const STANDARD: Self = Self {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits64,
    };

    /// True when this profile is the default `Int32`/`Float64` combination.
    pub fn is_standard(self) -> bool {
        self == Self::STANDARD
    }

    /// Stable fingerprint byte distinguishing the four supported combinations.
    ///
    /// WHY: build-config fingerprints feed one byte only for `Int`/`Float` contracts, so the
    ///      encoding must stay stable across builds. The numeric value distinguishes every
    ///      combination without depending on debug formatting.
    pub fn fingerprint_byte(self) -> u8 {
        match (self.int_width, self.float_precision) {
            (IntWidth::Bits32, FloatPrecision::Bits64) => 0,
            (IntWidth::Bits32, FloatPrecision::Bits32) => 1,
            (IntWidth::Bits64, FloatPrecision::Bits64) => 2,
            (IntWidth::Bits64, FloatPrecision::Bits32) => 3,
        }
    }
}

impl Default for NumericProfile {
    fn default() -> Self {
        Self::STANDARD
    }
}

impl std::fmt::Display for NumericProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let int = match self.int_width {
            IntWidth::Bits32 => "Int32",
            IntWidth::Bits64 => "Int64",
        };
        let float = match self.float_precision {
            FloatPrecision::Bits32 => "Float32",
            FloatPrecision::Bits64 => "Float64",
        };

        write!(formatter, "{int}/{float}")
    }
}
