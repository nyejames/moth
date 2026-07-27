//! HIR result-handling lowering regression tests.
//!
//! WHAT: checks how result propagation, multi-bind fallbacks, and error handlers lower into HIR
//!       control-flow and local bindings.
//! WHY: fallible handling spans expression and statement boundaries; targeted tests prevent
//!      regressions in error-path routing and fallback binding.

use crate::compiler_frontend::ast::ast_nodes::{MultiBindTargetKind, NodeKind};
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, FallibleExpressionHandling, FallibleHandling, Operator,
};
use crate::compiler_frontend::ast::expressions::expression_types::CatchErrorBinding;
use crate::compiler_frontend::ast::statements::fallible_handling::wrap_catch_expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::numeric::HirNumericOp;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node, test_location,
};

use crate::compiler_frontend::tests::type_id_fixture_support::{
    alias_candidates_return_slot, error_return_slot, fresh_success_returns, multi_bind_target,
    param_with_type_id, reference_expr, runtime_expr, runtime_handled_function_call_item,
    runtime_operand_item, runtime_operator_item, success_return_slot,
};
use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::hir::hir_builder::{build_ast, lower_ast};

#[test]
fn statement_result_propagation_with_unit_success_lowers_to_explicit_error_edge() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let can_fail_name = super::symbol("can_fail", &mut string_table);
    let location = test_location(1);

    let can_fail_function = function_node(
        can_fail_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![error_return_slot(builtin_type_ids::STRING)],
        },
        vec![node(
            NodeKind::ReturnError(Expression::string_slice(
                string_table.intern("boom"),
                location.clone(),
                ValueMode::ImmutableOwned,
            )),
            location.clone(),
        )],
        location.clone(),
    );

    let start_function = function_node(
        start_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![error_return_slot(builtin_type_ids::STRING)],
        },
        vec![node(
            NodeKind::ExpressionStatement(Expression::handled_fallible_function_call(
                can_fail_name,
                vec![],
                vec![],
                FallibleExpressionHandling::Propagate,
                location.clone(),
            )),
            location.clone(),
        )],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(vec![can_fail_function, start_function], entry_path),
        &mut string_table,
    )
    .expect("statement propagation lowering should succeed");

    let start_function = module
        .functions
        .iter()
        .find(|function| Some(function.id) == module.start_function)
        .expect("start function should exist");
    let start_entry = module
        .blocks
        .iter()
        .find(|block| block.id == start_function.entry)
        .expect("start entry block should exist");

    let HirTerminator::FallibleBranch {
        success_block: _,
        error_block,
        ..
    } = start_entry.terminator
    else {
        panic!("unit-success statement propagation should lower to FallibleBranch");
    };

    let error_block = module
        .blocks
        .iter()
        .find(|block| block.id == error_block)
        .expect("statement propagation should create an error block");
    assert!(
        matches!(error_block.terminator, HirTerminator::ReturnError(_)),
        "statement propagation error edge should return through the enclosing error slot"
    );
}

#[test]
fn direct_return_result_propagation_lowers_to_explicit_success_and_error_edges() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let can_fail_name = super::symbol("can_fail", &mut string_table);
    let forward_name = super::symbol("forward", &mut string_table);
    let location = test_location(3);

    let can_fail_function = function_node(
        can_fail_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::Return(vec![Expression::string_slice(
                string_table.intern("ok"),
                location.clone(),
                ValueMode::ImmutableOwned,
            )]),
            location.clone(),
        )],
        location.clone(),
    );

    let mut expression_types = TypeEnvironment::new();
    let propagated_call = Expression::handled_fallible_function_call_with_typed_arguments(
        can_fail_name,
        vec![],
        vec![builtin_type_ids::STRING],
        FallibleExpressionHandling::Propagate,
        &mut expression_types,
        location.clone(),
    );

    let forward_function = function_node(
        forward_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::Return(vec![propagated_call]),
            location.clone(),
        )],
        location.clone(),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location.clone())],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(
            vec![can_fail_function, forward_function, start_function],
            entry_path,
        ),
        &mut string_table,
    )
    .expect("direct return propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == &forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");

    let HirTerminator::FallibleBranch {
        success_block,
        error_block,
        ..
    } = forward_entry.terminator
    else {
        panic!("direct return propagation should lower to FallibleBranch");
    };

    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == success_block)
        .expect("direct return propagation should create a success block");
    let HirTerminator::ReturnSuccess(success_value) = &success_block.terminator else {
        panic!("success edge should return through ReturnSuccess");
    };
    assert!(
        matches!(
            success_value.kind,
            HirExpressionKind::FallibleUnwrapSuccess { .. }
        ),
        "success edge should unwrap the carrier success payload"
    );

    let error_block = module
        .blocks
        .iter()
        .find(|block| block.id == error_block)
        .expect("direct return propagation should create an error block");
    let HirTerminator::ReturnError(error_value) = &error_block.terminator else {
        panic!("error edge should return through ReturnError");
    };
    assert!(
        matches!(
            error_value.kind,
            HirExpressionKind::FallibleUnwrapError { .. }
        ),
        "error edge should unwrap the carrier error payload"
    );
}

