//! Borrow-checker state invariant tests.
//!
//! WHAT: protects slot-backed alias rebinding, independent value-provenance and direct-alias root
//!       sets, must-joined alias derivation, join growth reporting, visibility narrowing, the
//!       skip a covering visibility mask takes, and the membership shortcut that alias conflict
//!       scans use in place of the set builder.
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

    // Each incoming state grows exactly one fact of local 2: its mode, its value roots, its
    // direct-alias roots or its write targets. Worklist scheduling depends on every kind of
    // growth being reported. Each later state adds a root bit the earlier states do not hold,
    // because the three root sets are independent facts and each joined bit must report growth
    // on its own.
    let mut mode_growth = base.clone();
    mode_growth.update_local_state(
        2,
        LocalState {
            mode: LocalMode::SLOT,
            value_roots: roots(local_count, &[0]),
            direct_alias_roots: roots(local_count, &[0]),
            write_target_roots: roots(local_count, &[0]),
            derived_from_roots: RootSet::empty(local_count),
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
        LocalState::alias_with_cells(
            roots(local_count, &[0]),
            roots(local_count, &[0, 2]),
            roots(local_count, &[0]),
        ),
    );
    let mut target_growth = base.clone();
    target_growth.update_local_state(
        2,
        LocalState::alias_with_cells(
            roots(local_count, &[0]),
            roots(local_count, &[0]),
            roots(local_count, &[0, 1]),
        ),
    );

    let mut joined = base.clone();
    for incoming in [&mode_growth, &value_growth, &direct_growth, &target_growth] {
        assert!(joined.join_in_place(incoming));
        assert!(
            !joined.join_in_place(incoming),
            "repeating a join adds nothing"
        );
    }

    let local = joined.local_state(2);
    assert_eq!(local.mode, LocalMode::ALIAS.union(LocalMode::SLOT));
    assert_eq!(members(&local.value_roots), vec![0, 1]);
    assert_eq!(members(&local.direct_alias_roots), vec![0, 2]);
    assert_eq!(members(&local.write_target_roots), vec![0, 1]);
}

#[test]
fn join_intersects_derived_alias_facts() {
    let local_count = 3;
    let mut left = BorrowState::new_uninitialized(local_count);
    let mut left_alias =
        LocalState::alias_with_direct(roots(local_count, &[0]), roots(local_count, &[0]));
    left_alias.derived_from_roots = roots(local_count, &[0, 2]);
    left.update_local_state(1, left_alias);

    let mut right = BorrowState::new_uninitialized(local_count);
    let mut right_alias =
        LocalState::alias_with_direct(roots(local_count, &[0]), roots(local_count, &[0]));
    right_alias.derived_from_roots = roots(local_count, &[0, 1]);
    right.update_local_state(1, right_alias);

    assert!(left.join_in_place(&right));
    assert_eq!(
        members(&left.local_state(1).derived_from_roots),
        vec![0],
        "only an alias derived on every incoming path is a must fact"
    );
    assert!(!left.join_in_place(&right), "repeating a join adds nothing");
}

