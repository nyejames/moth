//! Builtin cast evidence table, lookup helpers, and metadata for evidence
//! registration.
//!
//! WHAT: defines a single static table that records every initial compiler-owned
//!      evidence row, plus lookup helpers that resolve a
//!      `(BuiltinCastTarget, BuiltinCastTarget)` pair into its `BuiltinCastPolicyId`
//!      and fallibility. Exposes the trait id for each `(source, target)` pair so
//!      the AST environment builder can register builtin evidence rows without
//!      re-deriving trait names.
//! WHY: keeping the table in one place means the policy owner cannot drift on
//!      which (source, target) pairs are valid, and later phases can swap the
//!      storage shape without rewriting every call site.

use super::targets::{BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget};
use super::traits::CoreCastTrait;

/// Static row describing a single initial builtin evidence entry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BuiltinCastEvidenceRow {
    pub(crate) source: BuiltinCastTarget,
    pub(crate) target: BuiltinCastTarget,
    pub(crate) fallibility: BuiltinCastFallibility,
    pub(crate) policy: BuiltinCastPolicyId,
}

/// The complete set of initial builtin evidence rows registered by the compiler.
///
/// WHAT: every row maps a (source, target) pair to its fallibility classification
///      and the stable `BuiltinCastPolicyId` that the policy owner will dispatch on.
/// WHY: holding the table as one `const` array makes the per-row list obvious in
///      code review and prevents duplicated entries or fallibility drift.
const INITIAL_BUILTIN_EVIDENCE_ROWS: &[BuiltinCastEvidenceRow] = &[
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Int,
        target: BuiltinCastTarget::Float,
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::IntToFloat,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Int,
        target: BuiltinCastTarget::String,
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::IntToString,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Float,
        target: BuiltinCastTarget::String,
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::FloatToString,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Bool,
        target: BuiltinCastTarget::String,
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::BoolToString,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Char,
        target: BuiltinCastTarget::String,
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::CharToString,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Char,
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::CharToInt,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::String,
        target: BuiltinCastTarget::Error,
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::StringToError,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Error,
        target: BuiltinCastTarget::String,
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::ErrorToString,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Float,
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Fallible,
        policy: BuiltinCastPolicyId::FloatToInt,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Int,
        target: BuiltinCastTarget::Char,
        fallibility: BuiltinCastFallibility::Fallible,
        policy: BuiltinCastPolicyId::IntToChar,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::String,
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Fallible,
        policy: BuiltinCastPolicyId::StringToInt,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::String,
        target: BuiltinCastTarget::Float,
        fallibility: BuiltinCastFallibility::Fallible,
        policy: BuiltinCastPolicyId::StringToFloat,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::String,
        target: BuiltinCastTarget::Bool,
        fallibility: BuiltinCastFallibility::Fallible,
        policy: BuiltinCastPolicyId::StringToBool,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::String,
        target: BuiltinCastTarget::Char,
        fallibility: BuiltinCastFallibility::Fallible,
        policy: BuiltinCastPolicyId::StringToChar,
    },
];

/// Looks up a builtin evidence row for a (source, target) pair.
pub(crate) fn lookup_builtin_evidence(
    source: BuiltinCastTarget,
    target: BuiltinCastTarget,
) -> Option<BuiltinCastEvidenceRow> {
    INITIAL_BUILTIN_EVIDENCE_ROWS
        .iter()
        .copied()
        .find(|row| row.source == source && row.target == target)
}

/// Returns the full list of initial builtin evidence rows.
pub(crate) fn builtin_evidence_rows() -> &'static [BuiltinCastEvidenceRow] {
    INITIAL_BUILTIN_EVIDENCE_ROWS
}

/// Reports the fallibility of a builtin evidence row, or `None` when no row exists.
pub(crate) fn builtin_evidence_fallibility(
    source: BuiltinCastTarget,
    target: BuiltinCastTarget,
) -> Option<BuiltinCastFallibility> {
    lookup_builtin_evidence(source, target).map(|row| row.fallibility)
}

/// Reports the policy id for a builtin evidence row, or `None` when no row exists.
pub(crate) fn builtin_evidence_policy(
    source: BuiltinCastTarget,
    target: BuiltinCastTarget,
) -> Option<BuiltinCastPolicyId> {
    lookup_builtin_evidence(source, target).map(|row| row.policy)
}

/// Resolves a `BuiltinCastTarget` to its canonical `TypeId` in the supplied
/// `TypeEnvironment`.
///
/// WHAT: bridges the cast trait catalogue (which uses `BuiltinCastTarget`
///      enums) to the `TypeEnvironment` handles that builtin evidence needs.
///      `Error` is resolved through the nominal path lookup because the
///      builtin error struct is registered as a regular nominal type.
/// WHY: registration code builds one builtin evidence row per trait kind and
///      must convert the source/target classifiers into the `TypeId`s that
///      `TraitEvidenceEnvironment::insert_builtin` expects.
pub(crate) fn type_id_for_builtin_target(
    target: BuiltinCastTarget,
    type_environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
) -> Option<crate::compiler_frontend::datatypes::ids::TypeId> {
    use crate::compiler_frontend::datatypes::ids::TypeId;
    let builtins = type_environment.builtins();
    match target {
        BuiltinCastTarget::Bool => Some(builtins.bool),
        BuiltinCastTarget::Int => Some(builtins.int),
        BuiltinCastTarget::String => Some(builtins.string),
        BuiltinCastTarget::Char => Some(builtins.char),
        BuiltinCastTarget::Float => Some(builtins.float),
        BuiltinCastTarget::Error => {
            let path = crate::compiler_frontend::builtins::error_type::builtin_error_type_path(
                path_fork,
                string_table,
            );
            let nominal_id = type_environment.nominal_id_for_path(&path)?;
            let type_id: TypeId = type_environment.type_id_for_nominal_id(nominal_id)?;
            Some(type_id)
        }
    }
}

/// Returns the `CoreCastTrait` variant for a builtin evidence row.
///
/// WHAT: maps a builtin evidence row's target and fallibility to the core
///      cast trait that proves the row. The lookup scans the single
///      `BUILTIN_CAST_TRAIT_ROWS` table so there is exactly one source of
///      truth for the trait catalogue.
/// WHY: lets `register_builtin_cast_evidence` and its tests share one
///      (source, target) → trait mapping instead of re-deriving the
///      (source, target) → `CoreCastTrait` translation in multiple places.
pub(crate) fn builtin_evidence_trait_kind_for_row(
    row: BuiltinCastEvidenceRow,
) -> Option<CoreCastTrait> {
    for metadata in super::traits::BUILTIN_CAST_TRAIT_ROWS {
        if metadata.target == row.target && metadata.fallibility == row.fallibility {
            return Some(metadata.kind);
        }
    }
    None
}