#[test]
fn direct_return_result_propagation_allows_alias_success_return() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let source_name = super::symbol("source", &mut string_table);
    let source_input = super::symbol("input", &mut string_table);
    let forward_name = super::symbol("forward", &mut string_table);
    let forward_input = super::symbol("input", &mut string_table);
    let location = test_location(4);

    let source_function = function_node(
        source_name.clone(),
        FunctionSignature {
            parameters: vec![param_with_type_id(
                source_input.clone(),
                builtin_type_ids::STRING,
                false,
                location.clone(),
            )],
            returns: vec![
                alias_candidates_return_slot(vec![0], builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::Return(vec![reference_expr(
                source_input,
                builtin_type_ids::STRING,
                location.clone(),
                ValueMode::ImmutableReference,
            )]),
            location.clone(),
        )],
        location.clone(),
    );

    let mut expression_types = TypeEnvironment::new();
    let propagated_call = Expression::handled_fallible_function_call_with_typed_arguments(
        source_name,
        vec![CallArgument::positional(
            reference_expr(
                forward_input.clone(),
                builtin_type_ids::STRING,
                location.clone(),
                ValueMode::ImmutableReference,
            ),
            CallAccessMode::Shared,
            location.clone(),
        )],
        vec![builtin_type_ids::STRING],
        FallibleExpressionHandling::Propagate,
        &mut expression_types,
        location.clone(),
    );

    let forward_function = function_node(
        forward_name.clone(),
        FunctionSignature {
            parameters: vec![param_with_type_id(
                forward_input,
                builtin_type_ids::STRING,
                false,
                location.clone(),
            )],
            returns: vec![
                alias_candidates_return_slot(vec![0], builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::Return(vec![propagated_call]),
            location.clone(),
        )],
        location.clone(),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location.clone())],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(
            vec![source_function, forward_function, start_function],
            entry_path,
        ),
        &mut string_table,
    )
    .expect("direct alias-return propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == &forward_name)
        })
        .expect("forward function should exist");
    assert_eq!(
        forward_function.return_aliases,
        vec![Some(vec![0])],
        "forwarding through ! should preserve the declared alias success slot"
    );

    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");
    let HirTerminator::FallibleBranch { success_block, .. } = forward_entry.terminator else {
        panic!("alias-return propagation should lower to FallibleBranch");
    };

    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == success_block)
        .expect("alias-return propagation should create a success block");
    let HirTerminator::ReturnSuccess(success_value) = &success_block.terminator else {
        panic!("alias success edge should return through ReturnSuccess");
    };
    assert!(
        matches!(
            success_value.kind,
            HirExpressionKind::FallibleUnwrapSuccess { .. }
        ),
        "alias success edge should forward the unwrapped success payload"
    );
}

