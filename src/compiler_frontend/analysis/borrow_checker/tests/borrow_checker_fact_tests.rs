//! Borrow-checker fact-generation regression tests.
//!
//! WHAT: checks the low-level facts emitted for borrows, optional transfers, assignments, and returns.
//! WHY: these facts are the borrow checker's source of truth, so targeted tests catch drift
//! before it reaches higher-level diagnostics.

use crate::compiler_frontend::analysis::borrow_checker::types::{
    ValueAccessClassification, ValueBorrowFact,
};
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
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::{HirExpressionStore, HirValueRange};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapOp, ValueKind,
};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, function_node, make_test_variable, node, param_with_datatype,
    reference_expr_with_datatype, symbol, test_if_branch_metadata, test_source_location,
};
use crate::compiler_frontend::tests::borrow_fixture_support::{
    assert_borrow_error_kind, assert_invalid_mutable_access_reason, run_borrow_checker,
};
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::{entry_and_start, lower_hir};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tests::type_id_fixture_support::{
    build_ast_with_registered_types, lower_ast, runtime_expr, runtime_operand_item,
};
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashSet;
use std::collections::VecDeque;
use std::sync::Arc;

#[test]
fn shared_value_fact_merge_is_conservative_and_deterministic() {
    let mut fact = ValueBorrowFact {
        classification: ValueAccessClassification::SharedRead,
        roots: vec![LocalId(5), LocalId(2)],
        optional_transfer: OptionalTransferStatus::Transfer,
    };
    fact.merge(ValueBorrowFact {
        classification: ValueAccessClassification::MutableArgument,
        roots: vec![LocalId(5), LocalId(3)],
        optional_transfer: OptionalTransferStatus::Borrow,
    });

    assert_eq!(fact.classification, ValueAccessClassification::Mixed);
    assert_eq!(fact.roots, vec![LocalId(2), LocalId(3), LocalId(5)]);
    assert_eq!(fact.optional_transfer, OptionalTransferStatus::Borrow);
}

#[test]
fn shared_expression_row_merges_branch_roots_in_either_successor_order() {
    let source = r#"wrapper |left String, right String, flag Bool|:
    chosen = if flag:
        then left
    else
        then right
    ;
;"#;

    for reverse_successor_visit in [false, true] {
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
        let entry_path = ast.entry_path;
        let wrapper_name = path_fork
            .try_intern_child(entry_path, string_table.intern("wrapper"))
            .expect("test path fits");
        let (mut hir, type_environment) = lower_ast(ast, &mut string_table, &mut path_fork)
            .expect("the valid branch source should lower");
        let external_package_registry = default_external_package_registry(&mut string_table);
        let wrapper = hir
            .functions
            .iter()
            .find(|function| {
                hir.side_table
                    .function_name_path(function.id)
                    .is_some_and(|path| path == wrapper_name)
            })
            .expect("wrapper should lower to HIR");
        let left_local = wrapper.params[0];
        let right_local = wrapper.params[1];

        let (then_block_id, else_block_id) = {
            let branch = hir
                .blocks
                .iter_mut()
                .find(|block| matches!(&block.terminator, HirTerminator::If { .. }))
                .expect("wrapper value-if should lower to one branch");
            let HirTerminator::If {
                then_block,
                else_block,
                ..
            } = &mut branch.terminator
            else {
                unreachable!("the selected block terminates with an if")
            };
            if reverse_successor_visit {
                std::mem::swap(then_block, else_block);
            }
            (*then_block, *else_block)
        };

        let (then_definition, else_definition) = {
            let branch_definition = |block_id| {
                let block = hir
                    .blocks
                    .iter()
                    .find(|block| block.id == block_id)
                    .expect("branch target should identify a HIR block");
                let (index, statement) = block
                    .statements
                    .iter()
                    .enumerate()
                    .find(|(_, statement)| {
                        matches!(
                            &statement.kind,
                            HirStatementKind::Write {
                                target: HirWriteTarget::DefineLocal(_),
                                ..
                            }
                        )
                    })
                    .expect("each value-if branch should initialize its result local");
                let HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(target),
                    value,
                } = &statement.kind
                else {
                    unreachable!("selected branch statement is a definition")
                };
                let expression = hir.expressions.expression(*value);
                let place = match &expression.kind {
                    HirExpressionKind::Copy(place) | HirExpressionKind::Load(place) => *place,
                    _ => panic!("branch result should read its parameter place"),
                };
                assert!(place.projections.is_empty());
                assert!([left_local, right_local].contains(&place.root));
                (index, *target, *value, place.root)
            };
            (
                branch_definition(then_block_id),
                branch_definition(else_block_id),
            )
        };
        assert_eq!(then_definition.1, else_definition.1);
        assert_ne!(then_definition.3, else_definition.3);

        // The lowered branch value is an RValue Load. This fixture replaces it with a direct
        // Place Load so the scratch local carries a branch-specific alias into the join; it tests
        // fixed-point provenance propagation, not the source-level RValue materialization policy.
        for (block_id, definition) in [
            (then_block_id, then_definition),
            (else_block_id, else_definition),
        ] {
            let original = hir.expressions.expression(definition.2);
            let load = HirExpression {
                kind: HirExpressionKind::Load(HirPlace::local(definition.3)),
                ty: original.ty,
                value_kind: ValueKind::Place,
                region: original.region,
                span: None,
            };
            let load_id = hir
                .expressions
                .append_expression(load)
                .expect("the synthetic alias initializer should fit the test store");
            let block = hir
                .blocks
                .iter_mut()
                .find(|block| block.id == block_id)
                .expect("the branch initialization block should exist");
            let HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(target),
                value,
            } = &mut block.statements[definition.0].kind
            else {
                unreachable!("the selected branch statement initializes the scratch slot")
            };
            assert_eq!(*target, definition.1);
            *value = load_id;
        }

        let (receiving_local_id, receiving_region, receiving_type) = hir
            .blocks
            .iter()
            .flat_map(|block| &block.locals)
            .find(|local| local.id == then_definition.1)
            .map(|local| (local.id, local.region, local.ty))
            .expect("the value-if result local should be defined");
        assert_eq!(
            hir.side_table.local_origin_kind(receiving_local_id),
            Some(HirLocalOriginKind::CompilerTemp),
            "the alias carrier is compiler-owned scratch"
        );

        // This HIR-only fixture adds one shared read of the compiler-owned scratch local in both
        // branch contexts, after its alias initialization on each path.
        let shared_read = hir
            .expressions
            .append_expression(HirExpression {
                kind: HirExpressionKind::Load(HirPlace::local(receiving_local_id)),
                ty: receiving_type,
                value_kind: ValueKind::Place,
                region: receiving_region,
                span: None,
            })
            .expect("one shared scratch-local Load should fit the building store");
        let mut next_statement_id = hir
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .map(|statement| statement.id.0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .expect("HIR statement IDs should have room for the two branch reads");
        for (block_id, index) in [
            (then_block_id, then_definition.0),
            (else_block_id, else_definition.0),
        ] {
            let block = hir
                .blocks
                .iter_mut()
                .find(|block| block.id == block_id)
                .expect("branch target should remain present");
            let read_statement = HirStatement {
                id: HirNodeId(next_statement_id),
                kind: HirStatementKind::Expr(shared_read),
                span: None,
            };
            next_statement_id = next_statement_id
                .checked_add(1)
                .expect("HIR statement IDs should have room for the two branch reads");
            hir.side_table.map_statement(None, &read_statement);
            block.statements.insert(index + 1, read_statement);
        }

        let merge_read = hir
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| {
                let HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(target),
                    value,
                } = &statement.kind
                else {
                    return None;
                };
                let expression = hir.expressions.expression(*value);
                (*target != receiving_local_id
                    && matches!(
                        &expression.kind,
                        HirExpressionKind::Load(place) if place.root == receiving_local_id
                    ))
                .then_some(*value)
            })
            .expect("the source result should have its separate merge-local Load");
        assert_ne!(shared_read, merge_read);

        crate::compiler_frontend::hir::validate_hir_module(&hir, &type_environment)
            .expect("shared alias reads should preserve the module's HIR invariants");

        let report =
            run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
                .expect("the shared scratch alias should be borrow-safe");
        let fact = report
            .analysis
            .value_fact(shared_read)
            .expect("the shared branch Load should retain a merged fact");
        assert!(
            fact.roots.contains(&left_local),
            "left root was lost: {:?}",
            fact.roots
        );
        assert!(
            fact.roots.contains(&right_local),
            "right root was lost: {:?}",
            fact.roots
        );
        let mut expected_roots = [left_local, right_local];
        expected_roots.sort_unstable_by_key(|local| local.0);
        assert_eq!(fact.roots, expected_roots);
        assert_eq!(fact.classification, ValueAccessClassification::SharedRead);
    }
}

