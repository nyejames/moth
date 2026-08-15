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

/// The set of compiler-owned core cast traits.
///
/// Each builtin cast target has one infallible evidence trait and one
/// fallible evidence trait. The metadata rows below are the authoritative
/// source for their source spellings, requirement names, targets, and
/// fallibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CoreCastTrait {
    CastableToInt,
    TryCastableToInt,
    CastableToFloat,
    TryCastableToFloat,
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
        kind: CoreCastTrait::CastableToInt,
        trait_name: "CASTABLE_TO_INT",
        requirement_name: "to_int",
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToInt,
        trait_name: "TRY_CASTABLE_TO_INT",
        requirement_name: "try_to_int",
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Fallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::CastableToFloat,
        trait_name: "CASTABLE_TO_FLOAT",
        requirement_name: "to_float",
        target: BuiltinCastTarget::Float,
        fallibility: BuiltinCastFallibility::Infallible,
    },
    CoreCastTraitMetadata {
        kind: CoreCastTrait::TryCastableToFloat,
        trait_name: "TRY_CASTABLE_TO_FLOAT",
        requirement_name: "try_to_float",
        target: BuiltinCastTarget::Float,
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

/// Returns the source-defined trait name for a core cast trait.
pub(crate) fn builtin_cast_trait_name(trait_kind: CoreCastTrait) -> &'static str {
    builtin_cast_trait_metadata(trait_kind).trait_name
}

/// Returns `true` when `name` matches one of the twelve compiler-owned core
/// cast trait source spellings.
///
/// WHAT: centralises the exact-name check so header symbol collection, dependency binding
///      registration, and public export validation can all reject user code
///      that tries to claim a core cast trait name.
/// WHY: the trait name table is the single source of truth; every collision
///      check should consult the same list rather than maintaining a parallel
///      name set.
pub(crate) fn is_core_cast_trait_name(name: &str) -> bool {
    BUILTIN_CAST_TRAIT_ROWS
        .iter()
        .any(|row| row.trait_name == name)
}

/// Calls `callback` once for each compiler-owned core cast trait source name.
///
/// WHAT: lets header and dependency-binding code reserve or enumerate the 12 core cast trait
///      names without depending on the full metadata table shape.
/// WHY: keeps the core cast trait name set as the single source of truth while
///      allowing stage-local consumers such as the visible-name registry to
///      pre-reserve the names.
pub(crate) fn for_each_core_cast_trait_name<F: FnMut(&'static str)>(mut callback: F) {
    for row in BUILTIN_CAST_TRAIT_ROWS {
        callback(row.trait_name);
    }
}
