//! Borrow-checker scope and nesting regression tests.
//!
//! WHAT: validates how lexical scopes and nested blocks constrain borrow visibility and drops.
//! WHY: scope boundaries drive many lifetime rules, so regressions here tend to cascade widely.

use crate::compiler_frontend::ast::ast_nodes::{MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, PatternValue,
};
use crate::compiler_frontend::compiler_messages::BorrowDiagnosticKind;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::HirNodeId;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::source::SourceSpan;
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
use crate::compiler_frontend::tests::type_id_fixture_support::build_ast_with_registered_types;

use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn i32_expression(
    builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value: i64,
    span: Option<SourceSpan>,
    mode: ValueMode,
) -> ExprId {
    builder
        .fixed_scalar(
            const_values,
            FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range"),
            span,
            mode,
        )
        .expect("fixture scalar fits store")
        .id()
}

#[test]
fn if_branch_local_alias_does_not_escape_merge() {
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();

    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);
    let flag = symbol("flag", &mut path_fork, &mut string_table);

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
                    i32_expression(
                        &mut builder,
                        &mut const_values,
                        1,
                        test_source_location(1),
                        ValueMode::MutableOwned,
                    ),
                    &builder,
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    flag,
                    builder
                        .bool(
                            &mut const_values,
                            true,
                            test_source_location(1),
                            ValueMode::ImmutableOwned,
                        )
                        .expect("fixture flag fits store")
                        .id(),
                    &builder,
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::If(
                    immutable_reference_expr(
                        &mut builder,
                        flag,
                        builtin_type_ids::BOOL,
                        test_source_location(2),
                    ),
                    vec![node(
                        NodeKind::VariableDeclaration(make_test_variable(
                            y,
                            immutable_reference_expr(
                                &mut builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(3),
                            ),
                            &builder,
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
                    target: assignment_target(
                        &mut builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(4),
                    ),
                    value: i32_expression(
                        &mut builder,
                        &mut const_values,
                        2,
                        test_source_location(4),
                        ValueMode::ImmutableOwned,
                    ),
                },
                test_source_location(4),
            ),
        ],
        None,
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            builder,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("branch-local alias should not be visible after merge");
}

#[test]
fn match_arm_local_alias_does_not_escape_merge() {
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();

    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);

    let arm = MatchArm {
        pattern: MatchPattern::Literal({
            let value = i32_expression(
                &mut builder,
                &mut const_values,
                1,
                None,
                ValueMode::ImmutableOwned,
            );
            PatternValue::from_expression(builder.view(value).expect("fixture pattern value"))
                .expect("single-valued fixture pattern")
        }),
        guard: None,
        body: vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                y,
                immutable_reference_expr(
                    &mut builder,
                    x,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    test_source_location(4),
                ),
                &builder,
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
                    x,
                    i32_expression(
                        &mut builder,
                        &mut const_values,
                        1,
                        test_source_location(1),
                        ValueMode::MutableOwned,
                    ),
                    &builder,
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::Match {
                    scrutinee: i32_expression(
                        &mut builder,
                        &mut const_values,
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
                    target: assignment_target(
                        &mut builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(5),
                    ),
                    value: i32_expression(
                        &mut builder,
                        &mut const_values,
                        2,
                        test_source_location(5),
                        ValueMode::ImmutableOwned,
                    ),
                },
                test_source_location(5),
            ),
        ],
        None,
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            builder,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("match-arm local alias should not be visible after merge");
}

#[test]
fn while_body_local_alias_does_not_escape_exit() {
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();

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
                    i32_expression(
                        &mut builder,
                        &mut const_values,
                        1,
                        test_source_location(1),
                        ValueMode::MutableOwned,
                    ),
                    &builder,
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::WhileLoop(
                    builder
                        .bool(
                            &mut const_values,
                            false,
                            test_source_location(2),
                            ValueMode::ImmutableOwned,
                        )
                        .expect("fixture loop condition fits store")
                        .id(),
                    vec![node(
                        NodeKind::VariableDeclaration(make_test_variable(
                            y,
                            immutable_reference_expr(
                                &mut builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(3),
                            ),
                            &builder,
                        )),
                        test_source_location(3),
                    )],
                ),
                test_source_location(2),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        &mut builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(4),
                    ),
                    value: i32_expression(
                        &mut builder,
                        &mut const_values,
                        2,
                        test_source_location(4),
                        ValueMode::ImmutableOwned,
                    ),
                },
                test_source_location(4),
            ),
        ],
        None,
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            builder,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("while-body local alias should not be visible in exit block");
}

#[test]
fn dead_local_access_reports_borrow_error() {
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();

    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);
    let flag = symbol("flag", &mut path_fork, &mut string_table);

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
                    i32_expression(
                        &mut builder,
                        &mut const_values,
                        1,
                        test_source_location(1),
                        ValueMode::MutableOwned,
                    ),
                    &builder,
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    flag,
                    builder
                        .bool(
                            &mut const_values,
                            true,
                            test_source_location(1),
                            ValueMode::ImmutableOwned,
                        )
                        .expect("fixture flag fits store")
                        .id(),
                    &builder,
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::If(
                    immutable_reference_expr(
                        &mut builder,
                        flag,
                        builtin_type_ids::BOOL,
                        test_source_location(2),
                    ),
                    vec![node(
                        NodeKind::VariableDeclaration(make_test_variable(
                            y,
                            immutable_reference_expr(
                                &mut builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(3),
                            ),
                            &builder,
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
                    target: assignment_target(
                        &mut builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(4),
                    ),
                    value: i32_expression(
                        &mut builder,
                        &mut const_values,
                        2,
                        test_source_location(4),
                        ValueMode::ImmutableOwned,
                    ),
                },
                test_source_location(4),
            ),
        ],
        None,
    );

    let mut hir = lower_hir(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            builder,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
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
                .resolve_local_name(local.id, &path_fork, &string_table)
                .filter(|name| {
                    path_fork
                        .component(y)
                        .is_some_and(|component| string_table.resolve(component) == *name)
                })
                .map(|_| local.clone())
        })
        .expect("then local should exist");

    let synthetic_value = crate::compiler_frontend::tests::hir_fixture_support::expression(
        HirExpressionKind::Load(HirPlace::local(then_local.id)),
        then_local.ty,
        hir.blocks[merge_block.0 as usize].region,
        ValueKind::Place,
        &mut hir.expressions,
    );
    let synthetic_statement = HirStatement {
        id: HirNodeId(77_000),
        kind: HirStatementKind::Expr(synthetic_value),
        span: None,
    };
    hir.blocks[merge_block.0 as usize]
        .statements
        .insert(0, synthetic_statement.clone());
    hir.side_table
        .map_statement(synthetic_statement.span, &synthetic_statement);
    hir.side_table
        .map_value(synthetic_statement.span, synthetic_value);

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("dead local access should fail");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::UseOfUninitializedLocal);
}