#[test]
fn statement_terminator_and_value_facts_are_populated() {
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    Expression::int(1, test_source_location(1), ValueMode::MutableOwned),
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    Expression::int(0, test_source_location(2), ValueMode::ImmutableOwned),
                )),
                test_source_location(2),
            ),
            node(
                NodeKind::If(
                    runtime_expr(
                        vec![runtime_operand_item(Expression::bool(
                            true,
                            test_source_location(3),
                            ValueMode::ImmutableOwned,
                        ))],
                        builtin_type_ids::BOOL,
                        test_source_location(3),
                        ValueMode::ImmutableOwned,
                    ),
                    vec![node(
                        NodeKind::Assignment {
                            target: assignment_target(
                                x,
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_source_location(4),
                            ),
                            value: Expression::int(
                                2,
                                test_source_location(4),
                                ValueMode::ImmutableOwned,
                            ),
                        },
                        test_source_location(4),
                    )],
                    Some(vec![node(
                        NodeKind::Assignment {
                            target: assignment_target(
                                x,
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_source_location(5),
                            ),
                            value: Expression::int(
                                3,
                                test_source_location(5),
                                ValueMode::ImmutableOwned,
                            ),
                        },
                        test_source_location(5),
                    )]),
                    test_if_branch_metadata(true),
                ),
                test_source_location(3),
            ),
        ],
        None,
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
        &mut path_fork,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
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
        }
    }

    let mut value_ids = FxHashSet::default();
    for block_id in &reachable {
        let block = &hir.blocks[block_id.0 as usize];
        for statement in &block.statements {
            collect_statement_values(&hir.expressions, &statement.kind, &mut value_ids);
        }
        collect_terminator_values(&hir.expressions, &block.terminator, &mut value_ids);
    }

    for value_id in value_ids {
        assert!(
            report.analysis.value_fact(value_id).is_some(),
            "missing value fact for value {value_id:?}"
        );
    }
}

#[test]
fn assertion_failure_message_is_collected_as_a_borrow_value_root() {
    let _path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut expressions = HirExpressionStore::default();
    let message = crate::compiler_frontend::tests::hir_fixture_support::expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(7))),
        builtin_type_ids::STRING,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let terminator = HirTerminator::AssertFailure {
        message,
        message_evaluation: HirAssertionMessageEvaluation::Runtime,
    };
    let mut value_ids = FxHashSet::default();

    collect_terminator_values(&expressions, &terminator, &mut value_ids);

    assert!(
        value_ids.contains(&message),
        "assertion message values must remain visible to borrow fact collection"
    );
}

#[test]
fn drop_statement_produces_statement_fact() {
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let value = symbol("value", &mut path_fork, &mut string_table);
    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                value,
                Expression::int(1, test_source_location(1), ValueMode::MutableOwned),
            )),
            test_source_location(1),
        )],
        None,
    );

    let mut hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
        &mut path_fork,
    );
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
        span: None,
    });

    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
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
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let data = symbol("data", &mut path_fork, &mut string_table);
    let first_ref = symbol("first_ref", &mut path_fork, &mut string_table);
    let sink = symbol("sink", &mut path_fork, &mut string_table);
    let second_ref = symbol("second_ref", &mut path_fork, &mut string_table);

    let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                data,
                Expression::int(7, test_source_location(1), ValueMode::MutableOwned),
            )),
            test_source_location(1),
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                first_ref,
                Expression::reference_with_type_id(
                    data,
                    DataType::Int,
                    builtin_type_ids::INT,
                    test_source_location(2),
                    ValueMode::MutableReference,
                    crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                ),
            )),
            test_source_location(2),
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                sink,
                reference_expr_with_datatype(
                    first_ref,
                    DataType::Int,
                    builtin_type_ids::INT,
                    test_source_location(3),
                ),
            )),
            test_source_location(3),
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                second_ref,
                Expression::reference_with_type_id(
                    data,
                    DataType::Int,
                    builtin_type_ids::INT,
                    test_source_location(4),
                    ValueMode::MutableReference,
                    crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                ),
            )),
            test_source_location(4),
        ),
    ],
    None,
);

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
        &mut path_fork,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("reborrow after last-use should pass");

    let second_statement_id =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "second_ref")
            .expect("should locate the reborrow statement");
    let data_local = find_local_by_name(&hir, &path_fork, &string_table, "data")
        .expect("should locate the source local");
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
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let source = symbol("source", &mut path_fork, &mut string_table);
    let target = symbol("target", &mut path_fork, &mut string_table);
    let sentinel = symbol("sentinel", &mut path_fork, &mut string_table);

    let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                source,
                Expression::int(7, test_source_location(10), ValueMode::MutableOwned),
            )),
            test_source_location(10),
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                target,
                Expression::reference_with_type_id(
                    source,
                    DataType::Int,
                    builtin_type_ids::INT,
                    test_source_location(11),
                    ValueMode::MutableOwned,
                    crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                ),
            )),
            test_source_location(11),
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                sentinel,
                Expression::int(0, test_source_location(12), ValueMode::ImmutableOwned),
            )),
            test_source_location(12),
        ),
    ],
    None,
);

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
        &mut path_fork,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("inferred assignment transfer should pass");

    let source_local = find_local_by_name(&hir, &path_fork, &string_table, "source")
        .expect("should locate the source local");
    let target_local = find_local_by_name(&hir, &path_fork, &string_table, "target")
        .expect("should locate the target local");
    let sentinel_statement_id =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "sentinel")
            .expect("should locate the sentinel statement");
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

    let target_initializer = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| {
            if let HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } = &statement.kind
                && *local == target_local
            {
                Some(*value)
            } else {
                None
            }
        })
        .expect("should locate the optional target initializer");
    assert_eq!(
        report
            .analysis
            .value_fact(target_initializer)
            .expect("target initializer should have a borrow fact")
            .optional_transfer,
        OptionalTransferStatus::Transfer
    );
}

