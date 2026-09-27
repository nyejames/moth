//! Explicit-width builtin scalar identities.
//!
//! WHAT: the fixed-width numeric spellings `I8`..`F64` and the octet type `Byte`, each a
//!       distinct builtin identity that carries its own width, signedness or octet meaning.
//! WHY:  these types are declared and compared directly, so width and octet meaning must be
//!       semantic identity instead of something later stages re-derive from a rendered name.
//!
//! Exclusions: `Int` and `Float` remain separate profile-dependent identities and never alias a
//! fixed width that happens to match, even under a matching `NumericProfile`. `Number`/`NumberN`
//! and the inactive `Decimal` scaffold are not part of this vocabulary. No arithmetic, range,
//! conversion or storage-layout policy lives here; those consume these identities elsewhere.
//! [`FixedScalarValue`] is the one materialised-value carrier for these types; literal
//! materialisation and rounding policy live in `numeric_text`, not here.

use crate::compiler_frontend::numeric_text::binary16::round_f64_to_f16;

/// One fixed-width builtin scalar or the `Byte` octet type.
///
/// Discriminants follow `ALL` order so `builtin_type_ids::fixed_scalar` can derive the seeded
/// `TypeId` layout from this enum without a second ordering table.
///
/// Declared `pub` inside the crate-private `datatypes::fixed_scalar` module so public diagnostic
/// payloads such as `NumberLiteralErrorReason` can carry it; the effective visibility stays
/// crate-internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FixedScalar {
    I8 = 0,
    I16 = 1,
    I32 = 2,
    I64 = 3,
    U8 = 4,
    U16 = 5,
    U32 = 6,
    U64 = 7,
    F16 = 8,
    F32 = 9,
    F64 = 10,
    Byte = 11,
}

impl FixedScalar {
    /// Every fixed scalar in seeded `TypeId` order.
    ///
    /// `TypeEnvironment::new` seeds this sequence and `builtin_type_ids::fixed_scalar` mirrors it,
    /// so the order is a stable identity fact rather than a local convenience.
    pub(crate) const ALL: [FixedScalar; 12] = [
        FixedScalar::I8,
        FixedScalar::I16,
        FixedScalar::I32,
        FixedScalar::I64,
        FixedScalar::U8,
        FixedScalar::U16,
        FixedScalar::U32,
        FixedScalar::U64,
        FixedScalar::F16,
        FixedScalar::F32,
        FixedScalar::F64,
        FixedScalar::Byte,
    ];

    /// The exact source spelling of this scalar, which is also its keyword token spelling.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            FixedScalar::I8 => "I8",
            FixedScalar::I16 => "I16",
            FixedScalar::I32 => "I32",
            FixedScalar::I64 => "I64",
            FixedScalar::U8 => "U8",
            FixedScalar::U16 => "U16",
            FixedScalar::U32 => "U32",
            FixedScalar::U64 => "U64",
            FixedScalar::F16 => "F16",
            FixedScalar::F32 => "F32",
            FixedScalar::F64 => "F64",
            FixedScalar::Byte => "Byte",
        }
    }

    /// Resolves an exact source spelling back to its fixed scalar identity.
    ///
    /// `Int` and `Float` deliberately return `None`: they are profile-dependent identities with no
    /// fixed width, so a matching `NumericProfile` width never makes them alias an `I*`/`U*`/`F*`
    /// spelling.
    pub(crate) fn from_name(name: &str) -> Option<FixedScalar> {
        FixedScalar::ALL
            .into_iter()
            .find(|scalar| scalar.name() == name)
    }

    /// The value family this scalar belongs to.
    pub(crate) const fn class(self) -> FixedScalarClass {
        match self {
            FixedScalar::I8 | FixedScalar::I16 | FixedScalar::I32 | FixedScalar::I64 => {
                FixedScalarClass::SignedInteger
            }
            FixedScalar::U8 | FixedScalar::U16 | FixedScalar::U32 | FixedScalar::U64 => {
                FixedScalarClass::UnsignedInteger
            }
            FixedScalar::F16 | FixedScalar::F32 | FixedScalar::F64 => FixedScalarClass::BinaryFloat,
            FixedScalar::Byte => FixedScalarClass::Octet,
        }
    }

    /// Storage width in bits. `Byte` is one octet.
    pub(crate) const fn bit_width(self) -> u32 {
        match self {
            FixedScalar::I8 | FixedScalar::U8 | FixedScalar::Byte => 8,
            FixedScalar::I16 | FixedScalar::U16 | FixedScalar::F16 => 16,
            FixedScalar::I32 | FixedScalar::U32 | FixedScalar::F32 => 32,
            FixedScalar::I64 | FixedScalar::U64 | FixedScalar::F64 => 64,
        }
    }

    /// Inclusive `(min, max)` for signed integers; `None` for every other class.
    pub(crate) const fn signed_range(self) -> Option<(i64, i64)> {
        match self.class() {
            FixedScalarClass::SignedInteger => {
                let shift = self.bit_width() - 1;
                Some((i64::MIN >> (63 - shift), i64::MAX >> (63 - shift)))
            }
            _ => None,
        }
    }

    /// Inclusive maximum for unsigned integers and `Byte`; `None` for every other class.
    pub(crate) const fn unsigned_max(self) -> Option<u64> {
        match self.class() {
            FixedScalarClass::UnsignedInteger | FixedScalarClass::Octet => {
                Some(u64::MAX >> (64 - self.bit_width()))
            }
            _ => None,
        }
    }
}