#[test]
fn declaration_result_propagation_assigns_unwrapped_success_on_success_edge() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let can_fail_name = super::symbol("can_fail", &mut string_table);
    let forward_name = super::symbol("forward", &mut string_table);
    let value_name = forward_name.join_str("value", &mut string_table);
    let location = test_location(4);

    let can_fail_function = function_node(
        can_fail_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::Return(vec![Expression::string_slice(
                string_table.intern("ok"),
                location.clone(),
                ValueMode::ImmutableOwned,
            )]),
            location.clone(),
        )],
        location.clone(),
    );

    let mut expression_types = TypeEnvironment::new();
    let propagated_call = Expression::handled_fallible_function_call_with_typed_arguments(
        can_fail_name,
        vec![],
        vec![builtin_type_ids::STRING],
        FallibleExpressionHandling::Propagate,
        &mut expression_types,
        location.clone(),
    );

    let forward_function = function_node(
        forward_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    value_name.clone(),
                    propagated_call,
                )),
                location.clone(),
            ),
            node(
                NodeKind::Return(vec![reference_expr(
                    value_name,
                    builtin_type_ids::STRING,
                    location.clone(),
                    ValueMode::ImmutableReference,
                )]),
                location.clone(),
            ),
        ],
        location.clone(),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location.clone())],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(
            vec![can_fail_function, forward_function, start_function],
            entry_path,
        ),
        &mut string_table,
    )
    .expect("declaration result propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == &forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");

    let HirTerminator::FallibleBranch {
        success_block,
        error_block,
        ..
    } = forward_entry.terminator
    else {
        panic!("declaration propagation should lower to FallibleBranch");
    };

    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == success_block)
        .expect("declaration propagation should create a success block");
    assert!(
        success_block.statements.iter().any(|statement| matches!(
            statement.kind,
            HirStatementKind::Assign {
                value: crate::compiler_frontend::hir::expressions::HirExpression {
                    kind: HirExpressionKind::FallibleUnwrapSuccess { .. },
                    ..
                },
                ..
            }
        )),
        "success edge should assign the unwrapped success payload to the declaration local"
    );

    let error_block = module
        .blocks
        .iter()
        .find(|block| block.id == error_block)
        .expect("declaration propagation should create an error block");
    assert!(
        matches!(error_block.terminator, HirTerminator::ReturnError(_)),
        "declaration propagation error edge should return through the enclosing error slot"
    );
}

#[test]
fn multi_bind_result_propagation_projects_tuple_slots_after_success_edge() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let pair_name = super::symbol("pair", &mut string_table);
    let forward_name = super::symbol("forward", &mut string_table);
    let first_id = forward_name.join_str("first", &mut string_table);
    let count_id = forward_name.join_str("count", &mut string_table);
    let location = test_location(6);

    let pair_function = function_node(
        pair_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                success_return_slot(builtin_type_ids::INT),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::Return(vec![
                Expression::string_slice(
                    string_table.intern("ok"),
                    location.clone(),
                    ValueMode::ImmutableOwned,
                ),
                Expression::int(2, location.clone(), ValueMode::ImmutableOwned),
            ]),
            location.clone(),
        )],
        location.clone(),
    );

    let mut expression_types = TypeEnvironment::new();
    let propagated_call = Expression::handled_fallible_function_call_with_typed_arguments(
        pair_name,
        vec![],
        vec![builtin_type_ids::STRING, builtin_type_ids::INT],
        FallibleExpressionHandling::Propagate,
        &mut expression_types,
        location.clone(),
    );

    let forward_function = function_node(
        forward_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                success_return_slot(builtin_type_ids::INT),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![
            node(
                NodeKind::MultiBind {
                    targets: vec![
                        multi_bind_target(
                            first_id.clone(),
                            builtin_type_ids::STRING,
                            ValueMode::ImmutableOwned,
                            MultiBindTargetKind::Declaration,
                            location.clone(),
                        ),
                        multi_bind_target(
                            count_id.clone(),
                            builtin_type_ids::INT,
                            ValueMode::ImmutableOwned,
                            MultiBindTargetKind::Declaration,
                            location.clone(),
                        ),
                    ],
                    value: propagated_call,
                },
                location.clone(),
            ),
            node(
                NodeKind::Return(vec![
                    reference_expr(
                        first_id,
                        builtin_type_ids::STRING,
                        location.clone(),
                        ValueMode::ImmutableReference,
                    ),
                    reference_expr(
                        count_id,
                        builtin_type_ids::INT,
                        location.clone(),
                        ValueMode::ImmutableReference,
                    ),
                ]),
                location.clone(),
            ),
        ],
        location.clone(),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location.clone())],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(
            vec![pair_function, forward_function, start_function],
            entry_path,
        ),
        &mut string_table,
    )
    .expect("multi-bind result propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == &forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");

    let HirTerminator::FallibleBranch { success_block, .. } = forward_entry.terminator else {
        panic!("multi-bind propagation should lower to FallibleBranch");
    };

    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == success_block)
        .expect("multi-bind propagation should create a success block");
    assert!(
        success_block.statements.iter().any(|statement| matches!(
            statement.kind,
            HirStatementKind::Assign {
                value: crate::compiler_frontend::hir::expressions::HirExpression {
                    kind: HirExpressionKind::FallibleUnwrapSuccess { .. },
                    ..
                },
                ..
            }
        )),
        "success edge should materialize the unwrapped tuple payload before projection"
    );

    let tuple_get_count = success_block
        .statements
        .iter()
        .filter(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::Assign {
                    value: crate::compiler_frontend::hir::expressions::HirExpression {
                        kind: HirExpressionKind::TupleGet { .. },
                        ..
                    },
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        tuple_get_count, 2,
        "multi-bind propagation should project both tuple slots from the success payload"
    );
}

