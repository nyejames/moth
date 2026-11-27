//! Borrow-checker state invariant tests.
//!
//! WHAT: protects slot-backed rebinding, independent value provenance and cell identity,
//!       must-joined alias derivation, join growth reporting, visibility narrowing, the skip a
//!       covering visibility mask takes, and the allocation-free membership shortcut.
//! WHY: these facts live in transfer state and cannot be inspected from rendered output.

use super::super::state::{
    AddressSet, BindingSpec, BorrowState, CellId, Coverage, FieldKey, RootSet, StoredValue,
    WrapperAge, WrapperId,
};
use super::super::types::LocalMode;

#[test]
fn slot_backed_alias_rebinding_clears_old_value_roots() {
    let local_count = 2;
    let mut state = BorrowState::new_uninitialized(local_count);
    state.initialize_parameter(0);

    let mut aliased_root = RootSet::empty(local_count);
    aliased_root.insert(0);
    state.set_binding(
        1,
        BindingSpec::Slot,
        aliased_root,
        RootSet::empty(local_count),
    );

    assert_eq!(
        state.effective_roots(1).iter_ones().collect::<Vec<_>>(),
        vec![0]
    );

    state.initialize_parameter(1);

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

fn slot(local: usize) -> CellId {
    CellId::Slot(WrapperId::current(local))
}

fn prior_slot(local: usize) -> CellId {
    CellId::Slot(WrapperId {
        local,
        age: WrapperAge::Prior,
    })
}

fn bind_whole(
    state: &mut BorrowState,
    local: usize,
    source: usize,
    value_roots: RootSet,
    derivation: RootSet,
) {
    state.set_binding(
        local,
        BindingSpec::Link {
            source: WrapperId::current(source),
            writable: true,
        },
        value_roots,
        derivation,
    );
}

fn assert_uninitialized(state: &BorrowState, local: usize) {
    let binding = state.binding(local);
    assert_eq!(state.binding_mode(local), LocalMode::UNINIT);
    assert!(binding.value_roots().is_empty());
    assert!(binding.derivation().is_empty());
    assert!(state.resolve_cells(local).cells().is_empty());
}

#[test]
fn join_reports_growth_for_each_fact_and_keeps_joined_facts() {
    let count = 5;
    let empty = RootSet::empty(count);
    let source = roots(count, &[0]);
    let mut base = BorrowState::new_uninitialized(count);
    base.initialize_parameter(0);
    base.set_binding(1, BindingSpec::Slot, source.clone(), empty.clone());
    base.initialize_parameter(3);
    base.initialize_parameter(4);
    let holder = base.local_alloc_ids(4).0[0];
    let field = FieldKey::Field(0);
    let held = base.local_alloc_ids(0).0[0];
    base.add_alloc_edge(
        holder,
        field,
        StoredValue {
            id: held,
            writable: true,
        },
    );
    bind_whole(&mut base, 2, 0, source.clone(), empty.clone());

    // Each predecessor adds a fact of local 2. An ordinary slot and two distinct destinations
    // are separate alternatives, not independent root sets on a single binding.
    let mut mode_growth = base.clone();
    mode_growth.set_binding(2, BindingSpec::Slot, source.clone(), empty.clone());

    let mut value_growth = base.clone();
    let values = roots(count, &[0, 3]);
    value_growth.set_binding(0, BindingSpec::Slot, values.clone(), empty.clone());
    bind_whole(&mut value_growth, 2, 0, values, empty.clone());

    let mut direct_growth = base.clone();
    bind_whole(&mut direct_growth, 2, 1, source.clone(), empty.clone());

    let projected = CellId::Projected(holder, field);
    let mut target_growth = base.clone();
    target_growth.set_binding(
        2,
        BindingSpec::Alias {
            addresses: AddressSet::from_cells([projected], Coverage::Complete),
            route_roots: roots(count, &[4]),
        },
        source,
        empty,
    );

    let mut joined = base;
    for incoming in [&mode_growth, &value_growth, &direct_growth, &target_growth] {
        assert!(joined.join_in_place(incoming));
        assert!(
            !joined.join_in_place(incoming),
            "repeating a join adds nothing"
        );
    }

    let binding = joined.binding(2);
    assert_eq!(
        joined.binding_mode(2),
        LocalMode::ALIAS.union(LocalMode::SLOT)
    );
    assert_eq!(members(binding.value_roots()), vec![0, 3]);
    assert_eq!(
        joined.resolve_cells(2).cells(),
        &[slot(0), slot(1), slot(2), projected]
    );
    assert!(binding.route_roots().contains(4));
}

#[test]
fn implicit_slot_provenance_survives_every_predecessor_order() {
    let count = 3;
    let empty = RootSet::empty(count);
    let mut fresh = BorrowState::new_uninitialized(count);
    for local in 0..count {
        fresh.initialize_parameter(local);
    }
    let mut inherited = fresh.clone();
    inherited.set_binding(0, BindingSpec::Slot, roots(count, &[2]), empty.clone());
    let mut alias = fresh.clone();
    bind_whole(&mut alias, 0, 1, roots(count, &[1]), empty);
    let inputs = [&fresh, &inherited, &alias];

    for [a, b, c] in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        // Solve the last two predecessors first as well as varying insertion order.
        let mut pair = (*inputs[b]).clone();
        pair.join_in_place(inputs[c]);
        let mut joined = (*inputs[a]).clone();
        joined.join_in_place(&pair);
        assert_eq!(joined.resolve_cells(0).cells(), &[slot(0), slot(1)]);
        assert_eq!(members(&joined.effective_roots(0)), vec![0, 1, 2]);
    }
}

