//! Borrow-checker fact-generation regression tests.
//!
//! WHAT: checks the low-level facts emitted for borrows, optional transfers, assignments, and returns.
//! WHY: these facts are the borrow checker's source of truth, so targeted tests catch drift
//! before it reaches higher-level diagnostics.

use crate::compiler_frontend::analysis::borrow_checker::{LocalMode, OptionalTransferStatus};
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, FallibleExpressionHandling, HandledFallibleHostFunctionCallInput,
};
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAbiType, ExternalFunctionDef, ExternalFunctionLowerings, ExternalParameter,
    ExternalReturnSlot, ExternalSignatureType,
};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, HirMapOp};
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, function_node, make_test_variable, node, param, reference_expr, symbol,
    test_location,
};
use crate::compiler_frontend::tests::borrow_fixture_support::run_borrow_checker;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::{build_ast, entry_and_start, lower_hir};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashSet;
use std::collections::VecDeque;
use std::sync::Arc;

#[test]
fn statement_terminator_and_value_facts_are_populated() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let x = symbol("x", &mut string_table);
    let y = symbol("y", &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_location(1), ValueMode::MutableOwned),
                )),
                test_location(1),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y.clone(),
                    Expression::int(0, test_location(2), ValueMode::ImmutableOwned),
                )),
                test_location(2),
            ),
            node(
                NodeKind::If(
                    Expression::bool(true, test_location(3), ValueMode::ImmutableOwned),
                    vec![node(
                        NodeKind::Assignment {
                            target: assignment_target(
                                x.clone(),
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_location(4),
                            ),
                            value: Expression::int(2, test_location(4), ValueMode::ImmutableOwned),
                        },
                        test_location(4),
                    )],
                    Some(vec![node(
                        NodeKind::Assignment {
                            target: assignment_target(
                                x.clone(),
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_location(5),
                            ),
                            value: Expression::int(3, test_location(5), ValueMode::ImmutableOwned),
                        },
                        test_location(5),
                    )]),
                ),
                test_location(3),
            ),
        ],
        test_location(1),
    );

    let hir = lower_hir(build_ast(vec![start_fn], entry_path), &mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("borrow checking should succeed");

    let start = &hir.functions[hir
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let reachable = collect_reachable_blocks(&hir, start.entry);

    for block_id in &reachable {
        let block = &hir.blocks[block_id.0 as usize];
        assert!(
            report.analysis.terminator_fact(*block_id).is_some(),
            "missing terminator fact for block {block_id:?}"
        );

        for statement in &block.statements {
            assert!(
                report.analysis.statement_fact(statement.id).is_some(),
                "missing statement fact for statement {:?}",
                statement.id
            );
            assert!(
                hir.side_table
                    .hir_source_location_for_hir(HirLocation::Statement(statement.id))
                    .is_some(),
                "statement {:?} should have source mapping",
                statement.id
            );
        }
    }

    let mut value_ids = FxHashSet::default();
    for block_id in &reachable {
        let block = &hir.blocks[block_id.0 as usize];
        for statement in &block.statements {
            collect_statement_values(statement.kind.clone(), &mut value_ids);
        }
        collect_terminator_values(&block.terminator, &mut value_ids);
    }

    for value_id in value_ids {
        assert!(
            report.analysis.value_fact(value_id).is_some(),
            "missing value fact for value {value_id:?}"
        );
        assert!(
            hir.side_table.value_source_location(value_id).is_some(),
            "value {value_id:?} should have side-table source mapping"
        );
    }
}

#[test]
fn drop_statement_produces_statement_fact() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let value = symbol("value", &mut string_table);
    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                value,
                Expression::int(1, test_location(1), ValueMode::MutableOwned),
            )),
            test_location(1),
        )],
        test_location(1),
    );

    let mut hir = lower_hir(build_ast(vec![start_fn], entry_path), &mut string_table);
    let start = &hir.functions[hir
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &mut hir.blocks[start.entry.0 as usize];
    let drop_local = entry_block
        .locals
        .first()
        .expect("entry block should contain at least one local")
        .id;

    let next_statement_id = entry_block
        .statements
        .iter()
        .map(|statement| statement.id.0)
        .max()
        .unwrap_or(0)
        + 1;

    entry_block.statements.push(HirStatement {
        id: HirNodeId(next_statement_id),
        kind: HirStatementKind::Drop(drop_local),
        location: test_location(2),
    });

    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("borrow checking should succeed");

    let fact = report
        .analysis
        .statement_fact(HirNodeId(next_statement_id))
        .expect("drop statement should have a statement fact");
    assert!(fact.shared_roots.is_empty());
    assert!(fact.mutable_roots.is_empty());
}

