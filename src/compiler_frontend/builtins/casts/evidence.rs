//! Builtin cast evidence table, lookup helpers, and metadata for evidence
//! registration.
//!
//! WHAT: defines a single static table for the non-numeric compiler-owned
//!      evidence rows, answers every numeric-to-numeric pair through one
//!      range/precision owner, and exposes the trait id for each
//!      `(source, target)` pair so the AST environment builder can register
//!      builtin evidence rows without re-deriving trait names.
//! WHY: keeping the table in one place means the policy owner cannot drift on
//!      which (source, target) pairs are valid, and later phases can swap the
//!      storage shape without rewriting every call site.

use super::targets::{BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget};
use super::traits::CoreCastTrait;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;

/// Static row describing a single initial builtin evidence entry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BuiltinCastEvidenceRow {
    pub(crate) source: BuiltinCastTarget,
    pub(crate) target: BuiltinCastTarget,
    pub(crate) fallibility: BuiltinCastFallibility,
    pub(crate) policy: BuiltinCastPolicyId,
}

/// The compiler-owned evidence rows outside the numeric matrices.
///
/// WHAT: every row maps a (source, target) pair to its fallibility
///      classification and the stable `BuiltinCastPolicyId` that the policy
///      owner dispatches on. Distinct numeric pairs (including `Int` and
///      `Float`) are answered by `numeric_conversion_fallibility` and the
///      numeric text pairs by the text row generators instead. `Byte` is not
///      numeric, so its only conversions, to and from `U8`, live here.
/// WHY: holding the non-numeric rows as one `const` array makes the per-row
///      list obvious in code review and prevents duplicated entries or
///      fallibility drift.
const STATIC_BUILTIN_EVIDENCE_ROWS: &[BuiltinCastEvidenceRow] = &[
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
        source: BuiltinCastTarget::Int,
        target: BuiltinCastTarget::Char,
        fallibility: BuiltinCastFallibility::Fallible,
        policy: BuiltinCastPolicyId::IntToChar,
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
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Fixed(FixedScalar::Byte),
        target: BuiltinCastTarget::Fixed(FixedScalar::U8),
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::ByteToU8,
    },
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::Fixed(FixedScalar::U8),
        target: BuiltinCastTarget::Fixed(FixedScalar::Byte),
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::U8ToByte,
    },
];

/// Classifies one numeric-to-numeric conversion.
///
/// WHAT: the single owner for every distinct numeric pair (`Int`, `Float` and fixed
///      int/float; never `Byte`). Integer conversion is infallible when the complete source
///      range fits the target range. Integer to binary float is infallible when both range
///      endpoints stay finite after rounding, which excludes only wide sources into `F16`.
///      Float widening, including equal-precision distinct types, is infallible and narrowing
///      is fallible. Float to integer is always fallible.
/// WHY: the cast contract classifies by the complete source/target types, never by an
///      optimiser's value proof, so one function owns every numeric row.
pub(crate) fn numeric_conversion_fallibility(
    source: NumericScalar,
    target: NumericScalar,
    profile: NumericProfile,
) -> BuiltinCastFallibility {
    let infallible = match (
        source.integer_range(profile),
        target.integer_range(profile),
        source.binary_float_precision(profile),
        target.binary_float_precision(profile),
    ) {
        (Some((source_min, source_max)), Some((target_min, target_max)), _, _) => {
            source_min >= target_min && source_max <= target_max
        }

        // Rounding is monotonic, so finite endpoints mean every source value stays finite.
        (Some((source_min, source_max)), None, _, Some(target_precision)) => {
            target_precision.round_integer(source_min).is_finite()
                && target_precision.round_integer(source_max).is_finite()
        }

        (None, None, Some(source_precision), Some(target_precision)) => {
            target_precision >= source_precision
        }

        // Float to integer truncates, then checks the target range.
        _ => false,
    };

    if infallible {
        BuiltinCastFallibility::Infallible
    } else {
        BuiltinCastFallibility::Fallible
    }
}

/// The generated evidence row for one distinct numeric pair.
fn numeric_evidence_row(
    source: NumericScalar,
    target: NumericScalar,
    profile: NumericProfile,
) -> BuiltinCastEvidenceRow {
    BuiltinCastEvidenceRow {
        source: source.into(),
        target: target.into(),
        fallibility: numeric_conversion_fallibility(source, target, profile),
        policy: BuiltinCastPolicyId::NumericConversion { source, target },
    }
}

/// Every numeric domain in canonical order: `Int`, `Float`, then the non-`Byte` fixed scalars.
///
/// WHY: the numeric conversion matrix and both numeric text matrices cover exactly the same
///      domains, so one iterator owns that domain list instead of three parallel spellings.
fn numeric_scalars() -> impl Iterator<Item = NumericScalar> {
    [NumericScalar::Int, NumericScalar::Float]
        .into_iter()
        .chain(
            FixedScalar::ALL
                .into_iter()
                .filter(|scalar| *scalar != FixedScalar::Byte)
                .map(NumericScalar::Fixed),
        )
}