#[test]
fn mutable_alias_final_read_in_same_statement_blocks_source_read() {
    use crate::compiler_frontend::compiler_messages::BorrowDiagnosticKind;

    for comparison in ["data is writer", "writer is data"] {
        let source = format!("data ~= 1\nwriter ~= data\nsame = {comparison}\n");
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(&source);
        let hir = lower_hir(ast, &mut string_table, &mut path_fork);
        let external_package_registry = default_external_package_registry(&mut string_table);
        let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
            .expect_err("exclusive holder is still active during its final-read statement");
        assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
    }
}

#[test]
fn mutable_alias_reachable_write_keeps_loan_active() {
    use crate::compiler_frontend::compiler_messages::BorrowDiagnosticKind;

    for body in [
        "snapshot = copy data\nif flag:\nwriter = 2\n;\n",
        "observed = writer\nsnapshot = copy data\nif flag:\nwriter = 2\n;\n",
        "loop flag:\nwriter = 2\nsnapshot = copy data\n;\n",
    ] {
        let source = format!("probe |flag Bool|:\ndata ~= 1\nwriter ~= data\n{body};\n");
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(&source);
        let hir = lower_hir(ast, &mut string_table, &mut path_fork);
        let external_package_registry = default_external_package_registry(&mut string_table);
        let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
            .expect_err("a reachable write through the holder keeps its exclusive loan active");
        assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
    }
}

#[test]
fn slot_self_update_preserves_value_backed_role_and_provenance() {
    let source = r#"identity |input String| -> String:
return input
;
source ~= "source"
target ~= identity(source)
target = target
sentinel = 0
observed = copy target
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("self-updating a slot must preserve its call-result value role");
    let target = find_local_by_name(&hir, &path_fork, &string_table, "target")
        .expect("the fixture declares a mutable target slot");
    let source_local = find_local_by_name(&hir, &path_fork, &string_table, "source")
        .expect("the identity call aliases the source allocation");
    let update = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Write {
                    target: HirWriteTarget::AssignPlace(destination),
                    value,
                } if destination.root == target
                    && hir.expressions.projections(destination.projections).is_empty()
                    && matches!(
                        &hir.expressions.expression(*value).kind,
                        HirExpressionKind::Load(source_place)
                            if source_place.root == target
                                && hir.expressions.projections(source_place.projections).is_empty()
                    )
            )
        })
        .expect("the direct self-assignment should lower to a local Update");
    let sentinel =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "sentinel")
            .expect("the sentinel follows the self-update");

    for (statement_id, context) in [
        (update.id, "before self-update"),
        (sentinel, "after self-update"),
    ] {
        let snapshot = report.analysis.statement_entry_states[&statement_id]
            .locals
            .iter()
            .find(|local| local.local == target)
            .expect("the statement snapshot should contain the target");
        assert_eq!(
            snapshot.mode,
            LocalMode::SLOT,
            "target should remain a value-backed slot {context}"
        );
        assert!(
            snapshot.alias_roots.contains(&source_local),
            "target should retain the call-result allocation root {context}"
        );
        assert!(
            !snapshot.alias_roots.contains(&target),
            "target must not become an alias of its own binding cell {context}"
        );
    }
}

#[test]
fn fresh_update_preserves_possible_alias_write_through_after_join() {
    let source = "probe |flag Bool|:\n\
source ~= \"source\"\n\
target ~= \"old\"\n\
if flag:\n\
    target = source\n\
else\n\
    target = \"branch\"\n\
;\n\
target = \"replacement\"\n\
sentinel = 0\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("the joined update has no competing holder");
    let target = find_local_by_name(&hir, &path_fork, &string_table, "target")
        .expect("the fixture declares its update destination");
    let source_local = find_local_by_name(&hir, &path_fork, &string_table, "source")
        .expect("the alias branch has a source root");
    let sentinel =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "sentinel")
            .expect("the sentinel follows the update");
    let update = hir
        .blocks
        .iter()
        .find_map(|block| {
            let sentinel_index = block
                .statements
                .iter()
                .position(|statement| statement.id == sentinel)?;
            block.statements[..sentinel_index]
                .iter()
                .rfind(|statement| {
                    matches!(
                        &statement.kind,
                        HirStatementKind::Write {
                            target: HirWriteTarget::AssignPlace(place),
                            ..
                        } if place.root == target
                            && hir.expressions.projections(place.projections).is_empty()
                    )
                })
        })
        .expect("the source assignment should lower to an update");
    let update_entry = &report.analysis.statement_entry_states[&update.id];
    let target_before = update_entry
        .locals
        .iter()
        .find(|local| local.local == target)
        .expect("the update entry should contain its destination");
    assert!(target_before.mode.contains(LocalMode::SLOT));
    assert!(target_before.mode.contains(LocalMode::ALIAS));
    assert!(target_before.alias_roots.contains(&source_local));

    let target_after = report.analysis.statement_entry_states[&sentinel]
        .locals
        .iter()
        .find(|local| local.local == target)
        .expect("the sentinel entry should contain the updated destination");
    assert!(target_after.mode.contains(LocalMode::SLOT));
    assert!(target_after.mode.contains(LocalMode::ALIAS));
    assert!(target_after.alias_roots.contains(&source_local));
}

#[test]
fn dedicated_alias_call_update_preserves_mixed_destination_roots() {
    let source = "identity |input String| -> String:\n\
return input\n\
;\n\
probe |flag Bool|:\n\
argument ~= \"argument\"\n\
source ~= \"source\"\n\
target ~= \"old\"\n\
if flag:\n\
    target = source\n\
else\n\
    target = \"branch\"\n\
;\n\
unused = identity(argument)\n\
sentinel = 0\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (mut hir, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("the fixture should lower");
    let external_package_registry = default_external_package_registry(&mut string_table);
    let target = find_local_by_name(&hir, &path_fork, &string_table, "target")
        .expect("the fixture declares its update destination");

    let (call_result, call_region) = hir
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find_map(|statement| match &statement.kind {
                    HirStatementKind::Call {
                        args,
                        result: Some(HirLocalDestination::Define(local)),
                        ..
                    } if args.len() == 1 => Some((*local, block.region)),
                    _ => None,
                })
        })
        .expect("the identity call should define a result local");
    let target_type = hir
        .blocks
        .iter()
        .flat_map(|block| &block.locals)
        .find(|local| local.id == target)
        .map(|local| local.ty)
        .expect("the update destination should have a declared type");
    let replacement_value = crate::compiler_frontend::tests::hir_fixture_support::expression(
        HirExpressionKind::Load(HirPlace::local(target)),
        target_type,
        call_region,
        ValueKind::Place,
        &mut hir.expressions,
    );

    let mut updated_call_id = None;
    let mut retargeted_consumer = false;
    for block in &mut hir.blocks {
        for statement in &mut block.statements {
            match &mut statement.kind {
                HirStatementKind::Call {
                    args,
                    result: Some(destination),
                    ..
                } if args.len() == 1
                    && matches!(
                        &*destination,
                        HirLocalDestination::Define(local) if *local == call_result
                    ) =>
                {
                    *destination = HirLocalDestination::Update(target);
                    updated_call_id = Some(statement.id);
                }
                HirStatementKind::Write { value, .. }
                    if matches!(
                        &hir.expressions.expression(*value).kind,
                        HirExpressionKind::Load(place) if place.root == call_result
                    ) =>
                {
                    *value = replacement_value;
                    retargeted_consumer = true;
                }
                _ => {}
            }
        }
    }
    assert!(
        updated_call_id.is_some(),
        "the call result should now update the mixed local"
    );
    assert!(
        retargeted_consumer,
        "the following value use should load the updated destination"
    );

    crate::compiler_frontend::hir::validate_hir_module(&hir, &type_environment)
        .expect("the dedicated result update must remain structurally valid HIR");
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("the alias return should preserve the mixed update roots");
    let sentinel =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "sentinel")
            .expect("the sentinel follows the operation update");
    let entry = &report.analysis.statement_entry_states[&sentinel];
    let state = entry
        .locals
        .iter()
        .find(|local| local.local == target)
        .expect("the sentinel should retain the result destination");
    let source = find_local_by_name(&hir, &path_fork, &string_table, "source")
        .expect("the mixed alias branch has a source root");
    let argument = find_local_by_name(&hir, &path_fork, &string_table, "argument")
        .expect("the returned value has an independent source root");
    assert!(state.mode.contains(LocalMode::SLOT));
    assert!(state.mode.contains(LocalMode::ALIAS));
    assert!(state.alias_roots.contains(&source));
    assert!(state.alias_roots.contains(&argument));
}