#[test]
fn statement_entry_state_reflects_last_use_reborrow_window() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let data = symbol("data", &mut string_table);
    let first_ref = symbol("first_ref", &mut string_table);
    let sink = symbol("sink", &mut string_table);
    let second_ref = symbol("second_ref", &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    data.clone(),
                    Expression::int(7, test_location(1), ValueMode::MutableOwned),
                )),
                test_location(1),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    first_ref.clone(),
                    Expression::reference(
                        data.clone(),
                        DataType::Int,
                        test_location(2),
                        ValueMode::MutableReference,
                    ),
                )),
                test_location(2),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    sink,
                    reference_expr(
                        first_ref,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_location(3),
                    ),
                )),
                test_location(3),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    second_ref,
                    Expression::reference(
                        data,
                        DataType::Int,
                        test_location(4),
                        ValueMode::MutableReference,
                    ),
                )),
                test_location(4),
            ),
        ],
        test_location(1),
    );

    let hir = lower_hir(build_ast(vec![start_fn], entry_path), &mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("reborrow after last-use should pass");

    let second_statement_id = find_statement_id_for_line(&hir, 4)
        .expect("should locate the reborrow statement by source line");
    let data_local = find_assigned_local_for_line(&hir, 1)
        .expect("should locate the source local by declaration line");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&second_statement_id)
        .expect("reborrow statement should have an entry snapshot");
    let data_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == data_local)
        .expect("entry snapshot should include the data local");

    assert!(
        data_snapshot.alias_roots.is_empty(),
        "data local should not retain live alias roots at the reborrow point"
    );
}

#[test]
fn optional_assignment_transfer_keeps_source_state_and_records_advisory_fact() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let source = symbol("source", &mut string_table);
    let target = symbol("target", &mut string_table);
    let sentinel = symbol("sentinel", &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    source.clone(),
                    Expression::int(7, test_location(10), ValueMode::MutableOwned),
                )),
                test_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    target,
                    Expression::reference(
                        source,
                        DataType::Int,
                        test_location(11),
                        ValueMode::MutableOwned,
                    ),
                )),
                test_location(11),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    sentinel,
                    Expression::int(0, test_location(12), ValueMode::ImmutableOwned),
                )),
                test_location(12),
            ),
        ],
        test_location(2),
    );

    let hir = lower_hir(build_ast(vec![start_fn], entry_path), &mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("inferred assignment transfer should pass");

    let source_local = find_assigned_local_for_line(&hir, 10)
        .expect("should locate the source local by declaration line");
    let target_local = find_assigned_local_for_line(&hir, 11)
        .expect("should locate the target local by declaration line");
    let sentinel_statement_id =
        find_statement_id_for_line(&hir, 12).expect("should locate the sentinel statement");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&sentinel_statement_id)
        .expect("sentinel statement should have an entry snapshot");
    let source_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == source_local)
        .expect("entry snapshot should include the source local");
    let target_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == target_local)
        .expect("entry snapshot should include the target local");

    assert!(
        source_snapshot.mode.contains(LocalMode::SLOT),
        "optional transfer must keep the source initialized in mandatory state, got source mode {:?} with aliases {:?}; target mode {:?} with aliases {:?}",
        source_snapshot.mode,
        source_snapshot.alias_roots,
        target_snapshot.mode,
        target_snapshot.alias_roots
    );
    assert!(
        target_snapshot.mode.contains(LocalMode::ALIAS),
        "borrow fallback should keep the target rooted in the source, got mode {:?} with aliases {:?}",
        target_snapshot.mode,
        target_snapshot.alias_roots
    );
    assert!(
        target_snapshot.alias_roots.contains(&source_local),
        "borrow fallback should retain the source root on the target"
    );

    let assignment_value = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| {
            if let HirStatementKind::Assign { target, value } = &statement.kind
                && matches!(target, HirPlace::Local(local) if *local == target_local)
            {
                Some(value.id)
            } else {
                None
            }
        })
        .expect("should locate the optional assignment value");
    assert_eq!(
        report
            .analysis
            .value_fact(assignment_value)
            .expect("assignment value should have a borrow fact")
            .optional_transfer,
        OptionalTransferStatus::Transfer
    );
}

// WHAT: hidden map-operation transfer facts that integration output cannot inspect.
// WHY: Phase 6 integration owns user-visible map borrow behavior; these narrow state
//      assertions protect the receiver-alias shape, MayConsume last-use classification,
//      and recursive aggregate-literal advisory transfer facts.

