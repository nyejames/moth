//! Source and state regressions for writes through the sparse allocation layer.

use super::conflicts::{OperandAccessIndex, Overlap, footprint, overlap};
use super::{AccessFootprint, PlaceUse};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, AllocId, BindingSpec, BorrowState, CapturedValue, CellId, Coverage, EntryFamilies,
    FieldKey, LocalState, ReceiverDependency, Replacement, ResolvedPlace, RootSet, StoredValue,
    TypedStore, UNKNOWN_ORDER_KEY, ValueAlt, ValueKind, WrapperAge, WrapperId,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::InvalidMutableAccessView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, InvalidMutableAccessReason};
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::validate_hir_module;
use crate::compiler_frontend::tests::borrow_fixture_support::run_borrow_checker_with_types;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tests::type_id_fixture_support::lower_ast;

fn check_source(source: &str) -> Result<(), BorrowCheckError> {
    let (ast, mut paths, mut strings) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut strings);
    let (module, types) =
        lower_ast(ast, &mut strings, &mut paths, &registry).expect("fixture source should lower");
    validate_hir_module(&module, &types, &registry).expect("fixture HIR should validate");
    run_borrow_checker_with_types(&module, &types, &registry, &paths, &strings).map(|_| ())
}

fn parameter_allocations<const N: usize>() -> (BorrowState, [AllocId; N]) {
    let mut state = BorrowState::new_uninitialized(N);
    let ids = std::array::from_fn(|local| {
        state.initialize_parameter(local);
        state.local_alloc_ids(local).0[0]
    });
    (state, ids)
}

fn graph_access(id: AllocId, views: Vec<AllocId>, usage: PlaceUse) -> AccessFootprint {
    let mut access = AccessFootprint::cells(AddressSet::from_cells([], Coverage::Complete), usage);
    access.content = Some(ResolvedPlace::from_values(
        &[StoredValue { id, writable: true }],
        views,
    ));
    access
}

fn projected_replacement(holder: AllocId, field: FieldKey) -> AccessFootprint {
    AccessFootprint::cells(
        AddressSet::from_cells([CellId::Projected(holder, field)], Coverage::Complete),
        PlaceUse::Replace,
    )
}

#[test]
fn indexed_call_operands_match_full_scan_with_sharing_and_reach() {
    let (mut state, [root, first, second, third, member, detached, disjoint]) =
        parameter_allocations::<7>();
    let field = FieldKey::Field(0);
    let lazy = state.alloc_child(root, field);
    let grandchild = state.alloc_child(detached, field);
    state.add_alloc_edge(
        lazy,
        field,
        StoredValue {
            id: detached,
            writable: true,
        },
    );
    state.add_view_member(first, member);
    state.add_view_member(second, member);
    state.relate_views(second, third, true);
    state.relate_views(first, third, false);

    let mut partial = AccessFootprint::cells(
        AddressSet::from_cells([], Coverage::Partial),
        PlaceUse::Replace,
    );
    let mut content = ResolvedPlace::from_values(&[], Vec::new());
    content.coverage = Coverage::Partial;
    partial.content = Some(content);
    let accesses = [
        graph_access(root, vec![], PlaceUse::Read),
        graph_access(first, vec![first], PlaceUse::Read),
        graph_access(second, vec![second], PlaceUse::Read),
        graph_access(third, vec![third], PlaceUse::Read),
        graph_access(lazy, vec![], PlaceUse::Read),
        graph_access(grandchild, vec![], PlaceUse::Read),
        projected_replacement(grandchild, field),
        projected_replacement(disjoint, field),
        projected_replacement(first, field),
        projected_replacement(second, field),
        partial,
    ];
    let mut probes = OperandAccessIndex::new(accesses.len(), accesses.iter().enumerate());
    for (position, access) in accesses.iter().enumerate() {
        let probe = AccessFootprint {
            address: access.address.clone(),
            traversal: access.traversal.clone(),
            traversal_coverage: access.traversal_coverage,
            content: access.content.clone(),
            usage: access.usage.for_transfer_probe(),
        };
        let expected: Vec<_> = (position + 1..accesses.len())
            .filter(|&other| overlap(&state, &probe, &accesses[other]) != Overlap::No)
            .collect();
        let mut actual = Vec::new();
        probes.any_later(&state, position, &probe, |other| {
            if overlap(&state, &probe, &accesses[other]) != Overlap::No {
                actual.push(other);
            }
            false
        });
        actual.sort_unstable();
        assert_eq!(actual, expected, "argument {position}");
    }
    let mut indexed = OperandAccessIndex::new(accesses.len(), accesses.iter().enumerate());
    let mut examined = Vec::new();
    indexed.any_later(&state, 0, &accesses[0], |other| {
        examined.push(other);
        false
    });
    assert!(examined.contains(&4) && examined.contains(&5) && examined.contains(&6));
    assert!(
        !examined.contains(&7),
        "an unrelated holder with the same selector"
    );
}

#[test]
fn optional_operand_index_does_not_expand_unrelated_or_repeated_roots() {
    for width in [16, 64] {
        let (mut state, [broad, unrelated, separate]) = parameter_allocations::<3>();
        let field = FieldKey::Field(0);
        for slot in 0..width {
            let key = FieldKey::Field(slot as u32);
            for root in [broad, unrelated] {
                let id = state.alloc_child(root, key);
                state.add_alloc_edge(root, key, StoredValue { id, writable: true });
            }
        }
        let read = graph_access(broad, vec![], PlaceUse::Read);
        let write = graph_access(broad, vec![], PlaceUse::Mutate { deep: true });
        let other = graph_access(separate, vec![], PlaceUse::Mutate { deep: true });
        let mut index =
            OperandAccessIndex::new(width + 1, (1..=width).map(|position| (position, &read)));
        let mut exact_checks = 0;
        assert!(index.any_later(&state, 0, &write, |other| {
            exact_checks += 1;
            other == 1 && overlap(&state, &write, &read) != Overlap::No
        }));
        assert_eq!(exact_checks, 1);
        assert_eq!(
            index.reach_work(),
            (1, false),
            "direct identity must avoid reach expansion"
        );

        let mut index =
            OperandAccessIndex::new(width + 1, (1..=width).map(|position| (position, &read)));
        assert!(!index.any_later(&state, 0, &other, |_| panic!(
            "unrelated deep root was posted"
        )));
        assert_eq!(
            index.reach_work(),
            (1, true),
            "only captured operand roots are posted"
        );

        let first = projected_replacement(unrelated, field);
        let second = projected_replacement(unrelated, FieldKey::Field(1));
        let mut index = OperandAccessIndex::new(2, [(1, &second)].into_iter());
        assert!(!index.any_later(&state, 0, &first, |_| panic!(
            "different selectors were posted"
        )));
        assert_eq!(
            index.reach_work(),
            (1, false),
            "projected scalar cells do not expand allocation reach"
        );
    }
}

#[test]
fn allocation_incidence_tracks_store_rekey_join_and_narrow() {
    let (mut state, [root, first, second]) = parameter_allocations::<3>();
    let field = FieldKey::Field(0);
    let holder = state.alloc_child(root, field);
    let address = || {
        AddressSet::from_cells([CellId::Projected(holder, field)], Coverage::Complete)
            .with_singleton_proof()
    };
    state.publish_store(
        &TypedStore::replace_cells(address(), writable_value(first)),
        0,
        None,
    );
    state.relate_views(root, second, true);
    state.assert_allocation_incidence();
    let mut other = state.clone();
    other.publish_store(
        &TypedStore::replace_cells(address(), writable_value(second)),
        1,
        None,
    );
    other.assert_allocation_incidence();
    assert!(state.join_in_place(&other));
    state.assert_allocation_incidence();
    let _ = state.retire_slot_value(0, 2);
    state.assert_allocation_incidence();
    state.kill_invisible(&roots(3, &[1]));
    state.assert_allocation_incidence();
}

#[test]
fn final_family_members_preserve_occurrence_and_empty_group_evidence() {
    use crate::compiler_frontend::analysis::borrow_checker::state::FamilyMember;

    let (mut state, [holder, old, fresh]) = parameter_allocations::<3>();
    let member = FamilyMember {
        value: writable_value(fresh),
        at_most_once: true,
    };
    let field = FieldKey::Elem;
    state.add_alloc_edge(
        holder,
        field,
        StoredValue {
            id: old,
            writable: true,
        },
    );
    let mut weak = state.clone();
    weak.publish_family_members(
        &[holder],
        Coverage::Partial,
        field,
        Replacement::Must,
        std::slice::from_ref(&member),
    );
    assert!(weak.alloc_reaches_typed(&[holder], old, true, false));
    assert!(weak.alloc_reaches_typed(&[holder], fresh, true, false));
    weak.assert_allocation_incidence();

    for _ in 0..2 {
        state.publish_family_members(
            &[holder],
            Coverage::Complete,
            field,
            Replacement::Must,
            std::slice::from_ref(&member),
        );
        assert!(state.retained_edge_groups(holder).next().unwrap()[0].single);
    }
    assert!(!state.alloc_reaches_typed(&[holder], old, true, false));
    let mut duplicate = state.clone();
    duplicate.publish_family_members(
        &[holder],
        Coverage::Complete,
        field,
        Replacement::Must,
        &[member.clone(), member.clone()],
    );
    assert!(!duplicate.retained_edge_groups(holder).next().unwrap()[0].single);
    duplicate.remove_alloc_element(&[holder], false, None);
    assert!(duplicate.alloc_reaches_typed(&[holder], fresh, true, false));

    state.remove_alloc_element(&[holder], false, None);
    assert!(!state.alloc_reaches_typed(&[holder], fresh, true, false));
    let lazy = state.alloc_child(holder, field);
    state.publish_family_members(
        &[holder],
        Coverage::Complete,
        field,
        Replacement::May,
        std::slice::from_ref(&member),
    );
    assert!(!state.alloc_reaches_typed(&[holder], lazy, true, false));
    assert!(state.alloc_reaches_typed(&[holder], fresh, true, false));
    assert!(state.retained_edge_groups(holder).next().unwrap()[0].single);
    let mut overlapping = state.clone();
    overlapping.publish_family_members(
        &[holder],
        Coverage::Complete,
        field,
        Replacement::May,
        &[member],
    );
    assert!(!overlapping.retained_edge_groups(holder).next().unwrap()[0].single);
    overlapping.remove_alloc_element(&[holder], false, None);
    assert!(overlapping.alloc_reaches_typed(&[holder], fresh, true, false));
    state.remove_alloc_element(&[holder], false, None);
    assert!(!state.alloc_reaches_typed(&[holder], fresh, true, false));
    overlapping.assert_allocation_incidence();
    state.assert_allocation_incidence();
    duplicate.assert_allocation_incidence();
}

