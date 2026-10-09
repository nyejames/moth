//! Core cast trait names and registration helpers.
//!
//! WHAT: defines the central `CoreCastTrait` enum, the canonical source
//!      names, and the per-trait metadata (requirement name, builtin target,
//!      fallibility) that downstream phases use while registering builtin
//!      cast traits. Also exposes single `builtin_cast_trait_*` lookups
//!      that return canonical source names from a `CoreCastTrait` variant.
//! WHY: every later phase that wires up trait registration needs the same
//!      stable name and metadata table. Centralising them here means
//!      registration code can refer to variants rather than re-typing the
//!      literal strings and accidentally drift on casing, requirement
//!      naming, target classification, or fallibility.

use super::targets::{BuiltinCastFallibility, BuiltinCastTarget};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// The set of compiler-owned core cast traits.
///
/// Each cast target has one infallible evidence trait and one fallible evidence trait. The
/// metadata rows below are authoritative for their source spellings, requirement names, targets,
/// and fallibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CoreCastTrait {
    CastableToI8,
    TryCastableToI8,
    CastableToI16,
    TryCastableToI16,
    CastableToI32,
    TryCastableToI32,
    CastableToI64,
    TryCastableToI64,
    CastableToU8,
    TryCastableToU8,
    CastableToU16,
    TryCastableToU16,
    CastableToU32,
    TryCastableToU32,
    CastableToU64,
    TryCastableToU64,
    CastableToF16,
    TryCastableToF16,
    CastableToF32,
    TryCastableToF32,
    CastableToF64,
    TryCastableToF64,
    CastableToBool,
    TryCastableToBool,
    CastableToString,
    TryCastableToString,
    CastableToChar,
    TryCastableToChar,
    CastableToError,
    TryCastableToError,
}

/// Complete static metadata for one compiler-owned core cast trait row.
///
/// WHAT: pairs a `CoreCastTrait` variant with its source-defined trait
///      name, requirement name, builtin target, and fallibility so the
///      registration code can build trait definitions and evidence rows
///      without per-trait special cases.
/// WHY: keeping the metadata in one table means a new core cast trait
///      only needs one row, not parallel updates across the registry and
///      the trait environment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CoreCastTraitMetadata {
    pub(crate) kind: CoreCastTrait,
    pub(crate) trait_name: &'static str,
    pub(crate) requirement_name: &'static str,
    pub(crate) target: BuiltinCastTarget,
    pub(crate) fallibility: BuiltinCastFallibility,
}