#[test]
fn repeated_retirement_keeps_lazy_and_explicit_payloads() {
    let count = 4;
    let empty = RootSet::empty(count);
    let visible = roots(count, &[1, 2, 3]);
    for explicit_first in [false, true] {
        let mut state = BorrowState::new_uninitialized(count);
        state.initialize_parameter(3);
        let explicit = state.local_alloc_ids(3).0[0];
        for observer in [1, 2] {
            state.initialize_parameter(0);
            if (observer == 1) == explicit_first {
                state.bind_alloc_ids(0, &[explicit], false);
            }
            bind_whole(&mut state, observer, 0, empty.clone(), empty.clone());
            state.kill_invisible(&visible);
            assert_eq!(state.resolve_cells(observer).cells(), &[prior_slot(0)]);
        }

        let (held, _) = state.local_alloc_ids(1);
        assert_eq!(held.len(), 2);
        assert!(held.contains(&explicit));
        assert_eq!(state.local_alloc_ids(2).0, held);
    }
}

#[test]
fn visibility_edge_preserves_payloads_independently_of_interning() {
    let count = 3;
    let empty = RootSet::empty(count);
    let root = roots(count, &[0]);
    for with_link in [false, true] {
        let visible = roots(count, &[1, 2]);
        let mut fresh = BorrowState::new_uninitialized(count);
        fresh.initialize_parameter(0);
        if with_link {
            bind_whole(&mut fresh, 1, 0, root.clone(), empty.clone());
        }
        fresh.set_binding(2, BindingSpec::Slot, root.clone(), empty.clone());

        // Use the same table so the test compares abstract states, not unrelated numeric IDs.
        // The first edge runs before the sibling has caused a lookup of the implicit root.
        let mut preinterned = fresh.clone();
        fresh.kill_invisible(&visible);
        let current = preinterned.local_alloc_ids(0).0[0];
        preinterned.kill_invisible(&visible);
        assert_eq!(fresh, preinterned);

        assert_eq!(fresh.resolve_cells(2).cells(), &[slot(2)]);
        let retained = fresh.local_alloc_ids(2).0;
        assert_eq!(retained.len(), 1);
        assert_ne!(retained[0], current);
        assert_eq!(fresh.alloc_origin_local(retained[0]), Some(0));
        if with_link {
            assert_eq!(fresh.resolve_cells(1).cells(), &[prior_slot(0)]);
            assert_eq!(fresh.local_alloc_ids(1).0, retained);
        }
    }
}

