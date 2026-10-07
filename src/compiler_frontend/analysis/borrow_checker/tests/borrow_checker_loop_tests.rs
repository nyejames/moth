//! Borrow-checker CFG future-use regression tests.
//!
//! WHAT: protects CFG-carried aliases, projected access actors and independent collection roots.
//! WHY: linear last-use order must defer to CFG future use for source locals without extending
//! compiler-temporary aliases beyond their intended expiry, and CFG future use must stay a
//! liveness fact so a rebound alias stops blocking mutation once every path redefines it first.

use crate::compiler_frontend::analysis::borrow_checker::OptionalTransferStatus;
use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
use crate::compiler_frontend::compiler_messages::BorrowDiagnosticKind;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::validate_hir_module;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::borrow_fixture_support::{
    assert_borrow_error_kind, run_borrow_checker,
};
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::{expression, lower_hir};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tests::type_id_fixture_support::lower_ast;

fn borrow_check_source(source: &str) {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("source should pass borrow checking");
}

#[test]
fn collection_loop_mutation_of_iterable_reports_shared_mutable_conflict() {
    let source = r#"
items ~{Int} = {1, 2, 3}
loop items |item|:
~items.push(4)
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("mutating a collection while iterating it should fail");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn collection_loop_mutable_helper_call_on_iterable_reports_shared_mutable_conflict() {
    let source = r#"
mutate |values ~{Int}|:
~values.push(4)
;
items ~{Int} = {1, 2, 3}
loop items |item|:
mutate(~items)
;
after = items.length()
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("a mutable helper call on the active iterable should fail");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn collection_loop_mutation_through_iterable_alias_reports_shared_mutable_conflict() {
    let source = r#"
items ~{Int} = {1, 2, 3}
alias ~= items
loop items |item|:
~alias.push(4)
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("mutating an alias of the active iterable should fail");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn nested_collection_loop_mutation_of_outer_iterable_reports_shared_mutable_conflict() {
    let source = r#"
outer ~{Int} = {1, 2, 3}
inner ~{Int} = {4, 5, 6}
loop outer |outer_item|:
loop inner |inner_item|:
    ~outer.push(7)
;
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("the outer iterable must stay protected inside a nested loop");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn nested_collection_loop_mutation_of_inner_iterable_reports_shared_mutable_conflict() {
    let source = r#"
outer ~{Int} = {1, 2, 3}
inner ~{Int} = {4, 5, 6}
loop outer |outer_item|:
loop inner |inner_item|:
    ~inner.push(7)
;
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("the inner iterable must stay protected inside a nested loop");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn collection_loop_mutation_after_exit_is_valid() {
    borrow_check_source(
        r#"
items ~{Int} = {1, 2, 3}
loop items |item|:
;
~items.push(4)
"#,
    );
}

#[test]
fn collection_loop_mutation_of_unrelated_root_is_valid() {
    borrow_check_source(
        r#"
items ~{Int} = {1, 2, 3}
other ~{Int} = {4, 5}
loop items |item|:
~other.push(6)
;
"#,
    );
}

#[test]
fn collection_loop_item_call_borrows_source_while_iterable_carrier_is_live() {
    let source = r#"
render_card |card String| -> String:
return [: [card]]
;

render_listing |cards {String}| -> String:
output ~= [: <section>]
loop cards |card|:
    output = [: [output][render_card(card)]]
;
return [: [output]</section>]
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("an independent output accumulator should not conflict with the iterable");

    let item_argument = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| {
            let HirStatementKind::Call {
                target: CallTarget::Local(_),
                args,
                ..
            } = &statement.kind
            else {
                return None;
            };
            let argument_ids = hir.expressions.values(*args);
            if argument_ids.len() != 1 {
                return None;
            }
            let argument_id = argument_ids[0];
            let argument = hir.expressions.expression(argument_id);

            let HirExpressionKind::Load(place) = &argument.kind else {
                return None;
            };
            if !hir.expressions.projections(place.projections).is_empty() {
                return None;
            }
            let local = place.root;
            (hir.side_table
                .resolve_local_name(local, &path_fork, &string_table)
                == Some("card"))
            .then_some(argument_id)
        })
        .expect("should locate the collection item passed to the user call");

    assert_eq!(
        report
            .analysis
            .value_fact(item_argument)
            .expect("collection item call argument should have a borrow fact")
            .optional_transfer,
        OptionalTransferStatus::Borrow,
        "the source item must remain borrowed while the hidden iterable carrier has future uses"
    );
}

#[test]
fn projected_collection_loop_item_call_borrows_source_while_carrier_is_live() {
    let source = r#"
Listing = |
cards {String},
|

render_card |card String| -> String:
return [: [card]]
;

render_listing |listing Listing| -> String:
output ~= [: <section>]
loop listing.cards |card|:
    output = [: [output][render_card(card)]]
;
return [: [output]</section>]
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a projected collection source should keep its carrier root live");

    let item_argument = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| {
            let HirStatementKind::Call {
                target: CallTarget::Local(_),
                args,
                ..
            } = &statement.kind
            else {
                return None;
            };
            let argument_ids = hir.expressions.values(*args);
            if argument_ids.len() != 1 {
                return None;
            }
            let argument_id = argument_ids[0];
            let argument = hir.expressions.expression(argument_id);

            let HirExpressionKind::Load(place) = &argument.kind else {
                return None;
            };
            if !hir.expressions.projections(place.projections).is_empty() {
                return None;
            }
            let local = place.root;
            (hir.side_table
                .resolve_local_name(local, &path_fork, &string_table)
                == Some("card"))
            .then_some(argument_id)
        })
        .expect("should locate the projected collection item passed to the user call");

    assert_eq!(
        report
            .analysis
            .value_fact(item_argument)
            .expect("projected collection item call should have a borrow fact")
            .optional_transfer,
        OptionalTransferStatus::Borrow,
        "the projected source must remain borrowed while the hidden carrier has future uses"
    );
}

#[test]
fn collection_loop_mutation_of_source_copy_is_valid() {
    borrow_check_source(
        r#"
items ~{Int} = {1, 2, 3}
copied = copy items
loop copied |item|:
~items.push(4)
;
"#,
    );
}

#[test]
fn loop_body_rebinding_its_alias_allows_source_mutation() {
    // The map alias is redefined at the top of every iteration, so the value that reaches the
    // mutation is dead on every path, including the one that arrives over the back-edge.
    borrow_check_source(
        r#"
accumulate |keys {String}| -> {String = Int}, Error!:
totals ~{String = Int} = {}
loop keys |key|:
    current = totals.get(key) catch then 0
    next = current + 1
    ~totals.set(key, next)!
;
return totals
;
"#,
    );
}

#[test]
fn loop_definition_replaces_a_carried_alias_with_fresh_slot() {
    let source = r#"probe || -> Int:
left ~{Int} = {1}
writer ~= left
again ~= true
loop again:
writer = left
sentinel ~= 0
again = false
;
return 0
;"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (mut hir, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("fixture source should lower");
    let registry = default_external_package_registry(&mut string_table);
    let local_named = |name| {
        hir.blocks
            .iter()
            .flat_map(|block| &block.locals)
            .find(|local| {
                hir.side_table
                    .resolve_local_name(local.id, &path_fork, &string_table)
                    == Some(name)
            })
            .map(|local| local.id)
            .unwrap_or_else(|| panic!("fixture local {name} should lower"))
    };
    let left = local_named("left");
    let writer = local_named("writer");

    // This fixture exercises a valid HIR re-entry that source scoping does not spell: retain an
    // outer binding across the loop, then execute a typed Define on its back-edge. The source
    // assignments identify the locals and CFG; changing the update tag makes the hidden
    // definition-over-live-alias invariant explicit without introducing a test-only transfer.
    let (body_block, definition_index, definition_id, sentinel_id) = hir
        .blocks
        .iter()
        .find_map(|block| {
            let definition = block.statements.iter().enumerate().find(|(_, statement)| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::AssignPlace(place),
                        ..
                    } if place.root == writer && place.projections.is_empty()
                )
            });
            let sentinel = block.statements.iter().find(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(local),
                        ..
                    } if hir.side_table.resolve_local_name(*local, &path_fork, &string_table)
                        == Some("sentinel")
                )
            });
            match (definition, sentinel) {
                (Some((index, definition)), Some(sentinel)) => {
                    Some((block.id, index, definition.id, sentinel.id))
                }
                _ => None,
            }
        })
        .expect("loop body should contain the writer update and following sentinel");
    let (left_type, body_region) = {
        let left_local = hir
            .blocks
            .iter()
            .flat_map(|block| &block.locals)
            .find(|local| local.id == left)
            .expect("left local should remain declared");
        let body = &hir.blocks[body_block.0 as usize];
        (left_local.ty, body.region)
    };
    // Keep the new value independent of the old referent: reading `left` here would be a real
    // shared-alias conflict on later iterations, separate from the Define-over-alias invariant.
    let rvalue = expression(
        HirExpressionKind::Collection(HirValueRange::empty()),
        left_type,
        body_region,
        ValueKind::RValue,
        &mut hir.expressions,
    );
    let definition = &mut hir.blocks[body_block.0 as usize].statements[definition_index];
    let HirStatementKind::Write { target, value } = &mut definition.kind else {
        unreachable!("the selected loop statement is a write")
    };
    *target = HirWriteTarget::DefineLocal(writer);
    *value = rvalue;

    validate_hir_module(&hir, &type_environment)
        .expect("loop re-entry definition should preserve HIR invariants");
    let report = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
        .expect("re-entry should retire the stale alias before the new value is installed");

    let definition_entry = report
        .analysis
        .statement_entry_states
        .get(&definition_id)
        .expect("the loop definition should have an entry snapshot");
    let carried_writer = definition_entry
        .locals
        .iter()
        .find(|state| state.local == writer)
        .expect("the definition entry should include the carried writer");
    assert!(
        carried_writer.mode.contains(LocalMode::ALIAS),
        "the fixed-point loop entry must carry the previous alias into the Define"
    );
    assert!(carried_writer.alias_roots.contains(&left));

    let after_definition = report
        .analysis
        .statement_entry_states
        .get(&sentinel_id)
        .expect("the sentinel should snapshot state immediately after the Define");
    let rebound_writer = after_definition
        .locals
        .iter()
        .find(|state| state.local == writer)
        .expect("the sentinel should include the newly defined writer");
    assert_eq!(rebound_writer.mode, LocalMode::SLOT);
    assert!(
        rebound_writer.alias_roots.is_empty(),
        "the fresh definition must discard the carried alias roots"
    );
    assert!(
        !rebound_writer.alias_roots.contains(&left),
        "the old alias root must not survive the new value-backed definition"
    );
}

