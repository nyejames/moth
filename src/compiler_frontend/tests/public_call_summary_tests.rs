use super::super::public_call_summary::{
    CallAllocationFacts, CallAllocationSource, CallField, CallFieldHir, CallLaneSummary,
    CallOutputPath, CallParameterPath, CallPathSource, CallPathType, CallResultPath, CallSharing,
    CallSharingCondition, FunctionReturnAliasSummary, PublicCallMutationEffect,
    PublicCallParameterAccess, PublicCallParameterSummary, PublicCallSummary,
    PublicCallSummaryTransition, PublicCallTransferEffect, validate_public_call_allocation_layout,
    validate_public_call_summary_transition,
};
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::hir::ids::{FieldId, StructId};
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::symbols::path_interner::PathId;

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
        uses_fast_semantics: false,
        may_trap: false,
        may_fail: false,
    }
}

#[test]
fn identical_summary_is_an_unchanged_transition() {
    let current = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
        )],
        known_success(CallLaneSummary::default()),
    );

    assert_eq!(
        validate_public_call_summary_transition(&current, &current).unwrap(),
        PublicCallSummaryTransition::Unchanged
    );
}

#[test]
fn builtin_failure_bit_widens_but_never_narrows() {
    let initial = summary(vec![], known_success(CallLaneSummary::default()));
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
        known_success(CallLaneSummary::default()),
    );
    let writes = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::Writes,
        )],
        known_success(CallLaneSummary::default()),
    );

    assert_eq!(
        validate_public_call_summary_transition(&no_write, &writes).unwrap(),
        PublicCallSummaryTransition::Widened
    );
    assert!(validate_public_call_summary_transition(&writes, &no_write).is_err());
}

fn known_success(lane: CallLaneSummary) -> FunctionReturnAliasSummary {
    FunctionReturnAliasSummary::Known(Box::new(CallAllocationFacts {
        success: CallLaneSummary {
            reachable: true,
            ..lane
        },
        error: CallLaneSummary::default(),
    }))
}

fn result(slot: usize, path: Vec<CallField>, source: CallAllocationSource) -> CallPathSource {
    CallPathSource {
        output: CallOutputPath::Result(CallResultPath { slot, path }),
        source,
    }
}

fn mutable_parameters(count: usize) -> Vec<PublicCallParameterSummary> {
    vec![
        parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite
        );
        count
    ]
}

#[test]
fn allocation_widening_keeps_reachability_and_every_lane_component() {
    let parameters = mutable_parameters(2);
    let pending = summary(parameters.clone(), FunctionReturnAliasSummary::Pending);
    let bottom = summary(
        parameters.clone(),
        FunctionReturnAliasSummary::Known(Box::default()),
    );
    let empty = summary(
        parameters.clone(),
        known_success(CallLaneSummary::default()),
    );
    let left = result(
        0,
        vec![CallField::Field(0)],
        CallAllocationSource::Parameter(CallParameterPath {
            index: 0,
            path: vec![CallField::Elem],
        }),
    );
    let root = result(0, vec![], CallAllocationSource::Fresh);
    let right = result(1, vec![], CallAllocationSource::Fresh);
    let store = CallPathSource {
        output: CallOutputPath::Store {
            target: CallParameterPath {
                index: 1,
                path: vec![CallField::Field(0)],
            },
            path: vec![],
        },
        source: CallAllocationSource::Fresh,
    };
    let lane = CallLaneSummary {
        reachable: true,
        sources: vec![root.clone(), left.clone(), right.clone()],
        sharing: vec![CallSharing {
            left: right.output.clone(),
            right: store.output.clone(),
            condition: CallSharingCondition::Always,
        }],
        stores: vec![store],
    };
    let precise = summary(parameters, known_success(lane.clone()));

    for (previous, next) in [(&pending, &bottom), (&bottom, &empty), (&empty, &precise)] {
        assert_eq!(
            validate_public_call_summary_transition(previous, next).unwrap(),
            PublicCallSummaryTransition::Widened
        );
        assert!(validate_public_call_summary_transition(next, previous).is_err());
    }
    let fresh_child = result(0, vec![CallField::Field(0)], CallAllocationSource::Fresh);

    let change_source = CallLaneSummary {
        sources: vec![root.clone(), fresh_child.clone(), right.clone()],
        ..lane.clone()
    };
    let no_source = CallLaneSummary {
        sources: vec![root.clone(), right.clone()],
        ..lane.clone()
    };
    let no_sharing = CallLaneSummary {
        sharing: vec![],
        ..lane.clone()
    };
    let no_store = CallLaneSummary {
        stores: vec![],
        ..no_sharing.clone()
    };
    let add_source = CallLaneSummary {
        sources: vec![root, left, fresh_child, right],
        ..lane.clone()
    };
    let only_sharing_removed = summary(
        precise.parameters.clone(),
        known_success(no_sharing.clone()),
    );
    let store_also_removed = summary(precise.parameters.clone(), known_success(no_store));
    assert!(
        validate_public_call_summary_transition(&only_sharing_removed, &store_also_removed)
            .is_err()
    );
    for narrowed in [change_source, no_source, no_sharing] {
        let narrowed = summary(precise.parameters.clone(), known_success(narrowed));
        assert!(validate_public_call_summary_transition(&precise, &narrowed).is_err());
    }
    let widened = summary(precise.parameters.clone(), known_success(add_source));
    assert_eq!(
        validate_public_call_summary_transition(&precise, &widened).unwrap(),
        PublicCallSummaryTransition::Widened
    );

    let both_lanes = summary(
        precise.parameters.clone(),
        FunctionReturnAliasSummary::Known(Box::new(CallAllocationFacts {
            success: lane,
            error: CallLaneSummary {
                reachable: true,
                ..CallLaneSummary::default()
            },
        })),
    );
    assert_eq!(
        validate_public_call_summary_transition(&precise, &both_lanes).unwrap(),
        PublicCallSummaryTransition::Widened
    );
    assert!(validate_public_call_summary_transition(&both_lanes, &precise).is_err());
}

