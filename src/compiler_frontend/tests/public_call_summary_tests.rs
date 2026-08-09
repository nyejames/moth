use super::super::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallSummary,
    PublicCallSummaryTransition, PublicCallTransferEffect, PublicCallTransferEligibility,
    validate_public_call_summary_transition,
};

fn parameter(
    access: PublicCallParameterAccess,
    mutation: PublicCallMutationEffect,
    reactive_effect: PublicCallReactiveEffect,
) -> PublicCallParameterSummary {
    let (transfer_eligibility, transfer_effect) = match access {
        PublicCallParameterAccess::Reactive => (
            PublicCallTransferEligibility::Ineligible,
            PublicCallTransferEffect::NeverConsumes,
        ),
        PublicCallParameterAccess::Shared | PublicCallParameterAccess::Mutable => (
            PublicCallTransferEligibility::Eligible,
            PublicCallTransferEffect::MayConsume,
        ),
    };
    PublicCallParameterSummary {
        access,
        mutation,
        transfer_eligibility,
        transfer_effect,
        reactive_effect,
    }
}

fn summary(
    parameters: Vec<PublicCallParameterSummary>,
    return_alias: FunctionReturnAliasSummary,
) -> PublicCallSummary {
    PublicCallSummary {
        parameters,
        return_alias,
    }
}

#[test]
fn identical_summary_is_an_unchanged_transition() {
    let current = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::None,
        )],
        FunctionReturnAliasSummary::Fresh,
    );

    assert_eq!(
        validate_public_call_summary_transition(&current, &current).unwrap(),
        PublicCallSummaryTransition::Unchanged
    );
}

#[test]
fn mutation_and_reactive_bits_widen_but_do_not_narrow() {
    let no_write = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::None,
        )],
        FunctionReturnAliasSummary::Fresh,
    );
    let writes_and_invalidates = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::Writes,
            PublicCallReactiveEffect::Invalidates,
        )],
        FunctionReturnAliasSummary::Fresh,
    );

    assert_eq!(
        validate_public_call_summary_transition(&no_write, &writes_and_invalidates).unwrap(),
        PublicCallSummaryTransition::Widened
    );
    assert!(validate_public_call_summary_transition(&writes_and_invalidates, &no_write).is_err());
}

#[test]
fn reactive_subscription_can_widen_to_both_effects() {
    let subscribes = summary(
        vec![parameter(
            PublicCallParameterAccess::Reactive,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::Subscribes,
        )],
        FunctionReturnAliasSummary::Fresh,
    );
    let both = summary(
        vec![parameter(
            PublicCallParameterAccess::Reactive,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::SubscribesAndInvalidates,
        )],
        FunctionReturnAliasSummary::Fresh,
    );

    assert_eq!(
        validate_public_call_summary_transition(&subscribes, &both).unwrap(),
        PublicCallSummaryTransition::Widened
    );
    assert!(validate_public_call_summary_transition(&both, &subscribes).is_err());
}

#[test]
fn every_reactive_effect_transition_follows_bitwise_inclusion() {
    let effects = [
        PublicCallReactiveEffect::None,
        PublicCallReactiveEffect::Subscribes,
        PublicCallReactiveEffect::Invalidates,
        PublicCallReactiveEffect::SubscribesAndInvalidates,
    ];

    for previous_effect in effects {
        for next_effect in effects {
            let previous = summary(
                vec![parameter(
                    PublicCallParameterAccess::Reactive,
                    PublicCallMutationEffect::NoWrite,
                    previous_effect,
                )],
                FunctionReturnAliasSummary::Fresh,
            );
            let next = summary(
                vec![parameter(
                    PublicCallParameterAccess::Reactive,
                    PublicCallMutationEffect::NoWrite,
                    next_effect,
                )],
                FunctionReturnAliasSummary::Fresh,
            );
            let (previous_subscribes, previous_invalidates) = reactive_bits(previous_effect);
            let (next_subscribes, next_invalidates) = reactive_bits(next_effect);
            let allowed = (!previous_subscribes || next_subscribes)
                && (!previous_invalidates || next_invalidates);

            let transition = validate_public_call_summary_transition(&previous, &next);
            assert_eq!(transition.is_ok(), allowed);
            if allowed {
                assert_eq!(
                    transition.unwrap(),
                    if previous_effect == next_effect {
                        PublicCallSummaryTransition::Unchanged
                    } else {
                        PublicCallSummaryTransition::Widened
                    }
                );
            }
        }
    }
}

fn reactive_bits(effect: PublicCallReactiveEffect) -> (bool, bool) {
    match effect {
        PublicCallReactiveEffect::None => (false, false),
        PublicCallReactiveEffect::Subscribes => (true, false),
        PublicCallReactiveEffect::Invalidates => (false, true),
        PublicCallReactiveEffect::SubscribesAndInvalidates => (true, true),
    }
}