#[test]
fn loop_body_alias_live_across_back_edge_blocks_source_mutation() {
    // Same back-edge, but the alias is issued once before the loop and read again on every later
    // iteration. Killing a redefined alias must not also release a genuinely live one.
    let source = r#"
items ~{String} = {"a"}
shared = items
loop 0 to 2 |round|:
first = shared.get(0) catch then "missing"
~items.push(first)
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("an alias read on every iteration should block mutating its source");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn branch_join_future_use_preserves_alias_conflict_after_linear_expiry() {
    let source = r#"
items ~{Int} = {1, 2, 3}
alias = items
outer ~= true
inner ~= true
if outer:
branch_marker = 0
else
if inner:
    inner_marker = 0
else
    ~items.push(4)
;
;
value = alias
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("CFG future use through a branch join should preserve the alias conflict");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn projected_assignment_rooted_in_user_local_preserves_source_alias_conflict() {
    let (hir, mut string_table, _, path_fork) = projected_assignment_branch_fixture();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("a user-local projected mutation should preserve the source alias conflict");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn projected_assignment_rooted_in_compiler_temp_uses_linear_expiry() {
    let (mut hir, mut string_table, point_local, path_fork) = projected_assignment_branch_fixture();
    hir.side_table
        .bind_local_origin(point_local, HirLocalOriginKind::CompilerTemp, None, None);
    let external_package_registry = default_external_package_registry(&mut string_table);

    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("compiler-temporary projected mutation should use linear expiry");
}

fn projected_assignment_branch_fixture() -> (HirModule, StringTable, LocalId, PathInternerFork) {
    let source = r#"
Point = |
    value Int,
|
point ~= Point(1)
alias = point
outer ~= true
inner ~= true
if outer:
    branch_marker = 0
else
    if inner:
        inner_marker = 0
    else
        point.value = 2
    ;
;
value = alias.value
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let point_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| {
            hir.side_table
                .resolve_local_name(local.id, &path_fork, &string_table)
                == Some("point")
        })
        .map(|local| local.id)
        .expect("projected assignment root should be a named local");

    (hir, string_table, point_local, path_fork)
}