/// The complete list of compiler-owned core cast trait rows.
///
/// WHAT: every row maps a `CoreCastTrait` variant to its source-defined
///      trait name, requirement name, builtin target, and fallibility so
///      registration code can refer to the variant rather than the literal
///      strings at every call site.
/// WHY: the trait list is the only place that names the core cast traits
///      and records their per-trait metadata, and keeping it table-driven
///      means a single source of truth exists for the cast trait
///      catalogue.
pub(crate) const BUILTIN_CAST_TRAIT_ROWS: &[CoreCastTraitMetadata] = &[
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToI8,
        trait_name: "CASTABLE_TO_I8",
        requirement_name: "to_i8",
        target: BuiltinCastTarget::Fixed(FixedScalar::I8),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToI8,
        trait_name: "TRY_CASTABLE_TO_I8",
        requirement_name: "try_to_i8",
        target: BuiltinCastTarget::Fixed(FixedScalar::I8),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToI16,
        trait_name: "CASTABLE_TO_I16",
        requirement_name: "to_i16",
        target: BuiltinCastTarget::Fixed(FixedScalar::I16),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToI16,
        trait_name: "TRY_CASTABLE_TO_I16",
        requirement_name: "try_to_i16",
        target: BuiltinCastTarget::Fixed(FixedScalar::I16),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToI32,
        trait_name: "CASTABLE_TO_I32",
        requirement_name: "to_i32",
        target: BuiltinCastTarget::Fixed(FixedScalar::I32),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToI32,
        trait_name: "TRY_CASTABLE_TO_I32",
        requirement_name: "try_to_i32",
        target: BuiltinCastTarget::Fixed(FixedScalar::I32),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToI64,
        trait_name: "CASTABLE_TO_I64",
        requirement_name: "to_i64",
        target: BuiltinCastTarget::Fixed(FixedScalar::I64),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToI64,
        trait_name: "TRY_CASTABLE_TO_I64",
        requirement_name: "try_to_i64",
        target: BuiltinCastTarget::Fixed(FixedScalar::I64),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToU8,
        trait_name: "CASTABLE_TO_U8",
        requirement_name: "to_u8",
        target: BuiltinCastTarget::Fixed(FixedScalar::U8),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToU8,
        trait_name: "TRY_CASTABLE_TO_U8",
        requirement_name: "try_to_u8",
        target: BuiltinCastTarget::Fixed(FixedScalar::U8),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToU16,
        trait_name: "CASTABLE_TO_U16",
        requirement_name: "to_u16",
        target: BuiltinCastTarget::Fixed(FixedScalar::U16),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToU16,
        trait_name: "TRY_CASTABLE_TO_U16",
        requirement_name: "try_to_u16",
        target: BuiltinCastTarget::Fixed(FixedScalar::U16),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToU32,
        trait_name: "CASTABLE_TO_U32",
        requirement_name: "to_u32",
        target: BuiltinCastTarget::Fixed(FixedScalar::U32),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToU32,
        trait_name: "TRY_CASTABLE_TO_U32",
        requirement_name: "try_to_u32",
        target: BuiltinCastTarget::Fixed(FixedScalar::U32),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToU64,
        trait_name: "CASTABLE_TO_U64",
        requirement_name: "to_u64",
        target: BuiltinCastTarget::Fixed(FixedScalar::U64),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToU64,
        trait_name: "TRY_CASTABLE_TO_U64",
        requirement_name: "try_to_u64",
        target: BuiltinCastTarget::Fixed(FixedScalar::U64),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToF16,
        trait_name: "CASTABLE_TO_F16",
        requirement_name: "to_f16",
        target: BuiltinCastTarget::Fixed(FixedScalar::F16),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToF16,
        trait_name: "TRY_CASTABLE_TO_F16",
        requirement_name: "try_to_f16",
        target: BuiltinCastTarget::Fixed(FixedScalar::F16),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToF32,
        trait_name: "CASTABLE_TO_F32",
        requirement_name: "to_f32",
        target: BuiltinCastTarget::Fixed(FixedScalar::F32),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToF32,
        trait_name: "TRY_CASTABLE_TO_F32",
        requirement_name: "try_to_f32",
        target: BuiltinCastTarget::Fixed(FixedScalar::F32),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToF64,
        trait_name: "CASTABLE_TO_F64",
        requirement_name: "to_f64",
        target: BuiltinCastTarget::Fixed(FixedScalar::F64),
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToF64,
        trait_name: "TRY_CASTABLE_TO_F64",
        requirement_name: "try_to_f64",
        target: BuiltinCastTarget::Fixed(FixedScalar::F64),
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToBool,
        trait_name: "CASTABLE_TO_BOOL",
        requirement_name: "to_bool",
        target: BuiltinCastTarget::Bool,
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToBool,
        trait_name: "TRY_CASTABLE_TO_BOOL",
        requirement_name: "try_to_bool",
        target: BuiltinCastTarget::Bool,
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToString,
        trait_name: "CASTABLE_TO_STRING",
        requirement_name: "to_string",
        target: BuiltinCastTarget::String,
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToString,
        trait_name: "TRY_CASTABLE_TO_STRING",
        requirement_name: "try_to_string",
        target: BuiltinCastTarget::String,
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToChar,
        trait_name: "CASTABLE_TO_CHAR",
        requirement_name: "to_char",
        target: BuiltinCastTarget::Char,
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToChar,
        trait_name: "TRY_CASTABLE_TO_CHAR",
        requirement_name: "try_to_char",
        target: BuiltinCastTarget::Char,
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToError,
        trait_name: "CASTABLE_TO_ERROR",
        requirement_name: "to_error",
        target: BuiltinCastTarget::Error,
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToError,
        trait_name: "TRY_CASTABLE_TO_ERROR",
        requirement_name: "try_to_error",
        target: BuiltinCastTarget::Error,
        fallibility: BuiltinCastFallibility::Fallible,
    },
];

/// Returns the static metadata row for a core cast trait variant.
pub(crate) fn builtin_cast_trait_metadata(
    trait_kind: CoreCastTrait,
) -> &'static CoreCastTraitMetadata {
    BUILTIN_CAST_TRAIT_ROWS
        .iter()
        .find(|row| row.kind == trait_kind)
        .expect("core cast trait row list must cover every variant")
}

/// Returns the core cast trait proving a conversion to `target` with `fallibility`.
///
/// `Byte`, profile-sized numeric and `Dec` targets have no source-authored trait and return
/// `None`.
pub(crate) fn core_cast_trait_for_target_and_fallibility(
    target: BuiltinCastTarget,
    fallibility: BuiltinCastFallibility,
) -> Option<CoreCastTrait> {
    BUILTIN_CAST_TRAIT_ROWS
        .iter()
        .find(|row| row.target == target && row.fallibility == fallibility)
        .map(|row| row.kind)
}

/// Returns the source-defined trait name for a core cast trait.
pub(crate) fn builtin_cast_trait_name(trait_kind: CoreCastTrait) -> &'static str {
    builtin_cast_trait_metadata(trait_kind).trait_name
}

/// Returns `true` when `name` matches one of the thirty compiler-owned core cast trait spellings.
///
/// WHAT: centralises the exact-name check so header symbol collection, dependency binding
///      registration, and public export validation can all reject user code that tries to claim
///      a core cast trait name.
/// WHY: the trait name table is the single source of truth; every collision check should consult
///      the same list rather than maintaining a parallel name set.
pub(crate) fn is_core_cast_trait_name(name: &str) -> bool {
    BUILTIN_CAST_TRAIT_ROWS
        .iter()
        .any(|row| row.trait_name == name)
}

/// Calls `callback` once for each compiler-owned core cast trait source name.
///
/// WHAT: lets header and dependency-binding code reserve or enumerate all thirty core cast trait
///      names without depending on the full metadata table shape.
/// WHY: keeps the core cast trait name set as the single source of truth while allowing
///      stage-local consumers such as the visible-name registry to pre-reserve the names.
pub(crate) fn for_each_core_cast_trait_name<F: FnMut(&'static str)>(mut callback: F) {
    for row in BUILTIN_CAST_TRAIT_ROWS {
        callback(row.trait_name);
    }
}
