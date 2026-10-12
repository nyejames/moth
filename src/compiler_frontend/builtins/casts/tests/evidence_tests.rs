//! Evidence table unit tests for the builtin cast surface.
//!
//! WHAT: covers numeric evidence classification, exact-width rows,
//!      `Byte` evidence, and the complete row listing.
//! WHY: evidence class is fixed before HIR from the complete source/target
//!      types. Tests here pin the classification so it cannot drift silently.

use crate::compiler_frontend::builtins::casts::evidence::{
    lookup_builtin_evidence, numeric_conversion_fallibility,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

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
) {
    let row = lookup_builtin_evidence(source, target)
        .unwrap_or_else(|| panic!("{source:?} -> {target:?} evidence should exist"));
    assert_eq!(row.fallibility, fallibility);
    assert_eq!(row.policy, numeric_policy(source_scalar, target_scalar));
}

#[test]
fn fixed_integer_pairs_use_complete_source_ranges() {
    // `U8` fits `U16` and `I16` completely; the reverse and `I8 -> U8` do not.
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::U8),
            NumericScalar::Fixed(FixedScalar::U16),
        ),
        BuiltinCastFallibility::Infallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::U8),
            NumericScalar::Fixed(FixedScalar::I16),
        ),
        BuiltinCastFallibility::Infallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::U16),
            NumericScalar::Fixed(FixedScalar::U8),
        ),
        BuiltinCastFallibility::Fallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::I8),
            NumericScalar::Fixed(FixedScalar::U8),
        ),
        BuiltinCastFallibility::Fallible,
    );
}

#[test]
fn fixed_integer_narrowing_across_widths_is_fallible() {
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::I64),
            NumericScalar::Fixed(FixedScalar::I32),
        ),
        BuiltinCastFallibility::Fallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::U32),
            NumericScalar::Fixed(FixedScalar::I64),
        ),
        BuiltinCastFallibility::Infallible,
    );
}

#[test]
fn fixed_integer_to_float_is_infallible_except_f16_range() {
    assert_numeric_row(
        fixed_target(FixedScalar::I32),
        fixed_target(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::I32),
        NumericScalar::Fixed(FixedScalar::F64),
        BuiltinCastFallibility::Infallible,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::I64),
        fixed_target(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::I64),
        NumericScalar::Fixed(FixedScalar::F64),
        BuiltinCastFallibility::Infallible,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::U64),
        fixed_target(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::U64),
        NumericScalar::Fixed(FixedScalar::F32),
        BuiltinCastFallibility::Infallible,
    );

    // `I16` stays finite in `F16`; `U16` and `I32` do not.
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::I16),
            NumericScalar::Fixed(FixedScalar::F16),
        ),
        BuiltinCastFallibility::Infallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::U16),
            NumericScalar::Fixed(FixedScalar::F16),
        ),
        BuiltinCastFallibility::Fallible,
    );
    assert_eq!(
        numeric_conversion_fallibility(
            NumericScalar::Fixed(FixedScalar::I32),
            NumericScalar::Fixed(FixedScalar::F16),
        ),
        BuiltinCastFallibility::Fallible,
    );
}

#[test]
fn float_pairs_compare_precision_not_spelling() {
    // Narrowing is fallible.
    assert_numeric_row(
        fixed_target(FixedScalar::F64),
        fixed_target(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::F32),
        BuiltinCastFallibility::Fallible,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::F32),
        fixed_target(FixedScalar::F16),
        NumericScalar::Fixed(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::F16),
        BuiltinCastFallibility::Fallible,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::F64),
        fixed_target(FixedScalar::F16),
        NumericScalar::Fixed(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::F16),
        BuiltinCastFallibility::Fallible,
    );
    // Widening is infallible.
    assert_numeric_row(
        fixed_target(FixedScalar::F32),
        fixed_target(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::F64),
        BuiltinCastFallibility::Infallible,
    );
}

#[test]
fn float_to_int_is_always_fallible_with_numeric_policy() {
    assert_numeric_row(
        fixed_target(FixedScalar::F64),
        fixed_target(FixedScalar::I32),
        NumericScalar::Fixed(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::I32),
        BuiltinCastFallibility::Fallible,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::F64),
        fixed_target(FixedScalar::I64),
        NumericScalar::Fixed(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::I64),
        BuiltinCastFallibility::Fallible,
    );
}

