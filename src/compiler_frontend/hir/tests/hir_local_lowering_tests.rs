//! HIR local declaration lowering regression tests.
//!
//! WHAT: checks how variable declarations become HIR locals, including mutability, type lowering,
//!       and source-location mapping.
//! WHY: local metadata is the input to borrow analysis; drift here affects every ownership
//!      and lifetime check downstream.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, function_node, make_test_variable, node, test_location,
};

use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::hir::hir_builder::{build_ast, lower_ast};

use crate::compiler_frontend::tests::type_id_fixture_support::{
    fresh_success_returns, param_with_type_id, reference_expr,
};

/// The authored (non-generated) local names a block owns, in declaration order.
///
/// WHAT: filters out lowering temporaries, which are an implementation detail of HIR
///       construction rather than part of a declaration's contract.
/// WHY: `!locals.is_empty()` passes for a lowering that emitted only a temporary and dropped
///      the authored binding entirely.
fn authored_local_names(
    module: &crate::compiler_frontend::hir::module::HirModule,
    block: &crate::compiler_frontend::hir::blocks::HirBlock,
    string_table: &StringTable,
) -> Vec<String> {
    block
        .locals
        .iter()
        .filter_map(|local| module.side_table.resolve_local_name(local.id, string_table))
        .filter(|name| !name.starts_with("__hir_tmp_"))
        .map(str::to_string)
        .collect()
}

#[test]
fn allocates_parameter_locals_and_binds_names() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

    let body = vec![node(
        NodeKind::Return(vec![reference_expr(
            x.clone(),
            builtin_type_ids::INT,
            test_location(3),
            ValueMode::ImmutableReference,
        )]),
        test_location(3),
    )];

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::INT,
                false,
                test_location(2),
            )],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        body,
        test_location(2),
    );

    let ast = build_ast(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start_fn = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    assert_eq!(start_fn.params.len(), 1);

    // The function declares exactly one parameter and no other bindings, so the entry block
    // owns exactly one local. A non-empty check would also pass if lowering invented extras.
    let entry_block = &module.blocks[start_fn.entry.0 as usize];
    assert_eq!(
        authored_local_names(&module, entry_block, &string_table),
        vec!["x".to_string()],
        "the entry block should own exactly the declared parameter besides lowering temporaries"
    );
    assert_eq!(
        entry_block
            .locals
            .iter()
            .filter(|local| local.id == start_fn.params[0])
            .count(),
        1,
        "the parameter should be declared once in the entry block"
    );
    assert_eq!(
        module
            .side_table
            .resolve_local_name(start_fn.params[0], &string_table),
        Some("x")
    );
}

#[test]
fn variable_declaration_emits_local_and_assign_statement() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                x,
                Expression::int(42, test_location(4), ValueMode::ImmutableOwned),
            )),
            test_location(4),
        )],
        test_location(3),
    );

    let ast = build_ast(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start_fn = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start_fn.entry.0 as usize];

    // One declaration lowers to exactly one local and exactly one assignment. `any` would also
    // pass for a lowering that emitted the assignment twice.
    assert_eq!(
        authored_local_names(&module, entry_block, &string_table),
        vec!["x".to_string()],
        "one declaration should lower to exactly one authored local"
    );
    // Lowering also assigns through a temporary, so the contract is exactly one assignment
    // whose target is the authored local — not "some assignment exists".
    let declared_local = entry_block
        .locals
        .iter()
        .find(|local| {
            module
                .side_table
                .resolve_local_name(local.id, &string_table)
                == Some("x")
        })
        .expect("the authored local should be declared");
    let assignments_to_x = entry_block
        .statements
        .iter()
        .filter(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::Assign {
                    target: crate::compiler_frontend::hir::places::HirPlace::Local(local),
                    ..
                } if local == declared_local.id
            )
        })
        .count();
    assert_eq!(
        assignments_to_x, 1,
        "one initialised declaration should lower to one assignment to that local"
    );
}

#[test]
fn duplicate_local_declarations_in_same_scope_fail() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let var_name = super::symbol("my_var", &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    var_name.clone(),
                    Expression::int(1, test_location(2), ValueMode::ImmutableOwned),
                )),
                test_location(2),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    var_name.clone(),
                    Expression::int(2, test_location(3), ValueMode::ImmutableOwned),
                )),
                test_location(3),
            ),
        ],
        test_location(1),
    );

    let ast = build_ast(vec![start_function], entry_path);
    let error = lower_ast(ast, &mut string_table).expect_err("duplicate symbol should fail");
    let (_error_type, message, _location) = error
        .first_infrastructure_error_for_tests()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert!(message.contains("Local 'my_var' is already declared in this function scope"));
}