#[test]
fn loop_local_alias_next_declaration_does_not_prolong_previous_loan() {
    let source = "probe |flag Bool|:\n\
                  data ~= 1\nloop flag:\nwriter ~= data\n\
                  observed = copy writer\nsnapshot = copy data\n;\n;\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("the observed loop-local loan ends before the source read each iteration");
}

#[test]
fn unequal_path_diamond_expires_alias_after_merge_final_read() {
    for reverse_numbering in [false, true] {
        for mutable_alias in [false, true] {
            check_unequal_path_alias_activity(reverse_numbering, mutable_alias, false);
        }
    }
}

#[test]
fn unequal_path_diamond_keeps_alias_used_on_one_future_branch_active() {
    for reverse_numbering in [false, true] {
        for mutable_alias in [false, true] {
            check_unequal_path_alias_activity(reverse_numbering, mutable_alias, true);
        }
    }
}

fn check_unequal_path_alias_activity(
    reverse_numbering: bool,
    mutable_alias: bool,
    later_branch_use: bool,
) {
    use crate::compiler_frontend::compiler_messages::{
        BorrowDiagnosticKind, InvalidMutableAccessReason,
    };
    use crate::compiler_frontend::tests::hir_fixture_support::{
        bool_expression, expression, statement,
    };

    let source = "scores ~{String = Int} = {\"Priya\" = 10}\nresult = scores\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let mut hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let root = find_local_by_name(&hir, &path_fork, &string_table, "scores")
        .expect("fixture declares the map root");
    let result = find_local_by_name(&hir, &path_fork, &string_table, "result")
        .expect("fixture declares the result alias");
    let start = hir.start_function.expect("fixture has a start function");
    let entry_id = hir.functions[start.0 as usize].entry;
    let (region, result_type, success_definition) = {
        let entry = &mut hir.blocks[entry_id.0 as usize];
        let region = entry.region;
        let result_local = entry
            .locals
            .iter_mut()
            .find(|local| local.id == result)
            .expect("result is declared in the entry region");
        result_local.mutable = true;
        let result_type = result_local.ty;
        let definition_index = entry
            .statements
            .iter()
            .position(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(local),
                        ..
                    } if *local == result
                )
            })
            .expect("fixture binds the result alias");
        (
            region,
            result_type,
            entry.statements.remove(definition_index),
        )
    };
    let mut handler_definition = success_definition.clone();
    handler_definition.id = HirNodeId(80_000);
    if !mutable_alias {
        hir.side_table
            .bind_local_origin(result, HirLocalOriginKind::CompilerTemp, None, None);
    }

    // Both paths write the same result, but the error path has an adapter before its handler.
    // BFS therefore visits the merge before the handler, independently of block numbering.
    let base = hir.blocks.len() as u32;
    let block_id = |offset| {
        BlockId(
            base + if reverse_numbering {
                5 - offset
            } else {
                offset
            },
        )
    };
    let success = block_id(0);
    let transfer = block_id(1);
    let handler = block_id(2);
    let merge = block_id(3);
    let later_use = block_id(4);
    let exit = block_id(5);
    let load = |expressions: &mut HirExpressionStore, local| {
        expression(
            HirExpressionKind::Load(HirPlace::local(local)),
            result_type,
            region,
            ValueKind::Place,
            expressions,
        )
    };
    let jump = |target| HirTerminator::Jump {
        target,
        args: vec![],
    };
    let exit_terminator = hir.blocks[entry_id.0 as usize].terminator.clone();
    let condition = bool_expression(true, builtin_type_ids::BOOL, region, &mut hir.expressions);
    hir.blocks[entry_id.0 as usize].terminator = HirTerminator::If {
        condition,
        then_block: success,
        else_block: transfer,
    };
    let access = if mutable_alias {
        // This exercises the companion active-mutable-loan check for a shared root read.
        HirStatementKind::Expr(load(&mut hir.expressions, root))
    } else {
        HirStatementKind::MapOp {
            op: HirMapOp::Clear,
            receiver: load(&mut hir.expressions, root),
            args: HirValueRange::empty(),
            result: None,
        }
    };
    let result_read = load(&mut hir.expressions, result);
    let later_condition =
        bool_expression(true, builtin_type_ids::BOOL, region, &mut hir.expressions);
    let later_read = load(&mut hir.expressions, result);
    let mut blocks = vec![
        HirBlock {
            id: success,
            region,
            locals: vec![],
            statements: vec![success_definition],
            terminator: jump(merge),
        },
        HirBlock {
            id: transfer,
            region,
            locals: vec![],
            statements: vec![],
            terminator: jump(handler),
        },
        HirBlock {
            id: handler,
            region,
            locals: vec![],
            statements: vec![handler_definition],
            terminator: jump(merge),
        },
        HirBlock {
            id: merge,
            region,
            locals: vec![],
            statements: vec![
                statement(80_003, HirStatementKind::Expr(result_read), 3),
                statement(80_004, access, 4),
            ],
            terminator: if later_branch_use {
                HirTerminator::If {
                    condition: later_condition,
                    then_block: later_use,
                    else_block: exit,
                }
            } else {
                jump(exit)
            },
        },
        HirBlock {
            id: later_use,
            region,
            locals: vec![],
            statements: vec![statement(80_006, HirStatementKind::Expr(later_read), 6)],
            terminator: jump(exit),
        },
        HirBlock {
            id: exit,
            region,
            locals: vec![],
            statements: vec![],
            terminator: exit_terminator,
        },
    ];
    blocks.sort_by_key(|block| block.id.0);
    hir.blocks.extend(blocks);

    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table);
    if later_branch_use {
        let error = report.expect_err("one later-use branch must keep the alias active");
        if mutable_alias {
            assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
        } else {
            assert_invalid_mutable_access_reason(
                &error,
                InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
            );
        }
    } else {
        report.expect("the result's last read precedes access and must expire on this CFG");
    }
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
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a get with no later mutation should pass");

    let scores_local = find_local_by_name(&hir, &path_fork, &string_table, "scores")
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
        result_snapshot.mode.contains(LocalMode::SLOT),
        "get result should be stored in a caller slot, got mode {:?}",
        result_snapshot.mode
    );
    assert!(
        !result_snapshot.mode.contains(LocalMode::ALIAS),
        "get result should not be a write-through alias binding, got mode {:?}",
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
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let receiver_local = find_local_by_name(&hir, &path_fork, &string_table, "scores")
        .expect("should locate the map receiver by name");
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a remove with no later mutation should pass");

    let removed_local = find_local_by_name(&hir, &path_fork, &string_table, "removed")
        .expect("should locate the remove binding by name");
    let sentinel_statement =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "sentinel")
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
        !result_snapshot.alias_roots.contains(&receiver_local),
        "remove result should not carry the map receiver root, got {:?}",
        result_snapshot.alias_roots
    );
    assert!(
        !result_snapshot.alias_roots.is_empty(),
        "remove result should retain its independent allocation provenance"
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
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
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
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "sentinel")
            .expect("should locate the sentinel statement by its assigned local");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&sentinel_statement)
        .expect("sentinel statement should have an entry snapshot");
    for name in ["key", "value"] {
        let local = find_local_by_name(&hir, &path_fork, &string_table, name)
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
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a later-use mutable set should borrow and keep the value usable");

    let first_use_statement =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "key_label")
            .expect("should locate the first later-use statement by its assigned local");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&first_use_statement)
        .expect("first later-use statement should have an entry snapshot");

    for name in ["key", "value"] {
        let local = find_local_by_name(&hir, &path_fork, &string_table, name)
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
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a later-use nested literal should retain shared storage");

    let value_local = find_local_by_name(&hir, &path_fork, &string_table, "value")
        .expect("should locate the inner inserted value local by name");
    assert!(
        report.analysis.value_facts.values().any(|fact| {
            fact.optional_transfer == OptionalTransferStatus::Borrow
                && fact.roots.contains(&value_local)
        }),
        "later-use nested literal should record advisory borrow fallback"
    );

    let label_statement =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "label")
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
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a final-use nested literal with no later value use should pass");

    let value_local = find_local_by_name(&hir, &path_fork, &string_table, "value")
        .expect("should locate the inner inserted value local by name");
    let sentinel_statement =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "sentinel")
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
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a retained aliased result should borrow its final-use argument");

    let value_local = find_local_by_name(&hir, &path_fork, &string_table, "value")
        .expect("should locate the aliased argument local");
    let result_local = find_local_by_name(&hir, &path_fork, &string_table, "result")
        .expect("should locate the retained result local");
    let sentinel_statement =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "sentinel")
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
        result_snapshot.mode.contains(LocalMode::SLOT),
        "retained alias result should be stored in a caller slot, got mode {:?}",
        result_snapshot.mode
    );
    assert!(
        !result_snapshot.mode.contains(LocalMode::ALIAS),
        "retained alias result should not be a write-through alias binding, got mode {:?}",
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
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let entry_path = ast.entry_path;
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("transparent fallible success projection should not self-conflict");
    let identity_name = path_fork
        .try_intern_child(entry_path, string_table.intern("identity"))
        .expect("test path fits");

    let identity_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == identity_name)
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
                    result: Some(HirLocalDestination::Define(result)),
                } = &statement.kind
                else {
                    return None;
                };
                if *target != identity_id {
                    return None;
                }
                let argument_ids = hir.expressions.values(*args);
                let [argument_id] = argument_ids else {
                    return None;
                };
                let argument = hir.expressions.expression(*argument_id);
                let HirExpressionKind::FallibleUnwrapSuccess { result: payload } = &argument.kind
                else {
                    return None;
                };
                let payload = hir.expressions.expression(*payload);
                let HirExpressionKind::Load(place) = &payload.kind else {
                    return None;
                };
                if !hir.expressions.projections(place.projections).is_empty() {
                    return None;
                }
                Some((block.id, statement.id, place.root, *result))
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
        result_snapshot.mode.contains(LocalMode::SLOT),
        "identity result should retain slot-backed value provenance, got {:?}",
        result_snapshot.mode
    );
    assert!(
        !result_snapshot.mode.contains(LocalMode::ALIAS),
        "identity result should not be a write-through alias binding, got {:?}",
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
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
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

    let unknown_name = symbol("unknown", &mut path_fork, &mut string_table);
    let input_name = symbol("input", &mut path_fork, &mut string_table);
    let argument_name = symbol("argument", &mut path_fork, &mut string_table);
    let result_name = symbol("result", &mut path_fork, &mut string_table);
    let caller_name = symbol("caller", &mut path_fork, &mut string_table);
    let sentinel_name = symbol("sentinel", &mut path_fork, &mut string_table);
    let mut expression_types = TypeEnvironment::new();
    let external_call = Expression::handled_fallible_host_function_call_with_typed_arguments(
        HandledFallibleHostFunctionCallInput {
            id: external_id,
            args: vec![CallArgument::positional(
                reference_expr_with_datatype(
                    input_name,
                    DataType::StringSlice,
                    builtin_type_ids::STRING,
                    test_source_location(2),
                ),
                CallAccessMode::Shared,
                test_source_location(2),
            )],
            result_type_ids: vec![builtin_type_ids::STRING, builtin_type_ids::STRING],
            error_type_id: builtin_type_ids::STRING,
            handling: FallibleExpressionHandling::Propagate,
            span: None,
        },
        &mut expression_types,
    );
    let unknown = function_node(
        unknown_name,
        FunctionSignature {
            parameters: vec![param_with_datatype(
                input_name,
                DataType::StringSlice,
                builtin_type_ids::STRING,
                false,
                test_source_location(1),
            )],
            returns: vec![
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    channel: ReturnChannel::Error,
                },
            ],
        },
        vec![node(
            NodeKind::Return(vec![external_call]),
            test_source_location(2),
        )],
        None,
    );
    let caller = function_node(
        caller_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    channel: ReturnChannel::Error,
                },
            ],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    argument_name,
                    Expression::string_slice(
                        string_table.intern("hello"),
                        test_source_location(5),
                        ValueMode::MutableOwned,
                    ),
                )),
                test_source_location(5),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    result_name,
                    Expression::handled_fallible_function_call_with_typed_arguments(
                        unknown_name,
                        vec![CallArgument::positional(
                            reference_expr_with_datatype(
                                argument_name,
                                DataType::StringSlice,
                                builtin_type_ids::STRING,
                                test_source_location(6),
                            ),
                            CallAccessMode::Shared,
                            test_source_location(6),
                        )],
                        vec![builtin_type_ids::STRING, builtin_type_ids::STRING],
                        FallibleExpressionHandling::Propagate,
                        &mut expression_types,
                        test_source_location(6),
                    ),
                )),
                test_source_location(6),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    sentinel_name,
                    Expression::int(0, test_source_location(7), ValueMode::ImmutableOwned),
                )),
                test_source_location(7),
            ),
            node(
                NodeKind::Return(vec![
                    Expression::string_slice(
                        string_table.intern("done"),
                        test_source_location(8),
                        ValueMode::ImmutableOwned,
                    ),
                    Expression::string_slice(
                        string_table.intern("done"),
                        test_source_location(8),
                        ValueMode::ImmutableOwned,
                    ),
                ]),
                test_source_location(8),
            ),
        ],
        None,
    );
    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );
    let hir = lower_hir(
        build_ast_with_registered_types(vec![unknown, caller, start], entry_path),
        &mut string_table,
        &mut path_fork,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a retained unknown result should borrow a possible aliased argument");

    let unknown_function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == unknown_name)
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

    let argument_local = find_local_by_name(&hir, &path_fork, &string_table, "argument")
        .expect("should locate the possible aliased argument local");
    let result_definition =
        find_local_definition_statement_id_for_name(&hir, &path_fork, &string_table, "result")
            .expect("should locate the retained result definition");
    let caller_function = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == caller_name)
        })
        .expect("should locate the caller function");
    let caller_block = &hir.blocks[caller_function.entry.0 as usize];
    let call_result_local = caller_block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Call {
                result: Some(HirLocalDestination::Define(result)),
                ..
            } => Some(*result),
            _ => None,
        })
        .expect("caller should retain the call result before its binding definition");
    let entry_state = report
        .analysis
        .statement_entry_states
        .get(&result_definition)
        .expect("result definition should have an entry snapshot");
    let argument_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == argument_local)
        .expect("result definition entry should include the possible aliased argument");
    let result_snapshot = entry_state
        .locals
        .iter()
        .find(|snapshot| snapshot.local == call_result_local)
        .expect("result definition entry should include the retained call result");

    assert!(
        !argument_snapshot.mode.is_definitely_uninit(),
        "a retained unknown result must not move a possible source root, got mode {:?}",
        argument_snapshot.mode
    );
    assert!(
        result_snapshot.mode.contains(LocalMode::SLOT),
        "retained unknown result should use a caller slot for possible alias roots, got mode {:?}",
        result_snapshot.mode
    );
    assert!(
        !result_snapshot.mode.contains(LocalMode::ALIAS),
        "retained unknown result should not be a write-through alias binding, got mode {:?}",
        result_snapshot.mode
    );
    assert!(
        result_snapshot.alias_roots.contains(&argument_local),
        "retained unknown result should retain the possible argument root, got {:?}",
        result_snapshot.alias_roots
    );
}

