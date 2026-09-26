//! Canonical numeric scalar vocabulary.
//!
//! WHAT: the HIR-level numeric domains `Int`, `Float` and the explicit-width fixed scalars
//!       (`I8`..`F64`), each derived from a canonical frontend `TypeId`.
//! WHY: HIR numeric operations record a backend-neutral domain plus an operator instead of
//!      duplicating one variant per width, so later fixed-width operator typing can add domains
//!      without multiplying operation variants. Lowering, validation and backends share this one
//!      derivation rather than re-matching `TypeId`s at each use.
//!
//! Exclusions: `Byte` is not numeric and never maps to a domain. `Number`/`NumberN` and the
//! inactive `Decimal` scaffold are not part of this vocabulary. No arithmetic, promotion,
//! conversion or storage-layout policy lives here; those consume these identities elsewhere.

use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::{FixedScalar, FixedScalarClass};
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};

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
    Float,
    Fixed(FixedScalar),
}

impl NumericScalar {
    /// Derives the numeric domain for a canonical type, if it is one.
    ///
    /// WHAT: maps `Int`/`Float` builtins directly and fixed scalars through the existing
    ///       `fixed_scalar_of` derivation, rejecting `Byte` and everything else.
    /// WHY: loop lowering and later promotion policy need one canonical derivation from `TypeId`
    ///      instead of re-matching builtins at each call site.
    pub(crate) fn from_type_id(type_id: TypeId, environment: &TypeEnvironment) -> Option<Self> {
        let builtins = environment.builtins();

        if type_id == builtins.int {
            return Some(NumericScalar::Int);
        }

        if type_id == builtins.float {
            return Some(NumericScalar::Float);
        }

        let scalar = environment.fixed_scalar_of(type_id)?;

        if scalar == FixedScalar::Byte {
            return None;
        }

        Some(NumericScalar::Fixed(scalar))
    }

    /// Returns the canonical `TypeId` for this domain.
    pub(crate) fn type_id(self, environment: &TypeEnvironment) -> TypeId {
        match self {
            NumericScalar::Int => environment.builtins().int,
            NumericScalar::Float => environment.builtins().float,
            NumericScalar::Fixed(scalar) => builtin_type_ids::fixed_scalar(scalar),
        }
    }

    /// Whether this domain uses integer arithmetic.
    pub(crate) fn is_integer(self) -> bool {
        match self {
            NumericScalar::Int => true,
            NumericScalar::Float => false,
            NumericScalar::Fixed(scalar) => matches!(
                scalar.class(),
                FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger
            ),
        }
    }

    /// Whether this domain uses binary-float arithmetic.
    pub(crate) fn is_binary_float(self) -> bool {
        match self {
            NumericScalar::Int => false,
            NumericScalar::Float => true,
            NumericScalar::Fixed(scalar) => {
                matches!(scalar.class(), FixedScalarClass::BinaryFloat)
            }
        }
    }

    /// The canonical domain name used in HIR display (`Int`, `Float`, or the fixed spelling).
    pub(crate) fn name(self) -> &'static str {
        match self {
            NumericScalar::Int => "Int",
            NumericScalar::Float => "Float",
            NumericScalar::Fixed(scalar) => scalar.name(),
        }
    }
}