#[test]
fn byte_evidence_covers_only_u8_pair() {
    let row = lookup_builtin_evidence(
        fixed_target(FixedScalar::Byte),
        fixed_target(FixedScalar::U8),
    )
    .expect("Byte -> U8 evidence should exist");
    assert_eq!(row.fallibility, BuiltinCastFallibility::Infallible);
    assert_eq!(row.policy, BuiltinCastPolicyId::ByteToU8);

    let row = lookup_builtin_evidence(
        fixed_target(FixedScalar::U8),
        fixed_target(FixedScalar::Byte),
    )
    .expect("U8 -> Byte evidence should exist");
    assert_eq!(row.fallibility, BuiltinCastFallibility::Infallible);
    assert_eq!(row.policy, BuiltinCastPolicyId::U8ToByte);

    assert!(
        lookup_builtin_evidence(
            fixed_target(FixedScalar::Byte),
            fixed_target(FixedScalar::U16),
        )
        .is_none()
    );
    assert!(
        lookup_builtin_evidence(
            fixed_target(FixedScalar::I32),
            fixed_target(FixedScalar::Byte),
        )
        .is_none()
    );
}

#[test]
fn numeric_text_evidence_covers_every_numeric_domain_in_both_directions() {
    // Formatting is infallible for every numeric domain, and the row carries the domain so the
    // policy can format at that domain's own precision.
    let formatting_cases = [
        (
            fixed_target(FixedScalar::I32),
            NumericScalar::Fixed(FixedScalar::I32),
        ),
        (
            fixed_target(FixedScalar::F64),
            NumericScalar::Fixed(FixedScalar::F64),
        ),
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
        let row = lookup_builtin_evidence(source, BuiltinCastTarget::String)
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

    // Parsing is fallible for every numeric domain.
    let parsing_cases = [
        (
            NumericScalar::Fixed(FixedScalar::I32),
            fixed_target(FixedScalar::I32),
        ),
        (
            NumericScalar::Fixed(FixedScalar::F64),
            fixed_target(FixedScalar::F64),
        ),
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
        let row = lookup_builtin_evidence(BuiltinCastTarget::String, target)
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
fn fixed_numeric_text_evidence_uses_each_exact_target_identity() {
    for fixed_scalar in FixedScalar::ALL
        .into_iter()
        .filter(|scalar| *scalar != FixedScalar::Byte)
    {
        let target = fixed_target(fixed_scalar);
        let scalar = NumericScalar::Fixed(fixed_scalar);

        let formatting = lookup_builtin_evidence(target, BuiltinCastTarget::String)
            .unwrap_or_else(|| panic!("{target:?} -> String evidence should exist"));
        assert_eq!(formatting.fallibility, BuiltinCastFallibility::Infallible);
        assert_eq!(
            formatting.policy,
            BuiltinCastPolicyId::NumericToString(scalar)
        );

        let parsing = lookup_builtin_evidence(BuiltinCastTarget::String, target)
            .unwrap_or_else(|| panic!("String -> {target:?} evidence should exist"));
        assert_eq!(parsing.fallibility, BuiltinCastFallibility::Fallible);
        assert_eq!(parsing.policy, BuiltinCastPolicyId::StringToNumeric(scalar));
    }
}

#[test]
fn number_cast_evidence_classifies_exact_pair_boundaries() {
    let scale_zero = NumberScale::new(0).expect("zero is a valid Dec scale");
    let scale_two = NumberScale::new(2).expect("two is a valid Dec scale");
    let scale_three = NumberScale::new(3).expect("three is a valid Dec scale");

    let cases = [
        (
            fixed_target(FixedScalar::I32),
            BuiltinCastTarget::Number(scale_two),
            BuiltinCastFallibility::Infallible,
            numeric_policy(
                NumericScalar::Fixed(FixedScalar::I32),
                NumericScalar::Number(scale_two),
            ),
        ),
        (
            fixed_target(FixedScalar::U64),
            BuiltinCastTarget::Number(scale_two),
            BuiltinCastFallibility::Infallible,
            numeric_policy(
                NumericScalar::Fixed(FixedScalar::U64),
                NumericScalar::Number(scale_two),
            ),
        ),
        (
            BuiltinCastTarget::Number(scale_two),
            BuiltinCastTarget::Number(scale_three),
            BuiltinCastFallibility::Infallible,
            numeric_policy(
                NumericScalar::Number(scale_two),
                NumericScalar::Number(scale_three),
            ),
        ),
        (
            BuiltinCastTarget::Number(scale_three),
            BuiltinCastTarget::Number(scale_two),
            BuiltinCastFallibility::Fallible,
            numeric_policy(
                NumericScalar::Number(scale_three),
                NumericScalar::Number(scale_two),
            ),
        ),
        (
            BuiltinCastTarget::Number(scale_two),
            fixed_target(FixedScalar::I32),
            BuiltinCastFallibility::Fallible,
            numeric_policy(
                NumericScalar::Number(scale_two),
                NumericScalar::Fixed(FixedScalar::I32),
            ),
        ),
        (
            BuiltinCastTarget::String,
            BuiltinCastTarget::Number(scale_zero),
            BuiltinCastFallibility::Fallible,
            BuiltinCastPolicyId::StringToNumeric(NumericScalar::Number(scale_zero)),
        ),
        (
            BuiltinCastTarget::Number(scale_two),
            BuiltinCastTarget::String,
            BuiltinCastFallibility::Infallible,
            BuiltinCastPolicyId::NumericToString(NumericScalar::Number(scale_two)),
        ),
    ];

    for (source, target, fallibility, policy) in cases {
        let row = lookup_builtin_evidence(source, target)
            .unwrap_or_else(|| panic!("{source:?} -> {target:?} evidence should exist"));
        assert_eq!(row.fallibility, fallibility);
        assert_eq!(row.policy, policy);
    }

    for (source, target) in [
        (
            BuiltinCastTarget::Number(scale_two),
            BuiltinCastTarget::Number(scale_two),
        ),
        (
            BuiltinCastTarget::Number(scale_two),
            fixed_target(FixedScalar::F64),
        ),
        (
            fixed_target(FixedScalar::F64),
            BuiltinCastTarget::Number(scale_two),
        ),
        (
            fixed_target(FixedScalar::Byte),
            BuiltinCastTarget::Number(scale_two),
        ),
        (
            BuiltinCastTarget::Number(scale_two),
            fixed_target(FixedScalar::Byte),
        ),
    ] {
        assert!(
            lookup_builtin_evidence(source, target).is_none(),
            "{source:?} -> {target:?} must not be a builtin cast"
        );
    }
}

#[test]
fn byte_has_no_numeric_text_evidence() {
    // `Byte` is an octet, not a number: its text conversion composes through `U8`, so neither
    // direction has a numeric text row.
    assert!(
        lookup_builtin_evidence(fixed_target(FixedScalar::Byte), BuiltinCastTarget::String,)
            .is_none()
    );
    assert!(
        lookup_builtin_evidence(BuiltinCastTarget::String, fixed_target(FixedScalar::Byte),)
            .is_none()
    );
}

#[test]
fn same_target_has_no_evidence_and_non_numeric_pairs_fall_through() {
    assert!(
        lookup_builtin_evidence(
            fixed_target(FixedScalar::I32),
            fixed_target(FixedScalar::I32),
        )
        .is_none()
    );
    let row = lookup_builtin_evidence(BuiltinCastTarget::String, BuiltinCastTarget::Bool)
        .expect("String -> Bool evidence should exist");
    assert_eq!(row.fallibility, BuiltinCastFallibility::Fallible);
    assert_eq!(row.policy, BuiltinCastPolicyId::StringToBool);
    assert!(
        lookup_builtin_evidence(BuiltinCastTarget::Bool, fixed_target(FixedScalar::I32),).is_none()
    );
}

#[test]
fn fixed_u32_evidence_classifies_narrowing_and_text_pairs() {
    // U32 participates in the numeric evidence matrix with exact-width rows —
    // fallible narrowing to I32, infallible widening to F64, text in both
    // directions — while same-type casts stay invalid.
    assert_numeric_row(
        fixed_target(FixedScalar::U32),
        fixed_target(FixedScalar::I32),
        NumericScalar::Fixed(FixedScalar::U32),
        NumericScalar::Fixed(FixedScalar::I32),
        BuiltinCastFallibility::Fallible,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::I32),
        fixed_target(FixedScalar::U32),
        NumericScalar::Fixed(FixedScalar::I32),
        NumericScalar::Fixed(FixedScalar::U32),
        BuiltinCastFallibility::Fallible,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::U32),
        fixed_target(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::U32),
        NumericScalar::Fixed(FixedScalar::F64),
        BuiltinCastFallibility::Infallible,
    );
    assert_numeric_row(
        fixed_target(FixedScalar::F64),
        fixed_target(FixedScalar::U32),
        NumericScalar::Fixed(FixedScalar::F64),
        NumericScalar::Fixed(FixedScalar::U32),
        BuiltinCastFallibility::Fallible,
    );
    assert!(
        lookup_builtin_evidence(
            fixed_target(FixedScalar::U32),
            fixed_target(FixedScalar::U32),
        )
        .is_none()
    );

    let formatted =
        lookup_builtin_evidence(fixed_target(FixedScalar::U32), BuiltinCastTarget::String)
            .expect("U32 -> String evidence should exist");
    assert_eq!(formatted.fallibility, BuiltinCastFallibility::Infallible);
    assert_eq!(
        formatted.policy,
        BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::U32))
    );
    let parsed = lookup_builtin_evidence(BuiltinCastTarget::String, fixed_target(FixedScalar::U32))
        .expect("String -> U32 evidence should exist");
    assert_eq!(parsed.fallibility, BuiltinCastFallibility::Fallible);
    assert_eq!(
        parsed.policy,
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::U32))
    );
}