#[test]
fn assignment_lowers_value_prelude_before_assign() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);
    let helper = super::symbol("helper", &mut string_table);

    let helper_fn = function_node(
        helper.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Return(vec![Expression::int(
                1,
                test_location(1),
                ValueMode::ImmutableOwned,
            )]),
            test_location(1),
        )],
        test_location(1),
    );

    let assignment = node(
        NodeKind::Assignment {
            target: assignment_target(
                x.clone(),
                DataType::Int,
                builtin_type_ids::INT,
                test_location(5),
            ),
            value: Expression::function_call(
                helper,
                vec![],
                vec![builtin_type_ids::INT],
                test_location(5),
            ),
        },
        test_location(5),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::INT,
                true,
                test_location(4),
            )],
            returns: vec![],
        },
        vec![assignment],
        test_location(4),
    );

    let ast = build_ast(vec![helper_fn, start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let block = &module.blocks[start.entry.0 as usize];

    let call_pos = block
        .statements
        .iter()
        .position(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Call {
                    result: Some(_),
                    ..
                }
            )
        })
        .expect("entry block should contain a Call statement with a result");
    let assign_pos = block
        .statements
        .iter()
        .rposition(|statement| matches!(&statement.kind, HirStatementKind::Assign { .. }))
        .expect("entry block should contain an Assign statement");
    assert!(
        call_pos < assign_pos,
        "Call prelude must precede the final Assign"
    );
}

#[test]
fn call_expression_statements_materialize_result_values() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let callee = super::symbol("callee", &mut string_table);
    let alloc_id = ExternalFunctionId::Synthetic(0);

    let callee_fn = function_node(
        callee.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Return(vec![Expression::int(
                9,
                test_location(1),
                ValueMode::ImmutableOwned,
            )]),
            test_location(1),
        )],
        test_location(1),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::ExpressionStatement(Expression::function_call_with_arguments(
                    callee,
                    vec![],
                    vec![builtin_type_ids::INT],
                    test_location(2),
                )),
                test_location(2),
            ),
            node(
                NodeKind::ExpressionStatement(Expression::host_function_call_with_arguments(
                    alloc_id,
                    vec![CallArgument::positional(
                        Expression::int(1, test_location(3), ValueMode::ImmutableOwned),
                        CallAccessMode::Shared,
                        test_location(3),
                    )],
                    vec![builtin_type_ids::INT],
                    test_location(3),
                )),
                test_location(3),
            ),
        ],
        test_location(2),
    );

    let ast = build_ast(vec![callee_fn, start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let block = &module.blocks[start.entry.0 as usize];

    let call_results = block
        .statements
        .iter()
        .filter_map(|statement| match statement.kind {
            HirStatementKind::Call { result, .. } => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(call_results.len(), 2);
    assert!(
        call_results.iter().all(Option::is_some),
        "non-unit call expression statements should materialize their result before it is discarded"
    );
}

#[test]
fn return_lowering_handles_zero_one_and_many_values() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let one_name = super::symbol("one", &mut string_table);
    let many_name = super::symbol("many", &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_location(1))],
        test_location(1),
    );

    let one_fn = function_node(
        one_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Return(vec![Expression::int(
                8,
                test_location(2),
                ValueMode::ImmutableOwned,
            )]),
            test_location(2),
        )],
        test_location(2),
    );

    let many_fn = function_node(
        many_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT, builtin_type_ids::BOOL]),
        },
        vec![node(
            NodeKind::Return(vec![
                Expression::int(1, test_location(3), ValueMode::ImmutableOwned),
                Expression::bool(true, test_location(3), ValueMode::ImmutableOwned),
            ]),
            test_location(3),
        )],
        test_location(3),
    );

    let ast = build_ast(vec![start_fn, one_fn, many_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start_block = &module.blocks[module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize];
    assert!(matches!(
        &start_block.terminator,
        HirTerminator::Return(value)
            if matches!(
                &value.kind,
                HirExpressionKind::TupleConstruct { elements } if elements.is_empty()
            )
    ));

    let one_block = &module.blocks[module.functions[1].entry.0 as usize];
    assert!(matches!(
        &one_block.terminator,
        HirTerminator::Return(value)
            if matches!(&value.kind, HirExpressionKind::Int(8))
    ));

    let many_block = &module.blocks[module.functions[2].entry.0 as usize];
    assert!(matches!(
        &many_block.terminator,
        HirTerminator::Return(value)
            if matches!(
                &value.kind,
                HirExpressionKind::TupleConstruct { elements } if elements.len() == 2
            )
    ));
}
