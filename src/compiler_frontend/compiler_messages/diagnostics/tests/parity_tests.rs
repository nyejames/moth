//! Reason-key parity: the stable reason keys of family rows match their legacy strings.
//!
//! WHAT: compares the multi-bind and non-exhaustive reason keys with their literal legacy strings.
//! WHY: producers switch to these rows in P4.6, so the stable keys must agree before then.

use crate::compiler_frontend::compiler_messages::diagnostic_payload::reason_keys::StableReasonKey;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::InvalidMultiBindCode;
use crate::compiler_frontend::compiler_messages::{
    InvalidMultiBindReason, NonExhaustiveMatchReason,
};

#[test]
fn multi_bind_reason_keys_match_the_legacy_strings() {
    let cases = [
        (
            InvalidMultiBindReason::ThisTargetReserved,
            "invalid_multi_bind.this_target_reserved",
        ),
        (
            InvalidMultiBindReason::ExpectedTargetName,
            "invalid_multi_bind.expected_target_name",
        ),
        (
            InvalidMultiBindReason::MissingTargetAfterComma,
            "invalid_multi_bind.missing_target_after_comma",
        ),
        (
            InvalidMultiBindReason::MissingAssignmentOperator,
            "invalid_multi_bind.missing_assignment_operator",
        ),
        (
            InvalidMultiBindReason::InvalidTokenAfterTarget,
            "invalid_multi_bind.invalid_token_after_target",
        ),
        (
            InvalidMultiBindReason::MissingRightHandExpression,
            "invalid_multi_bind.missing_right_hand_expression",
        ),
        (
            InvalidMultiBindReason::MultipleRightHandExpressions,
            "invalid_multi_bind.multiple_right_hand_expressions",
        ),
        (
            InvalidMultiBindReason::MutableTargetNeedsExplicitType,
            "invalid_multi_bind.mutable_target_needs_explicit_type",
        ),
        (
            InvalidMultiBindReason::CompileTimeMarkerOnTarget,
            "invalid_multi_bind.compile_time_marker_on_target",
        ),
        (
            InvalidMultiBindReason::DuplicateTarget,
            "invalid_multi_bind.duplicate_target",
        ),
        (
            InvalidMultiBindReason::UnsupportedRhs,
            "invalid_multi_bind.unsupported_rhs",
        ),
        (
            InvalidMultiBindReason::ExistingTargetMutableMarker,
            "invalid_multi_bind.existing_target_mutable_marker",
        ),
        (
            InvalidMultiBindReason::ExistingTargetImmutable,
            "invalid_multi_bind.existing_target_immutable",
        ),
        (
            InvalidMultiBindReason::ArityMismatch {
                expected: 2,
                found: 1,
            },
            "invalid_multi_bind.arity_mismatch",
        ),
        (
            InvalidMultiBindReason::RhsNotMultiValue,
            "invalid_multi_bind.rhs_not_multi_value",
        ),
    ];
    assert_eq!(cases.len(), 15, "one case per multi-bind reason");
    for (reason, legacy_key) in &cases {
        assert_eq!(reason.stable_reason_key(), *legacy_key);
        assert_eq!(
            InvalidMultiBindCode::of(reason).stable_reason_key(),
            *legacy_key
        );
    }
}

#[test]
fn non_exhaustive_reason_keys_match_the_legacy_strings() {
    assert_eq!(
        NonExhaustiveMatchReason::MissingElseArm.stable_reason_key(),
        "non_exhaustive_match.missing_else_arm"
    );
    assert_eq!(
        NonExhaustiveMatchReason::MissingVariants.stable_reason_key(),
        "non_exhaustive_match.missing_variants"
    );
    assert_eq!(
        NonExhaustiveMatchReason::GuardedArmsRequireElse.stable_reason_key(),
        "non_exhaustive_match.guarded_arms_require_else"
    );
    assert_eq!(
        NonExhaustiveMatchReason::MissingOptionPatterns.stable_reason_key(),
        "non_exhaustive_match.missing_option_patterns"
    );
}