fn find_local_by_name(
    hir: &crate::compiler_frontend::hir::module::HirModule,
    path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> Option<LocalId> {
    hir.blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| {
            hir.side_table
                .resolve_local_name(local.id, path_fork, string_table)
                == Some(name)
        })
        .map(|local| local.id)
}

fn is_local_place(hir: &HirModule, place: HirPlace, local: LocalId) -> bool {
    place.root == local && hir.expressions.projections(place.projections).is_empty()
}

fn loaded_local(hir: &HirModule, expression_id: HirValueId) -> Option<LocalId> {
    let HirExpressionKind::Load(place) = &hir.expressions.expression(expression_id).kind else {
        return None;
    };
    hir.expressions
        .projections(place.projections)
        .is_empty()
        .then_some(place.root)
}

fn find_local_definition_statement_id_for_name(
    hir: &crate::compiler_frontend::hir::module::HirModule,
    path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> Option<HirNodeId> {
    for block in &hir.blocks {
        for statement in &block.statements {
            if let HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                ..
            } = &statement.kind
                && hir
                    .side_table
                    .resolve_local_name(*local, path_fork, string_table)
                    == Some(name)
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
                && let Some(HirLocalDestination::Define(result_local)) = *result
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

fn collect_statement_values(
    expressions: &HirExpressionStore,
    kind: &HirStatementKind,
    out: &mut FxHashSet<HirValueId>,
) {
    match kind {
        HirStatementKind::Write { target, value } => {
            if let HirWriteTarget::AssignPlace(place) = target {
                collect_place_index_values(expressions, *place, out);
            }
            collect_expression_values(expressions, *value, out);
        }
        HirStatementKind::Call { args, .. } => {
            for argument in expressions.values(*args) {
                collect_expression_values(expressions, *argument, out);
            }
        }
        HirStatementKind::MapOp { receiver, args, .. } => {
            collect_expression_values(expressions, *receiver, out);
            for argument in expressions.values(*args) {
                collect_expression_values(expressions, *argument, out);
            }
        }
        HirStatementKind::Expr(expression_id) => {
            collect_expression_values(expressions, *expression_id, out);
        }
        HirStatementKind::CastOp { source, .. } => {
            collect_expression_values(expressions, *source, out);
        }
        HirStatementKind::NumericOp { operands, .. } => match operands {
            crate::compiler_frontend::hir::numeric::HirNumericOperands::Unary { operand } => {
                collect_expression_values(expressions, *operand, out);
            }
            crate::compiler_frontend::hir::numeric::HirNumericOperands::Binary { left, right } => {
                collect_expression_values(expressions, *left, out);
                collect_expression_values(expressions, *right, out);
            }
        },
        HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. } => {
            collect_expression_values(expressions, *source, out);
        }
        HirStatementKind::FloatRangeCandidate {
            current,
            step,
            end,
            ascending,
            ..
        } => {
            collect_expression_values(expressions, *current, out);
            collect_expression_values(expressions, *step, out);
            collect_expression_values(expressions, *end, out);
            collect_expression_values(expressions, *ascending, out);
        }
        HirStatementKind::Drop(_) | HirStatementKind::RangeStepFailure { .. } => {}
        HirStatementKind::PushRuntimeFragment { value, .. } => {
            collect_expression_values(expressions, *value, out);
        }
    }
}

fn collect_terminator_values(
    expressions: &HirExpressionStore,
    terminator: &HirTerminator,
    out: &mut FxHashSet<HirValueId>,
) {
    match terminator {
        HirTerminator::If { condition, .. } => {
            collect_expression_values(expressions, *condition, out);
        }
        HirTerminator::FallibleBranch { result, .. } => {
            collect_expression_values(expressions, *result, out);
        }
        HirTerminator::Match { scrutinee, arms } => {
            collect_expression_values(expressions, *scrutinee, out);
            for arm in arms {
                if let crate::compiler_frontend::hir::patterns::HirPattern::Literal(value)
                | crate::compiler_frontend::hir::patterns::HirPattern::OptionValue { value }
                | crate::compiler_frontend::hir::patterns::HirPattern::OptionRelational {
                    value,
                    ..
                }
                | crate::compiler_frontend::hir::patterns::HirPattern::Relational {
                    value,
                    ..
                } = &arm.pattern
                {
                    collect_expression_values(expressions, *value, out);
                }
                if let Some(guard) = arm.guard {
                    collect_expression_values(expressions, guard, out);
                }
            }
        }
        HirTerminator::Return(value)
        | HirTerminator::ReturnSuccess(value)
        | HirTerminator::ReturnError(value) => {
            collect_expression_values(expressions, *value, out);
        }
        HirTerminator::AssertFailure { message, .. } => {
            collect_expression_values(expressions, *message, out);
        }
        HirTerminator::RuntimeFailure { .. }
        | HirTerminator::Uninitialized
        | HirTerminator::Jump { .. }
        | HirTerminator::Break { .. }
        | HirTerminator::Continue { .. } => {}
    }
}

fn collect_place_index_values(
    expressions: &HirExpressionStore,
    place: HirPlace,
    out: &mut FxHashSet<HirValueId>,
) {
    for projection in expressions.projections(place.projections) {
        if let crate::compiler_frontend::hir::expression_store::HirProjection::Index(index) =
            projection
        {
            collect_expression_values(expressions, *index, out);
        }
    }
}

fn collect_expression_values(
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
    out: &mut FxHashSet<HirValueId>,
) {
    if !out.insert(expression_id) {
        return;
    }

    match &expressions.expression(expression_id).kind {
        HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
            collect_place_index_values(expressions, *place, out);
        }
        HirExpressionKind::BinOp { left, right, .. } => {
            collect_expression_values(expressions, *left, out);
            collect_expression_values(expressions, *right, out);
        }
        HirExpressionKind::UnaryOp { operand, .. } => {
            collect_expression_values(expressions, *operand, out);
        }
        HirExpressionKind::StructConstruct { fields, .. } => {
            for (_, value) in expressions.struct_fields(*fields) {
                collect_expression_values(expressions, *value, out);
            }
        }
        HirExpressionKind::Collection(elements)
        | HirExpressionKind::TupleConstruct { elements } => {
            for element in expressions.values(*elements) {
                collect_expression_values(expressions, *element, out);
            }
        }
        HirExpressionKind::MapLiteral(entries) => {
            for entry in expressions.map_entries(*entries) {
                collect_expression_values(expressions, entry.key, out);
                collect_expression_values(expressions, entry.value, out);
            }
        }
        HirExpressionKind::TupleGet { tuple, .. } => {
            collect_expression_values(expressions, *tuple, out);
        }
        HirExpressionKind::Range { start, end } => {
            collect_expression_values(expressions, *start, out);
            collect_expression_values(expressions, *end, out);
        }
        HirExpressionKind::VariantConstruct { fields, .. } => {
            for field in expressions.variant_fields(*fields) {
                collect_expression_values(expressions, field.value, out);
            }
        }
        HirExpressionKind::FallibleUnwrapSuccess { result }
        | HirExpressionKind::FallibleUnwrapError { result }
        | HirExpressionKind::Cast { source: result, .. } => {
            collect_expression_values(expressions, *result, out);
        }
        HirExpressionKind::VariantPayloadGet { source, .. } => {
            collect_expression_values(expressions, *source, out);
        }
        HirExpressionKind::Number(_)
        | HirExpressionKind::Int(_)
        | HirExpressionKind::Uint(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::FixedScalar(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. } => {}
    }
}

#[test]
fn catch_failed_argument_releases_earlier_shared_and_mutable_call_borrows() {
    // The first argument retains an alias in a call-result scratch slot. Only the success
    // continuation uses it; the handler must be able to rebind its source.
    for parameter in ["String", "~String"] {
        let access = if parameter == "~String" { "~" } else { "" };
        let source = format!(
            "identity |input {parameter}| -> String:\nreturn input\n;\n\
             combine |text String, count Int| -> String:\nreturn text\n;\n\
             probe |trigger Int| -> String:\n\
             data ~= \"before\"\n\
             return combine(identity({access}data), trigger + 2147483647) catch:\n\
                 data = \"handler\"\n\
                 then data\n\
             ;\n;\n"
        );
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(&source);
        let hir = lower_hir(ast, &mut string_table, &mut path_fork);
        let registry = default_external_package_registry(&mut string_table);
        let report = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
            .expect("an abandoned argument alias must not conflict with its handler");
        let data = find_local_by_name(&hir, &path_fork, &string_table, "data")
            .expect("the source binding must exist");
        let handler_write = hir
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .rfind(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::AssignPlace(place),
                        ..
                    } if is_local_place(&hir, *place, data)
                )
            })
            .expect("the handler must rebind the source");
        let state = &report.analysis.statement_entry_states[&handler_write.id];
        let snapshot = state
            .locals
            .iter()
            .find(|local| local.local == data)
            .expect("the source must be visible in the handler");
        assert!(snapshot.mode.contains(LocalMode::SLOT));
        assert!(!snapshot.mode.is_definitely_uninit());
        let first_call = hir
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find(|statement| {
                let HirStatementKind::Call { args, .. } = &statement.kind else {
                    return false;
                };
                let [argument] = hir.expressions.values(*args) else {
                    return false;
                };
                loaded_local(&hir, *argument) == Some(data)
            })
            .expect("the first argument must access the source before the failing argument");
        let fact = report
            .analysis
            .statement_fact(first_call.id)
            .expect("the earlier call must have a borrow fact");
        if parameter == "~String" {
            assert!(fact.mutable_roots.contains(&data));
        } else {
            assert!(fact.shared_roots.contains(&data));
        }
    }
}

