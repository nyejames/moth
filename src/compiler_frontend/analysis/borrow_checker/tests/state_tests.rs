//! Borrow-checker state invariant tests.
//!
//! WHAT: protects slot-backed alias rebinding and root-set bookkeeping.
//! WHY: these facts live in transfer state and cannot be inspected from rendered output.

use super::super::state::{BorrowState, LocalState, RootSet};

#[test]
fn slot_backed_alias_rebinding_clears_old_value_roots() {
    let local_count = 2;
    let mut state = BorrowState::new_uninitialized(local_count);
    state.initialize_parameter(0);

    let mut aliased_root = RootSet::empty(local_count);
    aliased_root.insert(0);
    state.update_local_state(
        1,
        LocalState::slot_with_value_roots(aliased_root, RootSet::empty(local_count)),
    );

    assert_eq!(
        state.effective_roots(1).iter_ones().collect::<Vec<_>>(),
        vec![0]
    );

    state.update_local_state(1, LocalState::slot(local_count));

    assert!(state.effective_roots(1).contains(1));
    assert!(!state.effective_roots(1).contains(0));
}