#[test]
fn visibility_narrowing_prunes_each_root_set_independently() {
    let local_count = 7;
    let visible = roots(local_count, &[0, 1, 2, 4, 5]);
    let mut state = BorrowState::new_uninitialized(local_count);
    state.initialize_parameter(0);
    state.update_local_state(3, LocalState::slot(local_count));
    // Local 1 keeps a visible root in each set after narrowing. The invisible bits differ, so
    // no set may be pruned as a side effect of pruning another.
    let mut partially_visible = LocalState::alias_with_cells(
        roots(local_count, &[0, 3]),
        roots(local_count, &[2, 3]),
        roots(local_count, &[1, 3]),
    );
    partially_visible.derived_from_roots = roots(local_count, &[0, 3]);
    state.update_local_state(1, partially_visible);
    // Local 2 loses every value root but still observes a visible referent cell. That cell is
    // the alias view's remaining identity, so the binding must survive as an alias.
    let mut cell_only =
        LocalState::alias_with_direct(roots(local_count, &[3]), roots(local_count, &[1, 3]));
    cell_only.derived_from_roots = roots(local_count, &[3, 4]);
    state.update_local_state(2, cell_only);
    // Local 0 keeps only value provenance after its direct cell leaves visibility.
    let mut value_only =
        LocalState::alias_with_direct(roots(local_count, &[1]), roots(local_count, &[3]));
    value_only.derived_from_roots = roots(local_count, &[3]);
    state.update_local_state(0, value_only);
    // Local 4 keeps a slot's own storage but loses every root set.
    state.update_local_state(
        4,
        LocalState::slot_with_value_roots(roots(local_count, &[3]), roots(local_count, &[3])),
    );
    // Local 5 has no value provenance or direct cell left, but a visible write target keeps it
    // alive as an alias: it can still mutate that referent.
    state.update_local_state(
        5,
        LocalState::alias_with_cells(
            RootSet::empty(local_count),
            RootSet::empty(local_count),
            roots(local_count, &[1, 3]),
        ),
    );
    // Local 6 only remembers a derivation; it observes no allocation, so it cannot survive as an
    // alias and its ledger is cleared with the binding.
    let mut derived_only =
        LocalState::alias_with_direct(RootSet::empty(local_count), RootSet::empty(local_count));
    derived_only.derived_from_roots = roots(local_count, &[0, 3]);
    state.update_local_state(6, derived_only);

    state.kill_invisible(&visible);

    let partially_visible = state.local_state(1);
    assert_eq!(partially_visible.mode, LocalMode::ALIAS);
    assert_eq!(members(&partially_visible.value_roots), vec![0]);
    assert_eq!(members(&partially_visible.direct_alias_roots), vec![2]);
    assert_eq!(members(&partially_visible.write_target_roots), vec![1]);
    assert_eq!(members(&partially_visible.derived_from_roots), vec![0]);

    let cell_only = state.local_state(2);
    assert_eq!(cell_only.mode, LocalMode::ALIAS);
    assert!(cell_only.value_roots.is_empty());
    assert_eq!(members(&cell_only.direct_alias_roots), vec![1]);
    assert_eq!(members(&cell_only.derived_from_roots), vec![4]);

    let value_only = state.local_state(0);
    assert_eq!(value_only.mode, LocalMode::ALIAS);
    assert_eq!(members(&value_only.value_roots), vec![1]);
    assert!(value_only.direct_alias_roots.is_empty());
    assert!(value_only.derived_from_roots.is_empty());

    // Losing every root keeps a slot binding's own storage but leaves an alias view with nothing
    // to observe.
    let slot_backed = state.local_state(4);
    assert_eq!(slot_backed.mode, LocalMode::SLOT);
    assert!(slot_backed.value_roots.is_empty());
    assert!(slot_backed.direct_alias_roots.is_empty());
    assert!(slot_backed.write_target_roots.is_empty());

    let derived_only = state.local_state(6);
    assert_eq!(derived_only.mode, LocalMode::UNINIT);
    assert!(derived_only.derived_from_roots.is_empty());

    let target_only = state.local_state(5);
    assert_eq!(target_only.mode, LocalMode::ALIAS);
    assert!(target_only.value_roots.is_empty());
    assert!(target_only.direct_alias_roots.is_empty());
    assert_eq!(members(&target_only.write_target_roots), vec![1]);

    assert_eq!(state.local_state(3), &LocalState::uninit(local_count));
}