#[test]
fn map_get_operation_result_alias_retains_receiver_root() {
    // WHAT: the first-class HIR map-operation result aliases the receiver root before catch
    //      handling transfers the success value.
    // WHY: later conflict analysis reads this alias state; integration only sees the
    //      resulting conflict, not which root the get binding aliases.
    let source = r#"scores ~{String = Int} = {"Priya" = 10}
score = scores.get("Priya") catch:
    then 0
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("a get with no later mutation should pass");

    let scores_local = find_local_by_name(&hir, &string_table, "scores")
        .expect("should locate the receiver local by name");
    let (result_local, following_statement) =
        find_map_op_result_and_following_statement(&hir, HirMapOp::Get)
            .expect("should locate the get operation result and its consumer");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&following_statement)
        .expect("the operation-result consumer should have an entry snapshot");
    let result_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == result_local)
        .expect("entry snapshot should include the map-operation result");

    assert!(
        result_snapshot.mode.contains(LocalMode::ALIAS),
        "get result should alias the receiver, got mode {:?}",
        result_snapshot.mode
    );
    assert!(
        result_snapshot.alias_roots.contains(&scores_local),
        "get result alias root should be the receiver, got {:?}",
        result_snapshot.alias_roots
    );
}

#[test]
fn map_remove_result_is_fresh_owned() {
    // WHAT: the binding produced by fallible map `remove` is a fresh owned slot with no
    //      receiver alias root, unlike `get`.
    // WHY: the Fresh result-alias decision is a hidden transfer fact; if remove aliased
    //      the receiver, a later mutation would falsely conflict with the removed value.
    let source = r#"scores ~{String = String} = {"Priya" = "ten"}
removed = ~scores.remove("Priya") catch:
    then ""
;
sentinel = 0"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("a remove with no later mutation should pass");

    let removed_local = find_local_by_name(&hir, &string_table, "removed")
        .expect("should locate the remove binding by name");
    let sentinel_statement =
        find_assign_statement_id_for_local_name(&hir, &string_table, "sentinel")
            .expect("should locate the sentinel statement by its assigned local");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&sentinel_statement)
        .expect("sentinel statement should have an entry snapshot");
    let result_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == removed_local)
        .expect("entry snapshot should include the remove binding");

    assert!(
        result_snapshot.mode.contains(LocalMode::SLOT),
        "remove result should own a fresh slot, got mode {:?}",
        result_snapshot.mode
    );
    assert!(
        !result_snapshot.mode.contains(LocalMode::ALIAS),
        "remove result should not alias the receiver, got mode {:?} with aliases {:?}",
        result_snapshot.mode,
        result_snapshot.alias_roots
    );
    assert!(
        result_snapshot.alias_roots.is_empty(),
        "remove result should carry no alias roots, got {:?}",
        result_snapshot.alias_roots
    );
}

#[test]
fn map_set_final_use_records_advisory_transfer_without_invalidating_roots() {
    // WHAT: `set` MayConsumeShared on final-use non-copy key and value inputs records transfer advice.
    // WHY: optional destruction responsibility must not rewrite mandatory source state.
    let source = r#"scores ~{String = String} = {}
key ~= "key"
value ~= "hello"
~scores.set(key, value) catch:
;
sentinel = 0"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("a final-use set with no later value use should pass");

    let set_statement_id = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::MapOp {
                    op: HirMapOp::Set,
                    ..
                }
            )
            .then_some(statement.id)
        })
        .expect("should locate the final-use set operation");
    let set_fact = report
        .analysis
        .statement_fact(set_statement_id)
        .expect("set operation should have a statement fact");
    assert_eq!(
        set_fact.conflicts_checked, 3,
        "the isolated transfer probe must not increment the caller conflict count"
    );
    assert_eq!(
        set_fact.mutable_roots.len(),
        3,
        "the isolated transfer probe must not duplicate caller access roots"
    );
    assert_eq!(
        report.analysis.statement_facts.len(),
        hir.blocks
            .iter()
            .map(|block| block.statements.len())
            .sum::<usize>(),
        "the isolated transfer probe must not duplicate statement facts"
    );

    let sentinel_statement =
        find_assign_statement_id_for_local_name(&hir, &string_table, "sentinel")
            .expect("should locate the sentinel statement by its assigned local");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&sentinel_statement)
        .expect("sentinel statement should have an entry snapshot");
    for name in ["key", "value"] {
        let local = find_local_by_name(&hir, &string_table, name)
            .unwrap_or_else(|| panic!("should locate the inserted {name} local by name"));
        let snapshot = entry_state
            .locals
            .iter()
            .find(|snapshot| snapshot.local == local)
            .unwrap_or_else(|| panic!("entry snapshot should include the inserted {name} local"));

        assert!(
            snapshot.mode.contains(LocalMode::SLOT),
            "final-use set should keep the inserted {name} root initialized, got mode {:?} with aliases {:?}",
            snapshot.mode,
            snapshot.alias_roots
        );

        assert!(
            report.analysis.value_facts.values().any(|fact| {
                fact.optional_transfer == OptionalTransferStatus::Transfer
                    && fact.roots.contains(&local)
            }),
            "final-use set should record advisory transfer for {name}"
        );
    }
}

