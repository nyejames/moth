//! Evidence table unit tests for the builtin cast surface.
//!
//! WHAT: covers numeric evidence classification, profile-sensitive rows,
//!      `Byte` evidence, and the profile-complete row listing.
//! WHY: evidence class is fixed before HIR from the complete source/target
//!      types and the boundary profile. Tests here pin the classification so
//!      it cannot drift silently.

use crate::compiler_frontend::builtins::casts::evidence::{
    builtin_evidence_rows_for_profile, lookup_builtin_evidence, numeric_conversion_fallibility,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::numeric_profile::{
    FloatPrecision, IntWidth, NumericProfile,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;

fn fixed_target(scalar: FixedScalar) -> BuiltinCastTarget {
    BuiltinCastTarget::Fixed(scalar)
}

fn numeric_policy(source: NumericScalar, target: NumericScalar) -> BuiltinCastPolicyId {
    BuiltinCastPolicyId::NumericConversion { source, target }
}

fn assert_numeric_row(
    source: BuiltinCastTarget,
    target: BuiltinCastTarget,
    source_scalar: NumericScalar,
    target_scalar: NumericScalar,
    fallibility: BuiltinCastFallibility,
    profile: NumericProfile,
) {
    let row = lookup_builtin_evidence(source, target, profile)
        .unwrap_or_else(|| panic!("{source:?} -> {target:?} evidence should exist"));
    assert_eq!(row.fallibility, fallibility);
    assert_eq!(row.policy, numeric_policy(source_scalar, target_scalar));
}

#[test]
fn numeric_int_to_int_uses_complete_source_range() {
    let profile = NumericProfile::STANDARD;
    assert_numeric_row(
        BuiltinCastTarget::Int,
        fixed_target(FixedScalar::I32),
        NumericScalar::Int,
        NumericScalar::Fixed(FixedScalar::I32),
        BuiltinCastFallibility::Infallible,
        profile,
    );

    // `U8` fits `U16` and `I16` completely; the reverse and `I8 -> U8` do not.
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::U8),
            NumericScalar::Fixed(FixedScalar::U16),
            profile,
        ),
        BuiltinCastFallibility::Infallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::U8),
            NumericScalar::Fixed(FixedScalar::I16),
            profile,
        ),
        BuiltinCastFallibility::Infallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::U16),
            NumericScalar::Fixed(FixedScalar::U8),
            profile,
        ),
        BuiltinCastFallibility::Fallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::I8),
            NumericScalar::Fixed(FixedScalar::U8),
            profile,
        ),
        BuiltinCastFallibility::Fallible,
    );
}

#[test]
fn int_to_i32_is_profile_sensitive() {
    let int32 = NumericProfile::STANDARD;
    let int64 = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    };

    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Int,
            NumericScalar::Fixed(FixedScalar::I32),
            int32,
        ),
        BuiltinCastFallibility::Infallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Int,
            NumericScalar::Fixed(FixedScalar::I32),
            int64,
        ),
        BuiltinCastFallibility::Fallible,
    );
    // `I64 -> Int` only fits under the 64-bit `Int` width.
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::I64),
            NumericScalar::Int,
            int32,
        ),
        BuiltinCastFallibility::Fallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::I64),
            NumericScalar::Int,
            int64,
        ),
        BuiltinCastFallibility::Infallible,
    );
}

#[test]
fn int_to_float_is_infallible_except_f16_range() {
    let profile = NumericProfile::STANDARD;
    assert_numeric_row(
        BuiltinCastTarget::Int,
        BuiltinCastTarget::Float,
        NumericScalar::Int,
        NumericScalar::Float,
        BuiltinCastFallibility::Infallible,
        profile,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::I64),
        fixed_target(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::I64),
        NumericScalar::Fixed(FixedScalar::F64),
        BuiltinCastFallibility::Infallible,
        profile,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::U64),
        fixed_target(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::U64),
        NumericScalar::Fixed(FixedScalar::F32),
        BuiltinCastFallibility::Infallible,
        profile,
    );

    // `I16` stays finite in `F16`; `U16` and `I32` do not.
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::I16),
            NumericScalar::Fixed(FixedScalar::F16),
            profile,
        ),
        BuiltinCastFallibility::Infallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::U16),
            NumericScalar::Fixed(FixedScalar::F16),
            profile,
        ),
        BuiltinCastFallibility::Fallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::I32),
            NumericScalar::Fixed(FixedScalar::F16),
            profile,
        ),
        BuiltinCastFallibility::Fallible,
    );
}