#[test]
fn designated_cells_include_binding_ownership_and_write_targets() {
    let local_count = 4;
    let mut state = BorrowState::new_uninitialized(local_count);
    state.update_local_state(
        0,
        LocalState {
            mode: LocalMode::SLOT,
            value_roots: roots(local_count, &[3]),
            direct_alias_roots: roots(local_count, &[1]),
            write_target_roots: roots(local_count, &[2]),
            derived_from_roots: RootSet::empty(local_count),
        },
    );
    state.update_local_state(
        1,
        LocalState::alias_with_cells(
            roots(local_count, &[3]),
            roots(local_count, &[1]),
            roots(local_count, &[2]),
        ),
    );
    state.update_local_state(
        2,
        LocalState::alias_with_cells(
            roots(local_count, &[3]),
            RootSet::empty(local_count),
            RootSet::empty(local_count),
        ),
    );

    // A slot owns its own cell; an alias designates only its referents, including a projection's
    // base cell even though its value provenance is an older allocation.
    assert_eq!(members(&state.designated_cells(0)), vec![0, 1, 2]);
    assert_eq!(members(&state.designated_cells(1)), vec![1, 2]);
    assert!(state.designated_cells(2).is_empty());
}

// WHAT: an alias whose every root set loses visibility has nothing left to observe, so a
//       visibility sweep resets it to UNINIT even though the binding itself stays visible.
// WHY: a cell-only alias carried across a merge must not survive as a rootless alias view.
#[test]
fn visible_alias_with_no_roots_left_becomes_uninit() {
    let local_count = 4;
    let mut state = BorrowState::new_uninitialized(local_count);
    state.update_local_state(
        1,
        LocalState::alias_with_cells(
            roots(local_count, &[3]),
            roots(local_count, &[3]),
            roots(local_count, &[3]),
        ),
    );
    state.update_local_state(3, LocalState::slot(local_count));

    // Local 1 is visible; every root it observes is local 3, which is not.
    let visible = roots(local_count, &[0, 1, 2]);
    state.kill_invisible(&visible);

    assert_eq!(state.local_state(1), &LocalState::uninit(local_count));
    assert_eq!(state.local_state(3), &LocalState::uninit(local_count));
}

