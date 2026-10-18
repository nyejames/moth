//! Borrow-checker state invariant tests.
//!
//! WHAT: protects slot-backed alias rebinding, join growth reporting, visibility narrowing and
//!       the membership shortcut that alias conflict scans use in place of the set builder.
//! WHY: these facts live in transfer state and cannot be inspected from rendered output.

use super::super::state::{BorrowState, LocalState, RootSet};
use super::super::types::LocalMode;

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

fn roots(local_count: usize, members: &[usize]) -> RootSet {
    let mut set = RootSet::empty(local_count);
    for member in members {
        set.insert(*member);
    }
    set
}

fn members(set: &RootSet) -> Vec<usize> {
    set.iter_ones().collect()
}

#[test]
fn join_reports_growth_for_each_fact_and_keeps_joined_facts() {
    let local_count = 3;
    let mut base = BorrowState::new_uninitialized(local_count);
    base.update_local_state(
        2,
        LocalState::alias_with_direct(roots(local_count, &[0]), roots(local_count, &[0])),
    );

    // Each incoming state grows exactly one fact of local 2: its mode, its value roots or its
    // direct-alias roots. Worklist scheduling depends on every kind of growth being reported.
    // Direct-alias roots stay within value roots, as in production, so direct growth comes
    // last, after the joined value roots already hold its new root.
    let mut mode_growth = base.clone();
    mode_growth.update_local_state(
        2,
        LocalState {
            mode: LocalMode::SLOT,
            value_roots: roots(local_count, &[0]),
            direct_alias_roots: roots(local_count, &[0]),
        },
    );
    let mut value_growth = base.clone();
    value_growth.update_local_state(
        2,
        LocalState::alias_with_direct(roots(local_count, &[0, 1]), roots(local_count, &[0])),
    );
    let mut direct_growth = base.clone();
    direct_growth.update_local_state(
        2,
        LocalState::alias_with_direct(roots(local_count, &[0, 1]), roots(local_count, &[0, 1])),
    );

    let mut joined = base.clone();
    for incoming in [&mode_growth, &value_growth, &direct_growth] {
        assert!(joined.join_in_place(incoming));
        assert!(
            !joined.join_in_place(incoming),
            "repeating a join adds nothing"
        );
    }

    let local = joined.local_state(2);
    assert_eq!(local.mode, LocalMode::ALIAS.union(LocalMode::SLOT));
    assert_eq!(members(&local.value_roots), vec![0, 1]);
    assert_eq!(members(&local.direct_alias_roots), vec![0, 1]);
}

#[test]
fn visibility_narrowing_drops_only_invisible_roots_and_keeps_slot_storage() {
    let local_count = 4;
    let visible = roots(local_count, &[0, 1, 2]);
    let mut state = BorrowState::new_uninitialized(local_count);
    state.initialize_parameter(0);
    state.update_local_state(3, LocalState::slot(local_count));
    // Local 1 keeps a visible root after narrowing; locals 2 and 0 lose their only root.
    state.update_local_state(
        1,
        LocalState::alias_with_direct(roots(local_count, &[0, 3]), roots(local_count, &[3])),
    );
    state.update_local_state(
        2,
        LocalState::slot_with_value_roots(roots(local_count, &[3]), roots(local_count, &[3])),
    );
    state.update_local_state(
        0,
        LocalState::alias_with_direct(roots(local_count, &[3]), roots(local_count, &[3])),
    );

    state.kill_invisible(&visible);

    let partially_visible = state.local_state(1);
    assert_eq!(partially_visible.mode, LocalMode::ALIAS);
    assert_eq!(members(&partially_visible.value_roots), vec![0]);
    assert!(partially_visible.direct_alias_roots.is_empty());

    // Losing the final value root keeps a slot binding's own storage but not an alias view.
    let slot_backed = state.local_state(2);
    assert_eq!(slot_backed.mode, LocalMode::SLOT);
    assert!(slot_backed.value_roots.is_empty());
    assert!(slot_backed.direct_alias_roots.is_empty());
    let alias_view = state.local_state(0);
    assert_eq!(alias_view.mode, LocalMode::UNINIT);
    assert!(alias_view.value_roots.is_empty());

    assert_eq!(state.local_state(3), &LocalState::uninit(local_count));
}