#[test]
fn map_set_later_use_keeps_mutable_inputs_borrowed() {
    // WHAT: `set` MayConsumeShared on later-use key and value inputs borrows rather than moving.
    // WHY: last-use classification must not unconditionally move; the root stays live so
    //      the binding remains usable, which a regression to always-move would break.
    let source = r#"scores ~{String = String} = {}
key ~= "key"
value ~= "hello"
~scores.set(key, value) catch:
;
key_label = key
label = value
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("a later-use mutable set should borrow and keep the value usable");

    let first_use_statement =
        find_assign_statement_id_for_local_name(&hir, &string_table, "key_label")
            .expect("should locate the first later-use statement by its assigned local");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&first_use_statement)
        .expect("first later-use statement should have an entry snapshot");

    for name in ["key", "value"] {
        let local = find_local_by_name(&hir, &string_table, name)
            .unwrap_or_else(|| panic!("should locate the inserted {name} local by name"));
        let snapshot = entry_state
            .locals
            .iter()
            .find(|snapshot| snapshot.local == local)
            .unwrap_or_else(|| panic!("entry snapshot should include the inserted {name} local"));

        assert!(
            snapshot.mode.contains(LocalMode::SLOT),
            "later-use set should keep the {name} as a live slot, got mode {:?}",
            snapshot.mode
        );
        assert!(
            !snapshot.mode.is_definitely_uninit(),
            "later-use set should not move the {name} root, got mode {:?} with aliases {:?}",
            snapshot.mode,
            snapshot.alias_roots
        );
        assert!(
            report.analysis.value_facts.values().any(|fact| {
                fact.optional_transfer == OptionalTransferStatus::Borrow
                    && fact.roots.contains(&local)
            }),
            "later-use set should record advisory borrow fallback for {name}"
        );
    }
}

#[test]
fn later_use_nested_map_literal_records_borrow_without_invalidating_root() {
    let source = r#"value ~= "hello"
scores ~{String = {String = String}} = {"outer" = {"inner" = value}}
label = value
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("a later-use nested literal should retain shared storage");

    let value_local = find_local_by_name(&hir, &string_table, "value")
        .expect("should locate the inner inserted value local by name");
    assert!(
        report.analysis.value_facts.values().any(|fact| {
            fact.optional_transfer == OptionalTransferStatus::Borrow
                && fact.roots.contains(&value_local)
        }),
        "later-use nested literal should record advisory borrow fallback"
    );

    let label_statement = find_assign_statement_id_for_local_name(&hir, &string_table, "label")
        .expect("should locate the later value use");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&label_statement)
        .expect("later value use should have an entry snapshot");
    let value_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == value_local)
        .expect("entry snapshot should include the inserted value local");
    assert!(
        value_snapshot.mode.contains(LocalMode::SLOT),
        "later-use nested literal should keep the source initialized, got mode {:?}",
        value_snapshot.mode
    );
}

#[test]
fn nested_map_literal_records_inner_transfer_without_invalidating_root() {
    // WHAT: a nested map literal recursively records transfer advice for its inner value.
    // WHY: aggregate analysis must recurse while leaving mandatory source state intact.
    let source = r#"value ~= "hello"
scores ~{String = {String = String}} = {"outer" = {"inner" = value}}
sentinel = 0"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("a final-use nested literal with no later value use should pass");

    let value_local = find_local_by_name(&hir, &string_table, "value")
        .expect("should locate the inner inserted value local by name");
    let sentinel_statement =
        find_assign_statement_id_for_local_name(&hir, &string_table, "sentinel")
            .expect("should locate the sentinel statement by its assigned local");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&sentinel_statement)
        .expect("sentinel statement should have an entry snapshot");
    let value_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == value_local)
        .expect("entry snapshot should include the inner inserted value local");

    assert!(
        value_snapshot.mode.contains(LocalMode::SLOT),
        "nested literal should keep the inner inserted value initialized, got mode {:?} with aliases {:?}",
        value_snapshot.mode,
        value_snapshot.alias_roots
    );
    assert!(
        report.analysis.value_facts.values().any(|fact| {
            fact.optional_transfer == OptionalTransferStatus::Transfer
                && fact.roots.contains(&value_local)
        }),
        "nested literal should record advisory transfer for the inner value"
    );
}