#[test]
fn fixed_uint_classification_follows_complete_source_ranges() {
    // A fixed-width cast is infallible only when the complete source range fits
    // the target: U32 holds u32::MAX beside I64/U64 while U64 and I32 keep
    // their fallible narrowings.

    let cases = [
        (
            fixed_target(FixedScalar::U32),
            fixed_target(FixedScalar::I64),
            NumericScalar::Fixed(FixedScalar::U32),
            NumericScalar::Fixed(FixedScalar::I64),
            BuiltinCastFallibility::Infallible,
        ),
        (
            fixed_target(FixedScalar::U32),
            fixed_target(FixedScalar::U64),
            NumericScalar::Fixed(FixedScalar::U32),
            NumericScalar::Fixed(FixedScalar::U64),
            BuiltinCastFallibility::Infallible,
        ),
        (
            fixed_target(FixedScalar::U64),
            fixed_target(FixedScalar::U32),
            NumericScalar::Fixed(FixedScalar::U64),
            NumericScalar::Fixed(FixedScalar::U32),
            BuiltinCastFallibility::Fallible,
        ),
        (
            fixed_target(FixedScalar::I32),
            fixed_target(FixedScalar::U32),
            NumericScalar::Fixed(FixedScalar::I32),
            NumericScalar::Fixed(FixedScalar::U32),
            BuiltinCastFallibility::Fallible,
        ),
    ];

    for (source, target, source_scalar, target_scalar, fallibility) in cases {
        assert_numeric_row(source, target, source_scalar, target_scalar, fallibility);
    }

    // Fixed binary floats round once: U32 widens infallibly into F32/F64
    // while the reverse stays fallible.
    for scalar in [FixedScalar::F32, FixedScalar::F64] {
        assert_numeric_row(
            fixed_target(FixedScalar::U32),
            fixed_target(scalar),
            NumericScalar::Fixed(FixedScalar::U32),
            NumericScalar::Fixed(scalar),
            BuiltinCastFallibility::Infallible,
        );
        assert_numeric_row(
            fixed_target(scalar),
            fixed_target(FixedScalar::U32),
            NumericScalar::Fixed(scalar),
            NumericScalar::Fixed(FixedScalar::U32),
            BuiltinCastFallibility::Fallible,
        );
    }

    // U32 has no Byte/Char/Bool evidence at all.
    for target in [
        BuiltinCastTarget::Fixed(FixedScalar::Byte),
        BuiltinCastTarget::Char,
        BuiltinCastTarget::Bool,
    ] {
        assert!(
            lookup_builtin_evidence(fixed_target(FixedScalar::U32), target).is_none(),
            "U32 -> {target:?} must have no evidence"
        );
    }
}
