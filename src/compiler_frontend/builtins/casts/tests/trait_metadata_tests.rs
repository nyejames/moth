//! Core cast trait metadata unit tests.
//!
//! WHAT: pins down the table-driven metadata table for the twelve core
//!      cast traits so future additions cannot drift on the requirement
//!      name, builtin target, or fallibility classifier.
//! WHY: the metadata table is the single source of truth for the cast
//!      trait catalogue. Tests here prevent accidental edits to the
//!      requirement names and target classifications that other phases
//!      rely on.

use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::builtins::casts::traits::{
    BUILTIN_CAST_TRAIT_ROWS, CoreCastTrait, builtin_cast_trait_name, is_core_cast_trait_name,
};
use std::collections::HashMap;

#[test]
fn metadata_table_covers_every_core_cast_trait() {
    assert_eq!(BUILTIN_CAST_TRAIT_ROWS.len(), 12);
    let mut kinds: Vec<CoreCastTrait> =
        BUILTIN_CAST_TRAIT_ROWS.iter().map(|row| row.kind).collect();
    kinds.sort_by_key(|kind| format!("{kind:?}"));
    kinds.dedup();
    assert_eq!(kinds.len(), 12, "every variant must appear exactly once");
}

#[test]
fn infallible_traits_use_to_requirement_prefix() {
    for row in BUILTIN_CAST_TRAIT_ROWS {
        if row.fallibility == BuiltinCastFallibility::Infallible {
            let expected = format!(
                "to_{}",
                match row.target {
                    BuiltinCastTarget::Bool => "bool",
                    BuiltinCastTarget::Int => "int",
                    BuiltinCastTarget::String => "string",
                    BuiltinCastTarget::Char => "char",
                    BuiltinCastTarget::Float => "float",
                    BuiltinCastTarget::Error => "error",
                    BuiltinCastTarget::Fixed(_) => continue,
                }
            );
            assert_eq!(
                row.requirement_name, expected,
                "{:?} requirement name",
                row.kind
            );
        }
    }
}
#[test]
fn fallible_traits_use_try_to_requirement_prefix() {
    for row in BUILTIN_CAST_TRAIT_ROWS {
        if row.fallibility == BuiltinCastFallibility::Fallible {
            let expected = format!(
                "try_to_{}",
                match row.target {
                    BuiltinCastTarget::Bool => "bool",
                    BuiltinCastTarget::Int => "int",
                    BuiltinCastTarget::String => "string",
                    BuiltinCastTarget::Char => "char",
                    BuiltinCastTarget::Float => "float",
                    BuiltinCastTarget::Error => "error",
                    BuiltinCastTarget::Fixed(_) => continue,
                }
            );
            assert_eq!(
                row.requirement_name, expected,
                "{:?} requirement name",
                row.kind
            );
        }
    }
}

#[test]
fn trait_names_match_language_core_cast_spellings() {
    assert_eq!(
        builtin_cast_trait_name(CoreCastTrait::CastableToInt),
        "CASTABLE_TO_INT"
    );
    assert_eq!(
        builtin_cast_trait_name(CoreCastTrait::TryCastableToError),
        "TRY_CASTABLE_TO_ERROR"
    );
    assert_eq!(
        builtin_cast_trait_name(CoreCastTrait::CastableToFloat),
        "CASTABLE_TO_FLOAT"
    );
    assert_eq!(
        builtin_cast_trait_name(CoreCastTrait::CastableToString),
        "CASTABLE_TO_STRING"
    );
}

/// Verifies that the metadata table covers every builtin target with both
/// an infallible and a fallible trait row, and that no (target, fallibility)
/// pair duplicates.
#[test]
fn target_fallibility_pairs_cover_every_builtin_target() {
    use std::collections::HashSet;

    let mut seen: HashSet<(BuiltinCastTarget, BuiltinCastFallibility)> = HashSet::new();
    let mut by_target: HashMap<BuiltinCastTarget, (bool, bool)> = HashMap::new();

    for row in BUILTIN_CAST_TRAIT_ROWS {
        assert!(
            seen.insert((row.target, row.fallibility)),
            "duplicate (target, fallibility) row for {:?}",
            row.target
        );

        let entry = by_target.entry(row.target).or_insert((false, false));
        match row.fallibility {
            BuiltinCastFallibility::Infallible => entry.0 = true,
            BuiltinCastFallibility::Fallible => entry.1 = true,
        }
    }

    let expected_targets = [
        BuiltinCastTarget::Bool,
        BuiltinCastTarget::Int,
        BuiltinCastTarget::String,
        BuiltinCastTarget::Char,
        BuiltinCastTarget::Float,
        BuiltinCastTarget::Error,
    ];
    assert_eq!(by_target.len(), expected_targets.len());
    for target in expected_targets {
        let (infallible, fallible) = by_target.get(&target).copied().unwrap_or((false, false));
        assert!(infallible && fallible, "{target:?} must have both forms");
    }
}

#[test]
fn is_core_cast_trait_name_matches_exact_trait_spellings_only() {
    assert!(is_core_cast_trait_name("CASTABLE_TO_INT"));
    assert!(is_core_cast_trait_name("TRY_CASTABLE_TO_STRING"));
    assert!(!is_core_cast_trait_name("DISPLAYABLE"));
    assert!(!is_core_cast_trait_name("castable_to_int"));
    assert!(!is_core_cast_trait_name("CASTABLE_TO_COLOR"));
}
