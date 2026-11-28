//! Native Invoke edge-local initialization and repeated-outcome retirement invariants.

use super::{InvokeLayerOp, apply_invoke_edge_transfer};
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, BorrowState, CapturedValue, CellId, Coverage, EntryFamilies, FieldKey,
    FunctionLayout, FunctionLayoutInputs, LocalState, ReceiverDependency, RootSet, StoredValue,
    TypedStore, ValueAlt, ValueKind, WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::InvokeLaneTransfer;
use crate::compiler_frontend::analysis::borrow_checker::transfer::facts::{Carrier, CarrierValue};
use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, LocalId};
use crate::compiler_frontend::hir::numeric::LoopStepFailureCause;
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};

use super::apply_captured_call_batch;
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    CapturedCallBatch, CapturedDestination, CapturedDetachment,
    typed_call::{CapturedEdit, CapturedPost, CapturedResult, CapturedUncertainty},
};

fn call_batch(post: CapturedPost, destinations: Vec<CapturedDestination>) -> CapturedCallBatch {
    CapturedCallBatch {
        post,
        destinations: destinations.into_iter().collect(),
        member_reads: Default::default(),
        checkpoints: Vec::new(),
        detachment: None,
    }
}

#[test]
fn typed_invoke_retires_both_outcomes_and_rebases_retained_entry_cell() {
    use crate::compiler_frontend::analysis::borrow_checker::state::{BindingSpec, SmallSet};
    let (layout, _) = invoke_fixture(5);
    let mut predecessor = BorrowState::new_uninitialized(5);
    for index in 0..5 {
        predecessor.initialize_parameter(index);
    }
    let receiver = predecessor.local_alloc_ids(0).0[0];
    let old_value = ValueAlt {
        kind: ValueKind::Scalar,
        deps: vec![ReceiverDependency {
            entries: SmallSet::One(receiver),
            coverage: Coverage::Complete,
        }],
    };
    let old_slot = CellId::Slot(WrapperId::current(1));
    predecessor.publish_store(
        &TypedStore::replace_cells(
            AddressSet::from_cells([old_slot], Coverage::Complete).with_singleton_proof(),
            CapturedValue {
                alternatives: vec![old_value.clone()],
                ..Default::default()
            },
        )
        .with_explicit_slot_payload(),
        0,
        None,
    );
    predecessor.initialize_entry_binding(
        3,
        BindingSpec::Link {
            source: WrapperId::current(1),
            writable: false,
        },
    );

    for result in [1, 2] {
        let mut edge = predecessor.clone();
        let target = CellId::Projected(receiver, FieldKey::Field(9));
        let write = TypedStore::replace_cells(
            AddressSet::from_cells([target], Coverage::Complete).with_singleton_proof(),
            CapturedValue {
                alternatives: edge.value_cell(old_slot).to_vec(),
                ..Default::default()
            },
        );
        let post = CapturedPost {
            edits: vec![CapturedEdit::Cell(write)],
            results: vec![CapturedResult {
                slot: 0,
                value: CapturedValue {
                    alternatives: vec![ValueAlt::plain(ValueKind::Scalar)],
                    ..Default::default()
                },
                roots: None,
                derivation: None,
            }],
            ..Default::default()
        };
        apply_captured_call_batch(
            (&layout).into(),
            call_batch(post, vec![CapturedDestination::Define(result)]),
            [1, 2],
            1,
            &mut edge,
        )
        .unwrap();

        let held = edge.resolve_cells(3);
        assert_eq!(held.cells().len(), 1);
        assert_ne!(held.cells()[0], old_slot);
        assert_eq!(
            edge.value_cell(held.cells()[0]),
            std::slice::from_ref(&old_value)
        );
        assert_eq!(edge.value_cell(target), std::slice::from_ref(&old_value));
        assert_eq!(
            edge.value_cell(CellId::Slot(WrapperId::current(result))),
            &[ValueAlt::plain(ValueKind::Scalar)]
        );
        assert_eq!(
            edge.binding_mode(if result == 1 { 2 } else { 1 }),
            LocalMode::UNINIT
        );
    }
}