#[test]
fn family_batch_keeps_viewed_and_plain_members_in_the_same_projection() {
    use crate::compiler_frontend::analysis::borrow_checker::state::FamilyMember;

    let (mut state, [holder, viewed, plain]) = parameter_allocations::<3>();
    let view = state.call_view_id(1, false, 0);
    state.add_view_member(view, viewed);
    let mut occurrence = writable_value(viewed);
    occurrence.views.push(view);
    let members = [
        FamilyMember {
            value: occurrence,
            at_most_once: true,
        },
        FamilyMember {
            value: writable_value(plain),
            at_most_once: true,
        },
    ];
    state.publish_family_members(
        &[holder],
        Coverage::Complete,
        FieldKey::Elem,
        Replacement::Must,
        &members,
    );
    let mut projected = state.resolve_local_allocs(0);
    state.step_resolved(&mut projected, FieldKey::Elem, &[true; 3]);
    assert!(projected.views.contains(&view));
    assert!(projected.views.contains(&plain));
    assert!(state.alloc_paths_may_share(&projected.ids, &projected.views, &[plain], &[]));
    state.assert_allocation_incidence();
}

#[test]
fn family_batch_retains_absence_on_a_candidate_that_may_be_skipped() {
    use crate::compiler_frontend::analysis::borrow_checker::state::FamilyMember;

    let (mut empty, [holder, member]) = parameter_allocations::<2>();
    for family in [FieldKey::Elem, FieldKey::MapKey] {
        empty.publish_family_members(
            &[holder],
            Coverage::Complete,
            family,
            Replacement::Must,
            &[],
        );
        for coverage in [Coverage::Complete, Coverage::Partial] {
            let mut state = empty.clone();
            state.publish_family_members(
                &[holder],
                coverage,
                family,
                Replacement::May,
                &[FamilyMember {
                    value: writable_value(member),
                    at_most_once: true,
                }],
            );
            let mut projected = state.resolve_local_allocs(0);
            state.step_resolved(&mut projected, family, &[true; 2]);
            assert!(projected.ids.contains(&member));
            assert_eq!(projected.may_be_absent, coverage == Coverage::Partial);
            state.publish_family_members(
                &[holder],
                Coverage::Partial,
                family,
                Replacement::Must,
                &[],
            );
            let mut projected = state.resolve_local_allocs(0);
            state.step_resolved(&mut projected, family, &[true; 2]);
            assert!(projected.ids.contains(&member));
            assert!(projected.may_be_absent);
            state.assert_allocation_incidence();
        }
    }
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn projected_replacement_inspects_only_the_addressed_group() {
    use crate::compiler_frontend::instrumentation::{
        FrontendCounter, capture_counters_for_test, frontend_counter_test_values, lock_counter_test,
    };

    let _guard = lock_counter_test();
    let _capture = capture_counters_for_test();

    let (mut state, [holder, old, fresh, unrelated]) = parameter_allocations::<4>();
    let replace = |state: &mut BorrowState, holder, selector, value| {
        let address = AddressSet::from_cells(
            [CellId::Projected(holder, FieldKey::Field(selector))],
            Coverage::Complete,
        )
        .with_singleton_proof();
        state.publish_store(
            &TypedStore::replace_cells(address, writable_value(value)),
            0,
            None,
        );
    };
    for selector in 1..=64 {
        replace(&mut state, holder, selector, old);
        replace(&mut state, unrelated, selector, old);
    }
    let counters = [
        FrontendCounter::BorrowPublisherGroupRowsInspected,
        FrontendCounter::BorrowPublisherGroupRowsMoved,
    ];
    let before = frontend_counter_test_values(&counters);
    for selector in (1..=64).rev().chain(std::iter::once(0)) {
        replace(&mut state, holder, selector, fresh);
    }
    let after = frontend_counter_test_values(&counters);
    assert_eq!(after[0] - before[0], 64);
    assert_eq!(after[1] - before[1], 65);
    assert!(state.alloc_reaches_typed(&[unrelated], old, true, false));
    state.assert_allocation_incidence();
}

#[test]
fn allocation_incidence_join_tracks_groups_missing_from_either_predecessor() {
    let (entry, [root, target, view]) = parameter_allocations::<3>();
    let field = FieldKey::Field(0);
    let empty = FieldKey::Field(1);
    let holder = entry.alloc_child(root, field);
    let lazy = entry.alloc_child(holder, empty);
    let mut left = entry.clone();
    left.store_alloc_field_with_views(&[holder], empty, &[], &[], true, true);
    let mut right = entry;
    right.add_alloc_edge(
        holder,
        field,
        StoredValue {
            id: target,
            writable: true,
        },
    );
    right.relate_views(root, view, true);
    assert!(!left.alloc_reaches_typed(&[root], lazy, true, false));
    assert!(!left.alloc_reaches_typed(&[root], target, true, false));
    let mut merged = left.clone();
    assert!(merged.join_in_place(&right));
    assert!(right.join_in_place(&left));
    for state in [&merged, &right] {
        state.assert_allocation_incidence();
        assert!(state.alloc_reaches_typed(&[root], lazy, true, false));
        assert!(state.alloc_reaches_typed(&[root], target, true, false));
        assert!(state.alloc_paths_may_share(&[root], &[], &[view], &[]));
    }
    assert_eq!(merged, right);
}

#[test]
fn indexed_reach_matches_full_scan_on_small_states_and_joins() {
    let mut seed = 0x71f3_a5c9u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as usize
    };
    for _ in 0..24 {
        let (entry, parameters) = parameter_allocations::<4>();
        let mut ids = Vec::with_capacity(12);
        ids.extend(parameters);
        for index in 0..8 {
            ids.push(entry.alloc_child(ids[index], FieldKey::Field((index % 2) as u32)));
        }
        let mut arm = |mut state: BorrowState| {
            for _ in 0..12 {
                let holder = ids[next() % ids.len()];
                let target = ids[next() % ids.len()];
                let key = FieldKey::Field((next() % 3) as u32);
                let value = StoredValue {
                    id: target,
                    writable: next() & 1 != 0,
                };
                let values = [value];
                let values = if next() % 5 == 0 {
                    &[][..]
                } else {
                    &values[..]
                };
                state.store_alloc_field_with_views(&[holder], key, values, &[], true, true);
            }
            state
        };
        let mut left = arm(entry.clone());
        let mut right = arm(entry);
        // Pair widening can also introduce a new reverse posting on join.
        left.relate_views(ids[0], ids[1], false);
        right.relate_views(ids[0], ids[1], true);
        let mut joined_left = left.clone();
        let mut joined_right = right.clone();
        assert!(joined_left.join_in_place(&right));
        joined_right.join_in_place(&left);
        assert_eq!(joined_left, joined_right);
        for state in [&left, &right, &joined_left, &joined_right] {
            state.assert_allocation_incidence();
            for &source in &ids {
                for &target in &ids {
                    for writable in [false, true] {
                        assert_eq!(
                            state.alloc_reaches_typed(&[source], target, true, writable),
                            state.full_scan_typed_reach_for_test(&[source], target, writable),
                            "source {source:?}, target {target:?}, writable {writable}",
                        );
                    }
                }
            }
        }
    }
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn operand_reach_counter_growth_is_linear_for_shared_tails_and_nested_holders() {
    use crate::compiler_frontend::instrumentation::{
        FrontendCounter, capture_counters_for_test, frontend_counter_test_values, lock_counter_test,
    };

    let _guard = lock_counter_test();
    let _capture = capture_counters_for_test();
    let row_count = |counter| frontend_counter_test_values(&[counter])[0];
    let query = |width| {
        let mut state = BorrowState::new_uninitialized(2 * width + 1);
        let mut ids = Vec::new();
        for local in 0..2 * width + 1 {
            state.initialize_parameter(local);
            ids.push(state.local_alloc_ids(local).0[0]);
        }
        let field = FieldKey::Field(0);
        for &root in &ids[..width] {
            state.add_alloc_edge(
                root,
                field,
                StoredValue {
                    id: ids[width],
                    writable: true,
                },
            );
        }
        for pair in ids[width..].windows(2) {
            state.add_alloc_edge(
                pair[0],
                field,
                StoredValue {
                    id: pair[1],
                    writable: true,
                },
            );
        }
        let later: Vec<_> = ids[..width]
            .iter()
            .map(|&root| graph_access(root, vec![], PlaceUse::Read))
            .collect();
        let probe = graph_access(ids[2 * width], vec![], PlaceUse::Mutate { deep: false });
        let index = || {
            OperandAccessIndex::new(
                width + 1,
                later.iter().enumerate().map(|(i, access)| (i + 1, access)),
            )
        };
        let counter = FrontendCounter::BorrowOperandReachRows;
        let before = row_count(counter);
        let mut exact = 0;
        let overlap_before = row_count(FrontendCounter::BorrowOverlapCalls);
        assert!(index().any_later(&state, 0, &probe, |other| {
            exact += 1;
            overlap(&state, &probe, &later[other - 1]) == Overlap::Yes
        }));
        assert_eq!(exact, 1);
        assert_eq!(
            row_count(FrontendCounter::BorrowOverlapCalls) - overlap_before,
            1
        );
        let short_circuit_rows = row_count(counter) - before;
        exact = 0;
        let before = row_count(counter);
        assert!(!index().any_later(&state, 0, &probe, |_| {
            exact += 1;
            false
        }));
        assert_eq!(exact, width);
        [short_circuit_rows, row_count(counter) - before]
    };
    let maintain = |width| {
        let (mut state, [root, target]) = parameter_allocations::<2>();
        let mut holders = Vec::new();
        let mut parent = root;
        for _ in 0..width {
            parent = state.alloc_child(parent, FieldKey::Field(0));
            holders.push(parent);
        }
        let counter = FrontendCounter::BorrowReachIndexMaintenanceRows;
        let before = row_count(counter);
        for holder in holders.into_iter().rev() {
            state.add_alloc_edge(
                holder,
                FieldKey::Field(1),
                StoredValue {
                    id: target,
                    writable: true,
                },
            );
        }
        state.assert_allocation_incidence();
        row_count(counter) - before
    };
    let [tail_small, tail_large] = [16, 64].map(query);
    let [nested_small, nested_large] = [16, 64].map(maintain);
    for (small, large) in tail_small.into_iter().zip(tail_large) {
        assert!(small >= 16 && large >= 64 && large <= 5 * small);
    }
    assert!(nested_small >= 16 && nested_large >= 64 && nested_large <= 5 * nested_small);
    assert!(nested_large <= 4 * 64);
}

fn check_rejection(source: &str, access: &str, reason: InvalidMutableAccessReason) {
    check_rejection_at(check_source(source), source, access, reason);
}

fn check_rejection_at(
    decision: Result<(), BorrowCheckError>,
    source: &str,
    access: &str,
    reason: InvalidMutableAccessReason,
) {
    let draft = decision
        .expect_err("the final mutation must be rejected")
        .into_draft_or_infrastructure()
        .expect("typed borrow diagnostic");
    let bag = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = bag.first().expect("borrow diagnostic");
    let view = diagnostic
        .family_view::<InvalidMutableAccessView>()
        .expect("invalid mutable access");
    assert_eq!(view.reason, reason);
    // Projected receivers may point inside the receiver rather than at its first token.
    let access_start = source.find(access).expect("authored access");
    let line_start = source[..access_start].rfind('\n').expect("mutation line") + 1;
    let line_end = access_start + source[access_start..].find('\n').expect("next line");
    assert!(matches!(
        diagnostic.primary_site(),
        SiteRef::Local(span)
            if span.local().inline_range().is_some_and(|range| {
                (line_start..line_end).contains(&(range.start() as usize))
            })
    ));
}

#[test]
fn readonly_scalar_projection_can_supply_a_value_but_not_a_writable_link() {
    let acquire = "\
Pair = |left I32, right I32|\n\
probe || -> I32:\n\
    pair = Pair(1, 2)\n\
    code ~= 0\n\
    ~code = pair.left\n\
    return code\n\
;\n";
    check_rejection(
        acquire,
        "~code = pair.left",
        InvalidMutableAccessReason::ImmutablePlace,
    );

    let write_through = "\
Pair = |left I32, right I32|\n\
probe || -> I32:\n\
    pair = Pair(1, 2)\n\
    slot ~= 0\n\
    writer ~= slot\n\
    ~writer = pair.left\n\
    return writer\n\
;\n";
    check_source(write_through).expect("a write through an existing alias only reads its RHS");
}

#[test]
fn scalar_map_lookup_stability_ends_at_last_use() {
    let lookup = "\
scores ~{String = I32} = {\"Priya\" = 10}
score = scores.get(\"Priya\") catch:
    then 0
;
";
    let edit = "~scores.set(\"Rob\", 7) catch:\n;\n";
    let observation = "~[:[score]]\n";
    check_source(&format!("{lookup}{observation}{edit}"))
        .expect("the last lookup use precedes the map edit");
    check_rejection(
        &format!("{lookup}{edit}{observation}"),
        "scores.set",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn nested_lookup_stability_ends_at_last_use() {
    let lookup = "\
Bucket = |rows {{I32}}|
bucket ~= Bucket({{1, 2}})
row = bucket.rows.get(0)!
item = row.get(0)!
";
    let edit = "~bucket.rows.push({3})\n";
    let observation = "observed = item + 0\n";
    let result = "~[:[observed]]\n";
    check_source(&format!("{lookup}{observation}{edit}{result}"))
        .expect("a computed result doesn't extend the nested lookup loan");
    check_rejection(
        &format!("{lookup}{edit}{observation}{result}"),
        "bucket.rows.push",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn retained_rhs_before_whole_binding_replacement() {
    for (alias, replacement) in [
        ("", "~holder = Holder(holder.items)"),
        ("writer ~= holder", "~writer = Holder(writer.items)"),
    ] {
        let source = format!(
            "Holder = |items {{I32}}|\n\
             probe || -> I32:\n\
                 shared ~= {{1}}\n\
                 holder ~= Holder(shared)\n\
                 {alias}\n\
                 {replacement}\n\
                 ~shared.push(2)\n\
                 return holder.items.length()\n\
             ;"
        );
        check_rejection(
            &source,
            "shared.push",
            InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
        );
    }
}

#[test]
fn retained_rhs_rebases_a_lazy_child_of_the_retired_slot() {
    let prefix = "\
Holder = |items {I32}|\n\
probe || -> I32:\n\
    holder ~= Holder({1})\n\
    observer = Holder(holder.items)\n";
    let suffix = "\
    ~holder.items.push(2)\n\
    return observer.items.length()\n\
;";
    check_rejection(
        &format!("{prefix}~holder = Holder(holder.items)\n{suffix}"),
        "holder.items.push",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
    check_source(&format!("{prefix}~holder = Holder({{3}})\n{suffix}"))
        .expect("a replacement with a new child does not share the retired child");
}

#[test]
fn retained_rhs_before_projected_cell_replacement() {
    let source = "\
Holder = |items {I32}|\n\
Outer = |holder Holder|\n\
probe || -> I32:\n\
    shared ~= {1}\n\
    outer ~= Outer(Holder(shared))\n\
    writer ~= outer.holder\n\
    ~writer = Holder(writer.items)\n\
    ~shared.push(2)\n\
    return outer.holder.items.length()\n\
;";
    check_rejection(
        source,
        "shared.push",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn retained_rhs_does_not_launder_an_immutable_allocation() {
    let prefix = "\
Holder = |items {I32}|\n\
identity |value {I32}| -> {I32}:\n\
    return value\n\
;\n\
probe |input {I32}| -> I32:\n\
    slot ~= identity(input)\n\
    holder ~= Holder(slot)\n";
    let suffix = "\
    ~holder.items.push(4)\n\
    return holder.items.length()\n\
;";
    check_rejection(
        &format!("{prefix}{suffix}"),
        "holder.items.push",
        InvalidMutableAccessReason::ImmutablePlace,
    );
    check_source(&format!("{prefix}~holder.items = {{3}}\n{suffix}"))
        .expect("replacing the writable wrapper's field does not mutate its old referent");
}

#[test]
fn retained_rhs_keeps_a_retired_writable_allocation_writable() {
    let source = "\
Holder = |items {I32}|\n\
probe || -> I32:\n\
    shared ~= {1}\n\
    holder ~= Holder(shared)\n\
    ~shared = {2}\n\
    ~holder.items.push(3)\n\
    return holder.items.length()\n\
;";
    check_source(source).expect("retiring a writable allocation does not revoke permission");
}

#[test]
fn retained_projection_write_conflicts_with_a_live_reborrow() {
    let source = r"Pair = |first I32, second I32|
holder ~= Pair(1, 0)
writer ~= holder.first
~holder = Pair(2, 3)
view = writer
~writer = 4
~[:[view]]
";
    check_rejection(
        source,
        "~writer = 4",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn retained_projection_reborrow_can_write_through_its_live_parent() {
    let source = r"Pair = |first I32, second I32|
probe || -> I32:
    holder ~= Pair(1, 0)
    writer ~= holder.first
    ~holder = Pair(2, 3)
    child ~= writer
    ~child = 4
    result = child + 1
    return writer + result
;
";
    check_source(source).expect("a writable reborrow can exercise its parent loan");
}

#[test]
fn retained_slot_write_conflicts_with_a_live_reborrow_after_join() {
    let source = r"probe |flag Bool| -> I32:
    writer ~= 0
    if flag:
        left ~= 1
        ~writer = left
    else
        right ~= 2
        ~writer = right
    ;
    view = writer
    ~writer = 9
    return view
;
~[:[probe(true)]]
";
    check_rejection(
        source,
        "~writer = 9",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn partial_slot_write_does_not_pollute_an_unrelated_field() {
    let source = r"Box = |items {Bool}|
identity |value {I32}| -> {I32}:
    return value
;
probe |flag Bool, input {I32}| -> I32:
    raw ~= {true}
    unrelated ~= Box(raw)
    a ~= {1}
    b ~= {2}
    if flag:
        ~a = b
    else
        ~b = a
    ;
    ~a = identity(input)
    ~unrelated.items.push(false)
    return unrelated.items.length()
;
~[:[probe(true, {3})]]
";
    check_source(source).expect("a partial slot write cannot reach the unrelated field");
}

fn roots(count: usize, members: &[usize]) -> RootSet {
    let mut roots = RootSet::empty(count);
    for &member in members {
        roots.insert(member);
    }
    roots
}

fn slot(local: usize) -> CellId {
    CellId::Slot(WrapperId::current(local))
}

fn set_link(state: &mut BorrowState, count: usize, local: usize, source: usize, writable: bool) {
    state.set_binding(
        local,
        BindingSpec::Link {
            source: WrapperId::current(source),
            writable,
        },
        RootSet::empty(count),
        RootSet::empty(count),
    );
}

fn writable_value(id: AllocId) -> CapturedValue {
    CapturedValue::new(vec![StoredValue { id, writable: true }], Vec::new())
}

#[test]
fn place_resolution_distinguishes_complete_many_from_partial_singleton() {
    const COUNT: usize = 4;
    let mut state = BorrowState::new_uninitialized(COUNT);
    for local in 0..COUNT {
        state.initialize_parameter(local);
    }
    let first = state.local_alloc_ids(1).0[0];
    let second = state.local_alloc_ids(2).0[0];
    let child = state.local_alloc_ids(3).0[0];
    let left = FieldKey::Field(0);
    let right = FieldKey::Field(1);
    state.bind_alloc_ids(0, &[first, second], false);

    let resolve = |state: &BorrowState, key| {
        let mut place = state.resolve_place_path(0, &[true; COUNT], [key]);
        state.load_place_content(&mut place, &[true; COUNT], false);
        place
    };
    let read = resolve(&state, left);
    let write = resolve(&state, right);
    assert_eq!(read.address.coverage, Coverage::Complete);
    assert_eq!(read.traversal_coverage, Coverage::Complete);
    assert_eq!(read.address.cells().len(), 2);
    assert_eq!(read.address.definite(), None);
    assert_eq!(
        overlap(
            &state,
            &footprint(&read, PlaceUse::Read),
            &footprint(&write, PlaceUse::WriteEntries)
        ),
        Overlap::No,
    );

    for key in [left, right] {
        state.add_alloc_edge(
            first,
            key,
            StoredValue {
                id: child,
                writable: true,
            },
        );
    }
    let read = resolve(&state, left);
    let write = resolve(&state, right);
    assert_eq!(
        overlap(
            &state,
            &footprint(&read, PlaceUse::Read),
            &footprint(&write, PlaceUse::WriteEntries)
        ),
        Overlap::Yes,
    );

    state.set_binding(
        0,
        BindingSpec::Alias {
            addresses: AddressSet::from_cells([slot(1)], Coverage::Partial),
            route_roots: RootSet::empty(COUNT),
        },
        RootSet::empty(COUNT),
        RootSet::empty(COUNT),
    );
    let partial = resolve(&state, left);
    assert_eq!(partial.address.cells(), [CellId::Projected(first, left)]);
    let sibling = state.resolve_place_path(1, &[true; COUNT], [right]);
    assert!(
        !sibling
            .address
            .cells()
            .contains(&partial.address.cells()[0])
    );
    assert_eq!(partial.address.coverage, Coverage::Partial);
    assert_eq!(partial.traversal_coverage, Coverage::Partial);
    assert_eq!(partial.address.definite(), None);
    assert_eq!(
        overlap(
            &state,
            &footprint(&partial, PlaceUse::Read),
            &footprint(&sibling, PlaceUse::Replace)
        ),
        Overlap::Unknown,
    );
    assert_eq!(partial.content.unwrap().coverage, Coverage::Partial);
    set_link(&mut state, COUNT, 0, 1, true);
    let through = state.resolve_place_path(0, &[true; COUNT], []);
    assert_eq!(through.address.definite(), Some(slot(1)));
    assert!(through.names_terminal_slot(WrapperId::current(1)));
    assert!(!through.names_terminal_slot(WrapperId::current(2)));
    let projected = state.resolve_place_path(0, &[true; COUNT], [left]);
    assert_eq!(projected.projection_owner(), Some(WrapperId::current(1)));
}

#[test]
fn known_address_does_not_depend_on_terminal_payload() {
    let mut state = BorrowState::new_uninitialized(2);
    state.initialize_parameter(0);
    state.initialize_parameter(1);
    let holder = state.local_alloc_ids(0).0[0];
    let readonly = state.local_alloc_ids(1).0[0];
    let field = FieldKey::Field(0);
    let cell = CellId::Projected(holder, field);
    state.add_alloc_edge(
        holder,
        field,
        StoredValue {
            id: readonly,
            writable: false,
        },
    );

    let mut place = state.resolve_place_path(0, &[true, false], [field]);
    assert_eq!(place.address.definite(), Some(cell));
    assert_eq!(place.traversal.as_slice(), [slot(0)]);
    assert!(place.content.is_none());
    state.load_place_content(&mut place, &[true, false], false);
    assert_eq!(place.address.definite(), Some(cell));
    assert!(!place.permission.holder_readonly);
    let content = place.content.unwrap();
    assert!(content.readonly);
    assert_eq!(content.coverage, Coverage::Complete);
    assert_eq!(content.ids, [readonly]);

    let mut scalar = state.resolve_place_path(0, &[true, false], [field]);
    state.load_place_content(&mut scalar, &[true, false], true);
    assert_eq!(scalar.address.definite(), Some(cell));
    let content = scalar.content.unwrap();
    assert!(content.ids.is_empty());
    assert!(!content.may_be_absent);
    assert_eq!(content.coverage, Coverage::Complete);

    state.publish_store(
        &TypedStore::replace_cells(place.address, CapturedValue::default()),
        UNKNOWN_ORDER_KEY,
        None,
    );
    let mut absent = state.resolve_place_path(0, &[true, false], [field]);
    state.load_place_content(&mut absent, &[true, false], false);
    assert_eq!(absent.address.definite(), Some(cell));
    let content = absent.content.unwrap();
    assert!(content.ids.is_empty());
    assert!(content.may_be_absent);
    assert_eq!(content.coverage, Coverage::Complete);
    assert_eq!(content.holder_coverage(), Coverage::Partial);
}

#[test]
fn nested_field_replacement_observes_prefixes_but_not_old_payload() {
    let mut state = BorrowState::new_uninitialized(3);
    for local in 0..3 {
        state.initialize_parameter(local);
    }
    let root = state.local_alloc_ids(0).0[0];
    let field = FieldKey::Field(0);
    let key = FieldKey::Field(1);
    let child = state.alloc_child(root, field);
    let old = state.local_alloc_ids(1).0[0];
    let new = state.local_alloc_ids(2).0[0];
    state.add_alloc_edge(
        child,
        key,
        StoredValue {
            id: old,
            writable: false,
        },
    );

    let mutability = [true, false, true];
    let destination = state.resolve_place_path(0, &mutability, [field, key]);
    assert_eq!(
        destination.traversal.as_slice(),
        [slot(0), CellId::Projected(root, field)]
    );
    assert_eq!(
        destination.address.definite(),
        Some(CellId::Projected(child, key))
    );
    assert!(destination.content.is_none());
    assert!(!destination.permission.holder_readonly);
    state.publish_store(
        &TypedStore::replace_cells(destination.address, writable_value(new)),
        UNKNOWN_ORDER_KEY,
        None,
    );
    let captured = state.resolve_alloc_path(0, &mutability, [field, key]);
    assert_eq!(captured.ids, [new]);
    assert!(!captured.readonly);
    assert_eq!(state.local_alloc_ids(1).0, [old]);
}

#[test]
fn call_result_preserves_sharing_at_a_possible_whole_referent() {
    const WHOLE: usize = 0;
    const HOLDER: usize = 1;
    const RESULT: usize = 2;
    const OBSERVER: usize = 3;
    const WRITER: usize = 4;
    const LOCAL_COUNT: usize = 5;
    let mut state = BorrowState::new_uninitialized(LOCAL_COUNT);
    for slot in [WHOLE, HOLDER, RESULT, OBSERVER] {
        state.initialize_parameter(slot);
    }
    let old_whole = state.local_alloc_ids(WHOLE).0[0];
    state.bind_alloc_ids(OBSERVER, &[old_whole], false);
    let holder = state.local_alloc_ids(HOLDER).0[0];
    let field = FieldKey::Field(0);
    let old_field = state.alloc_child(holder, field);
    let result_parent = state.local_alloc_ids(RESULT).0[0];
    let result = state.alloc_child(result_parent, field);

    let mut projected_path = state.clone();
    set_link(&mut state, LOCAL_COUNT, WRITER, WHOLE, true);
    projected_path.set_binding(
        WRITER,
        BindingSpec::Projected {
            addresses: AddressSet::from_cells(
                [CellId::Projected(holder, field)],
                Coverage::Complete,
            ),
            origin: WrapperId::current(HOLDER),
            owner: Some(WrapperId::current(HOLDER)),
            writable: true,
            isolated: false,
            route_roots: roots(LOCAL_COUNT, &[HOLDER]),
        },
        RootSet::empty(LOCAL_COUNT),
        RootSet::empty(LOCAL_COUNT),
    );
    assert!(state.join_in_place(&projected_path));

    // The implicit whole-slot value must survive a possible receiving store.
    assert!(!state.has_layer_binding(WHOLE));
    let addresses = state.resolve_cells(WRITER);
    assert_eq!(
        addresses.cells(),
        &[slot(WHOLE), CellId::Projected(holder, field)]
    );
    assert_eq!(addresses.coverage, Coverage::Complete);
    assert_eq!(addresses.definite(), None);
    let store = TypedStore::receive_result(addresses, writable_value(result), WRITER, false)
        .with_slot_roots(roots(LOCAL_COUNT, &[RESULT]));
    state.publish_store(&store, UNKNOWN_ORDER_KEY, None);

    assert_eq!(state.local_alloc_ids(WRITER).0, [result]);
    let (whole_ids, _) = state.local_alloc_ids(WHOLE);
    assert_eq!(whole_ids.len(), 2);
    assert!(!whole_ids.contains(&result_parent));
    for (slot, path, old) in [
        (WHOLE, &[][..], old_whole),
        (HOLDER, &[field][..], old_field),
    ] {
        let contents = state.resolve_alloc_path(slot, &[true; LOCAL_COUNT], path.iter().copied());
        assert!(contents.ids.contains(&old));
        assert!(contents.ids.contains(&result));
    }
    let mut observer = state.resolve_place_path(OBSERVER, &[true; LOCAL_COUNT], []);
    let mut writer = state.resolve_place_path(WHOLE, &[true; LOCAL_COUNT], []);
    state.load_place_content(&mut observer, &[true; LOCAL_COUNT], false);
    state.load_place_content(&mut writer, &[true; LOCAL_COUNT], false);
    assert!(
        overlap(
            &state,
            &footprint(&observer, PlaceUse::Read),
            &footprint(&writer, PlaceUse::Mutate { deep: false }),
        ) == Overlap::Yes,
        "a later mutation of the possible whole referent must still see the old live observer"
    );
}

#[test]
fn write_through_joined_alias_replaces_its_readonly_observation() {
    assert_joined_projected_write([(0, FieldKey::Field(0)), (1, FieldKey::Field(0))]);
    assert_joined_projected_write([(0, FieldKey::Field(0)), (0, FieldKey::Field(1))]);
}

fn assert_joined_projected_write(routes: [(usize, FieldKey); 2]) {
    const LEFT: usize = 0;
    const RIGHT: usize = 1;
    const OLD: usize = 2;
    const NEW: usize = 3;
    const WRITER: usize = 4;
    const LOCAL_COUNT: usize = 5;
    let mut state = BorrowState::new_uninitialized(LOCAL_COUNT);
    for slot in [LEFT, RIGHT, OLD, NEW] {
        state.initialize_parameter(slot);
    }
    let old = state.local_alloc_ids(OLD).0[0];
    let new = state.local_alloc_ids(NEW).0[0];
    let cells = routes.map(|(local, field)| (state.local_alloc_ids(local).0[0], field));
    for (holder, field) in cells {
        state.add_alloc_edge(
            holder,
            field,
            StoredValue {
                id: old,
                writable: false,
            },
        );
    }
    let mut alternate = state.clone();
    for ((branch, (origin, _)), (holder, field)) in
        [(&mut state, routes[0]), (&mut alternate, routes[1])]
            .into_iter()
            .zip(cells)
    {
        branch.set_binding(
            WRITER,
            BindingSpec::Projected {
                addresses: AddressSet::from_cells(
                    [CellId::Projected(holder, field)],
                    Coverage::Complete,
                ),
                origin: WrapperId::current(origin),
                owner: Some(WrapperId::current(origin)),
                writable: true,
                isolated: false,
                route_roots: roots(LOCAL_COUNT, &[origin]),
            },
            RootSet::empty(LOCAL_COUNT),
            RootSet::empty(LOCAL_COUNT),
        );
    }
    assert!(state.join_in_place(&alternate));

    let addresses = state.resolve_cells(WRITER);
    assert_eq!(addresses.coverage, Coverage::Complete);
    assert_eq!(addresses.cells().len(), cells.len());
    for (holder, field) in cells {
        assert!(
            addresses
                .cells()
                .contains(&CellId::Projected(holder, field))
        );
    }
    assert_eq!(addresses.definite(), None);
    let store =
        TypedStore::write_through(addresses, writable_value(new), Replacement::Must, WRITER);
    state.publish_store(&store, UNKNOWN_ORDER_KEY, None);

    let writer = state.resolve_local_allocs(WRITER);
    assert_eq!(writer.ids, [new]);
    assert!(
        !writer.readonly,
        "the old immutable allocation is no longer observed"
    );
    for (local, field) in routes {
        let possible = state.resolve_alloc_path(local, &[true; LOCAL_COUNT], [field]);
        assert!(possible.ids.contains(&old));
        assert!(possible.ids.contains(&new));
        assert!(
            possible.readonly,
            "each possible referent may still contain the old value"
        );
    }
}

#[test]
fn diamond_keeps_a_readonly_restriction_on_a_shared_suffix() {
    let mut left = BorrowState::new_uninitialized(5);
    left.initialize_parameter(0);
    set_link(&mut left, 5, 1, 0, true);
    set_link(&mut left, 5, 2, 1, true);
    set_link(&mut left, 5, 3, 1, false);
    let mut right = left.clone();
    set_link(&mut left, 5, 4, 2, true);
    set_link(&mut right, 5, 4, 3, true);
    let mutable = [true; 5];
    assert!(
        left.resolve_place_path(4, &mutable, [])
            .permission
            .wrapper_readonly_source
            .is_none()
    );
    assert_eq!(
        right
            .resolve_place_path(4, &mutable, [])
            .permission
            .wrapper_readonly_source,
        Some(3)
    );
    let readonly_middle = [true, false, true, true, true];
    assert_eq!(
        left.resolve_place_path(4, &readonly_middle, [])
            .permission
            .wrapper_readonly_source,
        Some(1)
    );

    for (mut state, other) in [(left.clone(), &right), (right.clone(), &left)] {
        assert!(state.join_in_place(other));
        let resolved = state.resolve_place_path(4, &mutable, []);
        assert_eq!(resolved.address.cells(), &[slot(0)]);
        assert_eq!(resolved.address.definite(), Some(slot(0)));
        assert_eq!(resolved.permission.wrapper_readonly_source, Some(3));
        let route = state.binding(4).route_roots();
        assert!(route.contains(2) && route.contains(3));
    }
}

#[test]
fn cyclic_arm_leaves_a_known_slot_possible_but_not_definite() {
    let mut state = BorrowState::new_uninitialized(3);
    state.initialize_parameter(0);
    set_link(&mut state, 3, 1, 0, true);
    set_link(&mut state, 3, 2, 1, true);
    let mut cyclic = state.clone();
    set_link(&mut cyclic, 3, 1, 2, true);
    assert!(state.join_in_place(&cyclic));

    for local in [1, 2] {
        let addresses = state.resolve_cells(local);
        assert_eq!(addresses.cells(), &[slot(0)]);
        assert_eq!(addresses.coverage, Coverage::Partial);
        assert_eq!(addresses.definite(), None);
    }
}

#[test]
fn escaped_slot_retains_prior_address_but_pristine_definition_does_not_retire() {
    let mut state = BorrowState::new_uninitialized(3);
    state.initialize_parameter(0);
    set_link(&mut state, 3, 1, 0, true);
    let fresh_slot = || LocalState::slot_from_value_roots(RootSet::empty(3));
    state.rebind_local(0, fresh_slot(), 0);
    let addresses = state.resolve_cells(1);
    assert_eq!(
        addresses.cells(),
        &[CellId::Slot(WrapperId {
            local: 0,
            age: WrapperAge::Prior
        })],
    );
    assert_eq!(addresses.coverage, Coverage::Complete);
    assert_eq!(addresses.definite(), None);
    assert_eq!(state.resolve_cells(0).definite(), Some(slot(0)));

    assert!(state.has_pristine_alloc_binding(2));
    assert!(
        state
            .rebind_local_after_capture(2, fresh_slot(), 1, Some(2), true)
            .is_none(),
    );
    assert_eq!(state.resolve_cells(2).definite(), Some(slot(2)));
}

#[test]
fn projected_destination_stays_on_the_old_holder_after_slot_replacement() {
    const HOLDER: usize = 0;
    const REPLACEMENT: usize = 1;
    const OLD: usize = 2;
    const NEW: usize = 3;
    const WRITER: usize = 4;
    const COUNT: usize = 5;
    let mut state = BorrowState::new_uninitialized(COUNT);
    for slot in [HOLDER, REPLACEMENT, OLD, NEW] {
        state.initialize_parameter(slot);
    }
    let holder = state.local_alloc_ids(HOLDER).0[0];
    let replacement = state.local_alloc_ids(REPLACEMENT).0[0];
    let old = state.local_alloc_ids(OLD).0[0];
    let new = state.local_alloc_ids(NEW).0[0];
    let field = FieldKey::Field(0);
    state.add_alloc_edge(
        holder,
        field,
        StoredValue {
            id: old,
            writable: true,
        },
    );
    state.add_alloc_edge(
        replacement,
        field,
        StoredValue {
            id: old,
            writable: true,
        },
    );
    let cell = CellId::Projected(holder, field);
    state.set_binding(
        WRITER,
        BindingSpec::Projected {
            addresses: AddressSet::from_cells([cell], Coverage::Complete),
            origin: WrapperId::current(HOLDER),
            owner: Some(WrapperId::current(HOLDER)),
            writable: true,
            isolated: false,
            route_roots: roots(COUNT, &[HOLDER]),
        },
        roots(COUNT, &[HOLDER]),
        RootSet::empty(COUNT),
    );
    assert_eq!(state.resolve_cells(WRITER).definite(), Some(cell));
    let retirement = state.retire_slot_value(HOLDER, 0);
    let mut retired_holder = holder;
    retirement.remap_id(&state, &mut retired_holder);
    state.publish_store(
        &TypedStore::replace_cells(state.resolve_cells(HOLDER), writable_value(replacement)),
        0,
        None,
    );

    let addresses = state.resolve_cells(WRITER);
    assert_ne!(retired_holder, holder);
    assert_ne!(retired_holder, replacement);
    assert_eq!(
        addresses.cells(),
        &[CellId::Projected(retired_holder, field)]
    );
    assert_eq!(state.local_alloc_ids(WRITER).0, [old]);
    let store =
        TypedStore::write_through(addresses, writable_value(new), Replacement::Must, WRITER);
    state.publish_store(&store, 1, None);
    assert_eq!(state.local_alloc_ids(WRITER).0, [new]);
    assert_eq!(
        state
            .resolve_alloc_path(HOLDER, &[true; COUNT], [field])
            .ids,
        [old]
    );
}

fn test_copy(state: &mut BorrowState, from: usize, to: usize, site: u32) {
    let source = state.resolve_local_allocs(from);
    let site = state.site_id(site, BlockId(0), &Default::default());
    let ids = state.copy_alloc_graph(&source, site);
    assert!(!ids.is_empty());
    state.bind_alloc_ids(to, &ids, false);
}

fn overlaps(left: &[AllocId], right: &[AllocId]) -> bool {
    left.iter().any(|value| right.contains(value))
}

#[test]
fn copied_nested_sharing_is_pairwise_not_transitive() {
    let mut state = BorrowState::new_uninitialized(4);
    for slot in 0..4 {
        state.initialize_parameter(slot);
    }
    let root = state.local_alloc_ids(0).0[0];
    let common = state.local_alloc_ids(1).0[0];
    let inner = FieldKey::Field(8);
    let mut views = Vec::new();
    for ordinal in 0..3 {
        let key = FieldKey::Field(ordinal);
        let holder = state.alloc_child(root, key);
        let view = state.call_view_id(1, false, ordinal);
        state.add_alloc_edge(
            holder,
            inner,
            StoredValue {
                id: common,
                writable: true,
            },
        );
        state.add_view_member(view, common);
        state.link_view(holder, inner, view);
        views.push(view);
    }
    state.relate_views(views[0], views[1], true);
    state.relate_views(views[1], views[2], true);
    state.relate_views(views[0], views[2], false);

    test_copy(&mut state, 0, 2, 80);
    test_copy(&mut state, 2, 3, 81);
    for binding in [2, 3] {
        let slots = [0, 1, 2].map(|field| {
            state
                .resolve_alloc_path(binding, &[true; 4], [FieldKey::Field(field), inner])
                .ids
        });
        assert!(overlaps(&slots[0], &slots[1]));
        assert!(overlaps(&slots[1], &slots[2]));
        assert!(!overlaps(&slots[0], &slots[2]));
        assert!(slots.iter().all(|slot| !slot.contains(&common)));
    }
}

#[test]
fn publisher_kills_only_exact_dependent_scalar_replacements() {
    let mut entry = BorrowState::new_uninitialized(2);
    entry.initialize_parameter(0);
    entry.initialize_parameter(1);
    let receiver = entry.local_alloc_ids(0).0[0];
    let holder = entry.local_alloc_ids(1).0[0];
    let field = FieldKey::Field(0);
    let cell = CellId::Projected(holder, field);
    let old = ValueAlt {
        kind: ValueKind::Scalar,
        deps: vec![ReceiverDependency {
            entries: [receiver].into_iter().collect(),
            coverage: Coverage::Complete,
        }],
    };
    let capture = CapturedValue {
        alternatives: vec![old.clone()],
        ..Default::default()
    };
    entry.record_constructed_value(holder, field, &capture, true);

    for (coverage, replacement) in [
        (Coverage::Complete, Replacement::May),
        (Coverage::Partial, Replacement::Must),
        (Coverage::Complete, Replacement::Must),
    ] {
        let mut state = entry.clone();
        let addresses = AddressSet::from_cells([cell], coverage).with_singleton_proof();
        let store = match replacement {
            Replacement::May => TypedStore::write_cells(addresses, CapturedValue::default()),
            Replacement::Must => TypedStore::replace_cells(addresses, CapturedValue::default()),
        };
        let receipt = state.publish_store(&store, 0, None);
        if coverage == Coverage::Complete && replacement == Replacement::Must {
            assert_eq!(receipt.replaced[0], Some((cell, vec![old.clone()])));
            assert!(state.value_cell(cell).is_empty());
        } else {
            assert!(
                receipt
                    .replaced
                    .into_iter()
                    .all(|replace| replace.is_none())
            );
            assert!(state.value_cell(cell).contains(&old));
        }
    }
}

#[test]
fn publication_distinguishes_identical_old_and_new_carriers() {
    use crate::compiler_frontend::analysis::borrow_checker::state::PublicationCarriers;
    use crate::compiler_frontend::analysis::borrow_checker::transfer::facts::{
        Carrier, CarrierValue, Demand, ObservationBoundary, ObservationCheckpoint,
        ObservationSegment, ValueObservations,
    };

    let (mut state, [receiver, holder, copy_holder]) = parameter_allocations::<3>();
    let cell = CellId::Projected(holder, FieldKey::Elem);
    let detached = CellId::Projected(copy_holder, FieldKey::Elem);
    let value = ValueAlt {
        kind: ValueKind::Scalar,
        deps: vec![ReceiverDependency {
            entries: [receiver].into_iter().collect(),
            coverage: Coverage::Complete,
        }],
    };
    let captured = CapturedValue {
        alternatives: vec![value.clone()],
        ..Default::default()
    };
    for holder in [holder, copy_holder] {
        state.record_constructed_value(holder, FieldKey::Elem, &captured, true);
    }
    let carrier = |cell| Carrier::Cell(cell, CarrierValue::Scalar, value.deps.clone());
    let mut observations = ValueObservations::default();
    let checkpoint = |order| {
        ObservationBoundary::Checkpoint(ObservationCheckpoint {
            order,
            actor: None,
            span: None,
            candidates: Demand::default(),
            pending: Demand::default(),
        })
    };

    let receipt = state.publish_family_members(
        &[holder],
        Coverage::Complete,
        FieldKey::Elem,
        Replacement::Must,
        &[
            crate::compiler_frontend::analysis::borrow_checker::state::FamilyMember {
                value: captured,
                at_most_once: true,
            },
        ],
    );
    observations.split(checkpoint(0));
    observations.publish(receipt);
    observations.split(checkpoint(1));
    let mut later = ObservationSegment::default();
    later.read([carrier(cell), carrier(detached)]);
    observations.tail.then(later);

    let mut demand = Demand::default();
    observations.backward(&mut demand, |checkpoint, demand| {
        assert!(demand.contains(&carrier(detached)));
        assert_eq!(demand.contains(&carrier(cell)), checkpoint.order == 1);
    });
    assert_eq!(demand, [carrier(detached)].into_iter().collect());

    let mut weak = PublicationCarriers::default();
    weak.record(
        cell,
        std::slice::from_ref(&value),
        cell,
        state.value_cell(cell),
        false,
    );
    assert!(weak.definite_old.is_empty() && weak.introduced.is_empty());
}

#[test]
fn retirement_preserves_distinct_dependencies_on_one_allocation() {
    let mut state = BorrowState::new_uninitialized(3);
    for local in 0..3 {
        state.initialize_parameter(local);
    }
    let holder = state.local_alloc_ids(0).0[0];
    let other = state.local_alloc_ids(1).0[0];
    let child = StoredValue {
        id: state.local_alloc_ids(2).0[0],
        writable: true,
    };
    let key = FieldKey::Field(0);
    let alternatives = [holder, other].map(|receiver| ValueAlt {
        kind: ValueKind::Allocation(child),
        deps: vec![ReceiverDependency {
            entries: [receiver].into_iter().collect(),
            coverage: Coverage::Complete,
        }],
    });
    state.record_constructed_value(
        holder,
        key,
        &CapturedValue {
            values: vec![child],
            alternatives: alternatives.to_vec(),
            ..Default::default()
        },
        true,
    );
    let retirement = state.retire_slot_value(0, 1);
    let mut retired = holder;
    retirement.remap_id(&state, &mut retired);
    let (cell, retained) = state.value_cells().next().unwrap();
    assert_eq!(cell, CellId::Projected(retired, key));
    assert_ne!(holder, retired);
    assert_eq!(retained.len(), 2);
    assert!(
        retained
            .iter()
            .all(|alt| alt.kind == ValueKind::Allocation(child))
    );
    assert!(
        retained
            .iter()
            .any(|alt| alt.deps[0].entries.as_slice() == [retired])
    );
    assert!(
        retained
            .iter()
            .any(|alt| alt.deps[0].entries.as_slice() == [other])
    );
}

#[test]
fn family_erase_receipt_distinguishes_exact_partial_and_multiple_receivers() {
    let mut entry = BorrowState::new_uninitialized(2);
    entry.initialize_parameter(0);
    entry.initialize_parameter(1);
    let receiver = entry.local_alloc_ids(0).0[0];
    let other = entry.local_alloc_ids(1).0[0];
    let payload = CapturedValue {
        alternatives: vec![ValueAlt {
            kind: ValueKind::Scalar,
            deps: vec![ReceiverDependency {
                entries: [other].into_iter().collect(),
                coverage: Coverage::Complete,
            }],
        }],
        ..Default::default()
    };
    let keys = [FieldKey::Elem, FieldKey::MapKey];
    for key in keys {
        entry.record_constructed_value(receiver, key, &payload, true);
    }
    for (receivers, coverage, exact) in [
        (vec![receiver], Coverage::Complete, true),
        (vec![receiver], Coverage::Partial, false),
        (vec![receiver, other], Coverage::Complete, false),
    ] {
        let mut state = entry.clone();
        let receipt = state.publish_store(
            &TypedStore::EraseFamily {
                receivers,
                coverage,
                keys: EntryFamilies::MapEntries,
            },
            0,
            None,
        );
        let erased = receipt
            .replaced
            .into_iter()
            .flatten()
            .map(|(cell, values)| {
                assert_eq!(values, payload.alternatives);
                cell
            })
            .collect::<Vec<_>>();
        if exact {
            assert_eq!(erased, keys.map(|key| CellId::Projected(receiver, key)));
        } else {
            assert!(erased.is_empty());
        }
        assert_eq!(state.value_cells().count(), if exact { 0 } else { 2 });
    }
}

#[test]
fn proven_sole_remove_erases_member_and_keeps_detachment() {
    let mut state = BorrowState::new_uninitialized(4);
    for local in 0..4 {
        state.initialize_parameter(local);
    }
    let holder = state.local_alloc_ids(0).0[0];
    let old_receiver = state.local_alloc_ids(1).0[0];
    // A lazy Elem child summarises arbitrarily many entries and cannot prove a sole member.
    let child = StoredValue {
        id: state.local_alloc_ids(3).0[0],
        writable: true,
    };
    let value = CapturedValue {
        values: vec![child],
        alternatives: vec![ValueAlt {
            kind: ValueKind::Allocation(child),
            deps: vec![ReceiverDependency {
                entries: [old_receiver].into_iter().collect(),
                coverage: Coverage::Complete,
            }],
        }],
        ..Default::default()
    };
    state.record_constructed_value(holder, FieldKey::Elem, &value, true);
    let result = state.resolve_cells(2);
    state.publish_store(
        &TypedStore::receive_result(result, value.clone(), 2, false),
        0,
        None,
    );
    let receipt = state.remove_alloc_element(&[holder], false, Some(2));
    assert_eq!(
        receipt.replaced[0],
        Some((
            CellId::Projected(holder, FieldKey::Elem),
            value.alternatives.clone()
        ))
    );
    assert!(state.detached_from(2, &[holder]));
    assert_eq!(
        state
            .capture_jump_value(2, false, false)
            .unwrap()
            .alternatives,
        value.alternatives
    );
}

#[test]
fn exact_store_replaces_view_but_weak_store_preserves_old_value_and_siblings() {
    let mut state = BorrowState::new_uninitialized(6);
    for slot in 0..6 {
        state.initialize_parameter(slot);
    }
    let root = state.local_alloc_ids(0).0[0];
    let old = state.local_alloc_ids(1).0[0];
    let new = state.local_alloc_ids(2).0[0];
    let wrapper = state.local_alloc_ids(3).0[0];
    let left = FieldKey::Field(0);
    let right = FieldKey::Field(1);
    let old_view = state.call_view_id(1, false, 0);
    let right_view = state.call_view_id(1, false, 1);
    for (field, view) in [(left, old_view), (right, right_view)] {
        state.add_alloc_edge(
            root,
            field,
            StoredValue {
                id: old,
                writable: true,
            },
        );
        state.add_view_member(view, old);
        state.link_view(root, field, view);
    }
    state.relate_views(old_view, right_view, true);

    let old_value = state.resolve_alloc_path(0, &[true; 6], [left]);
    let right_value = state.resolve_alloc_path(0, &[true; 6], [right]);
    let cell = CellId::Projected(root, left);
    let partial = AddressSet::from_cells([cell], Coverage::Partial).with_singleton_proof();
    assert_eq!(partial.definite(), None);
    assert!(state.alloc_singular(root));
    let exact = AddressSet::from_cells([cell], Coverage::Complete).with_singleton_proof();
    let mut weak = state.clone();
    weak.publish_store(
        &TypedStore::replace_cells(partial, writable_value(new)),
        UNKNOWN_ORDER_KEY,
        None,
    );
    state.publish_store(
        &TypedStore::replace_cells(exact, writable_value(new)),
        UNKNOWN_ORDER_KEY,
        None,
    );
    let left_now = state.resolve_alloc_path(0, &[true; 6], [left]);
    let left_weak = weak.resolve_alloc_path(0, &[true; 6], [left]);
    assert!(left_weak.ids.contains(&old) && left_weak.ids.contains(&new));
    assert_eq!(weak.resolve_alloc_path(0, &[true; 6], [right]).ids, [old]);
    let shares_right = |state: &BorrowState, capture: &_| {
        let capture: &super::super::super::state::ResolvedPlace = capture;
        state.alloc_paths_may_share(
            &capture.ids,
            &capture.views,
            &right_value.ids,
            &right_value.views,
        )
    };
    assert!(!shares_right(&state, &left_now));
    assert!(shares_right(&weak, &left_weak));
    assert!(shares_right(&state, &old_value));

    for (key, capture) in [(left, old_value), (right, right_value)] {
        for id in capture.ids {
            state.add_alloc_edge(wrapper, key, StoredValue { id, writable: true });
        }
        for view in capture.views {
            state.link_view(wrapper, key, view);
        }
    }
    test_copy(&mut state, 3, 4, 82);
    let first = state.resolve_alloc_path(4, &[true; 6], [left]);
    let second = state.resolve_alloc_path(4, &[true; 6], [right]);
    assert!(overlaps(&first.ids, &second.ids));
}

#[test]
fn joining_covered_paths_does_not_intersect_joined_marginals() {
    let mut entry = BorrowState::new_uninitialized(4);
    for slot in 0..4 {
        entry.initialize_parameter(slot);
    }
    let root = entry.local_alloc_ids(0).0[0];
    let a = entry.local_alloc_ids(1).0[0];
    let b = entry.local_alloc_ids(2).0[0];
    let left = FieldKey::Field(0);
    let right = FieldKey::Field(1);
    let left_view = entry.call_view_id(3, false, 0);
    let right_view = entry.call_view_id(3, false, 1);
    let arm = |mut state: BorrowState, a, b| {
        for (field, id, view) in [(left, a, left_view), (right, b, right_view)] {
            state.add_alloc_edge(root, field, StoredValue { id, writable: true });
            state.add_view_member(view, id);
            state.link_view(root, field, view);
        }
        state.relate_views(left_view, right_view, false);
        state
    };
    let mut state = arm(entry.clone(), a, b);
    state.join_in_place(&arm(entry, b, a));
    let a = state.resolve_alloc_path(0, &[true; 4], [left]);
    let b = state.resolve_alloc_path(0, &[true; 4], [right]);
    assert_eq!(a.ids, b.ids);
    assert!(!state.alloc_paths_may_share(&a.ids, &a.views, &b.ids, &b.views));
    test_copy(&mut state, 0, 3, 83);
    let a = state.resolve_alloc_path(3, &[true; 4], [left]);
    let b = state.resolve_alloc_path(3, &[true; 4], [right]);
    assert!(!overlaps(&a.ids, &b.ids));
}

#[test]
fn construction_revisits_keep_plain_children_next_to_tracked_occurrences() {
    let mut entry = BorrowState::new_uninitialized(2);
    for slot in 0..2 {
        entry.initialize_parameter(slot);
    }
    let root = entry.local_alloc_ids(0).0[0];
    let shared = entry.local_alloc_ids(1).0[0];
    let field = FieldKey::Field(0);
    let tracked = entry.call_view_id(3, false, 0);
    let other = entry.call_view_id(3, false, 1);
    entry.add_view_member(tracked, shared);
    entry.add_view_member(other, shared);
    entry.relate_views(tracked, other, false);
    let value = StoredValue {
        id: shared,
        writable: true,
    };
    for tracked_first in [false, true] {
        let mut state = entry.clone();
        let ordinary = |state: &mut BorrowState| state.contribute_alloc_edge(root, field, value);
        let precise = |state: &mut BorrowState| {
            state.record_constructed_field(root, field, &[value], &[tracked], false);
        };
        if tracked_first {
            precise(&mut state);
            ordinary(&mut state);
        } else {
            ordinary(&mut state);
            precise(&mut state);
        }
        let result = state.resolve_alloc_path(0, &[true; 2], [field]);
        assert!(state.alloc_paths_may_share(&result.ids, &result.views, &[shared], &[other]));
        assert!(result.views.contains(&tracked));
    }
}

const DISJOINT_SELECT: &str = "\
Pair = |left {I32}, right {I32}|\n\
select |flag Bool, a {I32}, b {I32}| -> Pair:\n\
    if flag:\n\
        return Pair(a, {2})\n\
    ;\n\
    return Pair({3}, b)\n\
;\n";

#[test]
fn a_repeated_call_does_not_export_same_iteration_disjointness() {
    let source = format!(
        "{DISJOINT_SELECT}\
         Box = |items {{I32}}|\n\
         probe || -> I32:\n\
             items = {{1}}\n\
             saved_left ~= Box({{0}})\n\
             saved_right ~= Box({{0}})\n\
             loop 0 to 2 |index|:\n\
                 both = select(index is 0, items, items)\n\
                 if index is 0:\n\
                     ~saved_left.items = both.left\n\
                 else\n\
                     ~saved_right.items = both.right\n\
                 ;\n\
             ;\n\
             together = Pair(saved_left.items, saved_right.items)\n\
             owned ~= copy together\n\
             observed = owned.right\n\
             ~owned.left.push(4)\n\
             return observed.length()\n\
         ;"
    );
    check_rejection(
        &source,
        "owned.left.push",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn fresh_call_wrappers_keep_inherited_disjoint_child_occurrences() {
    let source = format!(
        "{DISJOINT_SELECT}\
         Box = |items {{I32}}|\n\
         Both = |left Box, right Box|\n\
         wrap |items {{I32}}| -> Box:\n\
             return Box(items)\n\
         ;\n\
         probe |flag Bool| -> I32:\n\
             items = {{1}}\n\
             selected = select(flag, items, items)\n\
             left = wrap(selected.left)\n\
             right = wrap(selected.right)\n\
             both = Both(left, right)\n\
             owned ~= copy both\n\
             observed = owned.right.items\n\
             ~owned.left.items.push(4)\n\
             return observed.length()\n\
         ;"
    );
    check_source(&source).expect("call wrappers must preserve the captured child occurrences");
}

#[test]
fn projected_return_retains_the_writer_exemption_without_adding_a_coarse_root() {
    let source = "\
Box = |items {I32}|\n\
extract |box Box| -> {I32}:\n\
    return box.items\n\
;\n\
probe || -> I32:\n\
    owner ~= Box({1})\n\
    writer ~= owner\n\
    view = extract(writer)\n\
    result = view.length()\n\
    ~writer.items.push(2)\n\
    return result\n\
;";
    check_source(source).expect("a projected return is derived from its exclusive writer");
}

#[test]
fn collection_set_error_outlives_a_later_receiver_mutation() {
    let source = "\
probe || -> String:\n\
    items ~= {1}\n\
    ~items.set(6, 2) catch |err|:\n\
        ~items.push(3)\n\
        return err.message\n\
    ;\n\
    return \"unexpected success\"\n\
;";
    check_source(source).expect("a builtin error must not retain its receiver");
}

#[test]
fn only_direct_sharing_pairs_conflict_with_a_live_observer_after_copy() {
    let prefix = "\
Triple = |a {I32}, b {I32}, c {I32}|\n\
select |flag Bool, left {I32}, right {I32}| -> Triple:\n\
    if flag:\n\
        return Triple(left, left, {2})\n\
    ;\n\
    return Triple({3}, right, right)\n\
;\n\
probe |flag Bool| -> I32:\n\
    items = {1}\n\
    selected = select(flag, items, items)\n\
    owned ~= copy selected\n";
    let with_observer = |field| {
        format!(
            "{prefix}    observed = owned.{field}\n\
             ~owned.a.push(4)\n\
             return observed.length()\n\
         ;"
        )
    };
    check_source(&with_observer("c")).expect("A/B and B/C sharing does not imply A/C sharing");
    check_rejection(
        &with_observer("b"),
        "owned.a.push",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn a_projected_call_actual_is_captured_before_replacing_its_holder() {
    let prefix = "\
Box = |items {I32}|\n\
wrap |value {I32}| -> Box:\n\
    return Box(value)\n\
;\n\
probe || -> I32:\n\
    shared ~= {1}\n\
    holder ~= Box(shared)\n";
    let suffix = "\
    ~shared.push(3)\n\
    return holder.items.length()\n\
;";
    check_rejection(
        &format!("{prefix}    ~holder = wrap(holder.items)\n{suffix}"),
        "shared.push",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
    check_source(&format!(
        "{prefix}    ~holder = wrap(copy holder.items)\n{suffix}"
    ))
    .expect("passing a copied projection must not retain the previous child");
}

#[test]
fn native_fresh_slot_excludes_actuals_but_can_share_with_another_native_slot() {
    use crate::compiler_frontend::analysis::borrow_checker::summarize_call_allocations;
    use crate::compiler_frontend::datatypes::builtin_type_ids::STRING;
    use crate::compiler_frontend::external_packages::{
        CallTarget, ExternalAbiType, ExternalAccessKind, ExternalFunctionDef,
        ExternalFunctionLowerings, ExternalNumericContract, ExternalParameter, ExternalReturnSlot,
        external_returns,
    };
    use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
    use crate::compiler_frontend::hir::ids::HirNodeId;
    use crate::compiler_frontend::hir::places::HirPlace;
    use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
    use crate::compiler_frontend::public_call_summary::{
        CallAllocationSource, CallOutputPath, CallResultPath, FunctionReturnAliasSummary,
    };
    use crate::compiler_frontend::tests::borrow_fixture_support::assert_invalid_mutable_access_reason;
    use crate::compiler_frontend::tests::hir_fixture_support::expression;
    use std::sync::Arc;

    // Author the result destinations through normal multi-bind lowering, then supply registry
    // contracts to exercise the opaque native boundary without requiring a backend adapter.
    let source = "\
pair |value String| -> String, String:\n\
    return value, \"fresh value\"\n\
;\n\
forward |value String| -> String, String:\n\
    left, right = pair(value)\n\
    return left, right\n\
;\n\
probe || -> String:\n\
    input ~= \"input value\"\n\
    left, right = pair(input)\n\
    return \"done\"\n\
;";
    for (mutate_result, observe_retained, accepts) in [
        (false, false, true),
        (false, true, false),
        (true, true, false),
    ] {
        let (ast, mut paths, mut strings) = parse_single_file_ast(source);
        let mut registry = default_external_package_registry(&mut strings);
        let mut register = |name: &str, access_kind, slots| {
            Arc::make_mut(&mut registry)
                .register_function(ExternalFunctionDef {
                    name: name.to_owned(),
                    parameters: vec![ExternalParameter {
                        language_type: ExternalAbiType::Utf8Str.into(),
                        access_kind,
                    }],
                    return_contract: external_returns(slots, None),
                    const_eval: None,
                    numeric_contract: ExternalNumericContract::Unknown,
                    lowerings: ExternalFunctionLowerings::default(),
                })
                .expect("native fixture registration")
        };
        let returns = register(
            "native_pair",
            ExternalAccessKind::Shared,
            vec![
                ExternalReturnSlot::alias_args(ExternalAbiType::Utf8Str, vec![0]),
                ExternalReturnSlot::fresh(ExternalAbiType::Utf8Str),
            ],
        );
        let mutates = register("native_mutates", ExternalAccessKind::Mutable, Vec::new());
        let (mut module, types) = lower_ast(ast, &mut strings, &mut paths, &registry)
            .expect("native fixture should lower");
        let entry = paths
            .try_intern_portable_path("@entry.moth", &mut strings)
            .unwrap();
        let forward_path = paths
            .try_intern_child(entry, strings.intern("forward"))
            .unwrap();
        let forward = module
            .functions
            .iter()
            .find(|function| {
                module.side_table.function_name_path(function.id) == Some(forward_path)
            })
            .expect("fixture should have a Moth forwarder");
        let forward_id = forward.id;
        let forward_parameter = forward.params[0];
        let expressions = &module.expressions;
        let mut probe_call = None;
        for block in &mut module.blocks {
            for statement in &mut block.statements {
                let HirStatementKind::Call {
                    target,
                    args,
                    results,
                } = &mut statement.kind
                else {
                    continue;
                };
                let [first, second] = results.as_slice() else {
                    continue;
                };
                *target = CallTarget::External(returns);
                let actual = expressions.values(*args)[0];
                if !matches!(&expressions.expression(actual).kind,
                    HirExpressionKind::Load(place) if place.root == forward_parameter)
                {
                    assert!(
                        probe_call
                            .replace((block.id, [first.local(), second.local()], actual))
                            .is_none()
                    );
                }
            }
        }
        let (block, slots, actual) = probe_call.expect("fixture should have a direct native probe");
        if !mutate_result && !observe_retained {
            let report = summarize_call_allocations(&module, &types, &registry, &paths, &strings)
                .expect("the Moth forwarder should summarize");
            let FunctionReturnAliasSummary::Known(facts) =
                &report.analysis.public_call_summaries[&forward_id].return_alias
            else {
                panic!("the Moth forwarder should have a known success lane");
            };
            let second_root = CallOutputPath::Result(CallResultPath {
                slot: 1,
                path: Vec::new(),
            });
            let sources: Vec<_> = facts
                .success
                .sources
                .iter()
                .filter(|row| row.output == second_root)
                .map(|row| &row.source)
                .collect();
            assert_eq!(sources, [&CallAllocationSource::OpaqueFresh]);
        }
        let HirExpressionKind::Load(input) = &module.expressions.expression(actual).kind else {
            panic!("whole original actual must be a place");
        };
        let actor = if mutate_result { slots[1] } else { input.root };
        module
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.locals)
            .find(|local| local.id == actor)
            .expect("mutated slot must be declared")
            .mutable = true;
        let region = module.blocks[block.0 as usize].region;
        let place = |local| HirExpressionKind::Load(HirPlace::local(local));
        let actor = expression(
            place(actor),
            STRING,
            region,
            ValueKind::Place,
            &mut module.expressions,
        );
        let observer = expression(
            place(slots[usize::from(!observe_retained)]),
            STRING,
            region,
            ValueKind::Place,
            &mut module.expressions,
        );
        let args = module
            .expressions
            .append_values(&[actor], None)
            .expect("valid argument range");
        module.blocks[block.0 as usize].statements.extend([
            HirStatement {
                id: HirNodeId(90_000),
                span: None,
                kind: HirStatementKind::Call {
                    target: CallTarget::External(mutates),
                    args,
                    results: Vec::new(),
                },
            },
            HirStatement {
                id: HirNodeId(90_001),
                span: None,
                kind: HirStatementKind::Expr(observer),
            },
        ]);
        let decision = run_borrow_checker_with_types(&module, &types, &registry, &paths, &strings);
        if accepts {
            decision.expect("the live fresh slot cannot observe the actual's allocation");
        } else {
            assert_invalid_mutable_access_reason(
                decision.expect_err("the live native slot may observe the mutation"),
                InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
            );
        }
    }
}

#[test]
fn imported_nested_call_sibling_conflicts_only_while_live() {
    use crate::compiler_frontend::analysis::borrow_checker::summarize_call_allocations;
    use crate::compiler_frontend::external_packages::CallTarget;
    use crate::compiler_frontend::hir::statements::HirStatementKind;
    use crate::compiler_frontend::semantic_identity::{
        ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity, StablePackageIdentity,
    };

    for (tail, live) in [
        ("first.inner.items.length()", false),
        ("second.inner.items.length()", true),
    ] {
        let source = format!(
            "\
Inner = |items {{I32}}|\n\
Box = |inner Inner|\n\
pair || -> Box, Box:\n\
    shared = {{1}}\n\
    return Box(Inner(shared)), Box(Inner(shared))\n\
;\n\
probe || -> I32:\n\
    first ~Box, second Box = pair()\n\
    ~first.inner.items.push(2)\n\
    return {tail}\n\
;"
        );
        let (ast, mut paths, mut strings) = parse_single_file_ast(&source);
        let registry = default_external_package_registry(&mut strings);
        let (mut module, types) = lower_ast(ast, &mut strings, &mut paths, &registry)
            .expect("fixture source should lower");
        validate_hir_module(&module, &types, &registry).expect("fixture HIR should validate");
        let mut provider = summarize_call_allocations(&module, &types, &registry, &paths, &strings)
            .expect("fixture provider should summarize");
        let origin = OriginFunctionId::new_free(
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("nested-result-sibling"),
                "helper".to_owned(),
                ModuleRootRole::Normal,
            ),
            "pair".to_owned(),
        );
        // Move the real provider contract through the imported target adapter, keeping the
        // caller and provider's concrete layout in this test's single type environment.
        let (function, target) = module
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find_map(|statement| match &mut statement.kind {
                HirStatementKind::Call {
                    target, results, ..
                } if results.len() == 2 => {
                    let CallTarget::Local(id) = target else {
                        return None;
                    };
                    Some((*id, target))
                }
                _ => None,
            })
            .expect("fixture should have a local multi-result call");
        *target = CallTarget::CrossModule(origin.clone());
        module.imported_call_summaries.insert(
            origin,
            provider
                .analysis
                .public_call_summaries
                .remove(&function)
                .expect("provider contract"),
        );
        let decision =
            run_borrow_checker_with_types(&module, &types, &registry, &paths, &strings).map(|_| ());
        if live {
            check_rejection_at(
                decision,
                &source,
                "first.inner.items.push",
                InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
            );
        } else {
            decision.expect("a dead immutable sibling does not make the shared result read-only");
        }
    }
}

#[test]
fn returned_optional_payload_conflicts_only_while_its_observer_is_live() {
    let prefix = "\
Link = |items {I32}|\n\
wrap_option |maybe {I32}?, retain Bool| -> Link:\n\
    if retain:\n\
        if maybe is |present|:\n\
            return Link(present)\n\
        ;\n\
    ;\n\
    return Link({2, 3})\n\
;\n\
probe |retain Bool| -> I32:\n\
    child ~= {1}\n\
    observer ~= Link(child)\n\
    source ~{I32}? = child\n\
    result ~= wrap_option(source, retain)\n\
    ~result.items.push(2)\n";
    let dead = format!("{prefix}return result.items.length()\n;");
    check_source(&dead).expect("the temporary payload binder does not restrict the mutable actual");
    let live = format!("{prefix}return result.items.length() * 10 + observer.items.length()\n;");
    check_rejection(
        &live,
        "result.items.push",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn a_call_can_return_owned_storage_from_a_lexically_immutable_local() {
    let source = "\
Box = |items {I32}|\n\
make || -> Box:\n\
    shared = {1}\n\
    return Box(shared)\n\
;\n\
probe || -> I32:\n\
    holder ~= make()\n\
    ~holder.items.push(2)\n\
    return holder.items.length()\n\
;";
    check_source(source).expect("the local's lexical immutability ends at the call boundary");
}

#[test]
fn copying_a_collapsed_construction_selector_preserves_plain_and_tracked_children() {
    let source = |children, other| {
        format!(
            "{DISJOINT_SELECT}\
         Together = |rows {{{{I32}}}}, other {{I32}}|\n\
         probe |flag Bool| -> I32:\n\
             items = {{1}}\n\
             selected = select(flag, items, items)\n\
             shared ~= {{9}}\n\
             together = Together({{{children}}}, {other})\n\
             owned ~= copy together\n\
             observed = owned.rows\n\
             ~owned.other.push(4)\n\
             seen = observed.get(1) catch then {{0}}\n\
             return seen.length()\n\
         ;"
        )
    };
    for children in ["selected.left, shared", "shared, selected.left"] {
        check_rejection(
            &source(children, "shared"),
            "owned.other.push",
            InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
        );
        check_source(&source(children, "{5}"))
            .expect("the independent field does not observe any member of the copied collection");
    }
}

#[test]
fn authored_error_retains_actual_until_the_handler_finishes_reading_it() {
    let source = |argument| {
        format!(
            "\
Failure = |items {{I32}}|\n\
fail |items {{I32}}| -> I32, Failure!:\n\
    return! Failure({argument})\n\
;\n\
probe || -> I32:\n\
    child ~= {{1}}\n\
    result = fail(child) catch |err|:\n\
        ~child.push(2)\n\
        return err.items.length()\n\
    ;\n\
    return result\n\
;"
        )
    };
    check_rejection(
        &source("items"),
        "child.push",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
    check_source(&source("copy items")).expect("a copied error does not retain the caller's child");
}

#[test]
fn call_return_and_store_can_share_one_fresh_allocation() {
    let source = |returned| {
        format!(
            "\
Link = |items {{I32}}|\n\
Outer = |link Link|\n\
replace |outer ~Outer| -> Link:\n\
    fresh ~= Link({{1}})\n\
    ~outer.link = fresh\n\
    return {returned}\n\
;\n\
probe || -> I32:\n\
    outer ~= Outer(Link({{0}}))\n\
    returned ~= replace(~outer)\n\
    ~returned.items = {{2}}\n\
    return outer.link.items.length()\n\
;"
        )
    };
    check_rejection(
        &source("fresh"),
        "returned.items =",
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
    check_source(&source("copy fresh")).expect("the copied return does not share the stored Link");
}

#[test]
fn fresh_call_stores_at_a_collapsed_entry_selector_are_not_singular() {
    use crate::compiler_frontend::analysis::borrow_checker::{
        diagnostics::BorrowDiagnostics, engine::BorrowChecker, transfer::BorrowTransferContext,
        transfer::transfer_block,
    };

    let source = "\
Holder = |items {I32}|\n\
fill |rows ~{Holder}|:\n\
    ~rows.push(Holder({1}))\n\
    ~rows.push(Holder({2}))\n\
;\n\
probe |rows ~{Holder}| -> I32:\n\
    fill(~rows)\n\
    return rows.length()\n\
;";
    let (ast, mut paths, mut strings) = parse_single_file_ast(source);
    let packages = default_external_package_registry(&mut strings);
    let (module, types) = lower_ast(ast, &mut strings, &mut paths, &packages).unwrap();
    let entry = paths
        .try_intern_portable_path("@entry.moth", &mut strings)
        .unwrap();
    let name = paths
        .try_intern_child(entry, strings.intern("probe"))
        .unwrap();
    let probe = module
        .functions
        .iter()
        .find(|function| module.side_table.function_name_path(function.id) == Some(name))
        .expect("authored probe");
    let mut checker = BorrowChecker::new(&module, &types, &packages, &paths, &strings);
    checker
        .build_public_call_summaries()
        .expect("summarize the two pushes");
    let blocks = checker.collect_reachable_blocks(probe).unwrap();
    let layout = checker.build_function_layout(probe, &blocks).unwrap();
    let context = BorrowTransferContext {
        external_package_registry: &packages,
        public_call_summaries: &checker.public_call_summaries,
        imported_call_summaries: &module.imported_call_summaries,
        module_private_call_summaries: &module.module_private_call_summaries,
        generated_call_summaries: &module.generated_call_summaries,
        local_types: &checker.local_types,
        structs: &module.structs,
        choices: &module.choices,
        expressions: &module.expressions,
        type_environment: &types,
        diagnostics: BorrowDiagnostics::new(&module, &paths, &strings),
    };
    let mut state = BorrowState::new_uninitialized(layout.local_count());
    let rows = layout.index_of(probe.params[0]).unwrap();
    state.initialize_parameter(rows);
    let block = checker.block_by_id_or_error(probe.entry, probe.id).unwrap();
    let mut capture_only = state.clone();
    let mut stats = super::BlockTransferStats::default();
    let (statement, target, args, results) = block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            crate::compiler_frontend::hir::statements::HirStatementKind::Call {
                target: target @ crate::compiler_frontend::external_packages::CallTarget::Local(_),
                args,
                results,
            } => Some((statement, target, *args, results)),
            _ => None,
        })
        .expect("the authored fill call");
    super::statement::transfer_immediate_call(
        &mut super::statement::CallTransferContext::new(
            &context,
            super::OperationView::from_layout(&layout),
            &mut capture_only,
            block.id,
            layout.statement_order_or_unknown(statement.id),
            None,
            &mut stats,
            None,
        ),
        target,
        context.expressions.values(args),
        results,
    )
    .expect("capture and publish the append without a validation sink");
    assert_eq!(stats.conflicts_checked, 0);
    assert!(stats.statement_facts.is_empty());
    transfer_block(&context, &layout, block, &mut state).expect("transfer the authored append");
    let (root_ids, _) = state.local_alloc_ids(rows);
    let [holder] = root_ids.as_slice() else {
        panic!("the incoming rows must have one root");
    };
    let original_element = state.alloc_child(*holder, FieldKey::Elem);
    let values = state
        .resolve_alloc_path(rows, &layout.local_mutable, [FieldKey::Elem])
        .ids;
    assert_eq!(
        capture_only
            .resolve_alloc_path(rows, &layout.local_mutable, [FieldKey::Elem])
            .ids,
        values,
    );
    let appended: Vec<_> = values
        .into_iter()
        .filter(|&id| id != original_element)
        .collect();
    assert!(
        !appended.is_empty(),
        "the published store must install its fresh allocations"
    );
    assert!(appended.into_iter().all(|id| !state.alloc_singular(id)));
}

#[test]
fn imported_selector_resolves_without_a_consumer_hir_struct_layout() {
    use crate::compiler_frontend::analysis::borrow_checker::summarize_call_allocations;
    use crate::compiler_frontend::external_packages::CallTarget;
    use crate::compiler_frontend::hir::statements::HirStatementKind;
    use crate::compiler_frontend::semantic_identity::{
        ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity, StablePackageIdentity,
    };

    let source = "\
Box = |items {I32}|\n\
make || -> Box:\n\
    return Box({1})\n\
;\n\
items |box Box| -> {I32}:\n\
    return box.items\n\
;\n\
probe || -> I32:\n\
    return items(make()).length()\n\
;";
    let (ast, mut paths, mut strings) = parse_single_file_ast(source);
    let packages = default_external_package_registry(&mut strings);
    let (mut module, types) = lower_ast(ast, &mut strings, &mut paths, &packages).unwrap();
    let entry = paths
        .try_intern_portable_path("@entry.moth", &mut strings)
        .unwrap();
    let mut function_id = |name| {
        let path = paths.try_intern_child(entry, strings.intern(name)).unwrap();
        module
            .functions
            .iter()
            .find(|function| module.side_table.function_name_path(function.id) == Some(path))
            .expect("authored fixture function")
            .id
    };
    let make = function_id("make");
    let items = function_id("items");
    let box_type = module
        .functions
        .iter()
        .find(|function| function.id == make)
        .unwrap()
        .return_contract
        .ordinary()
        .unwrap()
        .success()[0];
    let mut summaries = summarize_call_allocations(&module, &types, &packages, &paths, &strings)
        .expect("build provider summaries with the donor layout")
        .analysis
        .public_call_summaries;
    let provider = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("unmaterialised-call-selector"),
        "helper".to_owned(),
        ModuleRootRole::Normal,
    );
    for (id, name) in [(make, "make"), (items, "items")] {
        let origin = OriginFunctionId::new_free(provider.clone(), name.to_owned());
        module
            .imported_call_summaries
            .insert(origin.clone(), summaries.remove(&id).unwrap());
        let mut retargeted = 0;
        for statement in module
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
        {
            if let HirStatementKind::Call { target, .. } = &mut statement.kind
                && *target == CallTarget::Local(id)
            {
                *target = CallTarget::CrossModule(origin.clone());
                retargeted += 1;
            }
        }
        assert_eq!(retargeted, 1);
    }
    module
        .functions
        .retain(|function| function.id != make && function.id != items);
    module
        .structs
        .retain(|layout| layout.frontend_type_id != box_type);
    assert!(types.fields_for(box_type).is_some());
    assert!(
        module
            .structs
            .iter()
            .all(|layout| layout.frontend_type_id != box_type)
    );
    run_borrow_checker_with_types(&module, &types, &packages, &paths, &strings)
        .expect("the consumer only transports Box across the calls, using its ordinal selector");
}