#[test]
fn catch_failed_argument_keeps_optional_transfer_consistent_with_handler_use() {
    for (handler_value, expected_transfer) in [
        ("data", OptionalTransferStatus::Borrow),
        ("\"fallback\"", OptionalTransferStatus::Transfer),
    ] {
        let source = format!(
            "produce |input String| -> String:\nreturn \"fresh\"\n;\n\
             combine |text String, count Int| -> String:\nreturn text\n;\n\
             probe |data ~String, trigger Int| -> String:\n\
             return combine(produce(data), trigger + 2147483647) catch then {handler_value}\n;\n"
        );
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(&source);
        let hir = lower_hir(ast, &mut string_table, &mut path_fork);
        let registry = default_external_package_registry(&mut string_table);
        let report = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
            .expect("optional transfers must respect the error continuation's future reads");
        let data = find_local_by_name(&hir, &path_fork, &string_table, "data")
            .expect("the source parameter must exist");
        let argument = hir
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match &statement.kind {
                HirStatementKind::Call { args, .. }
                    if hir
                        .expressions
                        .values(*args)
                        .first()
                        .is_some_and(|argument| {
                            hir.expressions.values(*args).len() == 1
                                && loaded_local(&hir, *argument) == Some(data)
                        }) =>
                {
                    hir.expressions.values(*args).first().copied()
                }
                _ => None,
            })
            .expect("the earlier produce call must receive the source parameter");
        let fact = report
            .analysis
            .value_fact(argument)
            .expect("the earlier argument must have its own optional-transfer fact");
        assert!(fact.roots.contains(&data));
        assert_eq!(fact.optional_transfer, expected_transfer);
        let branch = hir
            .blocks
            .iter()
            .find(|block| block.locals.iter().any(|local| local.id == data))
            .expect("the protected region must hold the parameter");
        let HirTerminator::FallibleBranch { error_block, .. } = branch.terminator else {
            panic!("the later argument must have a real error edge");
        };
        let state = &report.analysis.block_entry_states[&error_block];
        let snapshot = state
            .locals
            .iter()
            .find(|local| local.local == data)
            .expect("the error edge must track the original source");
        assert!(
            snapshot.mode.contains(LocalMode::SLOT),
            "advisory transfer cannot invalidate mandatory source state"
        );
    }
}

