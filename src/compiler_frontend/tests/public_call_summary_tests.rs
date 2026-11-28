use super::super::public_call_summary::{
    CallAllocationFacts, CallAllocationSource, CallField, CallFieldHir, CallLaneSummary,
    CallOutputPath, CallParameterPath, CallPathSource, CallPathType, CallResultPath, CallSharing,
    CallSharingCondition, FunctionReturnAliasSummary, PublicCallMutationEffect,
    PublicCallParameterAccess, PublicCallParameterSummary, PublicCallSummary,
    PublicCallSummaryTransition, PublicCallTransferEffect, validate_public_call_allocation_layout,
    validate_public_call_summary_transition,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::hir::ids::{FieldId, StructId};
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::symbols::path_interner::PathId;

use super::super::public_call_summary::typed::{
    BindingPost, CellPort, CellPost, Component, Coverage, Derivation, EntryPort, ExportPort,
    Footprint, Guard, IdentityKind, Lane, ObjectRef, OutputPort, PathAccess, Permission, PortId,
    Post, Predicate, Product, ReachBound, ReachKind, ReceiverDependency, RetainedRoute, Role,
    ScalarSource, ScalarSourceBound, Shape, StorageKind, SupportId, TypedCallAllocationFacts,
    ValueRef, ValueSource,
};

fn scalar(source: ScalarSource) -> ValueRef {
    ValueRef {
        source: ValueSource::Scalar(source),
        access: PathAccess {
            restriction: None,
            derived_from: None,
        },
        dependencies: vec![],
    }
}

fn binary_effect(index: u32) -> Post {
    Post {
        cells: vec![(
            CellPort::Terminal(PortId(index)),
            CellPost::Replace(vec![Some(scalar(ScalarSource::Independent))]),
        )],
        ..Post::default()
    }
}

fn collecting_product(worlds: u8) -> Lane {
    if worlds == 0 {
        return Lane::Bottom;
    }
    let components = (0..2)
        .map(|index| Component {
            support: SupportId(index),
            alternatives: (0..4)
                .filter(|world| worlds & (1 << world) != 0)
                .map(|world| {
                    if world & (1 << index) == 0 {
                        Post::default()
                    } else {
                        binary_effect(index)
                    }
                })
                .collect(),
        })
        .collect();
    Lane::Returning(Product::new(components).unwrap())
}

#[test]
fn typed_product_normalisation_is_closed_under_incremental_delivery() {
    for left in 0..16 {
        for right in 0..16 {
            let expected = collecting_product(left | right);
            assert_eq!(
                collecting_product(left).union(&collecting_product(right)),
                expected
            );
            assert_eq!(
                collecting_product(right).union(&collecting_product(left)),
                expected
            );
            for cut in 0..=4 {
                let low = (1 << cut) - 1;
                assert_eq!(
                    collecting_product(left & low)
                        .union(&collecting_product(right))
                        .union(&collecting_product(left & !low)),
                    expected
                );
            }
        }
    }
    let independent = Product::new(
        (0..24)
            .map(|index| Component {
                support: SupportId(index),
                alternatives: vec![Post::default(), binary_effect(index)],
            })
            .collect(),
    )
    .unwrap();
    assert_eq!(
        independent
            .components()
            .iter()
            .map(|component| component.alternatives.len())
            .sum::<usize>(),
        48
    );
}

#[test]
fn typed_guard_rejects_inconsistent_equations_and_keeps_broad_preserve() {
    let equal = |a, b| Predicate::Equal(IdentityKind::Allocation, PortId(a), PortId(b));
    let ab = equal(0, 1);
    let ac = equal(0, 2);
    let bc = equal(1, 2);
    assert!(Guard::new([(ab, true), (bc, true), (ac, false)]).is_none());
    assert!(
        Guard::new([
            (Predicate::Role(PortId(0), Role::WholeAlias), true),
            (Predicate::Role(PortId(0), Role::ProjectedAlias), true),
        ])
        .is_none()
    );

    let written = Product::new(vec![Component {
        support: SupportId(0),
        alternatives: vec![binary_effect(0)],
    }])
    .unwrap();
    let effect = Lane::guarded(Guard::new([(ab, true)]).unwrap(), written);
    let preserve = Lane::Returning(Product::default());
    let joined = preserve.union(&effect);
    assert_eq!(joined, effect.union(&preserve));
    assert_eq!(
        joined.for_presentation(|_| false),
        Some(&Product::default())
    );
    assert_eq!(
        joined.for_presentation(|_| true).unwrap().components()[0].alternatives,
        [Post::default(), binary_effect(0)]
    );
}

#[test]
fn typed_shape_requires_explicit_scalar_results_and_monotone_executed_facts() {
    let ports = [EntryPort::Value {
        parameter: 0,
        path: Box::new([]),
    }];
    let shape = Shape {
        parameters: &[PublicCallParameterAccess::Shared],
        ports: &ports,
        support_count: 1,
        success_slots: Some(1),
        error_slot: false,
    };
    let returning = |source| TypedCallAllocationFacts {
        success: Lane::Returning(
            Product::new(vec![Component {
                support: SupportId(0),
                alternatives: vec![Post {
                    results: vec![(0, scalar(source))],
                    ..Post::default()
                }],
            }])
            .unwrap(),
        ),
        ..TypedCallAllocationFacts::default()
    };
    let input = returning(ScalarSource::EntryValue(PortId(0)));
    let independent = returning(ScalarSource::Independent);
    let both = TypedCallAllocationFacts {
        success: input.success.union(&independent.success),
        may_retain: vec![0],
        ..TypedCallAllocationFacts::default()
    };
    assert!(input.validate_transition(&both, &shape).is_ok());
    assert!(both.validate_transition(&input, &shape).is_err());
    assert!(
        returning(ScalarSource::EntryValue(PortId(1)))
            .validate(&shape)
            .is_err()
    );
    assert!(
        returning(ScalarSource::Unresolved(ScalarSourceBound::Values(vec![])))
            .validate(&shape)
            .is_err()
    );
    assert!(
        returning(ScalarSource::Unresolved(ScalarSourceBound::Values(vec![
            PortId(0)
        ])))
        .validate(&shape)
        .is_ok()
    );
    assert!(
        TypedCallAllocationFacts {
            success: Lane::Returning(Product::default()),
            ..TypedCallAllocationFacts::default()
        }
        .validate(&shape)
        .is_err()
    );
}

#[test]
fn typed_unknown_and_preserve_rows_do_not_absorb_each_other() {
    let prefix = binary_effect(0);
    let mut unknown = prefix.clone();
    unknown.coverage = Coverage::Unresolved(Footprint::Locations(vec![OutputPort::Cell(
        CellPort::Terminal(PortId(1)),
    )]));
    let lane = |post| {
        Lane::Returning(
            Product::new(vec![Component {
                support: SupportId(0),
                alternatives: vec![post],
            }])
            .unwrap(),
        )
    };
    let preserve = lane(Post::default());
    let known = lane(prefix.clone());
    let incomplete = lane(unknown.clone());
    let joined = preserve.union(&known).union(&incomplete);
    assert_eq!(joined, incomplete.union(&known).union(&preserve));
    assert_eq!(
        joined
            .for_presentation(|_| unreachable!())
            .unwrap()
            .components()[0]
            .alternatives,
        [Post::default(), prefix, unknown],
    );
}

#[test]
fn typed_guard_role_partitions_have_the_same_normal_form() {
    let port = PortId(0);
    let lane = |role, truth| {
        Lane::guarded(
            Guard::new([(Predicate::Role(port, role), truth)]).unwrap(),
            Product::default(),
        )
    };
    let not_slot = lane(Role::Slot, false);
    let whole = lane(Role::WholeAlias, true);
    let projected = lane(Role::ProjectedAlias, true);
    assert_eq!(not_slot, whole.union(&projected));
    assert_eq!(not_slot, projected.union(&whole));
    let shape = Shape {
        parameters: &[PublicCallParameterAccess::Mutable],
        ports: &[EntryPort::Wrapper(0)],
        support_count: 0,
        success_slots: Some(0),
        error_slot: false,
    };
    let facts = |success| TypedCallAllocationFacts {
        success,
        ..TypedCallAllocationFacts::default()
    };
    assert!(
        facts(not_slot)
            .validate_transition(&facts(whole.union(&projected)), &shape)
            .is_ok()
    );
}

#[test]
fn typed_equality_guard_partitions_have_the_same_normal_form() {
    let equal = |a, b| Predicate::Equal(IdentityKind::Cell, PortId(a), PortId(b));
    let ab = equal(0, 1);
    let bc = equal(1, 2);
    let lane = |literals| Lane::guarded(Guard::new(literals).unwrap(), Product::default());
    let direct = lane(vec![(bc, true)]);
    let equal = lane(vec![(ab, true), (bc, true)]);
    let distinct = lane(vec![(ab, false), (bc, true)]);
    assert_eq!(direct, equal.union(&distinct));
    assert_eq!(direct, distinct.union(&equal));
    assert_ne!(direct, equal);
    assert_ne!(direct, distinct);
}

#[test]
fn typed_products_reject_overlapping_and_readonly_writes() {
    let shape = Shape {
        parameters: &[PublicCallParameterAccess::Mutable; 2],
        ports: &[EntryPort::Terminal(0), EntryPort::Terminal(1)],
        support_count: 2,
        success_slots: Some(0),
        error_slot: false,
    };
    let component = |support, port| Component {
        support: SupportId(support),
        alternatives: vec![binary_effect(port)],
    };
    let facts = |literals, supports| TypedCallAllocationFacts {
        success: Lane::guarded(
            Guard::new(literals).unwrap(),
            Product::new(supports).unwrap(),
        ),
        ..TypedCallAllocationFacts::default()
    };
    assert!(
        facts(vec![], vec![component(0, 0), component(1, 0)])
            .validate(&shape)
            .is_err()
    );
    let equal = Predicate::Equal(IdentityKind::Cell, PortId(0), PortId(1));
    for (holds, valid) in [(true, false), (false, true)] {
        assert_eq!(
            facts(vec![(equal, holds)], vec![component(0, 0), component(1, 1)])
                .validate(&shape)
                .is_ok(),
            valid,
        );
    }
    assert!(
        facts(
            vec![(Predicate::Writable(PortId(0)), false)],
            vec![component(0, 0)]
        )
        .validate(&shape)
        .is_err()
    );
}

#[test]
fn typed_support_ownership_includes_own_slots_and_bounded_reaches() {
    let shape = Shape {
        parameters: &[PublicCallParameterAccess::Mutable; 2],
        ports: &[
            EntryPort::Wrapper(0),
            EntryPort::Terminal(0),
            EntryPort::Terminal(1),
            EntryPort::Value {
                parameter: 0,
                path: Box::new([]),
            },
            EntryPort::Value {
                parameter: 1,
                path: Box::new([]),
            },
        ],
        support_count: 2,
        success_slots: Some(0),
        error_slot: false,
    };
    let mut slot = binary_effect(1);
    slot.cells[0].0 = CellPort::Slot(ObjectRef::Entry(PortId(0)));
    let reach = |root, region| Post {
        coverage: Coverage::Unresolved(Footprint::WritableReach(ReachBound {
            roots: vec![PortId(root)],
            region,
            types: vec![CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String)],
        })),
        ..Post::default()
    };
    let validate = |first, second, relation: Option<_>| {
        let mut literals = vec![(Predicate::Role(PortId(0), Role::Slot), true)];
        literals.extend(relation);
        let product = Product::new(
            [first, second]
                .into_iter()
                .enumerate()
                .map(|(i, post)| Component {
                    support: SupportId(i as u32),
                    alternatives: vec![post],
                })
                .collect(),
        )
        .unwrap();
        TypedCallAllocationFacts {
            success: Lane::guarded(Guard::new(literals).unwrap(), product),
            ..TypedCallAllocationFacts::default()
        }
        .validate(&shape)
        .is_ok()
    };
    let distinct = Some((
        Predicate::Equal(IdentityKind::Cell, PortId(1), PortId(2)),
        false,
    ));
    let cell = |root| reach(root, ReachKind::Cell);
    assert!(!validate(slot.clone(), binary_effect(1), None));
    assert!(validate(slot.clone(), binary_effect(2), distinct));
    assert!(!validate(cell(1), slot, None));
    assert!(!validate(binary_effect(1), cell(1), None));
    assert!(!validate(cell(1), cell(1), None));
    assert!(validate(cell(1), cell(2), distinct));
    let compare_referents = |equal| {
        validate(
            reach(3, ReachKind::Referent),
            reach(4, ReachKind::EntryStructure),
            Some((
                Predicate::Equal(IdentityKind::Allocation, PortId(3), PortId(4)),
                equal,
            )),
        )
    };
    assert!(!compare_referents(true));
    assert!(compare_referents(false));
}