#[test]
fn retained_alias_result_borrows_named_final_use_argument() {
    let source = r#"alias |input String| -> String:
    return input
;
value ~= "hello"
result = alias(value)
sentinel = 0
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("a retained aliased result should borrow its final-use argument");

    let value_local = find_local_by_name(&hir, &string_table, "value")
        .expect("should locate the aliased argument local");
    let result_local = find_local_by_name(&hir, &string_table, "result")
        .expect("should locate the retained result local");
    let sentinel_statement =
        find_assign_statement_id_for_local_name(&hir, &string_table, "sentinel")
            .expect("should locate the sentinel statement");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&sentinel_statement)
        .expect("sentinel statement should have an entry snapshot");
    let value_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == value_local)
        .expect("entry snapshot should include the aliased argument");
    let result_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == result_local)
        .expect("entry snapshot should include the retained result");

    assert!(
        !value_snapshot.mode.is_definitely_uninit(),
        "a retained alias result must not move its source root, got mode {:?}",
        value_snapshot.mode
    );
    assert!(
        result_snapshot.mode.contains(LocalMode::ALIAS),
        "retained alias result should remain rooted in its argument, got mode {:?}",
        result_snapshot.mode
    );
    assert!(
        result_snapshot.alias_roots.contains(&value_local),
        "retained alias result should retain the named argument root, got {:?}",
        result_snapshot.alias_roots
    );
}

#[test]
fn transparent_fallible_success_projection_preserves_retained_alias_root() {
    // WHAT: a fallible success projection passed to an alias-retaining call is one direct
    //      place access, and the returned alias must retain that place's root.
    // WHY: optional transfer first records argument roots before deciding whether the call
    //      borrows or receives optional transfer responsibility; treating the unwrap as an aggregate
    //      creates a self-conflict and
    //      can lose the root needed by the retained-result state.
    let source = r#"User = |
    score Int,
|

identity |value User| -> User:
    return value
;

load_user || -> User, Error!:
    return! Error("missing user")
;

compute || -> User, Error!:
    return identity(load_user()!)
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("transparent fallible success projection should not self-conflict");
    let identity_name = symbol("identity", &mut string_table);

    let identity_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == identity_name.name())
        })
        .expect("should locate the alias-retaining function")
        .id;
    let (compute_block, call_statement_id, argument_root, result_local) = hir
        .blocks
        .iter()
        .find_map(|block| {
            block.statements.iter().find_map(|statement| {
                let HirStatementKind::Call {
                    target: CallTarget::Local(target),
                    args,
                    result: Some(result),
                } = &statement.kind
                else {
                    return None;
                };
                if *target != identity_id || args.len() != 1 {
                    return None;
                }
                let HirExpressionKind::FallibleUnwrapSuccess { result: payload } = &args[0].kind
                else {
                    return None;
                };
                let HirExpressionKind::Load(HirPlace::Local(root)) = &payload.kind else {
                    return None;
                };
                Some((block.id, statement.id, *root, *result))
            })
        })
        .expect("should locate the identity call with a transparent fallible projection");

    let fact = report
        .analysis
        .statement_fact(call_statement_id)
        .expect("identity call should have a statement fact");
    assert!(
        fact.shared_roots.contains(&argument_root),
        "alias-retaining optional transfer should record the direct root as shared, got {:?}",
        fact.shared_roots
    );
    assert!(
        !fact.mutable_roots.contains(&argument_root),
        "alias-retaining optional transfer must not move the direct root, got {:?}",
        fact.mutable_roots
    );

    let exit_state = report
        .analysis
        .block_exit_states
        .get(&compute_block)
        .expect("compute block should have an exit snapshot");
    let result_snapshot = exit_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == result_local)
        .expect("exit snapshot should include the retained identity result");
    assert!(
        result_snapshot.mode.contains(LocalMode::ALIAS),
        "identity result should retain alias mode, got {:?}",
        result_snapshot.mode
    );
    assert!(
        result_snapshot.alias_roots.contains(&argument_root),
        "identity result should retain the projected root, got {:?}",
        result_snapshot.alias_roots
    );
}