#[test]
fn join_intersects_derived_alias_facts() {
    let count = 5;
    let empty = RootSet::empty(count);
    let source = roots(count, &[4]);
    let mut left = BorrowState::new_uninitialized(count);
    left.initialize_parameter(4);
    bind_whole(&mut left, 0, 4, source.clone(), empty);
    let mut right = left.clone();
    for (state, intermediate) in [(&mut left, 2), (&mut right, 3)] {
        let derivation = state.derived_roots_from_source(0);
        bind_whole(state, intermediate, 0, source.clone(), derivation);
        let derivation = state.derived_roots_from_source(intermediate);
        bind_whole(state, 1, intermediate, source.clone(), derivation);
    }

    assert!(left.join_in_place(&right));
    assert_eq!(
        members(left.binding(1).derivation()),
        vec![0],
        "only an alias derived on every incoming path is a must fact"
    );
    assert!(!left.join_in_place(&right), "repeating a join adds nothing");
}

#[test]
fn visibility_narrowing_prunes_each_root_set_independently() {
    let count = 13;
    let visible = roots(count, &[0, 1, 2, 4, 5, 7, 8, 9, 11]);
    let empty = RootSet::empty(count);
    let departing_roots = roots(count, &[3]);
    let mut state = BorrowState::new_uninitialized(count);
    state.initialize_parameter(3);
    state.initialize_parameter(8);
    for local in [4, 7] {
        state.set_binding(
            local,
            BindingSpec::Slot,
            departing_roots.clone(),
            empty.clone(),
        );
    }

    let mut departing_path = state.clone();
    for (incoming, first, second) in [(&mut state, 8, 7), (&mut departing_path, 3, 3)] {
        // Each alias derives through both a visible and a departing wrapper, independently of
        // whether the ultimate referent is itself visible.
        for (local, visible_alias, departing_alias, referent) in
            [(1, 9, 10, first), (2, 11, 12, second)]
        {
            let value = incoming.effective_roots(referent);
            bind_whole(
                incoming,
                departing_alias,
                referent,
                value.clone(),
                empty.clone(),
            );
            let derivation = incoming.derived_roots_from_source(departing_alias);
            bind_whole(
                incoming,
                visible_alias,
                departing_alias,
                value.clone(),
                derivation,
            );
            let derivation = incoming.derived_roots_from_source(visible_alias);
            bind_whole(incoming, local, visible_alias, value, derivation);
        }

        // An unresolved alias can still observe a value, independently of a referent address.
        for (local, source, values) in [
            (6, 10, empty.clone()),
            (0, 6, incoming.effective_roots(first)),
        ] {
            let derivation = incoming.derived_roots_from_source(source);
            incoming.set_binding(
                local,
                BindingSpec::Alias {
                    addresses: AddressSet::from_cells([], Coverage::Partial),
                    route_roots: empty.clone(),
                },
                values,
                derivation,
            );
        }
    }
    assert!(state.join_in_place(&departing_path));

    // A projected scalar alias retains its address even though it has no value provenance.
    let holder = state.local_alloc_ids(8).0[0];
    let projected = CellId::Projected(holder, FieldKey::Field(0));
    state.set_binding(
        5,
        BindingSpec::Alias {
            addresses: AddressSet::from_cells([projected], Coverage::Complete),
            route_roots: roots(count, &[8]),
        },
        empty.clone(),
        empty,
    );

    state.kill_invisible(&visible);

    let partially_visible = state.binding(1);
    assert_eq!(state.binding_mode(1), LocalMode::ALIAS);
    assert_eq!(members(partially_visible.value_roots()), vec![8]);
    assert_eq!(state.resolve_cells(1).cells(), &[prior_slot(3), slot(8)]);
    assert_eq!(members(&partially_visible.route_roots()), vec![8, 9]);
    assert_eq!(members(partially_visible.derivation()), vec![9]);

    let cell_only = state.binding(2);
    assert_eq!(state.binding_mode(2), LocalMode::ALIAS);
    assert!(cell_only.value_roots().is_empty());
    assert_eq!(state.resolve_cells(2).cells(), &[prior_slot(3), slot(7)]);
    assert_eq!(members(cell_only.derivation()), vec![11]);

    let value_only = state.binding(0);
    assert_eq!(state.binding_mode(0), LocalMode::ALIAS);
    assert_eq!(members(value_only.value_roots()), vec![8]);
    assert!(state.resolve_cells(0).cells().is_empty());
    assert!(value_only.derivation().is_empty());

    let slot_backed = state.binding(4);
    assert_eq!(state.binding_mode(4), LocalMode::SLOT);
    assert!(slot_backed.value_roots().is_empty());
    assert_eq!(state.resolve_cells(4).cells(), &[slot(4)]);
    assert!(slot_backed.route_roots().is_empty());

    assert_uninitialized(&state, 6);

    let target_only = state.binding(5);
    assert_eq!(state.binding_mode(5), LocalMode::ALIAS);
    assert!(target_only.value_roots().is_empty());
    assert_eq!(state.resolve_cells(5).cells(), &[projected]);
    assert_eq!(members(&target_only.route_roots()), vec![8]);

    assert_uninitialized(&state, 3);
}