#[test]
fn typed_reach_uses_guarded_equality_of_a_holder_ancestor() {
    let field = CallField::Field;
    let ports = [
        EntryPort::Value {
            parameter: 0,
            path: Box::new([field(0)]),
        },
        EntryPort::Value {
            parameter: 0,
            path: Box::new([field(1)]),
        },
        EntryPort::Value {
            parameter: 0,
            path: Box::new([field(1), field(0)]),
        },
    ];
    let shape = Shape {
        parameters: &[PublicCallParameterAccess::Mutable],
        ports: &ports,
        support_count: 2,
        success_slots: Some(0),
        error_slot: false,
    };
    let mut edit = binary_effect(0);
    edit.cells[0].0 = CellPort::Edge(ObjectRef::Entry(PortId(2)), field(0));
    let product = Product::new(vec![
        Component {
            support: SupportId(0),
            alternatives: vec![Post {
                coverage: Coverage::Unresolved(Footprint::WritableReach(ReachBound {
                    roots: vec![PortId(0)],
                    region: ReachKind::Referent,
                    types: vec![CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String)],
                })),
                ..Post::default()
            }],
        },
        Component {
            support: SupportId(1),
            alternatives: vec![edit],
        },
    ])
    .unwrap();
    let equal = Predicate::Equal(IdentityKind::Allocation, PortId(0), PortId(1));
    for holds in [false, true] {
        let facts = TypedCallAllocationFacts {
            success: Lane::guarded(Guard::new([(equal, holds)]).unwrap(), product.clone()),
            ..TypedCallAllocationFacts::default()
        };
        assert_eq!(facts.validate(&shape).is_err(), holds);
    }
}