// WHAT: a cell-only alias survives a join and a visibility edge while its referent cell is
//       visible, then becomes UNINIT when the referent itself leaves visibility.
// WHY: the cell is the alias's only remaining identity after a branch-local value root is gone;
//       dropping the mode at either the join or the sweep loses the conflict the alias reports.
#[test]
fn cell_only_alias_survives_join_and_scope_narrowing() {
    let local_count = 4;
    let mut left = BorrowState::new_uninitialized(local_count);
    left.update_local_state(
        1,
        LocalState::alias_with_cells(
            RootSet::empty(local_count),
            roots(local_count, &[0]),
            RootSet::empty(local_count),
        ),
    );
    let mut right = BorrowState::new_uninitialized(local_count);
    right.update_local_state(
        1,
        LocalState::alias_with_cells(
            RootSet::empty(local_count),
            roots(local_count, &[0, 3]),
            RootSet::empty(local_count),
        ),
    );

    assert!(left.join_in_place(&right));
    let joined = left.local_state(1);
    assert_eq!(joined.mode, LocalMode::ALIAS);
    assert!(joined.value_roots.is_empty());
    assert_eq!(members(&joined.direct_alias_roots), vec![0, 3]);
    assert!(joined.write_target_roots.is_empty());

    // The branch-local referent cell leaves visibility; the outer one keeps the alias alive.
    left.kill_invisible(&roots(local_count, &[0, 1, 2]));
    let narrowed = left.local_state(1);
    assert_eq!(narrowed.mode, LocalMode::ALIAS);
    assert!(narrowed.value_roots.is_empty());
    assert_eq!(members(&narrowed.direct_alias_roots), vec![0]);
    assert!(narrowed.write_target_roots.is_empty());

    // A merge with a path that never rebound the binding keeps the alias alternative.
    let mut slot_branch = BorrowState::new_uninitialized(local_count);
    slot_branch.update_local_state(1, LocalState::slot(local_count));
    let mut merged = left.clone();
    assert!(merged.join_in_place(&slot_branch));
    assert!(merged.local_state(1).mode.contains(LocalMode::ALIAS));
    merged.kill_invisible(&roots(local_count, &[0, 1, 2]));
    let merged_state = merged.local_state(1);
    assert!(merged_state.mode.contains(LocalMode::ALIAS));
    assert!(merged_state.value_roots.is_empty());
    assert_eq!(members(&merged_state.direct_alias_roots), vec![0]);

    // Losing the last visible referent leaves nothing to observe.
    let mut pruned = left.clone();
    pruned.kill_invisible(&roots(local_count, &[1, 2, 3]));
    assert_eq!(pruned.local_state(1), &LocalState::uninit(local_count));
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

/// A sweep may only skip a mask that provably covers every local. Skipping one that misses a bit
/// would keep a root outside the block's visibility, so this pins the cover test itself: the
/// full mask of the same width covers, and an empty, narrower or one-bit-short mask does not.
#[test]
fn root_set_covers_all_only_for_a_full_mask_of_the_same_width() {
    // A partial final word and both sides of the word boundary are covered by the widths tried.
    const LOCAL_COUNT: usize = 70;
    for width in [0usize, 1, 63, 64, 65, LOCAL_COUNT] {
        let full = RootSet::full(width);
        assert!(full.covers_all(width));
        assert_eq!(RootSet::empty(width).covers_all(width), width == 0);
        // A mask for a larger or smaller universe is not this universe's cover.
        assert!(!full.covers_all(width + 1));
        if width > 0 {
            assert!(!full.covers_all(width - 1));
            let mut missing = RootSet::full(width);
            missing.subtract_with(&roots(width, &[width - 1]));
            assert!(!missing.covers_all(width));
        }
    }
}

/// A covering mask hides no local and removes no root bit, so the sweep must leave the state
/// alone; the same state swept with one visible bit removed must still lose the roots on it.
#[test]
fn visibility_sweep_leaves_a_state_unchanged_under_a_covering_mask() {
    const LOCAL_COUNT: usize = 70;
    const LAST: usize = LOCAL_COUNT - 1;
    let local_count = LOCAL_COUNT;
    let mut state = BorrowState::new_uninitialized(local_count);
    state.initialize_parameter(0);
    state.update_local_state(
        1,
        LocalState::alias_with_direct(roots(local_count, &[0, LAST]), roots(local_count, &[LAST])),
    );
    state.update_local_state(
        2,
        LocalState::slot_with_value_roots(roots(local_count, &[1]), roots(local_count, &[1])),
    );

    let before = state.clone();
    state.kill_invisible(&RootSet::full(local_count));
    assert_eq!(state, before);

    // Bit 1 carries local 2's only root and local 1's direct alias, and local 1 becomes invisible.
    let mut narrowed = RootSet::full(local_count);
    narrowed.subtract_with(&roots(local_count, &[1]));
    state.kill_invisible(&narrowed);

    assert_eq!(state.local_state(1), &LocalState::uninit(local_count));
    let narrowed_slot = state.local_state(2);
    assert_eq!(narrowed_slot.mode, LocalMode::SLOT);
    assert!(narrowed_slot.value_roots.is_empty());
    assert!(narrowed_slot.direct_alias_roots.is_empty());
    assert_eq!(state.local_state(0).mode, LocalMode::SLOT);
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
                        write_target_roots: RootSet::empty(local_count),
                        derived_from_roots: RootSet::empty(local_count),
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

/// Copy-on-write words must behave exactly like value semantics: an update shows on the receiver,
/// never on a clone, and `union_with` reports the growth a set with unique words reports. Each
/// update runs against a shared receiver and against both ways a receiver's words become unique:
/// fresh construction, and dropping the last clone that shared them. The tail covers an empty set
/// adopting a donor's words, complete removal followed by reinsertion, and `full`.
#[test]
fn root_set_updates_match_between_shared_and_unique_words() {
    // Two words with a partial final word: 70 locals leave bits 64..=69 in word 1.
    const LOCAL_COUNT: usize = 70;
    const LAST: usize = LOCAL_COUNT - 1;
    const MIDDLE: usize = LOCAL_COUNT / 2;
    const START: [usize; 4] = [0, 63, 64, LAST];

    // `None` where the operation reports nothing, so union's growth flag is compared exactly.
    type Update = fn(&mut RootSet) -> Option<bool>;
    let updates: [(&str, Update); 10] = [
        ("insert word-boundary bit", |set| {
            set.insert(64);
            None
        }),
        ("insert past the last local", |set| {
            set.insert(LOCAL_COUNT);
            None
        }),
        ("union with a new bit", |set| {
            Some(set.union_with(&roots(LOCAL_COUNT, &[MIDDLE])))
        }),
        ("union with present bits", |set| {
            Some(set.union_with(&roots(LOCAL_COUNT, &[0, LAST])))
        }),
        ("union with an empty set", |set| {
            Some(set.union_with(&roots(LOCAL_COUNT, &[])))
        }),
        ("intersect with boundary bits", |set| {
            set.intersect_with(&roots(LOCAL_COUNT, &[63, 64]));
            None
        }),
        ("intersect with an empty set", |set| {
            set.intersect_with(&roots(LOCAL_COUNT, &[]));
            None
        }),
        ("subtract final bits", |set| {
            set.subtract_with(&roots(LOCAL_COUNT, &[64, LAST]));
            None
        }),
        ("subtract an empty set", |set| {
            set.subtract_with(&roots(LOCAL_COUNT, &[]));
            None
        }),
        ("clear", |set| {
            set.clear();
            None
        }),
    ];

    let start = roots(LOCAL_COUNT, &START);
    for (name, update) in updates {
        let mut shared = roots(LOCAL_COUNT, &START);
        let sibling = shared.clone();
        let shared_flag = update(&mut shared);

        let mut unique_from_construction = roots(LOCAL_COUNT, &START);
        let construction_flag = update(&mut unique_from_construction);

        let mut unique_after_drop = roots(LOCAL_COUNT, &START);
        let last_clone = unique_after_drop.clone();
        drop(last_clone);
        let dropped_flag = update(&mut unique_after_drop);

        assert_eq!(
            members(&shared),
            members(&unique_from_construction),
            "{name}: shared differs from words built unique"
        );
        assert_eq!(
            members(&shared),
            members(&unique_after_drop),
            "{name}: shared differs from words made unique by dropping the last clone"
        );
        assert_eq!(shared_flag, construction_flag, "{name}: growth flag");
        assert_eq!(shared_flag, dropped_flag, "{name}: growth flag");
        assert_eq!(
            members(&sibling),
            members(&start),
            "{name}: a clone's contents must not change"
        );
    }

    // The loop above can only compare receivers; these assertions pin the expected values.
    let mut set = roots(LOCAL_COUNT, &START);
    assert!(set.union_with(&roots(LOCAL_COUNT, &[MIDDLE])));
    assert_eq!(members(&set), vec![0, MIDDLE, 63, 64, LAST]);
    assert!(!set.union_with(&roots(LOCAL_COUNT, &[MIDDLE])));
    let union_sibling = set.clone();
    assert!(set.union_with(&roots(LOCAL_COUNT, &[1])));
    assert_eq!(members(&set), vec![0, 1, MIDDLE, 63, 64, LAST]);
    assert_eq!(members(&union_sibling), vec![0, MIDDLE, 63, 64, LAST]);
    set.intersect_with(&roots(LOCAL_COUNT, &[63, 64, LAST]));
    assert_eq!(members(&set), vec![63, 64, LAST]);
    set.subtract_with(&roots(LOCAL_COUNT, &[64]));
    assert_eq!(members(&set), vec![63, LAST]);

    // Empty sets own no words: every operation keeps them empty, and clearing one changes nothing.
    let empty = roots(LOCAL_COUNT, &[]);
    let mut empty_receiver = empty.clone();
    let empty_sibling = empty_receiver.clone();
    assert!(!empty_receiver.union_with(&empty));
    empty_receiver.insert(MIDDLE);
    assert_eq!(members(&empty_receiver), vec![MIDDLE]);
    assert!(empty_sibling.is_empty());
    empty_receiver.clear();
    assert!(empty_receiver.is_empty());
    let mut shared_empty = empty_sibling.clone();
    shared_empty.clear();
    assert!(shared_empty.is_empty());
    empty_receiver.subtract_with(&empty);
    empty_receiver.intersect_with(&empty);
    empty_receiver.insert(LOCAL_COUNT);
    assert!(empty_receiver.is_empty());

    // An empty receiver adopts a donor's words, so whichever side writes first must copy. The
    // donor spans the word boundary and leaves bits 65..=LAST absent.
    let donor_bits: Vec<usize> = (0..=64).collect();
    let donor = roots(LOCAL_COUNT, &donor_bits);
    let mut adopted = roots(LOCAL_COUNT, &[]);
    assert!(adopted.union_with(&donor));
    assert_eq!(adopted, donor);
    assert!(!adopted.union_with(&donor));
    adopted.insert(LAST);
    assert_eq!(members(&donor), donor_bits);
    let mut adopted_bits = donor_bits.clone();
    adopted_bits.push(LAST);
    assert_eq!(members(&adopted), adopted_bits);

    // The donor writes after adoption. Bit 63 is already set, so inserting it returns before any
    // write; LAST is absent, so the write must copy the shared words.
    let mut donor_owner = roots(LOCAL_COUNT, &donor_bits);
    let mut adopter = roots(LOCAL_COUNT, &[]);
    assert!(adopter.union_with(&donor_owner));
    donor_owner.insert(LAST);
    assert_eq!(members(&adopter), donor_bits);
    let mut donor_owner_bits = donor_bits.clone();
    donor_owner_bits.push(LAST);
    assert_eq!(members(&donor_owner), donor_owner_bits);

    // Complete removal must reach the same `None` state for every receiver kind, and reinsertion
    // must not leak into a sibling clone that still holds the original [0, 64].
    let removals: [fn(&mut RootSet); 2] = [
        |set| {
            set.intersect_with(&roots(LOCAL_COUNT, &[63, 64, 65, 66, 67, 68, LAST]));
            assert_eq!(members(set), vec![63, 64]);
            set.subtract_with(&roots(LOCAL_COUNT, &[63, 64]));
        },
        |set| {
            let all: Vec<usize> = (0..=64).collect();
            set.subtract_with(&roots(LOCAL_COUNT, &all));
        },
    ];
    for removal in removals {
        for receiver_kind in ["fresh", "shared", "unique after drop"] {
            let mut receiver = roots(LOCAL_COUNT, &donor_bits);
            let sibling = match receiver_kind {
                "shared" => Some(receiver.clone()),
                "unique after drop" => {
                    // A clone dropped before the update leaves the receiver's words unique again.
                    drop(receiver.clone());
                    None
                }
                _ => None,
            };
            removal(&mut receiver);
            assert!(receiver.is_empty(), "{receiver_kind}");
            assert_eq!(receiver, RootSet::empty(LOCAL_COUNT), "{receiver_kind}");
            assert_eq!(members(&receiver), Vec::<usize>::new(), "{receiver_kind}");
            receiver.insert(LAST);
            assert_eq!(members(&receiver), vec![LAST], "{receiver_kind}");
            if let Some(sibling) = sibling {
                assert_eq!(members(&sibling), donor_bits, "{receiver_kind}");
            }
        }
    }

    // `full` must equal the set built bit by bit, including the zero-width universe and widths
    // on both sides of the word boundary.
    assert_eq!(RootSet::full(0), RootSet::empty(0));
    for width in [1, 64, 65, LOCAL_COUNT] {
        let all: Vec<usize> = (0..width).collect();
        assert_eq!(RootSet::full(width), roots(width, &all), "full({width})");
    }
}