#[test]
fn typed_receiving_preserves_each_selected_write_result_tuple() {
    use crate::compiler_frontend::analysis::borrow_checker::state::SmallSet;
    let (layout, _) = invoke_fixture(3);
    let mut predecessor = BorrowState::new_uninitialized(3);
    predecessor.initialize_parameter(0);
    let holder = predecessor.local_alloc_ids(0).0[0];
    let field = CellId::Projected(holder, FieldKey::Field(0));
    let receiver = CellId::Slot(WrapperId::current(1));
    let plain = ValueAlt::plain(ValueKind::Scalar);
    let dependent = ValueAlt {
        kind: ValueKind::Scalar,
        deps: vec![ReceiverDependency {
            entries: SmallSet::One(holder),
            coverage: Coverage::Complete,
        }],
    };
    let capture = |value: &ValueAlt| CapturedValue {
        alternatives: vec![value.clone()],
        ..Default::default()
    };
    for (stored, returned) in [(&dependent, &plain), (&plain, &dependent)] {
        let post = CapturedPost {
            edits: vec![CapturedEdit::Cell(TypedStore::replace_cells(
                AddressSet::from_cells([field], Coverage::Complete).with_singleton_proof(),
                capture(stored),
            ))],
            results: vec![CapturedResult {
                slot: 0,
                value: capture(returned),
                roots: None,
                derivation: None,
            }],
            ..Default::default()
        };
        let mut edge = predecessor.clone();
        apply_captured_call_batch(
            (&layout).into(),
            call_batch(post, vec![CapturedDestination::Define(1)]),
            [1, 2],
            1,
            &mut edge,
        )
        .unwrap();
        assert_eq!(edge.value_cell(field), std::slice::from_ref(stored));
        assert_eq!(edge.value_cell(receiver), std::slice::from_ref(returned));
    }
}

#[test]
fn typed_strong_write_kills_old_carrier_but_unresolved_suffix_preserves_it() {
    use crate::compiler_frontend::analysis::borrow_checker::state::SmallSet;
    let (layout, _) = invoke_fixture(1);
    let mut predecessor = BorrowState::new_uninitialized(1);
    predecessor.initialize_parameter(0);
    let holder = predecessor.local_alloc_ids(0).0[0];
    let cell = CellId::Projected(holder, FieldKey::Field(3));
    let old = ValueAlt {
        kind: ValueKind::Scalar,
        deps: vec![ReceiverDependency {
            entries: SmallSet::One(holder),
            coverage: Coverage::Complete,
        }],
    };
    let store = |value: ValueAlt| {
        TypedStore::replace_cells(
            AddressSet::from_cells([cell], Coverage::Complete).with_singleton_proof(),
            CapturedValue {
                alternatives: vec![value],
                ..Default::default()
            },
        )
    };
    predecessor.publish_store(&store(old.clone()), 0, None);
    let plain = ValueAlt::plain(ValueKind::Scalar);
    for (replace, unresolved) in [(true, false), (false, false), (true, true)] {
        let mut edge = predecessor.clone();
        let mut post = CapturedPost::default();
        let write = if replace {
            store(plain.clone())
        } else {
            TypedStore::write_cells(
                AddressSet::from_cells([cell], Coverage::Complete).with_singleton_proof(),
                CapturedValue {
                    alternatives: vec![plain.clone()],
                    ..Default::default()
                },
            )
        };
        let edit = CapturedEdit::Cell(write);
        if unresolved {
            post.uncertainty = Some(CapturedUncertainty {
                edits: vec![edit],
                ..Default::default()
            });
        } else {
            post.edits.push(edit);
        }
        let observations = apply_captured_call_batch(
            (&layout).into(),
            call_batch(post, Vec::new()),
            [],
            1,
            &mut edge,
        )
        .unwrap();
        assert!(edge.value_cell(cell).contains(&plain));
        let killed = replace && !unresolved;
        assert_eq!(edge.value_cell(cell).contains(&old), !killed);
        let old_carrier = Carrier::Cell(cell, CarrierValue::Scalar, old.deps.clone());
        assert_eq!(
            observations.tail.definite_kills.contains(&old_carrier)
                || observations
                    .splits
                    .iter()
                    .any(|(segment, _)| segment.definite_kills.contains(&old_carrier)),
            killed,
        );
    }
}

