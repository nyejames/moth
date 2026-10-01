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

use super::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
    builtin_cast_target_for_builtin_type,
};
use super::traits::CoreCastTrait;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::traits::environment::{CoreTraitKind, TraitEnvironment};
use crate::compiler_frontend::traits::ids::TraitId;

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
/// WHAT: classifies bounded numeric pairs by their complete ranges and precision, and classifies
///       Dec pairs lazily: integer-to-Dec and scale widening are infallible; Dec narrowing
///       and Dec-to-integer are fallible exact conversions. Binary-float/Dec pairs are absent.
/// WHY: the cast contract classifies by complete source/target types, never by a value proof, and
///      the Dec scale is semantic identity rather than an eagerly registered matrix row.
pub(crate) fn numeric_conversion_fallibility(
    source: NumericScalar,
    target: NumericScalar,
    profile: NumericProfile,
) -> BuiltinCastFallibility {
    match (source, target) {
        (NumericScalar::Number(source_scale), NumericScalar::Number(target_scale)) => {
            return if source_scale <= target_scale {
                BuiltinCastFallibility::Infallible
            } else {
                BuiltinCastFallibility::Fallible
            };
        }

        (NumericScalar::Number(_), target) if target.integer_range(profile).is_some() => {
            return BuiltinCastFallibility::Fallible;
        }

        (source, NumericScalar::Number(_)) if source.integer_range(profile).is_some() => {
            return BuiltinCastFallibility::Infallible;
        }

        (NumericScalar::Number(_), _) | (_, NumericScalar::Number(_)) => {
            return BuiltinCastFallibility::Fallible;
        }

        _ => {}
    }

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
pub(crate) fn numeric_scalars() -> impl Iterator<Item = NumericScalar> {
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
///       conversion composes through `U8` and has no row here. `Dec` stays out of the static
///       generator: its evidence resolves on demand per pair, so the bounded domain list never
///       expands to 257 scale rows.
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
    // Dec conversions resolve only for this exact source/target pair. They do not enter the
    // bounded numeric generator or register one row for every scale.
    if source != target {
        let number_conversion = match (source, target) {
            (BuiltinCastTarget::Number(source_scale), BuiltinCastTarget::Number(target_scale)) => {
                Some((
                    NumericScalar::Number(source_scale),
                    NumericScalar::Number(target_scale),
                ))
            }
            (BuiltinCastTarget::Number(source_scale), target) => target
                .numeric_scalar()
                .filter(|scalar| scalar.integer_range(profile).is_some())
                .map(|target| (NumericScalar::Number(source_scale), target)),
            (source, BuiltinCastTarget::Number(target_scale)) => source
                .numeric_scalar()
                .filter(|scalar| scalar.integer_range(profile).is_some())
                .map(|source| (source, NumericScalar::Number(target_scale))),
            _ => None,
        };

        if let Some((source_scalar, target_scalar)) = number_conversion {
            return Some(numeric_evidence_row(source_scalar, target_scalar, profile));
        }
    }

    if let (Some(source_scalar), Some(target_scalar)) =
        (source.numeric_scalar(), target.numeric_scalar())
    {
        return (source_scalar != target_scalar)
            .then(|| numeric_evidence_row(source_scalar, target_scalar, profile));
    }

    if target == BuiltinCastTarget::String {
        // Every bounded numeric domain formats to `String` through the static text row.
        if let Some(scalar) = source.numeric_scalar() {
            return Some(numeric_text_evidence_row(scalar));
        }

        // `Dec -> String` formats exactly through the canonical decimal text contract.
        if let BuiltinCastTarget::Number(scale) = source {
            return Some(BuiltinCastEvidenceRow {
                source,
                target,
                fallibility: BuiltinCastFallibility::Infallible,
                policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Number(scale)),
            });
        }
    }
    if source == BuiltinCastTarget::String
        && let BuiltinCastTarget::Number(scale) = target
    {
        return Some(string_numeric_evidence_row(NumericScalar::Number(scale)));
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

/// Returns whether the existing builtin cast pair proves one registered core cast trait.
///
/// The type classifier deliberately accepts builtin identities only; source nominals must use
/// registered evidence. Pair availability and fallibility remain owned by the cast evidence
/// table and the active numeric profile.
pub(crate) fn builtin_cast_proves_core_trait(
    source_type_id: TypeId,
    trait_id: TraitId,
    type_environment: &TypeEnvironment,
    trait_environment: &TraitEnvironment,
    numeric_profile: NumericProfile,
) -> bool {
    let Some(CoreTraitKind::Castable {
        target,
        fallibility,
    }) = trait_environment.core_trait_kind(trait_id)
    else {
        return false;
    };
    let Some(source) = builtin_cast_target_for_builtin_type(source_type_id, type_environment)
    else {
        return false;
    };

    lookup_builtin_evidence(source, target, numeric_profile)
        .is_some_and(|row| row.fallibility == fallibility)
}

/// The bounded builtin evidence rows registered for one numeric profile.
///
/// WHAT: static rows, distinct bounded numeric pairs and their numeric text rows.
/// WHY: registration stays finite. `Dec` scale pairs resolve on demand through
///      `lookup_builtin_evidence` rather than expanding the registered row set.
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
        BuiltinCastTarget::Number(scale) => type_environment.type_id_for_canonical_identity(
            &crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity::Builtin(
                crate::compiler_frontend::canonical_type_identity::CanonicalBuiltinType::Number(
                    scale,
                ),
            ),
        ),
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
///      cast trait that proves the row. Fixed and `Dec` targets carry compiler-owned
///      builtin evidence only and map to no source-authorable trait. The lookup scans
///      the single `BUILTIN_CAST_TRAIT_ROWS` table for the trait catalogue.
/// WHY: lets `register_builtin_cast_evidence` and its tests share one
///      (source, target) → trait mapping instead of re-deriving the
///      (source, target) → `CoreCastTrait` translation in multiple places.
pub(crate) fn builtin_evidence_trait_kind_for_row(
    row: BuiltinCastEvidenceRow,
) -> Option<CoreCastTrait> {
    if matches!(
        row.target,
        BuiltinCastTarget::Fixed(_) | BuiltinCastTarget::Number(_)
    ) {
        return None;
    }

    for metadata in super::traits::BUILTIN_CAST_TRAIT_ROWS {
        if metadata.target == row.target && metadata.fallibility == row.fallibility {
            return Some(metadata.kind);
        }
    }
    None
}
