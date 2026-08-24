//! Borrow-checker scope and nesting regression tests.
//!
//! WHAT: validates how lexical scopes and nested blocks constrain borrow visibility and drops.
//! WHY: scope boundaries drive many lifetime rules, so regressions here tend to cascade widely.

use crate::compiler_frontend::ast::ast_nodes::{MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::compiler_messages::BorrowDiagnosticKind;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::builtin_type_ids::BOOL;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::{HirNodeId, HirValueId};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, function_node, immutable_reference_expr, make_test_variable, node, symbol,
    test_if_branch_metadata, test_source_location,
};
use crate::compiler_frontend::tests::borrow_fixture_support::{
    assert_borrow_error_kind, run_borrow_checker,
};
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::{entry_and_start, lower_hir};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    build_ast_with_registered_types, runtime_expr, runtime_operand_item,
};

use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn if_branch_local_alias_does_not_escape_merge() {
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
                    Expression::int(1, test_source_location(1), ValueMode::MutableOwned),
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::If(
                    runtime_expr(
                        vec![runtime_operand_item(Expression::bool(
                            true,
                            test_source_location(2),
                            ValueMode::ImmutableOwned,
                        ))],
                        BOOL,
                        test_source_location(2),
                        ValueMode::ImmutableOwned,
                    ),
                    vec![node(
                        NodeKind::VariableDeclaration(make_test_variable(
                            y,
                            immutable_reference_expr(
                                x.clone(),
                                DataType::Int,
                                BOOL,
                                test_source_location(3),
                            ),
                        )),
                        test_source_location(3),
                    )],
                    Some(vec![]),
                    test_if_branch_metadata(true),
                ),
                test_source_location(2),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(x, DataType::Int, BOOL, test_source_location(4)),
                    value: Expression::int(2, test_source_location(4), ValueMode::ImmutableOwned),
                },
                test_source_location(4),
            ),
        ],
        test_source_location(1),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("branch-local alias should not be visible after merge");
}

#[test]
fn match_arm_local_alias_does_not_escape_merge() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let x = symbol("x", &mut string_table);
    let y = symbol("y", &mut string_table);

    let arm = MatchArm {
        pattern: MatchPattern::Literal(Expression::int(
            1,
            test_source_location(3),
            ValueMode::ImmutableOwned,
        )),
        guard: None,
        body: vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                y,
                immutable_reference_expr(x.clone(), DataType::Int, BOOL, test_source_location(4)),
            )),
            test_source_location(4),
        )],
    };

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
                    Expression::int(1, test_source_location(1), ValueMode::MutableOwned),
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::Match {
                    scrutinee: Expression::int(
                        1,
                        test_source_location(2),
                        ValueMode::ImmutableOwned,
                    ),
                    arms: vec![arm],
                    default: Some(vec![]),
                    exhaustiveness: MatchExhaustiveness::HasDefault,
                },
                test_source_location(2),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(x, DataType::Int, BOOL, test_source_location(5)),
                    value: Expression::int(2, test_source_location(5), ValueMode::ImmutableOwned),
                },
                test_source_location(5),
            ),
        ],
        test_source_location(1),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("match-arm local alias should not be visible after merge");
}

#[test]
fn while_body_local_alias_does_not_escape_exit() {
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
                    Expression::int(1, test_source_location(1), ValueMode::MutableOwned),
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::WhileLoop(
                    Expression::bool(false, test_source_location(2), ValueMode::ImmutableOwned),
                    vec![node(
                        NodeKind::VariableDeclaration(make_test_variable(
                            y,
                            immutable_reference_expr(
                                x.clone(),
                                DataType::Int,
                                BOOL,
                                test_source_location(3),
                            ),
                        )),
                        test_source_location(3),
                    )],
                ),
                test_source_location(2),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(x, DataType::Int, BOOL, test_source_location(4)),
                    value: Expression::int(2, test_source_location(4), ValueMode::ImmutableOwned),
                },
                test_source_location(4),
            ),
        ],
        test_source_location(1),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("while-body local alias should not be visible in exit block");
}

#[test]
fn dead_local_access_reports_borrow_error() {
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
                    Expression::int(1, test_source_location(1), ValueMode::MutableOwned),
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::If(
                    runtime_expr(
                        vec![runtime_operand_item(Expression::bool(
                            true,
                            test_source_location(2),
                            ValueMode::ImmutableOwned,
                        ))],
                        BOOL,
                        test_source_location(2),
                        ValueMode::ImmutableOwned,
                    ),
                    vec![node(
                        NodeKind::VariableDeclaration(make_test_variable(
                            y.clone(),
                            immutable_reference_expr(
                                x.clone(),
                                DataType::Int,
                                BOOL,
                                test_source_location(3),
                            ),
                        )),
                        test_source_location(3),
                    )],
                    Some(vec![]),
                    test_if_branch_metadata(true),
                ),
                test_source_location(2),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(x, DataType::Int, BOOL, test_source_location(4)),
                    value: Expression::int(2, test_source_location(4), ValueMode::ImmutableOwned),
                },
                test_source_location(4),
            ),
        ],
        test_source_location(1),
    );

    let mut hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
    );

    let start = &hir.functions[hir
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry = &hir.blocks[start.entry.0 as usize];
    let (then_block, _) = match &entry.terminator {
        crate::compiler_frontend::hir::terminators::HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (*then_block, *else_block),
        other => panic!("expected if terminator, found {other:?}"),
    };

    let merge_block = match &hir.blocks[then_block.0 as usize].terminator {
        crate::compiler_frontend::hir::terminators::HirTerminator::Jump { target, .. } => *target,
        other => panic!("expected then jump, found {other:?}"),
    };

    let then_local = hir.blocks[then_block.0 as usize]
        .locals
        .iter()
        .find_map(|local| {
            hir.side_table
                .resolve_local_name(local.id, &string_table)
                .filter(|name| *name == y.name_str(&string_table).unwrap_or_default())
                .map(|_| local.clone())
        })
        .expect("then local should exist");

    let synthetic_value = HirExpression {
        id: HirValueId(77_001),
        kind: HirExpressionKind::Load(crate::compiler_frontend::hir::places::HirPlace::Local(
            then_local.id,
        )),
        ty: then_local.ty,
        value_kind: ValueKind::Place,
        region: hir.blocks[merge_block.0 as usize].region,
    };
    let synthetic_statement = HirStatement {
        id: HirNodeId(77_000),
        kind: HirStatementKind::Expr(synthetic_value),
        location: test_source_location(100),
    };
    hir.blocks[merge_block.0 as usize]
        .statements
        .insert(0, synthetic_statement.clone());
    hir.side_table
        .map_statement(&synthetic_statement.location, &synthetic_statement);
    hir.side_table.map_value(
        &synthetic_statement.location,
        HirValueId(77_001),
        &synthetic_statement.location,
    );

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("dead local access should fail");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::UseOfUninitializedLocal);
}