#[test]
fn mixed_lane_unknown_starts_at_its_own_prefix() {
    let (layout, _) = invoke_fixture(5);
    let mut predecessor = BorrowState::new_uninitialized(5);
    for index in 0..5 {
        predecessor.initialize_parameter(index);
    }
    let [holder, old, success, error, suffix] =
        std::array::from_fn(|index| predecessor.local_alloc_ids(index).0[0]);
    let inside = FieldKey::Field(0);
    let outside = FieldKey::Field(1);
    let stored = |id| StoredValue { id, writable: true };
    let replace = |key, id| {
        CapturedEdit::Cell(TypedStore::replace_cells(
            AddressSet::from_cells([CellId::Projected(holder, key)], Coverage::Complete)
                .with_singleton_proof(),
            CapturedValue::new(vec![stored(id)], Vec::new()),
        ))
    };
    for key in [inside, outside] {
        let CapturedEdit::Cell(store) = replace(key, old) else {
            unreachable!()
        };
        predecessor.publish_store(&store, 0, None);
    }

    for order in [[success, error], [error, success]] {
        for prefix in order {
            let mut edge = predecessor.clone();
            let post = CapturedPost {
                edits: vec![replace(inside, prefix), replace(outside, prefix)],
                uncertainty: Some(CapturedUncertainty {
                    edits: vec![replace(inside, suffix)],
                    ..Default::default()
                }),
                ..Default::default()
            };
            apply_captured_call_batch(
                (&layout).into(),
                call_batch(post, Vec::new()),
                [],
                1,
                &mut edge,
            )
            .unwrap();
            assert_eq!(
                edge.resolve_alloc_path(0, &[true; 5], [outside]).ids,
                [prefix]
            );
            let values = edge.resolve_alloc_path(0, &[true; 5], [inside]).ids;
            assert_eq!(values.len(), 2);
            assert!(values.contains(&prefix) && values.contains(&suffix));
        }
    }
}