#[test]
fn typed_exact_cell_absence_is_not_preserve_or_a_scalar() {
    let shape = Shape {
        parameters: &[PublicCallParameterAccess::Mutable],
        ports: &[
            EntryPort::Terminal(0),
            EntryPort::Value {
                parameter: 0,
                path: Box::new([]),
            },
        ],
        support_count: 1,
        success_slots: Some(0),
        error_slot: false,
    };
    let lane = |contents| {
        Lane::Returning(
            Product::new(vec![Component {
                support: SupportId(0),
                alternatives: vec![Post {
                    cells: vec![(CellPort::Terminal(PortId(0)), CellPost::Replace(contents))],
                    ..Post::default()
                }],
            }])
            .unwrap(),
        )
    };
    let value = Some(ValueRef {
        source: ValueSource::Allocation(ObjectRef::Entry(PortId(1))),
        access: PathAccess {
            restriction: Some(Permission::ReadOnly),
            derived_from: None,
        },
        dependencies: vec![],
    });
    let erased = lane(vec![None]);
    let mixed = lane(vec![None, value.clone()]);
    let preserve = Lane::Returning(Product::default());
    assert_ne!(erased, preserve);
    assert_ne!(mixed, lane(vec![value]));
    for (success, valid) in [(erased, true), (mixed, true), (lane(vec![]), false)] {
        assert_eq!(
            TypedCallAllocationFacts {
                success,
                ..TypedCallAllocationFacts::default()
            }
            .validate(&shape)
            .is_ok(),
            valid
        );
    }
}