#[test]
fn catch_handler_numeric_failure_returns_outward_without_success_local() {
    let source = "identity |input String| -> String:\nreturn input\n;\n\
                  combine |text String, count Int| -> String:\nreturn text\n;\n\
                  probe |input String, trigger Int| -> String, Error!:\n\
                  return combine(identity(input), trigger + 2147483647) catch:\n\
                      handler_local = \"handler\"\n\
                      count = trigger + 2147483647\n\
                      then handler_local\n\
                  ;\n;\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
        .expect("a failing handler must use the enclosing function error lane");
    let handler_local = find_local_by_name(&hir, &path_fork, &string_table, "handler_local")
        .expect("the handler must declare its local");
    let handler_block = hir
        .blocks
        .iter()
        .find(|block| block.locals.iter().any(|local| local.id == handler_local))
        .expect("the handler's block must exist");
    let HirTerminator::FallibleBranch { error_block, .. } = handler_block.terminator else {
        panic!("the handler's checked addition must branch");
    };
    let outward = &hir.blocks[error_block.0 as usize];
    assert!(matches!(outward.terminator, HirTerminator::ReturnError(_)));
    assert!(
        outward.statements.is_empty(),
        "a failing handler cannot commit its then value"
    );
    let result_local = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } if *local != handler_local
                && matches!(
                    &hir.expressions.expression(*value).kind,
                    HirExpressionKind::Load(_)
                ) =>
            {
                Some(*local)
            }
            _ => None,
        })
        .expect("the success path must have a result slot");
    let outward_state = &report.analysis.block_entry_states[&outward.id];
    let success_slot = outward_state
        .locals
        .iter()
        .find(|local| local.local == result_local)
        .expect("the result slot must be tracked on the outward edge");
    assert!(success_slot.mode.is_definitely_uninit());
}