#[test]
fn call_argument_result_propagation_lowers_before_outer_call() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let can_fail_name = super::symbol("can_fail", &mut string_table);
    let consume_name = super::symbol("consume", &mut string_table);
    let consume_input = consume_name.join_str("input", &mut string_table);
    let forward_name = super::symbol("forward", &mut string_table);
    let value_name = forward_name.join_str("value", &mut string_table);
    let location = test_location(8);

    let can_fail_function = function_node(
        can_fail_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::Return(vec![Expression::string_slice(
                string_table.intern("ok"),
                location.clone(),
                ValueMode::ImmutableOwned,
            )]),
            location.clone(),
        )],
        location.clone(),
    );

    let consume_function = function_node(
        consume_name.clone(),
        FunctionSignature {
            parameters: vec![param_with_type_id(
                consume_input.clone(),
                builtin_type_ids::STRING,
                false,
                location.clone(),
            )],
            returns: vec![success_return_slot(builtin_type_ids::STRING)],
        },
        vec![node(
            NodeKind::Return(vec![reference_expr(
                consume_input,
                builtin_type_ids::STRING,
                location.clone(),
                ValueMode::ImmutableReference,
            )]),
            location.clone(),
        )],
        location.clone(),
    );

    let mut expression_types = TypeEnvironment::new();
    let propagated_call = Expression::handled_fallible_function_call_with_typed_arguments(
        can_fail_name,
        vec![],
        vec![builtin_type_ids::STRING],
        FallibleExpressionHandling::Propagate,
        &mut expression_types,
        location.clone(),
    );
    let outer_call = Expression::function_call_with_typed_arguments(
        consume_name.clone(),
        vec![CallArgument::positional(
            propagated_call,
            CallAccessMode::Shared,
            location.clone(),
        )],
        vec![builtin_type_ids::STRING],
        &mut expression_types,
        location.clone(),
    );

    let forward_function = function_node(
        forward_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(value_name.clone(), outer_call)),
                location.clone(),
            ),
            node(
                NodeKind::Return(vec![reference_expr(
                    value_name,
                    builtin_type_ids::STRING,
                    location.clone(),
                    ValueMode::ImmutableReference,
                )]),
                location.clone(),
            ),
        ],
        location.clone(),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location.clone())],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(
            vec![
                can_fail_function,
                consume_function,
                forward_function,
                start_function,
            ],
            entry_path,
        ),
        &mut string_table,
    )
    .expect("call-argument propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == &forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");
    let HirTerminator::FallibleBranch { success_block, .. } = forward_entry.terminator else {
        panic!("call argument propagation should branch before the outer call");
    };
    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == success_block)
        .expect("call argument propagation should create a success block");

    let entry_calls_consume = forward_entry.statements.iter().any(|statement| {
        let HirStatementKind::Call {
            target: CallTarget::Local(function_id),
            ..
        } = statement.kind
        else {
            return false;
        };

        module
            .side_table
            .function_name_path(function_id)
            .is_some_and(|path| path == &consume_name)
    });
    let success_calls_consume = success_block.statements.iter().any(|statement| {
        let HirStatementKind::Call {
            target: CallTarget::Local(function_id),
            ..
        } = statement.kind
        else {
            return false;
        };

        module
            .side_table
            .function_name_path(function_id)
            .is_some_and(|path| path == &consume_name)
    });

    assert!(
        !entry_calls_consume && success_calls_consume,
        "the outer call must lower only after fallible argument propagation succeeds"
    );
}