#[test]
fn typed_projected_routes_keep_their_own_loans_and_retained_derivation() {
    let ports = [
        EntryPort::Wrapper(0),
        EntryPort::Wrapper(1),
        EntryPort::Terminal(2),
        EntryPort::Value {
            parameter: 3,
            path: Box::new([]),
        },
        EntryPort::Value {
            parameter: 4,
            path: Box::new([]),
        },
    ];
    let shape = Shape {
        parameters: &[PublicCallParameterAccess::Mutable; 5],
        ports: &ports,
        support_count: 1,
        success_slots: Some(0),
        error_slot: false,
    };
    let anchor = ObjectRef::Exact(ExportPort::PrivateTerminal(2));
    let route = |receiver| {
        BindingPost::Projected(
            CellPort::Terminal(PortId(2)),
            RetainedRoute {
                access: PathAccess {
                    restriction: Some(Permission::ReadOnly),
                    derived_from: Some(Derivation::Retained(anchor.clone())),
                },
                dependencies: vec![ReceiverDependency::Known(vec![ObjectRef::Entry(PortId(
                    receiver,
                ))])],
            },
        )
    };
    let mut post = Post {
        bindings: vec![
            (ObjectRef::Entry(PortId(0)), route(3)),
            (ObjectRef::Entry(PortId(1)), route(4)),
        ],
        retained: vec![(
            anchor.clone(),
            StorageKind::Wrapper,
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
        )],
        ..Post::default()
    };
    assert_ne!(post.bindings[0].1, post.bindings[1].1);
    let facts = |post| TypedCallAllocationFacts {
        success: Lane::Returning(
            Product::new(vec![Component {
                support: SupportId(0),
                alternatives: vec![post],
            }])
            .unwrap(),
        ),
        ..TypedCallAllocationFacts::default()
    };
    assert!(facts(post.clone()).validate(&shape).is_ok());
    post.retained.clear();
    assert!(facts(post).validate(&shape).is_err());
}

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
        structural_only: false,
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
    // An empty target path stores into the caller's mutable argument cell.
    assert_eq!(
        validate_public_call_summary_transition(&root_store, &root_store).unwrap(),
        PublicCallSummaryTransition::Unchanged
    );
    parameters[0].access = PublicCallParameterAccess::Shared;
    let shared = make(parameters, vec![direct, child]);
    let error = validate_public_call_summary_transition(&shared, &shared).unwrap_err();
    assert!(
        error
            .message
            .contains("must name a mutable parameter or its projection")
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