#[test]
fn catch_handler_cannot_load_never_initialised_success_call_result() {
    use crate::compiler_frontend::compiler_messages::BorrowDiagnosticKind;

    let source = "identity |input String| -> String:\nreturn input\n;\n\
                  combine |text String, count Int| -> String:\nreturn text\n;\n\
                  probe |input String, trigger Int| -> String:\n\
                  return combine(identity(input), trigger + 2147483647) catch then \"fallback\"\n;\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let mut hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let registry = default_external_package_registry(&mut string_table);
    let record = hir
        .catch_protected_calls
        .iter()
        .find(|record| {
            hir.blocks
                .iter()
                .flat_map(|block| &block.statements)
                .any(|statement| {
                    statement.id == record.statement
                        && matches!(&statement.kind, HirStatementKind::Call { args, .. }
                    if hir.expressions.values(*args).len() == 2)
                })
        })
        .copied()
        .expect("the complete protected call must record its handler");
    let success_result = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Call {
                result: Some(HirLocalDestination::Define(local)),
                ..
            } if statement.id == record.statement => Some(*local),
            _ => None,
        })
        .expect("the protected call must have a success local");
    let success_type = hir
        .blocks
        .iter()
        .flat_map(|block| &block.locals)
        .find(|local| local.id == success_result)
        .expect("the result local must be declared")
        .ty;
    let handler_block_index = record.handler.block.0 as usize;
    let handler_region = hir.blocks[handler_block_index].region;
    let injected_value = crate::compiler_frontend::tests::hir_fixture_support::expression(
        HirExpressionKind::Load(HirPlace::local(success_result)),
        success_type,
        handler_region,
        ValueKind::Place,
        &mut hir.expressions,
    );
    let handler = &mut hir.blocks[handler_block_index];
    // Source scoping prevents naming this scratch slot. Inject its read, as the scope suite
    // does for dead locals, to protect the borrow consumer's mandatory initialisation rule.
    handler.statements.insert(
        0,
        HirStatement {
            id: HirNodeId(90_000),
            kind: HirStatementKind::Expr(injected_value),
            span: None,
        },
    );
    let error = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
        .expect_err("the handler must not read the skipped outer call's result");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::UseOfUninitializedLocal);
}

#[test]
fn equal_depth_diamond_merges_both_roots_in_one_merge_visit() {
    use crate::compiler_frontend::tests::hir_fixture_support::{
        bool_expression, expression, statement,
    };

    let source = "left ~{String = Int} = {\"Priya\" = 10}\n\
                  right ~{String = Int} = {\"Rob\" = 20}\n\
                  view = left\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let mut hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let left = find_local_by_name(&hir, &path_fork, &string_table, "left")
        .expect("fixture declares the left root");
    let right = find_local_by_name(&hir, &path_fork, &string_table, "right")
        .expect("fixture declares the right root");
    let view = find_local_by_name(&hir, &path_fork, &string_table, "view")
        .expect("fixture declares the view alias");
    let start = hir.start_function.expect("fixture has a start function");
    let entry_id = hir.functions[start.0 as usize].entry;
    let (region, view_type, left_definition) = {
        let entry = &mut hir.blocks[entry_id.0 as usize];
        let region = entry.region;
        let view_type = entry
            .locals
            .iter()
            .find(|local| local.id == view)
            .expect("view is declared in the entry region")
            .ty;
        let definition_index = entry
            .statements
            .iter()
            .position(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(local),
                        ..
                    } if *local == view
                )
            })
            .expect("fixture binds the view alias");
        (region, view_type, entry.statements.remove(definition_index))
    };
    let mut right_definition = left_definition.clone();
    right_definition.id = HirNodeId(90_000);
    let right_value = expression(
        HirExpressionKind::Load(HirPlace::local(right)),
        view_type,
        region,
        ValueKind::Place,
        &mut hir.expressions,
    );
    let HirStatementKind::Write {
        target: HirWriteTarget::DefineLocal(target),
        value,
    } = &mut right_definition.kind
    else {
        unreachable!("the selected statement is a definition");
    };
    assert_eq!(*target, view);
    *value = right_value;

    // Both arms sit one edge from the entry, so breadth-first scheduling reaches the merge from
    // the left arm and grows its input from the right arm before the merge is popped.
    let base = hir.blocks.len() as u32;
    let left_arm = BlockId(base);
    let right_arm = BlockId(base + 1);
    let merge = BlockId(base + 2);
    let exit_terminator = hir.blocks[entry_id.0 as usize].terminator.clone();
    let condition = bool_expression(true, builtin_type_ids::BOOL, region, &mut hir.expressions);
    hir.blocks[entry_id.0 as usize].terminator = HirTerminator::If {
        condition,
        then_block: left_arm,
        else_block: right_arm,
    };
    let jump_to_merge = HirTerminator::Jump {
        target: merge,
        args: vec![],
    };
    let read_view = expression(
        HirExpressionKind::Load(HirPlace::local(view)),
        view_type,
        region,
        ValueKind::Place,
        &mut hir.expressions,
    );
    hir.blocks.extend([
        HirBlock {
            id: left_arm,
            region,
            locals: vec![],
            statements: vec![left_definition],
            terminator: jump_to_merge.clone(),
        },
        HirBlock {
            id: right_arm,
            region,
            locals: vec![],
            statements: vec![right_definition],
            terminator: jump_to_merge,
        },
        HirBlock {
            id: merge,
            region,
            locals: vec![],
            statements: vec![statement(90_004, HirStatementKind::Expr(read_view), 4)],
            terminator: exit_terminator,
        },
    ]);

    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("reading a view of either root is legal");

    let merge_view = report.analysis.block_entry_states[&merge]
        .locals
        .iter()
        .find(|snapshot| snapshot.local == view)
        .expect("merge entry snapshot should include the view");
    assert!(merge_view.alias_roots.contains(&left));
    assert!(merge_view.alias_roots.contains(&right));

    // Entry, both arms and the merge: the grown merge input is visited once, not once per arm.
    let summary = &report.analysis.function_summaries[&start];
    assert_eq!(summary.reachable_blocks, 4);
    assert_eq!(summary.worklist_iterations, 4);
}