#[test]
fn float_to_float_compares_precision_not_spelling() {
    let float64 = NumericProfile::STANDARD;
    let float32 = NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits32,
    };

    // Narrowing is fallible.
    assert_numeric_row(
        fixed_target(FixedScalar::F64),
        fixed_target(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::F32),
        BuiltinCastFallibility::Fallible,
        float64,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::F32),
        fixed_target(FixedScalar::F16),
        NumericScalar::Fixed(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::F16),
        BuiltinCastFallibility::Fallible,
        float64,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::F64),
        fixed_target(FixedScalar::F16),
        NumericScalar::Fixed(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::F16),
        BuiltinCastFallibility::Fallible,
        float64,
    );
    // Widening is infallible.
    assert_numeric_row(
        fixed_target(FixedScalar::F32),
        fixed_target(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::F64),
        BuiltinCastFallibility::Infallible,
        float64,
    );

    // `Float -> F32` narrows under `Float64` but is equal-precision under `Float32`.
    assert_numeric_row(
        BuiltinCastTarget::Float,
        fixed_target(FixedScalar::F32),
        NumericScalar::Float,
        NumericScalar::Fixed(FixedScalar::F32),
        BuiltinCastFallibility::Fallible,
        float64,
    );
    assert_numeric_row(
        BuiltinCastTarget::Float,
        fixed_target(FixedScalar::F32),
        NumericScalar::Float,
        NumericScalar::Fixed(FixedScalar::F32),
        BuiltinCastFallibility::Infallible,
        float32,
    );
    // `F32 -> Float` is equal-precision under `Float32` and widening under `Float64`.
    assert_numeric_row(
        fixed_target(FixedScalar::F32),
        BuiltinCastTarget::Float,
        NumericScalar::Fixed(FixedScalar::F32),
        NumericScalar::Float,
        BuiltinCastFallibility::Infallible,
        float32,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::F32),
        BuiltinCastTarget::Float,
        NumericScalar::Fixed(FixedScalar::F32),
        NumericScalar::Float,
        BuiltinCastFallibility::Infallible,
        float64,
    );
}

#[test]
fn float_to_int_is_always_fallible_with_numeric_policy() {
    let profile = NumericProfile::STANDARD;
    assert_numeric_row(
        BuiltinCastTarget::Float,
        BuiltinCastTarget::Int,
        NumericScalar::Float,
        NumericScalar::Int,
        BuiltinCastFallibility::Fallible,
        profile,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::F64),
        fixed_target(FixedScalar::I64),
        NumericScalar::Fixed(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::I64),
        BuiltinCastFallibility::Fallible,
        profile,
    );
}

#[test]
fn byte_evidence_covers_only_u8_pair() {
    let profile = NumericProfile::STANDARD;
    let row = lookup_builtin_evidence(
        fixed_target(FixedScalar::Byte),
        fixed_target(FixedScalar::U8),
        profile,
    )
    .expect("Byte -> U8 evidence should exist");
    assert_eq!(row.fallibility, BuiltinCastFallibility::Infallible);
    assert_eq!(row.policy, BuiltinCastPolicyId::ByteToU8);

    let row = lookup_builtin_evidence(
        fixed_target(FixedScalar::U8),
        fixed_target(FixedScalar::Byte),
        profile,
    )
    .expect("U8 -> Byte evidence should exist");
    assert_eq!(row.fallibility, BuiltinCastFallibility::Infallible);
    assert_eq!(row.policy, BuiltinCastPolicyId::U8ToByte);

    assert!(
        lookup_builtin_evidence(
            fixed_target(FixedScalar::Byte),
            fixed_target(FixedScalar::U16),
            profile,
        )
        .is_none()
    );
    assert!(
        lookup_builtin_evidence(
            BuiltinCastTarget::Int,
            fixed_target(FixedScalar::Byte),
            profile,
        )
        .is_none()
    );
}