#[test]
fn invariant_parameter_access_cannot_change() {
    let mutable = summary(
        vec![parameter(
            PublicCallParameterAccess::Mutable,
            PublicCallMutationEffect::NoWrite,
        )],
        known_success(CallLaneSummary::default()),
    );
    let shared = summary(
        vec![parameter(
            PublicCallParameterAccess::Shared,
            PublicCallMutationEffect::NoWrite,
        )],
        known_success(CallLaneSummary::default()),
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
        known_success(CallLaneSummary::default()),
    );

    let mut invalid_transfer = current.parameters[0].clone();
    invalid_transfer.transfer_effect = PublicCallTransferEffect::AlwaysConsumes;
    assert!(
        validate_public_call_summary_transition(
            &current,
            &summary(vec![invalid_transfer], current.return_alias.clone())
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
                current.return_alias.clone(),
            )
        )
        .is_err()
    );
}

#[test]
fn validates_source_rows_and_topology_before_accepting_even_an_unchanged_summary() {
    let first = result(0, vec![], CallAllocationSource::Fresh);
    let second = result(1, vec![], CallAllocationSource::Fresh);
    let valid_edge = CallSharing {
        left: first.output.clone(),
        right: second.output.clone(),
        condition: CallSharingCondition::Always,
    };
    let unreachable_endpoint = CallSharing {
        right: result(2, vec![], CallAllocationSource::Fresh).output,
        ..valid_edge.clone()
    };
    let reversed_edge = CallSharing {
        left: second.output.clone(),
        right: first.output.clone(),
        ..valid_edge.clone()
    };
    let reversed_atom = CallSharing {
        condition: CallSharingCondition::Parameters(
            CallParameterPath {
                index: 1,
                path: vec![],
            },
            CallParameterPath {
                index: 0,
                path: vec![],
            },
        ),
        ..valid_edge.clone()
    };
    let same_endpoint = CallSharing {
        right: first.output.clone(),
        ..valid_edge.clone()
    };
    let well_formed = CallLaneSummary {
        sources: vec![first.clone(), second.clone()],
        sharing: vec![valid_edge.clone()],
        ..CallLaneSummary::default()
    };
    let make = |lane| summary(mutable_parameters(2), known_success(lane));
    let valid = make(well_formed.clone());
    assert_eq!(
        validate_public_call_summary_transition(&valid, &valid).unwrap(),
        PublicCallSummaryTransition::Unchanged
    );
    for sharing in [
        vec![valid_edge.clone(), valid_edge],
        vec![reversed_edge],
        vec![reversed_atom],
        vec![same_endpoint],
        vec![unreachable_endpoint],
    ] {
        let invalid = make(CallLaneSummary {
            sharing,
            ..well_formed.clone()
        });
        assert!(validate_public_call_summary_transition(&invalid, &invalid).is_err());
    }
    for sources in [vec![second, first.clone()], vec![first.clone(), first]] {
        let invalid = make(CallLaneSummary {
            sources,
            ..CallLaneSummary::default()
        });
        assert!(validate_public_call_summary_transition(&invalid, &invalid).is_err());
    }
}