#[test]
fn resolved_cells_include_slot_and_alias_alternatives_but_not_provenance() {
    let count = 5;
    let empty = RootSet::empty(count);
    let value = roots(count, &[3]);
    let mut state = BorrowState::new_uninitialized(count);
    state.initialize_parameter(3);
    state.set_binding(2, BindingSpec::Slot, value.clone(), empty.clone());

    let mut alternate = state.clone();
    alternate.initialize_parameter(4);
    let holder = alternate.local_alloc_ids(4).0[0];
    let key = FieldKey::Field(0);
    let projected = CellId::Projected(holder, key);
    let held = alternate.local_alloc_ids(3).0[0];
    alternate.add_alloc_edge(
        holder,
        key,
        StoredValue {
            id: held,
            writable: true,
        },
    );
    let mut whole = alternate.clone();
    bind_whole(&mut whole, 1, 2, value.clone(), empty.clone());
    alternate.set_binding(
        1,
        BindingSpec::Alias {
            addresses: AddressSet::from_cells([projected], Coverage::Complete),
            route_roots: roots(count, &[4]),
        },
        value.clone(),
        empty.clone(),
    );
    assert!(whole.join_in_place(&alternate));
    let addresses = whole.resolve_cells(1);
    assert_eq!(addresses.cells(), &[slot(2), projected]);

    state.set_binding(1, BindingSpec::Slot, value.clone(), empty.clone());
    assert!(state.join_in_place(&whole));
    assert_eq!(
        state.resolve_cells(1).cells(),
        &[slot(1), slot(2), projected]
    );

    state.set_binding(
        0,
        BindingSpec::Alias {
            addresses: AddressSet::from_cells([], Coverage::Partial),
            route_roots: empty.clone(),
        },
        value,
        empty,
    );
    assert!(state.resolve_cells(0).cells().is_empty());
}

#[test]
fn visible_alias_retains_a_scalar_slot_through_departing_wrappers() {
    let local_count = 4;
    let mut state = BorrowState::new_uninitialized(local_count);
    state.initialize_parameter(3);
    bind_whole(
        &mut state,
        2,
        3,
        RootSet::empty(local_count),
        RootSet::empty(local_count),
    );

    bind_whole(
        &mut state,
        1,
        2,
        RootSet::empty(local_count),
        RootSet::empty(local_count),
    );

    state.kill_invisible(&roots(local_count, &[0, 1]));

    assert_eq!(state.binding_mode(1), LocalMode::ALIAS);
    assert!(state.binding(1).value_roots().is_empty());
    let addresses = state.resolve_cells(1);
    assert_eq!(addresses.cells(), &[prior_slot(3)]);
    assert_eq!(addresses.coverage, Coverage::Complete);
    assert_eq!(
        addresses.definite(),
        None,
        "a prior family is not a single occurrence"
    );
    assert_uninitialized(&state, 2);
    assert_uninitialized(&state, 3);
}