#[test]
fn runtime_binary_result_propagation_lowers_before_operator() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let can_fail_name = super::symbol("can_fail", &mut string_table);
    let forward_name = super::symbol("forward", &mut string_table);
    let value_name = forward_name.join_str("value", &mut string_table);
    let location = test_location(9);

    let can_fail_function = function_node(
        can_fail_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::INT),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::Return(vec![Expression::int(
                41,
                location.clone(),
                ValueMode::ImmutableOwned,
            )]),
            location.clone(),
        )],
        location.clone(),
    );

    let runtime_value = runtime_expr(
        vec![
            runtime_handled_function_call_item(
                can_fail_name,
                vec![builtin_type_ids::INT],
                FallibleHandling::Propagate,
                location.clone(),
            ),
            runtime_operand_item(Expression::int(
                1,
                location.clone(),
                ValueMode::ImmutableOwned,
            )),
            runtime_operator_item(Operator::Add, location.clone()),
        ],
        builtin_type_ids::INT,
        location.clone(),
        ValueMode::MutableOwned,
    );

    let forward_function = function_node(
        forward_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::INT),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    value_name.clone(),
                    runtime_value,
                )),
                location.clone(),
            ),
            node(
                NodeKind::Return(vec![reference_expr(
                    value_name,
                    builtin_type_ids::INT,
                    location.clone(),
                    ValueMode::ImmutableReference,
                )]),
                location.clone(),
            ),
        ],
        location.clone(),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location.clone())],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(
            vec![can_fail_function, forward_function, start_function],
            entry_path,
        ),
        &mut string_table,
    )
    .expect("runtime binary propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == &forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");
    let HirTerminator::FallibleBranch { success_block, .. } = forward_entry.terminator else {
        panic!("runtime binary propagation should branch before the numeric operator");
    };
    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == success_block)
        .expect("runtime binary propagation should create a success block");

    let entry_has_numeric_op = forward_entry
        .statements
        .iter()
        .any(|statement| matches!(statement.kind, HirStatementKind::NumericOp { .. }));
    let success_has_add = success_block.statements.iter().any(|statement| {
        matches!(
            statement.kind,
            HirStatementKind::NumericOp {
                op: HirNumericOp::IntAdd,
                ..
            }
        )
    });

    assert!(
        !entry_has_numeric_op && success_has_add,
        "the numeric operator must lower only after fallible operand propagation succeeds"
    );
}

#[test]
fn return_bang_lowers_to_explicit_error_terminator() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let can_fail_name = super::symbol("can_fail", &mut string_table);
    let location = test_location(5);

    let can_fail_function = function_node(
        can_fail_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![error_return_slot(builtin_type_ids::STRING)],
        },
        vec![node(
            NodeKind::ReturnError(Expression::string_slice(
                string_table.intern("boom"),
                location.clone(),
                ValueMode::ImmutableOwned,
            )),
            location.clone(),
        )],
        location.clone(),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location.clone())],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(vec![can_fail_function, start_function], entry_path),
        &mut string_table,
    )
    .expect("return! lowering should succeed");

    let can_fail_function = module
        .functions
        .iter()
        .find(|function| Some(function.id) != module.start_function)
        .expect("fallible function should exist");
    let can_fail_entry = module
        .blocks
        .iter()
        .find(|block| block.id == can_fail_function.entry)
        .expect("fallible function entry block should exist");

    assert!(
        matches!(can_fail_entry.terminator, HirTerminator::ReturnError(_)),
        "return! should lower to a dedicated HIR error-return terminator"
    );
}

#[test]
fn fallible_success_return_lowers_to_explicit_success_terminator() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let can_succeed_name = super::symbol("can_succeed", &mut string_table);
    let location = test_location(7);

    let can_succeed_function = function_node(
        can_succeed_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::Return(vec![Expression::string_slice(
                string_table.intern("ok"),
                location.clone(),
                ValueMode::ImmutableOwned,
            )]),
            location.clone(),
        )],
        location.clone(),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location.clone())],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(vec![can_succeed_function, start_function], entry_path),
        &mut string_table,
    )
    .expect("fallible success return lowering should succeed");

    let can_succeed_function = module
        .functions
        .iter()
        .find(|function| Some(function.id) != module.start_function)
        .expect("fallible function should exist");
    let can_succeed_entry = module
        .blocks
        .iter()
        .find(|block| block.id == can_succeed_function.entry)
        .expect("fallible function entry block should exist");

    assert!(
        matches!(
            can_succeed_entry.terminator,
            HirTerminator::ReturnSuccess(_)
        ),
        "success return in a fallible function should lower to a dedicated HIR success terminator"
    );
}