/// Value family of a [`FixedScalar`]. `Octet` is `Byte`: whole-literal and unsigned-ordered, but
/// outside numeric arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FixedScalarClass {
    SignedInteger,
    UnsignedInteger,
    BinaryFloat,
    Octet,
}

/// One materialised value of a fixed scalar type.
///
/// WHAT: the scalar identity plus a 64-bit payload. Signed integers store the sign-extended
///       two's-complement `i64`, unsigned integers and `Byte` store the zero-extended value, and
///       binary floats store `f64::to_bits` of a finite value already rounded to the scalar's
///       precision (every `F16`/`F32` value is exact in `f64`).
/// WHY:  AST, const payloads, public folded values and HIR share one `Copy` carrier whose
///       equality and hash are exact-bit, so `U64` values above `i64::MAX` and signed zero
///       survive every stage without an `Int`/`Float` intermediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FixedScalarValue {
    scalar: FixedScalar,
    bits: u64,
}

impl FixedScalarValue {
    /// A signed-integer value, or `None` when `scalar` is not signed or `value` is out of range.
    pub(crate) fn signed(scalar: FixedScalar, value: i64) -> Option<Self> {
        let (min, max) = scalar.signed_range()?;
        (min..=max).contains(&value).then_some(Self {
            scalar,
            bits: value as u64,
        })
    }

    /// An unsigned-integer or `Byte` value, or `None` when the class or range does not match.
    pub(crate) fn unsigned(scalar: FixedScalar, value: u64) -> Option<Self> {
        (value <= scalar.unsigned_max()?).then_some(Self {
            scalar,
            bits: value,
        })
    }

    /// A binary-float value, or `None` when `scalar` is not a binary float or `value` is
    /// non-finite.
    ///
    /// The caller owns rounding: `value` must already be exactly representable at the scalar's
    /// precision (`numeric_text` performs the single ties-to-even rounding).
    pub(crate) fn binary_float(scalar: FixedScalar, value: f64) -> Option<Self> {
        if scalar.class() != FixedScalarClass::BinaryFloat || !value.is_finite() {
            return None;
        }
        debug_assert!(
            match scalar {
                FixedScalar::F16 => round_f64_to_f16(value).to_bits() == value.to_bits(),

                FixedScalar::F32 => f64::from(value as f32).to_bits() == value.to_bits(),

                FixedScalar::F64 => true,

                // The class check above already rejected every non-float scalar.
                _ => true,
            },
            "binary float values must be rounded to their destination precision before construction"
        );
        Some(Self {
            scalar,
            bits: value.to_bits(),
        })
    }

    pub(crate) const fn scalar(self) -> FixedScalar {
        self.scalar
    }

    /// The signed-integer value; `None` for other classes.
    pub(crate) fn as_i64(self) -> Option<i64> {
        (self.scalar.class() == FixedScalarClass::SignedInteger).then_some(self.bits as i64)
    }

    /// The unsigned-integer or `Byte` value; `None` for other classes.
    pub(crate) fn as_u64(self) -> Option<u64> {
        matches!(
            self.scalar.class(),
            FixedScalarClass::UnsignedInteger | FixedScalarClass::Octet
        )
        .then_some(self.bits)
    }

    /// The binary-float value; `None` for other classes.
    pub(crate) fn as_f64(self) -> Option<f64> {
        (self.scalar.class() == FixedScalarClass::BinaryFloat).then_some(f64::from_bits(self.bits))
    }
}

/// Scalar-qualified exact rendering for compiler dumps, such as `U64(18446744073709551615)` or
/// `F16(-0.0)`, so equal numbers of different scalars never render identically. Source-facing
/// formatting is a later phase and does not use this.
impl std::fmt::Display for FixedScalarValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.scalar.name();
        match self.scalar.class() {
            FixedScalarClass::SignedInteger => write!(formatter, "{name}({})", self.bits as i64),
            FixedScalarClass::UnsignedInteger | FixedScalarClass::Octet => {
                write!(formatter, "{name}({})", self.bits)
            }
            FixedScalarClass::BinaryFloat => {
                write!(formatter, "{name}({:?})", f64::from_bits(self.bits))
            }
        }
    }
}
