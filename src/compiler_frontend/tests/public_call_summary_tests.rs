use super::super::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallSummary, PublicCallSummaryTransition,
    PublicCallTransferEffect, validate_public_call_summary_transition,
};

fn parameter(
    access: PublicCallParameterAccess,
    mutation: PublicCallMutationEffect,
) -> PublicCallParameterSummary {
    PublicCallParameterSummary {
        access,
        mutation,
        transfer_effect: PublicCallTransferEffect::MayConsume,
    }
}

fn summary(
    parameters: Vec<PublicCallParameterSummary>,
    return_alias: FunctionReturnAliasSummary,
) -> PublicCallSummary {
    PublicCallSummary {
        parameters,
        return_alias,
        escapes_builtin_failure: false,
    }
}

#[test]
fn identical_summary_is_an_unchanged_transition() {
    let current = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
        )],
        FunctionReturnAliasSummary::Fresh,
    );

    assert_eq!(
        validate_public_call_summary_transition(&current, &current).unwrap(),
        PublicCallSummaryTransition::Unchanged
    );
}

#[test]
fn builtin_failure_bit_widens_but_never_narrows() {
    let initial = summary(vec![], FunctionReturnAliasSummary::Fresh);
    let mut escaping = initial.clone();
    escaping.escapes_builtin_failure = true;

    assert_eq!(
        validate_public_call_summary_transition(&initial, &escaping).unwrap(),
        PublicCallSummaryTransition::Widened
    );
    assert_eq!(
        validate_public_call_summary_transition(&escaping, &escaping).unwrap(),
        PublicCallSummaryTransition::Unchanged
    );
    assert!(validate_public_call_summary_transition(&escaping, &initial).is_err());
}

#[test]
fn mutation_effect_widens_but_does_not_narrow() {
    let no_write = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
        )],
        FunctionReturnAliasSummary::Fresh,
    );
    let writes = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::Writes,
        )],
        FunctionReturnAliasSummary::Fresh,
    );

    assert_eq!(
        validate_public_call_summary_transition(&no_write, &writes).unwrap(),
        PublicCallSummaryTransition::Widened
    );
    assert!(validate_public_call_summary_transition(&writes, &no_write).is_err());
}

#[test]
fn return_aliases_widen_from_fresh_to_supersets_or_unknown() {
    let parameters = vec![parameter(
        PublicCallParameterAccess::Mutable,
        PublicCallMutationEffect::NoWrite,
    )];
    let fresh = summary(parameters.clone(), FunctionReturnAliasSummary::Fresh);
    let aliases_parameter = summary(
        parameters.clone(),
        FunctionReturnAliasSummary::AliasParams(vec![0]),
    );
    let unknown = summary(parameters, FunctionReturnAliasSummary::Unknown);

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
        ),
        parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
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
    let incomparable = summary(parameters, FunctionReturnAliasSummary::AliasParams(vec![1]));

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
        )],
        FunctionReturnAliasSummary::Fresh,
    );
    let shared = summary(
        vec![parameter(
            PublicCallParameterAccess::Shared,
            PublicCallMutationEffect::NoWrite,
        )],
        FunctionReturnAliasSummary::Fresh,
    );

    assert!(validate_public_call_summary_transition(&mutable, &shared).is_err());
}

#[test]
fn invalid_transfer_effect_and_parameter_count_are_rejected() {
    let current = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
        )],
        FunctionReturnAliasSummary::Fresh,
    );

    let mut invalid_transfer = current.parameters[0].clone();
    invalid_transfer.transfer_effect = PublicCallTransferEffect::AlwaysConsumes;
    assert!(
        validate_public_call_summary_transition(
            &current,
            &summary(vec![invalid_transfer], FunctionReturnAliasSummary::Fresh)
        )
        .is_err()
    );

    let extra_parameter = parameter(
        PublicCallParameterAccess::Mutable,
        PublicCallMutationEffect::NoWrite,
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
            ),
            parameter(
                PublicCallParameterAccess::Mutable,
                PublicCallMutationEffect::NoWrite,
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

#[test]
fn empty_alias_params_summary_is_rejected() {
    let current = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
        )],
        FunctionReturnAliasSummary::Fresh,
    );
    let empty = summary(
        current.parameters.clone(),
        FunctionReturnAliasSummary::AliasParams(Vec::new()),
    );

    assert!(validate_public_call_summary_transition(&current, &empty).is_err());
}