#[test]
fn retained_unknown_result_borrows_possible_final_use_argument() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let external_id = Arc::make_mut(&mut external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "unknown_external".to_owned(),
            parameters: vec![ExternalParameter {
                language_type: ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
                access_kind:
                    crate::compiler_frontend::external_packages::ExternalAccessKind::Shared,
            }],
            returns: vec![
                ExternalReturnSlot::fresh(ExternalAbiType::Utf8Str),
                ExternalReturnSlot::fresh(ExternalAbiType::Utf8Str),
            ],
            error_return_type: Some(ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)),
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("unknown external fixture registration should succeed");

    let unknown_name = symbol("unknown", &mut string_table);
    let input_name = symbol("input", &mut string_table);
    let argument_name = symbol("argument", &mut string_table);
    let result_name = symbol("result", &mut string_table);
    let caller_name = symbol("caller", &mut string_table);
    let sentinel_name = symbol("sentinel", &mut string_table);
    let mut expression_types = TypeEnvironment::new();
    let external_call = Expression::handled_fallible_host_function_call_with_typed_arguments(
        HandledFallibleHostFunctionCallInput {
            id: external_id,
            args: vec![CallArgument::positional(
                reference_expr(
                    input_name.clone(),
                    DataType::StringSlice,
                    builtin_type_ids::STRING,
                    test_location(2),
                ),
                CallAccessMode::Shared,
                test_location(2),
            )],
            result_type_ids: vec![builtin_type_ids::STRING, builtin_type_ids::STRING],
            error_type_id: builtin_type_ids::STRING,
            handling: FallibleExpressionHandling::Propagate,
            location: test_location(2),
        },
        &mut expression_types,
    );
    let unknown = function_node(
        unknown_name.clone(),
        FunctionSignature {
            parameters: vec![param(
                input_name,
                DataType::StringSlice,
                builtin_type_ids::STRING,
                false,
                test_location(1),
            )],
            returns: vec![
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Error,
                },
            ],
        },
        vec![node(
            NodeKind::Return(vec![external_call]),
            test_location(2),
        )],
        test_location(1),
    );
    let caller = function_node(
        caller_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Error,
                },
            ],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    argument_name.clone(),
                    Expression::string_slice(
                        string_table.intern("hello"),
                        test_location(5),
                        ValueMode::MutableOwned,
                    ),
                )),
                test_location(5),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    result_name.clone(),
                    Expression::handled_fallible_function_call_with_typed_arguments(
                        unknown_name.clone(),
                        vec![CallArgument::positional(
                            reference_expr(
                                argument_name,
                                DataType::StringSlice,
                                builtin_type_ids::STRING,
                                test_location(6),
                            ),
                            CallAccessMode::Shared,
                            test_location(6),
                        )],
                        vec![builtin_type_ids::STRING, builtin_type_ids::STRING],
                        FallibleExpressionHandling::Propagate,
                        &mut expression_types,
                        test_location(6),
                    ),
                )),
                test_location(6),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    sentinel_name,
                    Expression::int(0, test_location(7), ValueMode::ImmutableOwned),
                )),
                test_location(7),
            ),
            node(
                NodeKind::Return(vec![
                    Expression::string_slice(
                        string_table.intern("done"),
                        test_location(8),
                        ValueMode::ImmutableOwned,
                    ),
                    Expression::string_slice(
                        string_table.intern("done"),
                        test_location(8),
                        ValueMode::ImmutableOwned,
                    ),
                ]),
                test_location(8),
            ),
        ],
        test_location(5),
    );
    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(9),
    );
    let hir = lower_hir(
        build_ast(vec![unknown, caller, start], entry_path),
        &mut string_table,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("a retained unknown result should borrow a possible aliased argument");

    let unknown_function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == unknown_name.name())
        })
        .expect("should locate the unknown-return function")
        .id;
    assert_eq!(
        report
            .analysis
            .public_call_summaries
            .get(&unknown_function_id)
            .expect("unknown-return function should have a call summary")
            .return_alias,
        FunctionReturnAliasSummary::Unknown
    );

    let argument_local = find_local_by_name(&hir, &string_table, "argument")
        .expect("should locate the possible aliased argument local");
    let result_assignment = find_assign_statement_id_for_local_name(&hir, &string_table, "result")
        .expect("should locate the retained result assignment");
    let caller_function = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == caller_name.name())
        })
        .expect("should locate the caller function");
    let caller_block = &hir.blocks[caller_function.entry.0 as usize];
    let call_result_local = caller_block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Call {
                result: Some(result),
                ..
            } => Some(*result),
            _ => None,
        })
        .expect("caller should retain the call result before assignment");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&result_assignment)
        .expect("result assignment should have an entry snapshot");
    let argument_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == argument_local)
        .expect("result assignment entry should include the possible aliased argument");
    let result_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == call_result_local)
        .expect("result assignment entry should include the retained call result");

    assert!(
        !argument_snapshot.mode.is_definitely_uninit(),
        "a retained unknown result must not move a possible source root, got mode {:?}",
        argument_snapshot.mode
    );
    assert!(
        result_snapshot.mode.contains(LocalMode::ALIAS),
        "retained unknown result should retain possible alias roots, got mode {:?}",
        result_snapshot.mode
    );
    assert!(
        result_snapshot.alias_roots.contains(&argument_local),
        "retained unknown result should retain the possible argument root, got {:?}",
        result_snapshot.alias_roots
    );
}

