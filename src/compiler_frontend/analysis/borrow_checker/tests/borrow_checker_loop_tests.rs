//! Borrow-checker CFG future-use regression tests.
//!
//! WHAT: protects CFG-carried aliases, projected access actors and independent collection roots.
//! WHY: linear last-use order must defer to CFG future use for source locals without extending
//! compiler-temporary aliases beyond their intended expiry.

use crate::compiler_frontend::analysis::borrow_checker::OptionalTransferStatus;
use crate::compiler_frontend::compiler_messages::BorrowDiagnosticKind;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::borrow_fixture_support::{
    assert_borrow_error_kind, run_borrow_checker,
};
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

fn borrow_check_source(source: &str) {
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("source should pass borrow checking");
}

#[test]
fn collection_loop_mutation_of_iterable_reports_shared_mutable_conflict() {
    let source = r#"
items ~{Int} = {1, 2, 3}
loop items |item|:
    ~items.push(4) catch:
    ;
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("mutating a collection while iterating it should fail");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn collection_loop_mutable_helper_call_on_iterable_reports_shared_mutable_conflict() {
    let source = r#"
mutate |values ~{Int}|:
    ~values.push(4) catch:
    ;
;
items ~{Int} = {1, 2, 3}
loop items |item|:
    mutate(~items)
;
after = items.length()
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("a mutable helper call on the active iterable should fail");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn collection_loop_mutation_through_iterable_alias_reports_shared_mutable_conflict() {
    let source = r#"
items ~{Int} = {1, 2, 3}
alias ~= items
loop items |item|:
    ~alias.push(4) catch:
    ;
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
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
        ~outer.push(7) catch:
        ;
    ;
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
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
        ~inner.push(7) catch:
        ;
    ;
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
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
~items.push(4) catch:
;
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
    ~other.push(6) catch:
    ;
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
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
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
            if args.len() != 1 {
                return None;
            }

            let HirExpressionKind::Load(HirPlace::Local(local)) = &args[0].kind else {
                return None;
            };
            (hir.side_table.resolve_local_name(*local, &string_table) == Some("card"))
                .then_some(args[0].id)
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
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
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
            if args.len() != 1 {
                return None;
            }

            let HirExpressionKind::Load(HirPlace::Local(local)) = &args[0].kind else {
                return None;
            };
            (hir.side_table.resolve_local_name(*local, &string_table) == Some("card"))
                .then_some(args[0].id)
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
    ~items.push(4) catch:
    ;
;
"#,
    );
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
        ~items.push(4) catch:
        ;
    ;
;
value = alias
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("CFG future use through a branch join should preserve the alias conflict");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn projected_assignment_rooted_in_user_local_preserves_source_alias_conflict() {
    let (hir, mut string_table, _) = projected_assignment_branch_fixture();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("a user-local projected mutation should preserve the source alias conflict");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
}

#[test]
fn projected_assignment_rooted_in_compiler_temp_uses_linear_expiry() {
    let (mut hir, mut string_table, point_local) = projected_assignment_branch_fixture();
    hir.side_table
        .bind_local_origin(point_local, HirLocalOriginKind::CompilerTemp, None, None);
    let external_package_registry = default_external_package_registry(&mut string_table);

    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("compiler-temporary projected mutation should use linear expiry");
}

fn projected_assignment_branch_fixture() -> (HirModule, StringTable, LocalId) {
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
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let point_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| hir.side_table.resolve_local_name(local.id, &string_table) == Some("point"))
        .map(|local| local.id)
        .expect("projected assignment root should be a named local");

    (hir, string_table, point_local)
}