#[test]
fn return_aliases_widen_from_fresh_to_supersets_or_unknown() {
    let fresh = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::None,
        )],
        FunctionReturnAliasSummary::Fresh,
    );
    let aliases_parameter = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::None,
        )],
        FunctionReturnAliasSummary::AliasParams(vec![0]),
    );
    let unknown = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::None,
        )],
        FunctionReturnAliasSummary::Unknown,
    );

    assert_eq!(
        validate_public_call_summary_transition(&fresh, &aliases_parameter).unwrap(),
        PublicCallSummaryTransition::Widened
    );
    assert_eq!(
        validate_public_call_summary_transition(&aliases_parameter, &unknown).unwrap(),
        PublicCallSummaryTransition::Widened
    );
    assert!(validate_public_call_summary_transition(&unknown, &aliases_parameter).is_err());
}

#[test]
fn alias_parameter_sets_must_grow_by_subset() {
    let parameters = vec![
        parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::None,
        ),
        parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::None,
        ),
    ];
    let one = summary(
        parameters.clone(),
        FunctionReturnAliasSummary::AliasParams(vec![0]),
    );
    let superset = summary(
        parameters.clone(),
        FunctionReturnAliasSummary::AliasParams(vec![0, 1]),
    );
    let incomparable = summary(
        parameters.clone(),
        FunctionReturnAliasSummary::AliasParams(vec![1]),
    );

    assert_eq!(
        validate_public_call_summary_transition(&one, &superset).unwrap(),
        PublicCallSummaryTransition::Widened
    );
    assert!(validate_public_call_summary_transition(&superset, &one).is_err());
    assert!(validate_public_call_summary_transition(&one, &incomparable).is_err());
}

#[test]
fn invariant_parameter_access_cannot_change() {
    let mutable = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::None,
        )],
        FunctionReturnAliasSummary::Fresh,
    );
    let shared = summary(
        vec![parameter(
            PublicCallParameterAccess::Shared,
            PublicCallMutationEffect::NoWrite,
            PublicCallReactiveEffect::None,
        )],
        FunctionReturnAliasSummary::Fresh,
    );

    assert!(validate_public_call_summary_transition(&mutable, &shared).is_err());
}

#[test]
fn transfer_invariants_and_parameter_count_cannot_change() {
    let current_parameter = parameter(
        PublicCallParameterAccess::Mutable,
        PublicCallMutationEffect::NoWrite,
        PublicCallReactiveEffect::None,
    );
    let current = summary(
        vec![current_parameter.clone()],
        FunctionReturnAliasSummary::Fresh,
    );

    let mut transfer_eligibility_changed = current_parameter.clone();
    transfer_eligibility_changed.transfer_eligibility = PublicCallTransferEligibility::Ineligible;
    assert!(
        validate_public_call_summary_transition(
            &current,
            &summary(
                vec![transfer_eligibility_changed],
                FunctionReturnAliasSummary::Fresh,
            )
        )
        .is_err()
    );

    let mut transfer_effect_changed = current_parameter;
    transfer_effect_changed.transfer_effect = PublicCallTransferEffect::NeverConsumes;
    assert!(
        validate_public_call_summary_transition(
            &current,
            &summary(
                vec![transfer_effect_changed],
                FunctionReturnAliasSummary::Fresh
            )
        )
        .is_err()
    );

    let extra_parameter = parameter(
        PublicCallParameterAccess::Mutable,
        PublicCallMutationEffect::NoWrite,
        PublicCallReactiveEffect::None,
    );
    assert!(
        validate_public_call_summary_transition(
            &current,
            &summary(
                vec![current.parameters[0].clone(), extra_parameter],
                FunctionReturnAliasSummary::Fresh,
            )
        )
        .is_err()
    );
}

#[test]
fn invalid_alias_shape_is_rejected_before_transition() {
    let current = summary(
        vec![
            parameter(
                PublicCallParameterAccess::Mutable,
                PublicCallMutationEffect::NoWrite,
                PublicCallReactiveEffect::None,
            ),
            parameter(
                PublicCallParameterAccess::Mutable,
                PublicCallMutationEffect::NoWrite,
                PublicCallReactiveEffect::None,
            ),
        ],
        FunctionReturnAliasSummary::AliasParams(vec![0]),
    );
    let invalid = summary(
        current.parameters.clone(),
        FunctionReturnAliasSummary::AliasParams(vec![1, 0]),
    );

    assert!(validate_public_call_summary_transition(&current, &invalid).is_err());

    assert!(
        validate_public_call_summary_transition(
            &current,
            &summary(
                current.parameters.clone(),
                FunctionReturnAliasSummary::AliasParams(vec![0, 0]),
            )
        )
        .is_err()
    );
}