fn find_statement_id_for_line(
    hir: &crate::compiler_frontend::hir::module::HirModule,
    line: i32,
) -> Option<HirNodeId> {
    for block in &hir.blocks {
        for statement in &block.statements {
            let Some(source) = hir
                .side_table
                .hir_source_location_for_hir(HirLocation::Statement(statement.id))
            else {
                continue;
            };
            if source.start_pos.line_number == line {
                return Some(statement.id);
            }
        }
    }
    None
}

fn find_assigned_local_for_line(
    hir: &crate::compiler_frontend::hir::module::HirModule,
    line: i32,
) -> Option<crate::compiler_frontend::hir::ids::LocalId> {
    for block in &hir.blocks {
        for statement in &block.statements {
            let Some(source) = hir
                .side_table
                .hir_source_location_for_hir(HirLocation::Statement(statement.id))
            else {
                continue;
            };
            if source.start_pos.line_number != line {
                continue;
            }
            if let HirStatementKind::Assign {
                target: HirPlace::Local(local),
                ..
            } = &statement.kind
            {
                return Some(*local);
            }
        }
    }
    None
}

fn find_local_by_name(
    hir: &crate::compiler_frontend::hir::module::HirModule,
    string_table: &StringTable,
    name: &str,
) -> Option<LocalId> {
    hir.blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| hir.side_table.resolve_local_name(local.id, string_table) == Some(name))
        .map(|local| local.id)
}

fn find_assign_statement_id_for_local_name(
    hir: &crate::compiler_frontend::hir::module::HirModule,
    string_table: &StringTable,
    name: &str,
) -> Option<HirNodeId> {
    for block in &hir.blocks {
        for statement in &block.statements {
            if let HirStatementKind::Assign {
                target: HirPlace::Local(local),
                ..
            } = &statement.kind
                && hir.side_table.resolve_local_name(*local, string_table) == Some(name)
            {
                return Some(statement.id);
            }
        }
    }
    None
}

/// Finds the semantic result state immediately after a first-class HIR map operation.
fn find_map_op_result_and_following_statement(
    hir: &crate::compiler_frontend::hir::module::HirModule,
    wanted_op: HirMapOp,
) -> Option<(LocalId, HirNodeId)> {
    for block in &hir.blocks {
        for (index, statement) in block.statements.iter().enumerate() {
            if let HirStatementKind::MapOp { op, result, .. } = &statement.kind
                && *op == wanted_op
                && let Some(result_local) = *result
                && let Some(following_statement) = block.statements.get(index + 1)
            {
                return Some((result_local, following_statement.id));
            }
        }
    }
    None
}

fn collect_reachable_blocks(
    hir: &crate::compiler_frontend::hir::module::HirModule,
    entry: BlockId,
) -> Vec<BlockId> {
    let mut visited = FxHashSet::default();
    let mut queue = VecDeque::new();
    let mut blocks = Vec::new();
    queue.push_back(entry);

    while let Some(block_id) = queue.pop_front() {
        if !visited.insert(block_id) {
            continue;
        }

        blocks.push(block_id);
        match &hir.blocks[block_id.0 as usize].terminator {
            HirTerminator::Jump { target, .. } => queue.push_back(*target),
            HirTerminator::If {
                then_block,
                else_block,
                ..
            } => {
                queue.push_back(*then_block);
                queue.push_back(*else_block);
            }
            HirTerminator::FallibleBranch {
                success_block,
                error_block,
                ..
            } => {
                queue.push_back(*success_block);
                queue.push_back(*error_block);
            }
            HirTerminator::Match { arms, .. } => {
                for arm in arms {
                    queue.push_back(arm.body);
                }
            }
            HirTerminator::Break { target } | HirTerminator::Continue { target } => {
                queue.push_back(*target);
            }
            HirTerminator::Return(_)
            | HirTerminator::ReturnSuccess(_)
            | HirTerminator::ReturnError(_)
            | HirTerminator::RuntimeFailure { .. }
            | HirTerminator::Uninitialized
            | HirTerminator::AssertFailure { .. } => {}
        }
    }

    blocks
}

