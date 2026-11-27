//! Native Invoke edge-local initialization and repeated-outcome retirement invariants.

use super::{InvokeLayerOp, apply_invoke_edge_transfer};
use crate::compiler_frontend::analysis::borrow_checker::state::{
    BorrowState, FieldKey, FunctionLayout, FunctionLayoutInputs, LocalState, RootSet, StoredValue,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::InvokeLaneTransfer;
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
    output.update_local_state(
        1,
        LocalState::slot_with_value_roots(roots, RootSet::empty(3)),
    );
    output.update_local_state(2, LocalState::slot(3));

    let success_lane = InvokeLaneTransfer {
        results: vec![(LocalId(1), output.local_state(1).clone())],
        ..Default::default()
    };
    let error_lane = InvokeLaneTransfer {
        results: vec![(LocalId(2), LocalState::slot(3))],
        ..Default::default()
    };
    let mut success = output.clone();
    apply_invoke_edge_transfer(&layout, &terminator, &success_lane, 0, &mut success);
    assert_eq!(success.local_state(2).mode, LocalMode::UNINIT);
    assert!(success.local_state(1).value_roots.contains(0));
    assert!(success.local_state(2).value_roots.is_empty());

    // A following failure cannot observe the preceding successful iteration's payload.
    let mut failure = success;
    apply_invoke_edge_transfer(&layout, &terminator, &error_lane, 0, &mut failure);
    assert_eq!(failure.local_state(1).mode, LocalMode::UNINIT);
    assert!(failure.local_state(1).value_roots.is_empty());
    assert_eq!(failure.local_state(2).mode, LocalMode::SLOT);
    assert!(failure.local_state(2).value_roots.is_empty());

    // Returning to success similarly discards the preceding error.
    apply_invoke_edge_transfer(&layout, &terminator, &success_lane, 0, &mut failure);
    assert_eq!(failure.local_state(2).mode, LocalMode::UNINIT);
    assert!(failure.local_state(1).value_roots.contains(0));
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
        state.update_local_state(
            observer,
            LocalState::slot_with_value_roots(roots, RootSet::empty(layout.local_count())),
        );
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
                ids: vec![previous_error_child],
                views: Vec::new(),
                readonly: true,
            },
            InvokeLayerOp::Store {
                holders: vec![previous_success],
                key: stored,
                values: vec![StoredValue {
                    id: previous_error_child,
                    writable: false,
                }],
                views: Vec::new(),
                distinct_occurrence: true,
            },
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
                ids: vec![previous_success_child],
                views: Vec::new(),
                readonly: false,
            },
            InvokeLayerOp::Store {
                holders: vec![previous_error],
                key: error_only,
                values: vec![StoredValue {
                    id: previous_success_child,
                    writable: true,
                }],
                views: Vec::new(),
                distinct_occurrence: true,
            },
        ],
        ..Default::default()
    };
    let mut failure = state.clone();
    apply_invoke_edge_transfer(&layout, &terminator, &error_lane, 1, &mut failure);
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

    apply_invoke_edge_transfer(&layout, &terminator, &success_lane, 1, &mut state);
    let observed_error = state.resolve_alloc_path(4, &layout.local_mutable, [field]);
    let observed_store = state.resolve_alloc_path(3, &layout.local_mutable, [stored]);
    assert_ne!(observed_error.ids, [previous_error_child]);
    assert_eq!(state.local_alloc_ids(1).0, observed_error.ids);
    assert!(state.alloc_binding_readonly(1));
    assert!(observed_store.ids.contains(&observed_error.ids[0]));
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
