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
    core_cast_trait_for_target_and_fallibility, is_core_cast_trait_name,
};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn every_source_authored_cast_target_has_both_evidence_classes() {
    let targets = FixedScalar::ALL
        .into_iter()
        .filter(|target| *target != FixedScalar::Byte)
        .map(BuiltinCastTarget::Fixed)
        .chain([
            BuiltinCastTarget::Bool,
            BuiltinCastTarget::String,
            BuiltinCastTarget::Char,
            BuiltinCastTarget::Error,
        ]);

    for target in targets {
        for fallibility in [
            BuiltinCastFallibility::Infallible,
            BuiltinCastFallibility::Fallible,
        ] {
            assert!(
                core_cast_trait_for_target_and_fallibility(target, fallibility).is_some(),
                "{target:?} must have {fallibility:?} cast evidence"
            );
        }
    }
}

#[test]
fn byte_decimal_and_profile_targets_have_no_source_authored_families() {
    let decimal_zero = NumberScale::new(0).expect("zero is a valid Dec scale");
    let targets = [
        BuiltinCastTarget::Int,
        BuiltinCastTarget::Uint,
        BuiltinCastTarget::Float,
        BuiltinCastTarget::Fixed(FixedScalar::Byte),
        BuiltinCastTarget::Number(decimal_zero),
    ];

    for target in targets {
        for fallibility in [
            BuiltinCastFallibility::Infallible,
            BuiltinCastFallibility::Fallible,
        ] {
            assert_eq!(
                core_cast_trait_for_target_and_fallibility(target, fallibility),
                None,
                "{target:?} must not expose source-authored cast evidence"
            );
        }
    }
}

#[test]
fn retired_profile_trait_names_are_not_core_cast_aliases() {
    assert!(is_core_cast_trait_name("CASTABLE_TO_I8"));
    assert!(is_core_cast_trait_name("TRY_CASTABLE_TO_F64"));
    assert!(is_core_cast_trait_name("TRY_CASTABLE_TO_STRING"));
    assert!(!is_core_cast_trait_name("CASTABLE_TO_INT"));
    assert!(!is_core_cast_trait_name("TRY_CASTABLE_TO_INT"));
    assert!(!is_core_cast_trait_name("CASTABLE_TO_FLOAT"));
    assert!(!is_core_cast_trait_name("TRY_CASTABLE_TO_FLOAT"));
    assert!(!is_core_cast_trait_name("DISPLAYABLE"));
    assert!(!is_core_cast_trait_name("castable_to_i8"));
    assert!(!is_core_cast_trait_name("CASTABLE_TO_COLOR"));
}