fn collect_statement_values(kind: HirStatementKind, out: &mut FxHashSet<HirValueId>) {
    match kind {
        HirStatementKind::Assign { value, .. } => collect_expression_values(&value, out),
        HirStatementKind::Call { args, .. } => {
            for arg in args {
                collect_expression_values(&arg, out);
            }
        }
        HirStatementKind::MapOp { receiver, args, .. } => {
            collect_expression_values(&receiver, out);
            for arg in args {
                collect_expression_values(&arg, out);
            }
        }
        HirStatementKind::Expr(expr) => collect_expression_values(&expr, out),
        HirStatementKind::CastOp { source, .. } => collect_expression_values(&source, out),
        HirStatementKind::NumericOp { operands, .. } => match operands {
            crate::compiler_frontend::hir::numeric::HirNumericOperands::Unary { operand } => {
                collect_expression_values(&operand, out);
            }
            crate::compiler_frontend::hir::numeric::HirNumericOperands::Binary { left, right } => {
                collect_expression_values(&left, out);
                collect_expression_values(&right, out);
            }
        },
        HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. } => collect_expression_values(&source, out),
        HirStatementKind::Drop(_) => {}
        HirStatementKind::PushRuntimeFragment { value, .. } => {
            collect_expression_values(&value, out)
        }
    }
}

fn collect_terminator_values(terminator: &HirTerminator, out: &mut FxHashSet<HirValueId>) {
    match terminator {
        HirTerminator::If { condition, .. } => collect_expression_values(condition, out),
        HirTerminator::FallibleBranch { result, .. } => collect_expression_values(result, out),
        HirTerminator::Match { scrutinee, arms } => {
            collect_expression_values(scrutinee, out);
            for arm in arms {
                if let crate::compiler_frontend::hir::patterns::HirPattern::Literal(value)
                | crate::compiler_frontend::hir::patterns::HirPattern::OptionValue { value } =
                    &arm.pattern
                {
                    collect_expression_values(value, out);
                }
                if let Some(guard) = &arm.guard {
                    collect_expression_values(guard, out);
                }
            }
        }
        HirTerminator::Return(value)
        | HirTerminator::ReturnSuccess(value)
        | HirTerminator::ReturnError(value) => collect_expression_values(value, out),
        HirTerminator::AssertFailure { .. } => {
            // Assertion messages are compile-time text, not expressions.
        }

        HirTerminator::RuntimeFailure { .. } => {
            // Runtime-failure messages are backend-facing text, not expressions.
        }

        HirTerminator::Uninitialized => {
            // Internal placeholder — no expressions to visit.
        }
        HirTerminator::Jump { .. }
        | HirTerminator::Break { .. }
        | HirTerminator::Continue { .. } => {}
    }
}

fn collect_expression_values(expression: &HirExpression, out: &mut FxHashSet<HirValueId>) {
    out.insert(expression.id);

    match &expression.kind {
        HirExpressionKind::BinOp { left, right, .. } => {
            collect_expression_values(left, out);
            collect_expression_values(right, out);
        }
        HirExpressionKind::UnaryOp { operand, .. } => collect_expression_values(operand, out),
        HirExpressionKind::StructConstruct { fields, .. } => {
            for (_, value) in fields {
                collect_expression_values(value, out);
            }
        }
        HirExpressionKind::Collection(elements)
        | HirExpressionKind::TupleConstruct { elements } => {
            for element in elements {
                collect_expression_values(element, out);
            }
        }
        HirExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                collect_expression_values(&entry.key, out);
                collect_expression_values(&entry.value, out);
            }
        }
        HirExpressionKind::TupleGet { tuple, .. } => {
            collect_expression_values(tuple, out);
        }
        HirExpressionKind::Range { start, end } => {
            collect_expression_values(start, out);
            collect_expression_values(end, out);
        }
        HirExpressionKind::VariantConstruct { fields, .. } => {
            for field in fields {
                collect_expression_values(&field.value, out);
            }
        }
        HirExpressionKind::FallibleUnwrapSuccess { result }
        | HirExpressionKind::FallibleUnwrapError { result }
        | HirExpressionKind::Cast { source: result, .. } => {
            collect_expression_values(result, out);
        }
        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::Copy(_)
        | HirExpressionKind::Load(_) => {}

        HirExpressionKind::VariantPayloadGet { source, .. } => {
            collect_expression_values(source, out);
        }
    }
}