#[test]
fn family_replacement_replays_clear_only_and_refill_in_both_lane_orders() {
    use crate::compiler_frontend::analysis::borrow_checker::state::SmallSet;
    use crate::compiler_frontend::analysis::borrow_checker::transfer::typed_call::{
        EntryCapture, ExplicitMatchInput, match_typed_lane,
    };
    use crate::compiler_frontend::datatypes::ids::TypeId;
    use crate::compiler_frontend::public_call_summary::{
        CallField,
        typed::{
            Component, FamilyPort, FamilyPost, Lane, ObjectRef, Occurrence, PathAccess, PortId,
            Post, Product, ScalarSource, SupportId, ValueRef, ValueSource,
        },
    };

    let (layout, _) = invoke_fixture(4);
    let mut predecessor = BorrowState::new_uninitialized(4);
    for local in 0..4 {
        predecessor.initialize_parameter(local);
    }
    let [holder, old_source, new_source, another] =
        std::array::from_fn(|local| predecessor.local_alloc_ids(local).0[0]);
    let alternative = |source| ValueAlt {
        kind: ValueKind::Scalar,
        deps: vec![ReceiverDependency {
            entries: SmallSet::One(source),
            coverage: Coverage::Complete,
        }],
    };
    let old = alternative(old_source);
    let new = alternative(new_source);
    let capture = |value: ValueAlt| CapturedValue {
        alternatives: vec![value],
        ..Default::default()
    };
    let selectors = [
        (CallField::Elem, FieldKey::Elem),
        (CallField::MapKey, FieldKey::MapKey),
    ];
    for (_, key) in selectors {
        for receiver in [holder, old_source] {
            predecessor.publish_store(
                &TypedStore::write_projected(
                    [receiver],
                    key,
                    Coverage::Complete,
                    capture(old.clone()),
                    true,
                ),
                0,
                None,
            );
        }
    }
    let value = ValueRef {
        source: ValueSource::Scalar(ScalarSource::EntryValue(PortId(1))),
        access: PathAccess {
            restriction: None,
            derived_from: None,
        },
        dependencies: Vec::new(),
    };
    let lanes = [false, true].map(|success| {
        Lane::Returning(
            Product::new(vec![Component {
                support: SupportId(0),
                alternatives: vec![Post {
                    families: selectors
                        .into_iter()
                        .map(|(selector, _)| {
                            (
                                FamilyPort {
                                    receiver: ObjectRef::Entry(PortId(0)),
                                    selector,
                                },
                                FamilyPost::ReplaceAll(
                                    success
                                        .then(|| (value.clone(), Occurrence::AtMostOnce))
                                        .into_iter()
                                        .collect(),
                                ),
                            )
                        })
                        .collect(),
                    ..Default::default()
                }],
            }])
            .unwrap(),
        )
    });
    for (receivers, coverage, strong) in [
        (vec![holder], Coverage::Complete, true),
        (vec![holder], Coverage::Partial, false),
        (vec![holder, another], Coverage::Complete, false),
    ] {
        let entries = [
            EntryCapture {
                ty: Some(TypeId(0)),
                writable: Some(true),
                value: CapturedValue::new(
                    receivers
                        .into_iter()
                        .map(|id| StoredValue { id, writable: true })
                        .collect(),
                    Vec::new(),
                ),
                allocation_coverage: Some(coverage),
                ..Default::default()
            },
            EntryCapture {
                value: capture(new.clone()),
                ..Default::default()
            },
        ];
        let input = ExplicitMatchInput {
            state: &predecessor,
            entries: &entries,
            retained: &[],
            uncertainty: &[],
        };
        for extract in [[0, 1], [1, 0]] {
            let [first, second] =
                extract.map(|lane| (lane, match_typed_lane(&input, &lanes[lane]).unwrap()));
            let mut batches = [first, second];
            batches.sort_by_key(|(lane, _)| *lane);
            for delivery in [[0, 1], [1, 0]] {
                for lane in delivery {
                    let mut replay = predecessor.clone();
                    let observations = apply_captured_call_batch(
                        (&layout).into(),
                        CapturedCallBatch {
                            post: batches[lane].1.clone(),
                            destinations: Default::default(),
                            member_reads: Default::default(),
                            checkpoints: Vec::new(),
                            detachment: None,
                        },
                        [],
                        1,
                        &mut replay,
                    )
                    .unwrap();
                    for (_, key) in selectors {
                        let cell = CellId::Projected(holder, key);
                        assert_eq!(replay.value_cell(cell).contains(&old), !strong);
                        assert_eq!(replay.value_cell(cell).contains(&new), lane == 1);
                        let carrier = Carrier::Cell(cell, CarrierValue::Scalar, old.deps.clone());
                        assert_eq!(
                            observations.tail.definite_kills.contains(&carrier)
                                || observations
                                    .splits
                                    .iter()
                                    .any(|(segment, _)| segment.definite_kills.contains(&carrier)),
                            strong
                        );
                        let unrelated = CellId::Projected(old_source, key);
                        assert_eq!(
                            replay.value_cell(unrelated),
                            predecessor.value_cell(unrelated)
                        );
                        let untouched =
                            Carrier::Cell(unrelated, CarrierValue::Scalar, old.deps.clone());
                        assert!(!observations.tail.definite_kills.contains(&untouched));
                        assert!(
                            observations
                                .splits
                                .iter()
                                .all(|(segment, _)| !segment.definite_kills.contains(&untouched))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn typed_removal_reads_entry_member_and_detaches_after_receiving() {
    use crate::compiler_frontend::analysis::borrow_checker::state::{
        FamilyMember, Replacement, SmallSet,
    };
    let (layout, _) = invoke_fixture(5);
    let mut state = BorrowState::new_uninitialized(5);
    for index in 0..5 {
        state.initialize_parameter(index);
    }
    let holder = state.local_alloc_ids(0).0[0];
    let item_id = state.local_alloc_ids(3).0[0];
    let loan = state.local_alloc_ids(4).0[0];
    let item = StoredValue {
        id: item_id,
        writable: true,
    };
    let captured = CapturedValue {
        values: vec![item],
        alternatives: vec![ValueAlt {
            kind: ValueKind::Allocation(item),
            deps: vec![ReceiverDependency {
                entries: SmallSet::One(loan),
                coverage: Coverage::Complete,
            }],
        }],
        ..Default::default()
    };
    state.publish_family_members(
        &[holder],
        Coverage::Complete,
        FieldKey::Elem,
        Replacement::Must,
        &[FamilyMember {
            value: captured.clone(),
            at_most_once: true,
        }],
    );
    let mut batch = call_batch(
        CapturedPost {
            results: vec![CapturedResult {
                slot: 0,
                value: captured,
                roots: None,
                derivation: None,
            }],
            ..Default::default()
        },
        vec![CapturedDestination::Define(1)],
    );
    batch.member_reads.insert(Carrier::Cell(
        CellId::Projected(holder, FieldKey::Elem),
        CarrierValue::Allocation(item_id),
        vec![ReceiverDependency {
            entries: SmallSet::One(loan),
            coverage: Coverage::Complete,
        }],
    ));
    batch.detachment = Some(CapturedDetachment {
        holders: vec![holder],
        map: false,
        result: Some(0),
    });
    let observations =
        apply_captured_call_batch((&layout).into(), batch, [1, 2], 1, &mut state).unwrap();
    assert!(observations.tail.upward_reads.iter().chain(
        observations.splits.iter().flat_map(|(segment, _)| segment.upward_reads.iter())
    ).any(|carrier| matches!(carrier, Carrier::Cell(CellId::Projected(id, FieldKey::Elem), _, _) if *id == holder)));
    assert_eq!(state.local_alloc_ids(1).0, [item_id]);
    assert_eq!(state.detached_holders(1).collect::<Vec<_>>(), [holder]);
    assert!(
        state
            .resolve_alloc_path(0, &[true; 5], [FieldKey::Elem])
            .ids
            .is_empty()
    );
}

#[test]
fn typed_receive_uses_predecessor_address_and_follows_callee_writes() {
    use crate::compiler_frontend::analysis::borrow_checker::state::{BindingSpec, Replacement};
    use crate::compiler_frontend::analysis::borrow_checker::transfer::typed_call::{
        BindingEdit, CapturedBinding,
    };
    let (layout, _) = invoke_fixture(5);
    let mut state = BorrowState::new_uninitialized(5);
    for local in 0..5 {
        state.initialize_parameter(local);
    }
    let holder = state.local_alloc_ids(0).0[0];
    let initial = state.local_alloc_ids(2).0[0];
    let callee = state.local_alloc_ids(3).0[0];
    let received = state.local_alloc_ids(4).0[0];
    let first = FieldKey::Field(0);
    let second = FieldKey::Field(1);
    let address = |key| {
        AddressSet::from_cells([CellId::Projected(holder, key)], Coverage::Complete)
            .with_singleton_proof()
    };
    let stored = |id| StoredValue { id, writable: true };
    let capture = |id| CapturedValue::new(vec![stored(id)], Vec::new());
    for key in [first, second] {
        state.publish_store(
            &TypedStore::replace_cells(address(key), capture(initial)),
            0,
            None,
        );
    }
    state.initialize_entry_binding(
        1,
        BindingSpec::Alias {
            addresses: address(first),
            route_roots: RootSet::empty(5),
        },
    );
    let update = CapturedDestination::Update {
        actor: 1,
        address: state.resolve_place_path(1, &[true; 5], []).address,
    };
    let post = CapturedPost {
        bindings: vec![CapturedBinding {
            target: WrapperId::current(1),
            update: BindingEdit::Projected {
                addresses: address(second),
                writable: true,
                origin: None,
            },
            replace: Replacement::Must,
            roots: None,
        }],
        edits: vec![CapturedEdit::Cell(TypedStore::replace_cells(
            address(first),
            capture(callee),
        ))],
        results: vec![CapturedResult {
            slot: 0,
            value: capture(received),
            roots: None,
            derivation: None,
        }],
        ..Default::default()
    };
    apply_captured_call_batch(
        (&layout).into(),
        call_batch(post, vec![update]),
        [],
        1,
        &mut state,
    )
    .unwrap();
    assert_eq!(
        state.resolve_cells(1).cells(),
        &[CellId::Projected(holder, second)]
    );
    assert_eq!(
        state.resolve_alloc_path(0, &[true; 5], [first]).ids,
        [received]
    );
    assert_eq!(
        state.resolve_alloc_path(0, &[true; 5], [second]).ids,
        [initial]
    );
}

fn invoke_fixture(local_count: usize) -> (FunctionLayout, HirTerminator) {
    let layout = FunctionLayout::new(FunctionLayoutInputs {
        local_ids: (0..local_count as u32).map(LocalId).collect(),
        local_mutable: vec![true; local_count],
        local_has_in_place_mutation_surface: vec![true; local_count],
        local_first_write_order: vec![0; local_count],
        local_last_use_order: vec![0; local_count],
        statement_order_by_id: Default::default(),
        terminator_order_by_block: Default::default(),
        block_local_max_use_order: Default::default(),
        block_successors: Default::default(),
        visible_locals_by_block: Default::default(),
        may_use_from_block: Default::default(),
        must_use_from_block: Default::default(),
        may_assign_from_block: Default::default(),
    });
    let terminator = HirTerminator::Invoke {
        id: HirNodeId(0),
        span: None,
        operation: HirFallibleOperation::LoopStepFailure {
            cause: LoopStepFailureCause::ZeroStep,
        },
        success: BlockId(1),
        success_results: vec![LocalId(1)],
        failure: BlockId(2),
        error_result: LocalId(2),
    };
    (layout, terminator)
}

#[test]
fn invoke_edges_retire_both_previous_outcomes_before_selected_definitions() {
    let (layout, terminator) = invoke_fixture(3);
    let mut output = BorrowState::new_uninitialized(3);
    output.initialize_parameter(0);
    let mut roots = RootSet::empty(3);
    roots.insert(0);
    let success_result = LocalState::slot_from_value_roots(roots);
    output.update_local_state(1, success_result.clone());
    output.update_local_state(2, LocalState::slot(3));

    let success_lane = InvokeLaneTransfer {
        results: vec![(LocalId(1), success_result)],
        ..Default::default()
    };
    let error_lane = InvokeLaneTransfer {
        results: vec![(LocalId(2), LocalState::slot(3))],
        ..Default::default()
    };
    let mut success = output.clone();
    apply_invoke_edge_transfer(
        (&layout).into(),
        &terminator,
        &success_lane,
        0,
        |_| false,
        &mut success,
    );
    assert_eq!(success.binding_mode(2), LocalMode::UNINIT);
    assert!(success.binding(1).value_roots().contains(0));
    assert!(success.binding(2).value_roots().is_empty());

    // A following failure cannot observe the preceding successful iteration's payload.
    let mut failure = success;
    apply_invoke_edge_transfer(
        (&layout).into(),
        &terminator,
        &error_lane,
        0,
        |_| false,
        &mut failure,
    );
    assert_eq!(failure.binding_mode(1), LocalMode::UNINIT);
    assert!(failure.binding(1).value_roots().is_empty());
    assert_eq!(failure.binding_mode(2), LocalMode::SLOT);
    assert!(failure.binding(2).value_roots().is_empty());

    // Returning to success similarly discards the preceding error.
    apply_invoke_edge_transfer(
        (&layout).into(),
        &terminator,
        &success_lane,
        0,
        |_| false,
        &mut failure,
    );
    assert_eq!(failure.binding_mode(2), LocalMode::UNINIT);
    assert!(failure.binding(1).value_roots().contains(0));
}

#[test]
fn invoke_success_rebases_captures_through_reused_outcomes() {
    let (layout, terminator) = invoke_fixture(6);
    let mut state = BorrowState::new_uninitialized(layout.local_count());
    for index in [0, 1, 2, 5] {
        state.initialize_parameter(index);
    }
    for (observer, outcome) in [(3, 1), (4, 2)] {
        let mut roots = RootSet::empty(layout.local_count());
        roots.insert(outcome);
        state.update_local_state(observer, LocalState::slot_from_value_roots(roots));
    }

    let holder = state.local_alloc_ids(0).0[0];
    let previous_success = state.local_alloc_ids(1).0[0];
    let previous_error = state.local_alloc_ids(2).0[0];
    let marker = state.local_alloc_ids(5).0[0];
    let marker_value = StoredValue {
        id: marker,
        writable: true,
    };
    let field = FieldKey::Field(0);
    let stored = FieldKey::Field(1);
    let contributed = FieldKey::Field(2);
    let previous_error_child = state.alloc_child(previous_error, field);
    state.add_alloc_edge(previous_success, FieldKey::Elem, marker_value);
    state.add_alloc_edge(holder, FieldKey::Elem, marker_value);

    let success_lane = InvokeLaneTransfer {
        results: vec![(LocalId(1), LocalState::slot(layout.local_count()))],
        operations: vec![
            InvokeLayerOp::Bind {
                local: LocalId(1),
                value: CapturedValue::new(
                    vec![StoredValue {
                        id: previous_error_child,
                        writable: false,
                    }],
                    Vec::new(),
                ),
            },
            InvokeLayerOp::Store(TypedStore::write_projected(
                [previous_success],
                stored,
                Coverage::Complete,
                CapturedValue::new(
                    vec![StoredValue {
                        id: previous_error_child,
                        writable: false,
                    }],
                    Vec::new(),
                ),
                true,
            )),
            InvokeLayerOp::Contribute {
                holder,
                key: contributed,
                value: StoredValue {
                    id: previous_success,
                    writable: true,
                },
            },
            InvokeLayerOp::Store(TypedStore::reset_families(
                vec![previous_success],
                Coverage::Partial,
                EntryFamilies::Elements,
            )),
            InvokeLayerOp::Store(TypedStore::reset_families(
                vec![holder],
                Coverage::Complete,
                EntryFamilies::Elements,
            )),
        ],
        ..Default::default()
    };
    let error_only = FieldKey::Field(3);
    let previous_success_child = state.alloc_child(previous_success, field);
    let error_lane = InvokeLaneTransfer {
        results: vec![(LocalId(2), LocalState::slot(layout.local_count()))],
        operations: vec![
            InvokeLayerOp::Bind {
                local: LocalId(2),
                value: CapturedValue::new(
                    vec![StoredValue {
                        id: previous_success_child,
                        writable: true,
                    }],
                    Vec::new(),
                ),
            },
            InvokeLayerOp::Store(TypedStore::write_projected(
                [previous_error],
                error_only,
                Coverage::Complete,
                CapturedValue::new(
                    vec![StoredValue {
                        id: previous_success_child,
                        writable: true,
                    }],
                    Vec::new(),
                ),
                true,
            )),
        ],
        ..Default::default()
    };
    let mut failure = state.clone();
    apply_invoke_edge_transfer(
        (&layout).into(),
        &terminator,
        &error_lane,
        1,
        |_| false,
        &mut failure,
    );
    let retired_success = failure.local_alloc_ids(3).0[0];
    assert_ne!(retired_success, previous_success);
    let retired_error = failure.local_alloc_ids(4).0[0];
    assert_ne!(retired_error, previous_error);
    let retired_success_child = failure.alloc_child(retired_success, field);
    assert_ne!(retired_success_child, previous_success_child);
    assert_eq!(failure.local_alloc_ids(2).0, [retired_success_child]);
    assert!(
        failure
            .resolve_alloc_path(4, &layout.local_mutable, [error_only])
            .ids
            .contains(&retired_success_child)
    );
    assert!(!failure.alloc_group_is_explicit(previous_error, error_only));
    assert!(!failure.alloc_group_is_explicit(retired_success, stored));
    assert!(!failure.alloc_group_is_explicit(holder, contributed));
    assert_eq!(
        failure
            .resolve_alloc_path(0, &layout.local_mutable, [FieldKey::Elem])
            .ids,
        [marker],
    );

    apply_invoke_edge_transfer(
        (&layout).into(),
        &terminator,
        &success_lane,
        1,
        |_| false,
        &mut state,
    );
    let observed_error = state.resolve_alloc_path(4, &layout.local_mutable, [field]);
    let observed_store = state.resolve_alloc_path(3, &layout.local_mutable, [stored]);
    assert_ne!(observed_error.ids, [previous_error_child]);
    assert_eq!(state.local_alloc_ids(1).0, observed_error.ids);
    assert!(state.alloc_binding_readonly(1));
    assert!(observed_store.ids.contains(&observed_error.ids[0]));
    assert!(
        observed_store
            .ids
            .contains(&state.alloc_child(retired_success, stored))
    );
    assert!(observed_store.readonly);
    assert!(!state.alloc_group_is_explicit(previous_success, stored));
    assert!(!state.alloc_group_is_explicit(retired_error, error_only));
    let contributed_values = state
        .resolve_alloc_path(0, &layout.local_mutable, [contributed])
        .ids;
    let lazy = state.alloc_child(holder, contributed);
    // A weak contribution adds the rebased referent without overwriting the entry field.
    assert_eq!(contributed_values.len(), 2);
    assert!(contributed_values.contains(&retired_success));
    assert!(contributed_values.contains(&lazy));
    assert!(!contributed_values.contains(&previous_success));
    // Removing from the retired summary cannot prove it emptied every instance.
    assert_eq!(
        state
            .resolve_alloc_path(3, &layout.local_mutable, [FieldKey::Elem])
            .ids,
        [marker],
    );
    assert!(
        state
            .resolve_alloc_path(0, &layout.local_mutable, [FieldKey::Elem])
            .ids
            .is_empty()
    );
}

#[test]
fn invoke_weak_cell_store_does_not_replace_a_new_outcome_slot() {
    let (layout, terminator) = invoke_fixture(4);
    let mut state = BorrowState::new_uninitialized(layout.local_count());
    for slot in [0, 1, 3] {
        state.initialize_parameter(slot);
    }
    let marker = state.local_alloc_ids(0).0[0];
    let live_slot = state.local_alloc_ids(3).0[0];
    let success = InvokeLaneTransfer {
        results: vec![(LocalId(1), LocalState::slot(layout.local_count()))],
        operations: vec![InvokeLayerOp::Store(TypedStore::write_cells(
            AddressSet::from_cells(
                [1, 3].map(|slot| CellId::Slot(WrapperId::current(slot))),
                Coverage::Complete,
            ),
            CapturedValue::new(
                vec![StoredValue {
                    id: marker,
                    writable: true,
                }],
                Vec::new(),
            ),
        ))],
        ..Default::default()
    };
    apply_invoke_edge_transfer(
        (&layout).into(),
        &terminator,
        &success,
        1,
        |_| false,
        &mut state,
    );
    assert!(!state.local_alloc_ids(1).0.contains(&marker));
    let (live_contents, _) = state.local_alloc_ids(3);
    assert!(live_contents.contains(&marker));
    assert!(live_contents.contains(&live_slot));
}

#[test]
fn invoke_erase_clears_key_and_element_families_only_on_complete_success() {
    let (layout, terminator) = invoke_fixture(3);
    let mut entry = BorrowState::new_uninitialized(layout.local_count());
    entry.initialize_parameter(0);
    let receiver = entry.local_alloc_ids(0).0[0];
    let keys = [FieldKey::Elem, FieldKey::MapKey];
    let old_members = keys.map(|key| {
        let id = entry.alloc_child(receiver, key);
        entry.add_alloc_edge(receiver, key, StoredValue { id, writable: true });
        id
    });

    let lane = |coverage| InvokeLaneTransfer {
        results: vec![(LocalId(1), LocalState::slot(layout.local_count()))],
        operations: vec![InvokeLayerOp::Store(TypedStore::reset_families(
            vec![receiver],
            coverage,
            EntryFamilies::MapEntries,
        ))],
        ..Default::default()
    };
    let failure_lane = InvokeLaneTransfer {
        results: vec![(LocalId(2), LocalState::slot(layout.local_count()))],
        ..Default::default()
    };
    for (selected, erased) in [
        (lane(Coverage::Complete), true),
        (lane(Coverage::Partial), false),
        (failure_lane, false),
    ] {
        let mut state = entry.clone();
        apply_invoke_edge_transfer(
            (&layout).into(),
            &terminator,
            &selected,
            1,
            |_| false,
            &mut state,
        );
        for (key, old) in keys.into_iter().zip(old_members) {
            let mut place = state.resolve_place_path(0, &layout.local_mutable, [key]);
            state.load_place_content(&mut place, &layout.local_mutable, false);
            let contents = place.content.expect("loaded receiver member").ids;
            if erased {
                assert!(contents.is_empty());
            } else {
                assert_eq!(contents, [old]);
            }
        }
    }
}

#[test]
fn invoke_success_observes_entry_member_and_rebases_lookup_payload() {
    let (layout, terminator) = invoke_fixture(3);
    let mut entry = BorrowState::new_uninitialized(layout.local_count());
    entry.initialize_parameter(0);
    entry.initialize_parameter(1);
    let old_outcome = entry.local_alloc_ids(1).0[0];
    let receiver = entry.local_alloc_ids(0).0[0];
    let dependency = |id| ReceiverDependency {
        entries: [id].into_iter().collect(),
        coverage: Coverage::Complete,
    };
    let member = CellId::Projected(old_outcome, FieldKey::Elem);
    let member_deps = vec![dependency(receiver)];
    entry.record_constructed_value(
        old_outcome,
        FieldKey::Elem,
        &CapturedValue {
            alternatives: vec![ValueAlt {
                kind: ValueKind::Scalar,
                deps: member_deps.clone(),
            }],
            ..Default::default()
        },
        true,
    );
    let observed = Carrier::Cell(member, CarrierValue::Scalar, member_deps.clone());
    let mut result_deps = member_deps;
    result_deps.push(dependency(old_outcome));
    let success = InvokeLaneTransfer {
        results: vec![(LocalId(1), LocalState::slot(layout.local_count()))],
        operations: vec![InvokeLayerOp::Bind {
            local: LocalId(1),
            value: CapturedValue {
                alternatives: vec![ValueAlt {
                    kind: ValueKind::Scalar,
                    deps: result_deps,
                }],
                ..Default::default()
            },
        }],
        member_reads: [observed.clone()].into_iter().collect(),
        ..Default::default()
    };
    let failure = InvokeLaneTransfer {
        results: vec![(LocalId(2), LocalState::slot(layout.local_count()))],
        ..Default::default()
    };
    for (lane, succeeds) in [(&success, true), (&failure, false)] {
        let mut state = entry.clone();
        let summary = apply_invoke_edge_transfer(
            (&layout).into(),
            &terminator,
            lane,
            7,
            |_| false,
            &mut state,
        );
        let mut reads = Default::default();
        summary.backward(&mut reads, |_, _| {
            panic!("a lookup does not mutate its receiver")
        });
        assert_eq!(reads.contains(&observed), succeeds);
        let output = state.capture_jump_value(1, true, false);
        if succeeds {
            let payload = output.expect("success binds the dependent value");
            let rebased = state.remap_retired_alloc_id(old_outcome, 7, |local| local == 1);
            assert_ne!(rebased, old_outcome);
            assert_eq!(payload.alternatives.len(), 1);
            assert!(payload.alternatives[0].deps.contains(&dependency(receiver)));
            assert!(payload.alternatives[0].deps.contains(&dependency(rebased)));
        } else {
            assert!(output.is_none());
        }
    }
}
