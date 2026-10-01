//! Core cast trait semantic invariant tests.
//!
//! WHAT: protects unique target/fallibility pairs for each source-backed core cast target and
//!       the exact-name recognition policy for core cast traits.
//! WHY: preserves semantic coverage and reserved-name behavior without pinning metadata table
//!      layout or duplicating requirement-name spellings.

use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::builtins::casts::traits::{
    BUILTIN_CAST_TRAIT_ROWS, is_core_cast_trait_name,
};
use std::collections::HashMap;

/// Verifies unique fallibility pairs and coverage for each source-backed core cast target.
///
/// Fixed-width and Dec targets intentionally have no source trait.
#[test]
fn target_fallibility_pairs_cover_every_core_cast_trait_target() {
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
