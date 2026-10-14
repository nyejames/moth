//! Native Invoke edge-local initialization and repeated-outcome retirement invariants.

use super::apply_invoke_edge_transfer;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    BorrowState, FunctionLayout, FunctionLayoutInputs, LocalState, RootSet,
};
use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, LocalId};
use crate::compiler_frontend::hir::numeric::RangeStepFailureCause;
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};

#[test]
fn invoke_edges_retire_both_previous_outcomes_before_selected_definitions() {
    let layout = FunctionLayout::new(FunctionLayoutInputs {
        local_ids: vec![LocalId(0), LocalId(1), LocalId(2)],
        local_mutable: vec![true; 3],
        local_first_write_order: vec![0; 3],
        local_last_use_order: vec![0; 3],
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
        operation: HirFallibleOperation::RangeStepFailure {
            cause: RangeStepFailureCause::ZeroStep,
        },
        success: BlockId(1),
        success_results: vec![LocalId(1)],
        failure: BlockId(2),
        error_result: LocalId(2),
    };
    let mut output = BorrowState::new_uninitialized(3);
    output.initialize_parameter(0);
    let mut roots = RootSet::empty(3);
    roots.insert(0);
    output.update_local_state(
        1,
        LocalState::slot_with_value_roots(roots, RootSet::empty(3)),
    );
    output.update_local_state(2, LocalState::slot(3));

    let results = [(LocalId(1), output.local_state(1).clone())];
    let mut success = output.clone();
    apply_invoke_edge_transfer(&layout, &terminator, BlockId(1), &results, &mut success);
    assert_eq!(success.local_state(2).mode, LocalMode::UNINIT);
    assert!(success.local_state(1).value_roots.contains(0));
    assert!(success.local_state(2).value_roots.is_empty());

    // A following failure cannot observe the preceding successful iteration's payload.
    let mut failure = success;
    apply_invoke_edge_transfer(&layout, &terminator, BlockId(2), &results, &mut failure);
    assert_eq!(failure.local_state(1).mode, LocalMode::UNINIT);
    assert!(failure.local_state(1).value_roots.is_empty());
    assert_eq!(failure.local_state(2).mode, LocalMode::SLOT);
    assert!(failure.local_state(2).value_roots.is_empty());

    // Returning to success similarly discards the preceding error.
    apply_invoke_edge_transfer(&layout, &terminator, BlockId(1), &results, &mut failure);
    assert_eq!(failure.local_state(2).mode, LocalMode::UNINIT);
    assert!(failure.local_state(1).value_roots.contains(0));
}
