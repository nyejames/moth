//! Source and state regressions for writes through the sparse allocation layer.

use super::layer::{LayerAliasWrite, LocalLayerValue};
use super::refresh_alias_referents;
use super::statement::bind_call_result_ids;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    BorrowState, FieldKey, LocalState, MutationTarget, RootSet, StoredValue,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::InvalidMutableAccessView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, InvalidMutableAccessReason};
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
    let draft = check_source(source)
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

    bind_call_result_ids(&mut state, WRITER, &[result], false);

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