#[test]
fn parameter_source_rows_must_be_sorted_unique_and_in_bounds() {
    let make = |indices: Vec<usize>| {
        summary(
            mutable_parameters(2),
            known_success(CallLaneSummary {
                sources: indices
                    .into_iter()
                    .map(|index| {
                        result(
                            0,
                            vec![],
                            CallAllocationSource::Parameter(CallParameterPath {
                                index,
                                path: vec![],
                            }),
                        )
                    })
                    .collect(),
                ..Default::default()
            }),
        )
    };
    let valid = make(vec![0, 1]);
    assert_eq!(
        validate_public_call_summary_transition(&valid, &valid).unwrap(),
        PublicCallSummaryTransition::Unchanged
    );
    for indices in [vec![1, 0], vec![0, 0], vec![2]] {
        let invalid = make(indices);
        assert!(validate_public_call_summary_transition(&invalid, &invalid).is_err());
    }
}

#[test]
fn store_rows_require_a_mutable_entry_cell_and_connected_descendants() {
    let mut parameters = mutable_parameters(1);
    let target = CallParameterPath {
        index: 0,
        path: vec![CallField::Field(0)],
    };
    let direct = CallPathSource {
        output: CallOutputPath::Store {
            target: target.clone(),
            path: vec![],
        },
        source: CallAllocationSource::Fresh,
    };
    let child = CallPathSource {
        output: CallOutputPath::Store {
            target: target.clone(),
            path: vec![CallField::Elem],
        },
        ..direct.clone()
    };
    let grandchild = CallPathSource {
        output: CallOutputPath::Store {
            target,
            path: vec![CallField::Elem, CallField::Elem],
        },
        ..child.clone()
    };
    let make = |parameters, stores| {
        summary(
            parameters,
            known_success(CallLaneSummary {
                stores,
                ..CallLaneSummary::default()
            }),
        )
    };
    let valid = make(
        parameters.clone(),
        vec![direct.clone(), child.clone(), grandchild.clone()],
    );
    assert_eq!(
        validate_public_call_summary_transition(&valid, &valid).unwrap(),
        PublicCallSummaryTransition::Unchanged
    );
    let orphan = make(parameters.clone(), vec![direct.clone(), grandchild]);
    let error = validate_public_call_summary_transition(&orphan, &orphan).unwrap_err();
    assert!(error.message.contains("no immediate parent row"));

    let root_store = make(
        parameters.clone(),
        vec![CallPathSource {
            output: CallOutputPath::Store {
                target: CallParameterPath {
                    index: 0,
                    path: vec![],
                },
                path: vec![],
            },
            ..direct.clone()
        }],
    );
    assert!(validate_public_call_summary_transition(&root_store, &root_store).is_err());
    parameters[0].access = PublicCallParameterAccess::Shared;
    let shared = make(parameters, vec![direct, child]);
    let error = validate_public_call_summary_transition(&shared, &shared).unwrap_err();
    assert!(
        error
            .message
            .contains("projected cell of a mutable parameter")
    );
}

#[test]
fn explicit_result_descendants_cannot_omit_their_parent() {
    let invalid = summary(
        vec![],
        known_success(CallLaneSummary {
            sources: vec![result(
                0,
                vec![CallField::Field(0)],
                CallAllocationSource::Fresh,
            )],
            ..CallLaneSummary::default()
        }),
    );
    let error = validate_public_call_summary_transition(&invalid, &invalid).unwrap_err();
    assert!(error.message.contains("no immediate parent row"));
}

#[test]
fn unreachable_lanes_cannot_contain_facts() {
    let invalid = summary(
        vec![],
        FunctionReturnAliasSummary::Known(Box::new(CallAllocationFacts {
            success: CallLaneSummary {
                sources: vec![result(0, vec![], CallAllocationSource::OpaqueFresh)],
                ..CallLaneSummary::default()
            },
            error: CallLaneSummary::default(),
        })),
    );
    let error = validate_public_call_summary_transition(&invalid, &invalid).unwrap_err();
    assert!(
        error
            .message
            .contains("unreachable lane has allocation facts")
    );
}