/// The generated evidence row for one numeric domain's text conversion.
///
/// WHAT: every numeric domain formats to `String` infallibly; `Byte` is not numeric, so its text
///       conversion composes through `U8` and has no row here.
/// WHY: the text contract covers every numeric target, so the row set is generated from the same
///      domain list as the conversion matrix rather than hand-written per width.
fn numeric_text_evidence_row(scalar: NumericScalar) -> BuiltinCastEvidenceRow {
    BuiltinCastEvidenceRow {
        source: scalar.into(),
        target: BuiltinCastTarget::String,
        fallibility: BuiltinCastFallibility::Infallible,
        policy: BuiltinCastPolicyId::NumericToString(scalar),
    }
}

/// The generated evidence row for one numeric domain's text parse.
///
/// WHAT: every numeric domain parses from `String` fallibly, materialising inside its own range
///       or precision.
/// WHY: parsing is fallible for every numeric target, including the profile-selected `Int` and
///      `Float`, so the row set is generated alongside the formatting rows.
fn string_numeric_evidence_row(scalar: NumericScalar) -> BuiltinCastEvidenceRow {
    BuiltinCastEvidenceRow {
        source: BuiltinCastTarget::String,
        target: scalar.into(),
        fallibility: BuiltinCastFallibility::Fallible,
        policy: BuiltinCastPolicyId::StringToNumeric(scalar),
    }
}

/// Looks up a builtin evidence row for a (source, target) pair.
///
/// WHAT: answers distinct numeric pairs through `numeric_conversion_fallibility`, numeric text
///      pairs through the text row generators, and every other pair, including `Byte <-> U8`,
///      through the static table.
/// WHY: the evidence class is fixed before HIR from the complete source/target types and the
///      boundary profile, so resolution asks this one owner.
pub(crate) fn lookup_builtin_evidence(
    source: BuiltinCastTarget,
    target: BuiltinCastTarget,
    profile: NumericProfile,
) -> Option<BuiltinCastEvidenceRow> {
    if let (Some(source_scalar), Some(target_scalar)) =
        (source.numeric_scalar(), target.numeric_scalar())
    {
        return (source_scalar != target_scalar)
            .then(|| numeric_evidence_row(source_scalar, target_scalar, profile));
    }

    if target == BuiltinCastTarget::String
        && let Some(scalar) = source.numeric_scalar()
    {
        return Some(numeric_text_evidence_row(scalar));
    }

    if source == BuiltinCastTarget::String
        && let Some(scalar) = target.numeric_scalar()
    {
        return Some(string_numeric_evidence_row(scalar));
    }

    STATIC_BUILTIN_EVIDENCE_ROWS
        .iter()
        .copied()
        .find(|row| row.source == source && row.target == target)
}

/// Every builtin evidence row for one numeric profile.
///
/// WHAT: the static rows, the generated row for every distinct numeric pair, and the generated
///      numeric text rows in both directions.
/// WHY: registration and tests see exactly the row set `lookup_builtin_evidence` answers.
pub(crate) fn builtin_evidence_rows_for_profile(
    profile: NumericProfile,
) -> impl Iterator<Item = BuiltinCastEvidenceRow> {
    let numeric_rows = numeric_scalars().flat_map(move |source| {
        numeric_scalars()
            .filter(move |target| *target != source)
            .map(move |target| numeric_evidence_row(source, target, profile))
    });

    let text_rows = numeric_scalars()
        .map(numeric_text_evidence_row)
        .chain(numeric_scalars().map(string_numeric_evidence_row));

    STATIC_BUILTIN_EVIDENCE_ROWS
        .iter()
        .copied()
        .chain(numeric_rows)
        .chain(text_rows)
}

/// Resolves a `BuiltinCastTarget` to its canonical `TypeId` in the supplied
/// `TypeEnvironment`.
///
/// WHAT: bridges the cast trait catalogue (which uses `BuiltinCastTarget`
///      enums) to the `TypeEnvironment` handles that builtin evidence needs.
///      Fixed scalars resolve through the seeded fixed-scalar identities;
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
        BuiltinCastTarget::Fixed(scalar) => {
            Some(crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(scalar))
        }
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
///      cast trait that proves the row. Fixed targets carry compiler-owned
///      builtin evidence only and map to no source-authorable trait. The
///      lookup scans the single `BUILTIN_CAST_TRAIT_ROWS` table so there is
///      exactly one source of truth for the trait catalogue.
/// WHY: lets `register_builtin_cast_evidence` and its tests share one
///      (source, target) → trait mapping instead of re-deriving the
///      (source, target) → `CoreCastTrait` translation in multiple places.
pub(crate) fn builtin_evidence_trait_kind_for_row(
    row: BuiltinCastEvidenceRow,
) -> Option<CoreCastTrait> {
    if matches!(row.target, BuiltinCastTarget::Fixed(_)) {
        return None;
    }

    for metadata in super::traits::BUILTIN_CAST_TRAIT_ROWS {
        if metadata.target == row.target && metadata.fallibility == row.fallibility {
            return Some(metadata.kind);
        }
    }
    None
}
