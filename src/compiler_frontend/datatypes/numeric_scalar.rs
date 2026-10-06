//! Canonical numeric scalar vocabulary.
//!
//! WHAT: the HIR-level numeric domains `Int`, `Uint`, `Float`, the explicit-width fixed
//!       scalars (`I8`..`F64`) and the exact decimal `Dec` domain, each derived from a
//!       canonical frontend `TypeId`. `Uint` follows the profile-selected `Int` width.
//! WHY: HIR numeric operations record a backend-neutral domain plus an operator instead of
//!      duplicating one variant per width, so fixed-width domains share operator typing
//!      without multiplying operation variants. Lowering, validation and backends share this one
//!      derivation rather than re-matching `TypeId`s at each use.
//!
//! Exclusions: `Byte` is not numeric and never maps to a domain. The exact decimal `Dec`
//! domain has no bounded integer range or binary-float precision and is not classified by those
//! consumers. Exact Dec arithmetic and conversions use its coefficient and scale instead. The
//! ranges, precisions and single-rounding steps here are format facts; promotion, conversion
//! classification and storage-layout policy consume them elsewhere.

use std::fmt::{Display, Formatter, Result as FmtResult};

use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::NumericProfile;

/// One canonical numeric scalar domain.
///
/// Declared `pub` inside the crate-private `datatypes::numeric_scalar` module so public HIR types
/// such as `HirNumericOp` can carry it; the effective visibility stays crate-internal.
///
/// Invariant: `Fixed` never holds `FixedScalar::Byte` (`Byte` is an octet value outside numeric
/// arithmetic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumericScalar {
    Int,
    /// Profile-sized unsigned integer, following the selected `Int` width.
    Uint,
    Float,
    Fixed(FixedScalar),
    /// One exact decimal `Dec` scale.
    ///
    /// WHAT: the lazy `Dec`/`Dec0`..`Dec256` identities carry their exact scale through
    ///       literal receiving, arithmetic, comparisons and conversions. Dec ranges
    ///       are outside the language contract.
    /// WHY: deriving the domain from `TypeId` gives every consumer one canonical scale fact
    ///      without a bounded integer or binary-float approximation.
    Number(NumberScale),
}

impl Display for NumericScalar {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Int => write!(formatter, "Int"),
            Self::Uint => write!(formatter, "Uint"),
            Self::Float => write!(formatter, "Float"),
            Self::Fixed(scalar) => write!(formatter, "{}", scalar.name()),
            Self::Number(scale) => write!(formatter, "{scale}"),
        }
    }
}

impl NumericScalar {
    /// Derives the numeric domain for a canonical type, if it is one.
    ///
    /// WHAT: maps `Int`/`Uint`/`Float` builtins, direct fixed scalars (never options) and the
    ///       lazy `Dec` scale identities, rejecting `Byte` and everything else.
    /// WHY: loop lowering and later promotion policy need one canonical derivation from `TypeId`
    ///      instead of re-matching builtins at each call site.
    pub(crate) fn from_type_id(type_id: TypeId, environment: &TypeEnvironment) -> Option<Self> {
        let builtins = environment.builtins();

        if type_id == builtins.int {
            return Some(NumericScalar::Int);
        }

        if type_id == builtins.uint {
            return Some(NumericScalar::Uint);
        }

        if type_id == builtins.float {
            return Some(NumericScalar::Float);
        }

        if let Some(scale) = environment.number_scale(type_id) {
            return Some(NumericScalar::Number(scale));
        }

        let scalar = environment.fixed_scalar(type_id)?;

        if scalar == FixedScalar::Byte {
            return None;
        }

        Some(NumericScalar::Fixed(scalar))
    }

    /// Returns the canonical `TypeId` for this domain.
    ///
    /// Dec domains resolve through the environment's canonical-identity lookup: the scale type
    /// was already interned by the consumer that derived this domain, so the reverse lookup must
    /// always succeed there. There is no synthetic fallback identity.
    pub(crate) fn type_id(self, environment: &TypeEnvironment) -> TypeId {
        match self {
            NumericScalar::Int => environment.builtins().int,
            NumericScalar::Uint => environment.builtins().uint,
            NumericScalar::Float => environment.builtins().float,
            NumericScalar::Fixed(scalar) => builtin_type_ids::fixed_scalar(scalar),
            NumericScalar::Number(scale) => environment
                .type_id_for_canonical_identity(&CanonicalTypeIdentity::Builtin(
                    CanonicalBuiltinType::Number(scale),
                ))
                .expect("Number scale identity must be interned in the paired environment"),
        }
    }

    /// Whether this domain uses integer arithmetic.
    ///
    /// `Dec` is exact decimal, not a bounded integer, so it stays outside this classifier.
    pub(crate) fn is_integer(self) -> bool {
        match self {
            NumericScalar::Int | NumericScalar::Uint => true,
            NumericScalar::Float | NumericScalar::Number(_) => false,
            NumericScalar::Fixed(scalar) => matches!(
                scalar.class(),
                FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger
            ),
        }
    }

    /// Whether this domain uses binary-float arithmetic.
    ///
    /// `Dec` is exact decimal, never a binary float.
    pub(crate) fn is_binary_float(self) -> bool {
        match self {
            NumericScalar::Int | NumericScalar::Uint | NumericScalar::Number(_) => false,
            NumericScalar::Float => true,
            NumericScalar::Fixed(scalar) => {
                matches!(scalar.class(), FixedScalarClass::BinaryFloat)
            }
        }
    }

    /// Inclusive value range of an integer domain; `None` for binary floats.
    ///
    /// WHAT: `Int` follows the profile width and fixed integers their declared ranges, widened to
    ///       `i128` so `U64` values above `i64::MAX` compare exactly.
    /// WHY: conversion evidence, conversion policies and operator promotion all compare complete
    ///      ranges, so they read one range fact instead of re-deriving width bounds.
    pub(crate) fn integer_range(self, profile: NumericProfile) -> Option<(i128, i128)> {
        match self {
            NumericScalar::Int => Some((
                i128::from(profile.int_width.min_value()),
                i128::from(profile.int_width.max_value()),
            )),
            NumericScalar::Uint => Some((0, i128::from(profile.int_width.unsigned_max_value()))),
            // `Dec` is unbounded exact decimal; there is no bounded range fact to report, so
            // every range consumer rejects it instead of deriving an accidental Dec range.
            NumericScalar::Float | NumericScalar::Number(_) => None,
            NumericScalar::Fixed(scalar) => match scalar.class() {
                FixedScalarClass::SignedInteger => {
                    let (min, max) = scalar.signed_range()?;
                    Some((i128::from(min), i128::from(max)))
                }
                FixedScalarClass::UnsignedInteger => Some((0, i128::from(scalar.unsigned_max()?))),
                FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => None,
            },
        }
    }

    /// Binary-float precision of a float domain; `None` for integers.
    ///
    /// WHY: `Float` shares its precision with `F32` or `F64` under the selected profile, so
    ///      widening, narrowing and rounding decisions compare precisions rather than spellings.
    pub(crate) fn binary_float_precision(
        self,
        profile: NumericProfile,
    ) -> Option<BinaryFloatPrecision> {
        match self {
            NumericScalar::Int | NumericScalar::Uint | NumericScalar::Number(_) => None,
            NumericScalar::Float => Some(profile.float_precision.into()),
            NumericScalar::Fixed(scalar) => scalar.binary_float_precision(),
        }
    }
}