#[test]
fn result_slots_and_reachable_lanes_follow_the_callable_contract() {
    let make_lane = |slot| CallLaneSummary {
        reachable: true,
        sources: vec![result(slot, vec![], CallAllocationSource::Fresh)],
        ..CallLaneSummary::default()
    };
    let validate = |summary: &PublicCallSummary, success_slots, has_error| {
        validate_public_call_allocation_layout(
            summary,
            success_slots,
            has_error,
            |_| None,
            |_, _| Some(CallPathType::Known(0_u8)),
            |_, _| None,
        )
    };
    let some_success = summary(vec![], known_success(make_lane(1)));
    assert!(validate(&some_success, Some(2), false).is_ok());
    assert!(validate(&some_success, Some(1), false).is_err());
    assert!(validate(&some_success, None, false).is_err());
    let error_summary = |slot| {
        summary(
            vec![],
            FunctionReturnAliasSummary::Known(Box::new(CallAllocationFacts {
                success: CallLaneSummary::default(),
                error: make_lane(slot),
            })),
        )
    };
    assert!(validate(&error_summary(0), Some(0), true).is_ok());
    assert!(validate(&error_summary(0), Some(0), false).is_err());
    assert!(validate(&error_summary(1), Some(0), true).is_err());
}

#[test]
fn topology_requires_compatible_typed_endpoints() {
    let first = result(0, vec![], CallAllocationSource::Fresh);
    let second = result(1, vec![], CallAllocationSource::Fresh);
    let summary = summary(
        vec![],
        known_success(CallLaneSummary {
            sharing: vec![CallSharing {
                left: first.output.clone(),
                right: second.output.clone(),
                condition: CallSharingCondition::Always,
            }],
            sources: vec![first, second],
            ..CallLaneSummary::default()
        }),
    );
    let validate = |different_types: bool| {
        validate_public_call_allocation_layout(
            &summary,
            Some(2),
            false,
            |_| None,
            |_, result| Some(CallPathType::Known(different_types && result.slot == 1)),
            |_, _| None,
        )
    };
    assert!(validate(false).is_ok());
    assert!(
        validate(true)
            .unwrap_err()
            .message
            .contains("sharing endpoints")
    );
}

#[test]
fn field_ordinals_resolve_to_the_consumer_local_hir_ids() {
    let register = |types: &mut TypeEnvironment| {
        let member = types.builtins().string;
        let (_, record) = types.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: PathId::ROOT,
            fields: Box::new([FieldDefinition {
                name: PathId::ROOT,
                type_id: member,
                span: None,
            }]),
            generic_parameters: None,
            const_record: false,
        });
        (record, member)
    };
    let mut provider_types = TypeEnvironment::new();
    let (provider_record, provider_member) = register(&mut provider_types);
    let mut consumer_types = TypeEnvironment::new();
    let padding = consumer_types.builtins().bool;
    consumer_types.intern_option(padding);
    let (record, member) = register(&mut consumer_types);
    assert_ne!(provider_record, record);

    let layout = |record, member, id| HirStruct {
        id: StructId(1),
        frontend_type_id: record,
        fields: vec![HirField {
            id: FieldId(id),
            ty: member,
        }],
    };
    let selector = CallField::Field(0);
    let provider_layouts = [layout(provider_record, provider_member, 7)];
    let consumer_layouts = [layout(record, member, 42)];
    assert!(matches!(
        selector.hir_child(provider_record, &provider_types, &provider_layouts, &[]),
        Some((CallFieldHir::Field(7), ty)) if ty == provider_member
    ));
    assert!(matches!(
        selector.hir_child(record, &consumer_types, &consumer_layouts, &[]),
        Some((CallFieldHir::Field(42), ty)) if ty == member
    ));
    assert!(matches!(
        selector.hir_child(record, &consumer_types, &[], &[]),
        Some((CallFieldHir::UnmaterializedField(0), ty)) if ty == member
    ));
    assert!(
        CallField::Field(42)
            .hir_child(record, &consumer_types, &consumer_layouts, &[])
            .is_none()
    );
    assert!(CallPathType::hir_allocation(member, &consumer_types).is_some());
    assert!(CallPathType::hir_allocation(padding, &consumer_types).is_none());
}