#[test]
fn numeric_text_evidence_covers_every_numeric_domain_in_both_directions() {
    let profile = NumericProfile::STANDARD;

    // Formatting is infallible for every numeric domain, including the fixed scalars, and the row
    // carries the domain so the policy can format at that domain's own precision.
    let formatting_cases = [
        (BuiltinCastTarget::Int, NumericScalar::Int),
        (BuiltinCastTarget::Float, NumericScalar::Float),
        (
            fixed_target(FixedScalar::U8),
            NumericScalar::Fixed(FixedScalar::U8),
        ),
        (
            fixed_target(FixedScalar::I64),
            NumericScalar::Fixed(FixedScalar::I64),
        ),
        (
            fixed_target(FixedScalar::F16),
            NumericScalar::Fixed(FixedScalar::F16),
        ),
    ];

    for (source, scalar) in formatting_cases {
        let row = lookup_builtin_evidence(source, BuiltinCastTarget::String, profile)
            .unwrap_or_else(|| panic!("{source:?} -> String evidence should exist"));
        assert_eq!(
            row.fallibility,
            BuiltinCastFallibility::Infallible,
            "{source:?}"
        );
        assert_eq!(
            row.policy,
            BuiltinCastPolicyId::NumericToString(scalar),
            "{source:?}"
        );
    }

    // Parsing is fallible for every numeric domain, including the profile-selected `Int`/`Float`.
    let parsing_cases = [
        (NumericScalar::Int, BuiltinCastTarget::Int),
        (NumericScalar::Float, BuiltinCastTarget::Float),
        (
            NumericScalar::Fixed(FixedScalar::U8),
            fixed_target(FixedScalar::U8),
        ),
        (
            NumericScalar::Fixed(FixedScalar::F64),
            fixed_target(FixedScalar::F64),
        ),
    ];

    for (scalar, target) in parsing_cases {
        let row = lookup_builtin_evidence(BuiltinCastTarget::String, target, profile)
            .unwrap_or_else(|| panic!("String -> {target:?} evidence should exist"));
        assert_eq!(
            row.fallibility,
            BuiltinCastFallibility::Fallible,
            "{target:?}"
        );
        assert_eq!(
            row.policy,
            BuiltinCastPolicyId::StringToNumeric(scalar),
            "{target:?}"
        );
    }
}

#[test]
fn byte_has_no_numeric_text_evidence() {
    let profile = NumericProfile::STANDARD;

    // `Byte` is an octet, not a number: its text conversion composes through `U8`, so neither
    // direction has a numeric text row.
    assert!(
        lookup_builtin_evidence(
            fixed_target(FixedScalar::Byte),
            BuiltinCastTarget::String,
            profile
        )
        .is_none()
    );
    assert!(
        lookup_builtin_evidence(
            BuiltinCastTarget::String,
            fixed_target(FixedScalar::Byte),
            profile
        )
        .is_none()
    );
}

#[test]
fn same_target_has_no_evidence_and_non_numeric_pairs_fall_through() {
    let profile = NumericProfile::STANDARD;
    assert!(
        lookup_builtin_evidence(BuiltinCastTarget::Int, BuiltinCastTarget::Int, profile).is_none()
    );
    let row = lookup_builtin_evidence(BuiltinCastTarget::String, BuiltinCastTarget::Bool, profile)
        .expect("String -> Bool evidence should exist");
    assert_eq!(row.fallibility, BuiltinCastFallibility::Fallible);
    assert_eq!(row.policy, BuiltinCastPolicyId::StringToBool);
    assert!(
        lookup_builtin_evidence(BuiltinCastTarget::Bool, BuiltinCastTarget::Int, profile).is_none()
    );
}

#[test]
fn profile_row_listing_matches_lookup() {
    // Every builtin cast source and target, so the listing can be checked for completeness
    // instead of only for its arithmetic size.
    let targets = [
        BuiltinCastTarget::Bool,
        BuiltinCastTarget::Int,
        BuiltinCastTarget::String,
        BuiltinCastTarget::Char,
        BuiltinCastTarget::Float,
        BuiltinCastTarget::Error,
        BuiltinCastTarget::Fixed(FixedScalar::Byte),
        BuiltinCastTarget::Fixed(FixedScalar::U8),
        BuiltinCastTarget::Fixed(FixedScalar::U16),
        BuiltinCastTarget::Fixed(FixedScalar::U32),
        BuiltinCastTarget::Fixed(FixedScalar::U64),
        BuiltinCastTarget::Fixed(FixedScalar::I8),
        BuiltinCastTarget::Fixed(FixedScalar::I16),
        BuiltinCastTarget::Fixed(FixedScalar::I32),
        BuiltinCastTarget::Fixed(FixedScalar::I64),
        BuiltinCastTarget::Fixed(FixedScalar::F16),
        BuiltinCastTarget::Fixed(FixedScalar::F32),
        BuiltinCastTarget::Fixed(FixedScalar::F64),
    ];

    for profile in [
        NumericProfile::STANDARD,
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
    ] {
        let rows: Vec<_> = builtin_evidence_rows_for_profile(profile).collect();

        for row in &rows {
            let looked_up = lookup_builtin_evidence(row.source, row.target, profile)
                .unwrap_or_else(|| panic!("listed row {:?} must look up", row.source));
            assert_eq!(looked_up.fallibility, row.fallibility);
            assert_eq!(looked_up.policy, row.policy);
        }

        // The listing is exactly the row set the lookup answers: no pair is answered without
        // being listed, and no listed pair is absent from the lookup.
        for source in targets {
            for target in targets {
                let listed = rows
                    .iter()
                    .any(|row| row.source == source && row.target == target);
                assert_eq!(
                    listed,
                    lookup_builtin_evidence(source, target, profile).is_some(),
                    "{source:?} -> {target:?} listing and lookup must agree"
                );
            }
        }
    }
}
