//! Compact type identifiers and canonical keys.
//!
//! WHAT: defines all dense ID newtypes and the stable keys used for interning.
//! WHY: `TypeId` equality is the canonical semantic type equality check.
//!      Deterministic lookup comes from stable keys, not from numeric IDs.

// -----------------------------------------------------------
//  Compact Type Identifiers
// -----------------------------------------------------------

use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// Dense module-local type identifier.
///
/// Valid only with the `TypeEnvironment` that created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub u32);

/// Builtin `TypeId` constants matching the seeding order of `TypeEnvironment::new()`.
///
/// WHAT: deterministic IDs for builtin types so that factory methods and tests
/// can construct expressions without threading a full `TypeEnvironment`.
///
/// WHY: `Expression::int()` and similar factories should not need `&TypeEnvironment`
/// just to refer to `Int`. These constants are valid for any `TypeEnvironment`
/// created via `TypeEnvironment::new()` or `Default::default()`.
///
/// WARNING: if `TypeEnvironment::new()` changes its builtin seeding order,
/// these constants must be updated to match.
pub mod builtin_type_ids {
    use super::{FixedScalar, TypeId};

    pub const BOOL: TypeId = TypeId(0);
    pub const INT: TypeId = TypeId(1);
    pub const FLOAT: TypeId = TypeId(2);
    pub const STRING: TypeId = TypeId(3);
    pub const CHAR: TypeId = TypeId(4);
    pub const RANGE: TypeId = TypeId(5);
    pub const NONE: TypeId = TypeId(6);

    /// First seeded fixed-scalar `TypeId`; fixed scalars occupy the ids after `None`.
    ///
    /// `Dec` types are NOT seeded here: their 257 scale identities intern lazily through
    /// `TypeEnvironment::intern_number`, so they take the ids after every fixed scalar.
    const FIRST_FIXED_SCALAR: u32 = 7;

    /// Deterministic `TypeId` for one fixed-width builtin scalar.
    ///
    /// WHAT: mirrors the `FixedScalar::ALL` seeding sequence `TypeEnvironment::new` inserts
    ///       immediately after `None`.
    /// WHY: builtin identities are stable facts shared by parser, bridge and projection code, so
    ///      they come from this one mapping rather than a per-call-site arithmetic offset.
    pub const fn fixed_scalar(scalar: FixedScalar) -> TypeId {
        TypeId(FIRST_FIXED_SCALAR + scalar as u32)
    }
}

/// Dense identifier for a nominal struct or choice definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NominalTypeId(pub u32);

/// Dense identifier for a single generic parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GenericParameterId(pub u32);

/// Dense identifier for a list of generic parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GenericParameterListId(pub u32);

// -----------------------------------------------------------
//  Canonical Keys
// -----------------------------------------------------------

/// Keys for builtin scalar types.
///
/// These are seeded once when `TypeEnvironment` is created. `FixedScalar` identities carry their
/// own width, signedness or octet meaning, so `Int` and `Float` never alias a fixed width that
/// happens to match the active `NumericProfile`. `Dec` identities carry their scale and intern
/// lazily instead of being seeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinTypeKey {
    Bool,
    Int,
    Float,
    String,
    Char,
    Range,
    None,
    /// Arbitrary-precision decimal identity at one canonical scale.
    ///
    /// `Dec` means scale zero; `Dec0` shares that identity. The scale is part of the
    /// canonical identity, so `Dec1`..`Dec256` stay distinct from each other and from
    /// scale zero. No per-scale seeded id exists: the key interns through
    /// `TypeEnvironment::intern_number` on first use.
    Number(NumberScale),
    FixedScalar(FixedScalar),
}

/// A type constructor paired with its arguments.
///
/// Used for collections, options, results, and nominal generic instances.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeConstructor {
    Builtin(BuiltinTypeConstructor),
}

/// Builtin type constructors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinTypeConstructor {
    Collection { fixed_capacity: Option<usize> },
    OrderedMap,
    Option,
    FallibleCarrier,
    Tuple,
}

/// Key for a constructed type (collection, option, result, or nominal instance).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConstructedTypeKey {
    pub constructor: TypeConstructor,
    pub arguments: Box<[TypeId]>,
}

/// Key for a generic nominal instance.
///
/// WHAT: base nominal + concrete argument IDs.
/// WHY: canonicalises `Box of Int` so repeated uses share one `TypeId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericInstanceKey {
    pub base: NominalTypeId,
    pub arguments: Box<[TypeId]>,
}

/// Key for a function type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionTypeKey {
    pub parameters: Box<[TypeId]>,
    pub returns: Box<[TypeId]>,
    pub error_return: Option<TypeId>,
}
