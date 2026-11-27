//! Source and state regressions for writes through the sparse allocation layer.

use super::layer::{LayerAliasWrite, LocalLayerValue};
use super::refresh_alias_referents;
use super::statement::bind_call_result_ids;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocId, BorrowState, FieldKey, LocalState, MutationTarget, RootSet, StoredValue,
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

    // Seed a join whose only legacy write root is WHOLE but whose sparse layer also
    // records a projected alternative. The legacy roots alone cannot prove exactness.
    let mut whole_root = RootSet::empty(LOCAL_COUNT);
    whole_root.insert(WHOLE);
    state.update_local_state(
        WRITER,
        LocalState::alias_with_direct(whole_root.clone(), whole_root),
    );
    state.bind_alloc_ids(WRITER, &[old_whole, old_field], false);
    state.bind_alias_cells(WRITER, &[(holder, field)], false, false);

    // Ordinary call-result transfer refreshes the root facts before publishing sparse IDs.
    // The whole slot has no override: its pre-write allocation is only an implicit self root.
    assert!(!state.has_layer_binding(WHOLE));
    let mut result_roots = RootSet::empty(LOCAL_COUNT);
    result_roots.insert(RESULT);
    let alias = state.local_state(WRITER).clone();
    let (_, retirement) = refresh_alias_referents(
        &mut state,
        WRITER,
        &alias,
        Some(&result_roots),
        0,
        true,
        true,
    );
    assert!(retirement.is_none());

    bind_call_result_ids(&mut state, WRITER, &[result], &[], false);

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
    let observer = state.resolve_local_allocs(OBSERVER);
    let targets: Vec<_> = state
        .local_alloc_ids(WHOLE)
        .0
        .into_iter()
        .map(|id| MutationTarget {
            id,
            layer_reached: true,
            deep: false,
        })
        .collect();
    assert!(
        state.alloc_reaches(&observer.ids, observer.layer_only, &targets),
        "a later mutation of the possible whole referent must still see the old live observer"
    );
}

#[test]
fn write_through_joined_alias_replaces_its_readonly_observation() {
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
    let field = FieldKey::Field(0);
    let old = state.local_alloc_ids(OLD).0[0];
    let new = state.local_alloc_ids(NEW).0[0];
    let cells = [LEFT, RIGHT].map(|slot| (state.local_alloc_ids(slot).0[0], field));
    for &(holder, key) in &cells {
        state.store_alloc_field(
            &[holder],
            key,
            &[StoredValue {
                id: old,
                writable: false,
            }],
            true,
        );
    }
    let mut holders = RootSet::empty(LOCAL_COUNT);
    holders.insert(LEFT);
    holders.insert(RIGHT);
    let alias = LocalState::alias_with_cells(holders.clone(), RootSet::empty(LOCAL_COUNT), holders);
    state.update_local_state(WRITER, alias.clone());
    state.bind_alloc_ids(WRITER, &[old], true);
    state.bind_alias_cells(WRITER, &cells, false, false);

    LayerAliasWrite::Cells {
        cells: cells.to_vec(),
        exact: false,
        stored: vec![StoredValue {
            id: new,
            writable: true,
        }],
        incoming_alias: LocalLayerValue::None,
        views: Vec::new(),
    }
    .apply(&mut state, WRITER, &alias, None);

    let writer = state.resolve_local_allocs(WRITER);
    assert_eq!(writer.ids, [new]);
    assert!(
        !writer.readonly,
        "the old immutable allocation is no longer observed"
    );
    for holder in [LEFT, RIGHT] {
        let possible = state.resolve_alloc_path(holder, &[true; LOCAL_COUNT], [field]);
        assert!(possible.ids.contains(&old));
        assert!(possible.ids.contains(&new));
        assert!(
            possible.readonly,
            "each possible referent may still contain the old value"
        );
    }
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
fn exact_store_replaces_view_but_old_capture_and_weak_store_keep_it() {
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
    let assigned = [StoredValue {
        id: new,
        writable: true,
    }];
    let mut weak = state.clone();
    weak.store_alloc_field(&[root], left, &assigned, false);
    state.store_alloc_field(&[root], left, &assigned, true);
    let left_now = state.resolve_alloc_path(0, &[true; 6], [left]);
    let left_weak = weak.resolve_alloc_path(0, &[true; 6], [left]);
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
    transfer_block(&context, &layout, block, &mut state).expect("transfer the authored append");
    let (root_ids, _) = state.local_alloc_ids(rows);
    let [holder] = root_ids.as_slice() else {
        panic!("the incoming rows must have one root");
    };
    let original_element = state.alloc_child(*holder, FieldKey::Elem);
    let values = state
        .resolve_alloc_path(rows, &layout.local_mutable, [FieldKey::Elem])
        .ids;
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