#[test]
fn statement_catch_handler_lowering_builds_explicit_result_branching() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let can_fail_name = super::symbol("can_fail", &mut string_table);
    let location = test_location(10);
    let error_binding = start_name.join_str("err", &mut string_table);

    let can_fail_function = function_node(
        can_fail_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(
            NodeKind::ReturnError(Expression::string_slice(
                string_table.intern("boom"),
                location.clone(),
                ValueMode::ImmutableOwned,
            )),
            location.clone(),
        )],
        location.clone(),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::ExpressionStatement(wrap_catch_expression(
                Expression::handled_fallible_function_call(
                    can_fail_name,
                    vec![],
                    vec![builtin_type_ids::STRING],
                    FallibleExpressionHandling::Recover,
                    location.clone(),
                ),
                FallibleHandling::Handler {
                    error: Some(CatchErrorBinding { error_binding }),
                    body: vec![],
                },
                vec![],
            )),
            location.clone(),
        )],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(vec![can_fail_function, start_function], entry_path),
        &mut string_table,
    )
    .expect("catch-handler statement lowering should succeed");

    let saw_fallible_branch = module
        .blocks
        .iter()
        .any(|block| matches!(block.terminator, HirTerminator::FallibleBranch { .. }));
    assert!(
        saw_fallible_branch,
        "expected catch-handler lowering to branch with an explicit fallible HIR terminator"
    );

    let saw_err_unwrap_assign = module.blocks.iter().any(|block| {
        block.statements.iter().any(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::Assign {
                    value: crate::compiler_frontend::hir::expressions::HirExpression {
                        kind: HirExpressionKind::FallibleUnwrapError { .. },
                        ..
                    },
                    ..
                }
            )
        })
    });
    assert!(
        saw_err_unwrap_assign,
        "expected catch-handler lowering to unwrap and bind the err branch payload"
    );
}

#[test]
fn multi_bind_lowering_projects_tuple_slots_from_single_rhs_call() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let pair_name = super::symbol("pair", &mut string_table);
    let location = test_location(20);

    let pair_function = function_node(
        pair_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT, builtin_type_ids::STRING]),
        },
        vec![node(
            NodeKind::Return(vec![
                Expression::int(1, location.clone(), ValueMode::ImmutableOwned),
                Expression::string_slice(
                    string_table.intern("value"),
                    location.clone(),
                    ValueMode::ImmutableOwned,
                ),
            ]),
            location.clone(),
        )],
        location.clone(),
    );

    let left_id = start_name.join_str("left", &mut string_table);
    let right_id = start_name.join_str("right", &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::MultiBind {
                targets: vec![
                    multi_bind_target(
                        left_id,
                        builtin_type_ids::INT,
                        ValueMode::ImmutableOwned,
                        MultiBindTargetKind::Declaration,
                        location.clone(),
                    ),
                    multi_bind_target(
                        right_id,
                        builtin_type_ids::STRING,
                        ValueMode::ImmutableOwned,
                        MultiBindTargetKind::Declaration,
                        location.clone(),
                    ),
                ],
                value: Expression::function_call(
                    pair_name,
                    vec![],
                    vec![builtin_type_ids::INT, builtin_type_ids::STRING],
                    location.clone(),
                ),
            },
            location.clone(),
        )],
        location.clone(),
    );

    let (module, _type_environment) = lower_ast(
        build_ast(vec![pair_function, start_function], entry_path),
        &mut string_table,
    )
    .expect("multi-bind lowering should succeed");

    let call_count = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter(|statement| matches!(statement.kind, HirStatementKind::Call { .. }))
        .count();
    assert_eq!(
        call_count, 1,
        "multi-bind RHS call should lower exactly once"
    );

    let tuple_get_assignments = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::Assign {
                    value: crate::compiler_frontend::hir::expressions::HirExpression {
                        kind: HirExpressionKind::TupleGet { .. },
                        ..
                    },
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        tuple_get_assignments, 2,
        "multi-bind lowering should assign both tuple slots in order"
    );
}