// Cell identity survives visibility narrowing even without value provenance.
#[test]
fn cell_only_alias_survives_join_and_scope_narrowing() {
    let local_count = 4;
    let empty = RootSet::empty(local_count);
    let mut left = BorrowState::new_uninitialized(local_count);
    left.initialize_parameter(0);
    left.initialize_parameter(3);
    let mut right = left.clone();
    let mut slot_branch = left.clone();
    bind_whole(&mut left, 1, 0, empty.clone(), empty.clone());
    bind_whole(&mut right, 1, 3, empty.clone(), empty);

    assert!(left.join_in_place(&right));
    assert_eq!(left.binding_mode(1), LocalMode::ALIAS);
    assert!(left.binding(1).value_roots().is_empty());
    assert_eq!(left.resolve_cells(1).cells(), &[slot(0), slot(3)]);

    // The branch-local referent loses its name, but the live alias retains its cell.
    left.kill_invisible(&roots(local_count, &[0, 1, 2]));
    assert_eq!(left.binding_mode(1), LocalMode::ALIAS);
    assert!(left.binding(1).value_roots().is_empty());
    assert_eq!(left.resolve_cells(1).cells(), &[slot(0), prior_slot(3)]);

    // A merge with a path that never rebound the binding keeps the alias alternative.
    slot_branch.initialize_parameter(1);
    slot_branch.kill_invisible(&roots(local_count, &[0, 1, 2]));
    let mut merged = left.clone();
    assert!(merged.join_in_place(&slot_branch));
    assert!(merged.binding_mode(1).contains(LocalMode::ALIAS));
    merged.kill_invisible(&roots(local_count, &[0, 1, 2]));
    assert!(merged.binding_mode(1).contains(LocalMode::ALIAS));
    assert!(merged.binding(1).value_roots().is_empty());
    assert_eq!(
        merged.resolve_cells(1).cells(),
        &[slot(0), slot(1), prior_slot(3)]
    );

    // Neither destination keeps a current name.
    let mut pruned = left.clone();
    pruned.kill_invisible(&roots(local_count, &[1, 2, 3]));
    assert_eq!(pruned.binding_mode(1), LocalMode::ALIAS);
    assert_eq!(
        pruned.resolve_cells(1).cells(),
        &[prior_slot(0), prior_slot(3)]
    );
    assert_eq!(pruned.resolve_cells(1).definite(), None);
}

#[test]
fn joining_states_narrowed_to_one_mask_stays_narrowed() {
    let local_count = 4;
    let visible = roots(local_count, &[0, 1, 2]);
    let empty = RootSet::empty(local_count);
    let mut left = BorrowState::new_uninitialized(local_count);
    left.initialize_parameter(0);
    left.initialize_parameter(3);
    let mut right = left.clone();
    bind_whole(&mut left, 1, 0, roots(local_count, &[0]), empty.clone());
    right.set_binding(
        1,
        BindingSpec::Slot,
        roots(local_count, &[0]),
        empty.clone(),
    );
    bind_whole(&mut right, 2, 3, roots(local_count, &[3]), empty);
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
    state.initialize_parameter(LAST);
    bind_whole(
        &mut state,
        1,
        LAST,
        roots(local_count, &[LAST]),
        RootSet::empty(local_count),
    );
    state.set_binding(
        2,
        BindingSpec::Slot,
        roots(local_count, &[1]),
        RootSet::empty(local_count),
    );

    let before = state.clone();
    state.kill_invisible(&RootSet::full(local_count));
    assert_eq!(state, before);

    // Bit 1 carries local 2's only root and local 1's direct alias, and local 1 becomes invisible.
    let mut narrowed = RootSet::full(local_count);
    narrowed.subtract_with(&roots(local_count, &[1]));
    state.kill_invisible(&narrowed);

    assert_uninitialized(&state, 1);
    assert_eq!(state.binding_mode(2), LocalMode::SLOT);
    assert!(state.binding(2).value_roots().is_empty());
    assert_eq!(state.resolve_cells(2).cells(), &[slot(2)]);
    assert_eq!(state.binding_mode(0), LocalMode::SLOT);
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