#[test]
fn joining_states_narrowed_to_one_mask_stays_narrowed() {
    let local_count = 4;
    let visible = roots(local_count, &[0, 1, 2]);
    let mut left = BorrowState::new_uninitialized(local_count);
    left.initialize_parameter(0);
    left.update_local_state(
        1,
        LocalState::alias_with_direct(roots(local_count, &[0, 3]), roots(local_count, &[0, 3])),
    );
    let mut right = BorrowState::new_uninitialized(local_count);
    right.update_local_state(3, LocalState::slot(local_count));
    right.update_local_state(
        1,
        LocalState::slot_with_value_roots(roots(local_count, &[2]), roots(local_count, &[2])),
    );
    right.update_local_state(
        2,
        LocalState::alias_with_direct(roots(local_count, &[3]), roots(local_count, &[3])),
    );
    left.kill_invisible(&visible);
    right.kill_invisible(&visible);

    left.join_in_place(&right);
    let mut renarrowed = left.clone();
    renarrowed.kill_invisible(&visible);

    assert_eq!(renarrowed, left);
}

/// `effective_root_contains` must answer exactly what `effective_roots(..).contains` answers, for
/// every mode and value shape. Alias conflict scans use the shortcut in place of the set builder,
/// so any disagreement would change which conflicts are reported.
#[test]
fn effective_root_contains_matches_effective_roots_membership() {
    // Two words, so the check covers bits on both sides of the word boundary.
    let local_count = 70;
    let modes = [
        LocalMode::UNINIT,
        LocalMode::SLOT,
        LocalMode::ALIAS,
        LocalMode::UNINIT.union(LocalMode::SLOT),
        LocalMode::UNINIT.union(LocalMode::ALIAS),
        LocalMode::SLOT.union(LocalMode::ALIAS),
        LocalMode::UNINIT
            .union(LocalMode::SLOT)
            .union(LocalMode::ALIAS),
    ];
    let all_roots: Vec<usize> = (0..local_count).collect();

    for local_index in 0..local_count {
        let value_shapes = [
            roots(local_count, &[]),
            roots(local_count, &[local_index]),
            roots(local_count, &[0]),
            roots(local_count, &[local_count - 1]),
            roots(local_count, &[local_index, local_count - 1]),
            roots(local_count, &all_roots),
        ];

        for mode in modes {
            for value_roots in &value_shapes {
                let mut state = BorrowState::new_uninitialized(local_count);
                state.update_local_state(
                    local_index,
                    LocalState {
                        mode,
                        value_roots: value_roots.clone(),
                        direct_alias_roots: RootSet::empty(local_count),
                    },
                );

                let expected = state.effective_roots(local_index);
                // One past the last index: both sides must reject an index outside the set.
                for root_index in 0..=local_count {
                    assert_eq!(
                        state.effective_root_contains(local_index, root_index),
                        expected.contains(root_index),
                        "local {local_index}, root {root_index}, mode {mode:?}, value roots {:?}",
                        members(value_roots),
                    );
                }
            }
        }
    }
}

/// `intersects` must answer exactly what intersecting a clone answers, including pairs whose only
/// shared bit sits on either side of the word boundary. Conflict scans use it instead of building
/// the intersection, so a disagreement would change which call arguments count as aliasing.
#[test]
fn intersects_matches_intersection_emptiness() {
    // Two words, so bit 63 and bit 64 exercise both the last bit of word 0 and the first of word 1.
    let local_count = 70;
    let all_roots: Vec<usize> = (0..local_count).collect();
    let shapes = [
        roots(local_count, &[]),
        roots(local_count, &[0]),
        roots(local_count, &[63]),
        roots(local_count, &[64]),
        roots(local_count, &[local_count - 1]),
        roots(local_count, &[63, 64]),
        roots(local_count, &[0, 63, 64, local_count - 1]),
        roots(local_count, &all_roots),
    ];

    for left in &shapes {
        for right in &shapes {
            let mut intersection = left.clone();
            intersection.intersect_with(right);
            assert_eq!(
                left.intersects(right),
                !intersection.is_empty(),
                "left {:?}, right {:?}",
                members(left),
                members(right),
            );
        }
    }
}
