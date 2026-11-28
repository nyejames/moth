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
            InvokeLayerOp::Remove {
                holders: vec![previous_success],
                map: false,
                result: Some(LocalId(1)),
            },
            InvokeLayerOp::Remove {
                holders: vec![holder],
                map: false,
                result: None,
            },
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
    assert_eq!(
        state
            .resolve_alloc_path(0, &layout.local_mutable, [contributed])
            .ids,
        [retired_success],
    );
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
        operations: vec![InvokeLayerOp::Store(TypedStore::EraseFamily {
            receivers: vec![receiver],
            coverage,
            keys: EntryFamilies::MapEntries,
        })],
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
    let (mut layout, terminator) = invoke_fixture(3);
    layout.lookup_observations = true;
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
