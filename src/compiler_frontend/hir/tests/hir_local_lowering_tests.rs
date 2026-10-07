//! HIR local declaration lowering regression tests.
//!
//! WHAT: checks how variable declarations become HIR locals, including mutability, type lowering,
//!       and source-location mapping.
//! WHY: local metadata is the input to borrow analysis; drift here affects every ownership
//!      and lifetime check downstream.

use crate::compiler_frontend::ast::ast_nodes::{MultiBindTargetKind, NodeKind};
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, fresh_success_returns, function_node, make_test_variable, node,
    param_with_type_id, reference_expr_with_type_id,
};

use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::hir::hir_builder::{build_ast_with_registered_types, lower_ast};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::tests::type_id_fixture_support::multi_bind_target;

/// The authored (non-generated) local names a block owns, in declaration order.
///
/// WHAT: filters out lowering temporaries, which are an implementation detail of HIR
///       construction rather than part of a declaration's contract.
/// WHY: `!locals.is_empty()` passes for a lowering that emitted only a temporary and dropped
///      the authored binding entirely.
fn authored_local_names(
    module: &crate::compiler_frontend::hir::module::HirModule,
    block: &crate::compiler_frontend::hir::blocks::HirBlock,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Vec<String> {
    block
        .locals
        .iter()
        .filter_map(|local| {
            module
                .side_table
                .resolve_local_name(local.id, path_fork, string_table)
        })
        .filter(|name| !name.starts_with("__hir_tmp_"))
        .map(str::to_string)
        .collect()
}

#[test]
fn allocates_parameter_locals_and_binds_names() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);

    let body = vec![node(
        NodeKind::Return(vec![reference_expr_with_type_id(
            x,
            builtin_type_ids::INT,
            None,
            ValueMode::ImmutableReference,
        )]),
        None,
    )];

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(x, builtin_type_ids::INT, false, None)],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        body,
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

    let start_fn = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    assert_eq!(start_fn.params.len(), 1);

    // The function declares exactly one parameter and no other bindings, so the entry block
    // owns exactly one local. A non-empty check would also pass if lowering invented extras.
    let entry_block = &module.blocks[start_fn.entry.0 as usize];
    assert_eq!(
        authored_local_names(&module, entry_block, &path_fork, &string_table),
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
            .resolve_local_name(start_fn.params[0], &path_fork, &string_table),
        Some("x")
    );
}

#[test]
fn variable_declaration_emits_local_and_definition_write() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                x,
                Expression::int(42, None, ValueMode::ImmutableOwned),
            )),
            None,
        )],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

    let start_fn = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start_fn.entry.0 as usize];

    // One declaration lowers to exactly one local and exactly one definition write. `any` would
    // also pass for a lowering that emitted the write twice.
    assert_eq!(
        authored_local_names(&module, entry_block, &path_fork, &string_table),
        vec!["x".to_string()],
        "one declaration should lower to exactly one authored local"
    );
    // Lowering also writes through a temporary, so the contract is exactly one definition
    // whose target is the authored local — not "some write exists".
    let declared_local = entry_block
        .locals
        .iter()
        .find(|local| {
            module
                .side_table
                .resolve_local_name(local.id, &path_fork, &string_table)
                == Some("x")
        })
        .expect("the authored local should be declared");
    let definitions_of_x = entry_block
        .statements
        .iter()
        .filter(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(local),
                    ..
                } if *local == declared_local.id
            )
        })
        .count();
    assert_eq!(
        definitions_of_x, 1,
        "one initialised declaration should lower to one definition of that local"
    );
}

#[test]
fn immutable_and_mutable_place_initializers_keep_place_classification() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let immutable_source = super::symbol("immutable_source", &mut path_fork, &mut string_table);
    let mutable_source = super::symbol("mutable_source", &mut path_fork, &mut string_table);
    let immutable_copy = super::symbol("immutable_copy", &mut path_fork, &mut string_table);
    let mutable_copy = super::symbol("mutable_copy", &mut path_fork, &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![
                param_with_type_id(immutable_source, builtin_type_ids::INT, false, None),
                param_with_type_id(mutable_source, builtin_type_ids::INT, true, None),
            ],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    immutable_copy,
                    reference_expr_with_type_id(
                        immutable_source,
                        builtin_type_ids::INT,
                        None,
                        ValueMode::ImmutableReference,
                    ),
                )),
                None,
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    mutable_copy,
                    reference_expr_with_type_id(
                        mutable_source,
                        builtin_type_ids::INT,
                        None,
                        ValueMode::MutableReference,
                    ),
                )),
                None,
            ),
        ],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");
    let start = &module.functions[module.start_function.expect("start function").0 as usize];
    let entry = &module.blocks[start.entry.0 as usize];

    for (name, source, expected_mutability) in [
        ("immutable_copy", start.params[0], false),
        ("mutable_copy", start.params[1], true),
    ] {
        let local = entry
            .locals
            .iter()
            .find(|local| {
                module
                    .side_table
                    .resolve_local_name(local.id, &path_fork, &string_table)
                    == Some(name)
            })
            .expect("initializer declaration should own an authored local");
        assert_eq!(local.mutable, expected_mutability);
        let value = entry
            .statements
            .iter()
            .find_map(|statement| match &statement.kind {
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(target),
                    value,
                } if *target == local.id => Some(*value),
                _ => None,
            })
            .expect("initializer should define its authored local");
        let row = module.expressions.expression(value);
        assert_eq!(row.value_kind, ValueKind::Place);
        assert!(matches!(
            &row.kind,
            HirExpressionKind::Load(place)
                if place.root == source && super::is_local_place(place)
        ));
    }
}

#[test]
fn duplicate_local_declarations_in_same_scope_fail() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let var_name = super::symbol("my_var", &mut path_fork, &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    var_name,
                    Expression::int(1, None, ValueMode::ImmutableOwned),
                )),
                None,
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    var_name,
                    Expression::int(2, None, ValueMode::ImmutableOwned),
                )),
                None,
            ),
        ],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let error = lower_ast(ast, &mut string_table, &mut path_fork)
        .expect_err("duplicate symbol should fail");
    let error = error
        .infrastructure_error()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert!(
        error
            .msg
            .contains("Local 'my_var' is already declared in this function scope")
    );
}

#[test]
fn assignment_lowers_value_prelude_before_assign() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let helper = super::symbol("helper", &mut path_fork, &mut string_table);

    let helper_fn = function_node(
        helper,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Return(vec![Expression::int(1, None, ValueMode::ImmutableOwned)]),
            None,
        )],
        None,
    );

    let assignment = node(
        NodeKind::Assignment {
            target: assignment_target(x, DataType::Int, builtin_type_ids::INT, None),
            value: Expression::function_call(helper, vec![], vec![builtin_type_ids::INT], None),
        },
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(x, builtin_type_ids::INT, true, None)],
            returns: vec![],
        },
        vec![assignment],
        None,
    );

    let ast = build_ast_with_registered_types(vec![helper_fn, start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

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
    let HirStatementKind::Call {
        result: Some(HirLocalDestination::Define(call_result)),
        ..
    } = &block.statements[call_pos].kind
    else {
        panic!("caller call results should define a fresh local binding");
    };
    let call_result = *call_result;
    let assign_pos = block
        .statements
        .iter()
        .rposition(|statement| matches!(&statement.kind, HirStatementKind::Write { .. }))
        .expect("entry block should contain a Write statement");
    assert!(
        call_pos < assign_pos,
        "Call prelude must precede the final Write"
    );
    assert!(matches!(
        &block.statements[assign_pos].kind,
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(place),
            ..
        } if place.root == start.params[0] && super::is_local_place(place)
    ));
    let HirStatementKind::Write { value, .. } = &block.statements[assign_pos].kind else {
        unreachable!("the assignment position was selected from Write statements");
    };
    let value = module.expressions.expression(*value);
    assert_eq!(value.value_kind, ValueKind::RValue);
    assert!(matches!(
        &value.kind,
        HirExpressionKind::Load(place)
            if place.root == call_result && super::is_local_place(place)
    ));
}

#[test]
fn call_expression_statements_materialize_result_values() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let callee = super::symbol("callee", &mut path_fork, &mut string_table);
    let alloc_id = ExternalFunctionId::Synthetic(0);

    let callee_fn = function_node(
        callee,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Return(vec![Expression::int(9, None, ValueMode::ImmutableOwned)]),
            None,
        )],
        None,
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
                    None,
                )),
                None,
            ),
            node(
                NodeKind::ExpressionStatement(Expression::host_function_call_with_arguments(
                    alloc_id,
                    vec![CallArgument::positional(
                        Expression::int(1, None, ValueMode::ImmutableOwned),
                        CallAccessMode::Shared,
                        None,
                    )],
                    vec![builtin_type_ids::INT],
                    None,
                )),
                None,
            ),
        ],
        None,
    );

    let ast = build_ast_with_registered_types(vec![callee_fn, start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

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
        call_results
            .iter()
            .all(|result| matches!(result, Some(HirLocalDestination::Define(_)))),
        "non-unit call expression statements should define their result before it is discarded"
    );
}

#[test]
fn multi_bind_assignment_reads_every_rhs_slot_before_updating_targets() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let pair_name = super::symbol("pair", &mut path_fork, &mut string_table);
    let left = super::symbol("left", &mut path_fork, &mut string_table);
    let right = super::symbol("right", &mut path_fork, &mut string_table);

    let pair_function = function_node(
        pair_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT, builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Return(vec![
                Expression::int(1, None, ValueMode::ImmutableOwned),
                Expression::int(2, None, ValueMode::ImmutableOwned),
            ]),
            None,
        )],
        None,
    );
    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![
                param_with_type_id(left, builtin_type_ids::INT, true, None),
                param_with_type_id(right, builtin_type_ids::INT, true, None),
            ],
            returns: vec![],
        },
        vec![node(
            NodeKind::MultiBind {
                targets: vec![
                    multi_bind_target(
                        left,
                        builtin_type_ids::INT,
                        ValueMode::MutableOwned,
                        MultiBindTargetKind::Assignment,
                        None,
                    ),
                    multi_bind_target(
                        right,
                        builtin_type_ids::INT,
                        ValueMode::MutableOwned,
                        MultiBindTargetKind::Assignment,
                        None,
                    ),
                ],
                value: Expression::function_call(
                    pair_name,
                    vec![],
                    vec![builtin_type_ids::INT, builtin_type_ids::INT],
                    None,
                ),
            },
            None,
        )],
        None,
    );
    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(vec![pair_function, start_function], entry_path),
        &mut string_table,
        &mut path_fork,
    )
    .expect("multi-bind assignment lowering should succeed");

    let start = &module.functions[module.start_function.expect("start function").0 as usize];
    let block = &module.blocks[start.entry.0 as usize];
    let call_result = block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Call { result, .. } => *result,
            _ => None,
        });
    assert!(
        matches!(call_result, Some(HirLocalDestination::Define(_))),
        "multi-bind RHS call result should be defined as an RValue binding"
    );

    let tuple_reads = block
        .statements
        .iter()
        .enumerate()
        .filter_map(|(index, statement)| {
            let HirStatementKind::Write { value, .. } = &statement.kind else {
                return None;
            };
            matches!(
                &module.expressions.expression(*value).kind,
                HirExpressionKind::TupleGet { .. }
            )
            .then_some(index)
        })
        .collect::<Vec<_>>();
    assert_eq!(tuple_reads.len(), 2, "both RHS slots should be read once");
    let first_update = block
        .statements
        .iter()
        .position(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Write {
                    target: HirWriteTarget::AssignPlace(_),
                    ..
                }
            )
        })
        .expect("assignment-only multi-bind should update existing targets");
    assert!(
        tuple_reads.iter().all(|index| *index < first_update),
        "all tuple slots must be read before the first existing local is updated"
    );
    let updated_roots = block
        .statements
        .iter()
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(place),
                ..
            } => Some(place.root),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(updated_roots, start.params);
}

#[test]
fn mixed_multi_bind_preserves_definition_and_update_destination_tags() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let pair_name = super::symbol("pair", &mut path_fork, &mut string_table);
    let existing = super::symbol("existing", &mut path_fork, &mut string_table);
    let declared = path_fork
        .try_intern_child(start_name, string_table.intern("declared"))
        .expect("test path fits");

    let pair_function = function_node(
        pair_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT, builtin_type_ids::BOOL]),
        },
        vec![node(
            NodeKind::Return(vec![
                Expression::int(1, None, ValueMode::ImmutableOwned),
                Expression::bool(true, None, ValueMode::ImmutableOwned),
            ]),
            None,
        )],
        None,
    );
    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                existing,
                builtin_type_ids::BOOL,
                true,
                None,
            )],
            returns: vec![],
        },
        vec![node(
            NodeKind::MultiBind {
                targets: vec![
                    multi_bind_target(
                        declared,
                        builtin_type_ids::INT,
                        ValueMode::ImmutableOwned,
                        MultiBindTargetKind::Declaration,
                        None,
                    ),
                    multi_bind_target(
                        existing,
                        builtin_type_ids::BOOL,
                        ValueMode::MutableOwned,
                        MultiBindTargetKind::Assignment,
                        None,
                    ),
                ],
                value: Expression::function_call(
                    pair_name,
                    vec![],
                    vec![builtin_type_ids::INT, builtin_type_ids::BOOL],
                    None,
                ),
            },
            None,
        )],
        None,
    );
    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(vec![pair_function, start_function], entry_path),
        &mut string_table,
        &mut path_fork,
    )
    .expect("mixed multi-bind lowering should succeed");

    let start = &module.functions[module.start_function.expect("start function").0 as usize];
    let block = &module.blocks[start.entry.0 as usize];
    let declared_local = block
        .locals
        .iter()
        .find(|local| {
            module
                .side_table
                .resolve_local_name(local.id, &path_fork, &string_table)
                == Some("declared")
        })
        .expect("mixed multi-bind should create the declaration target")
        .id;

    let tuple_reads = block
        .statements
        .iter()
        .enumerate()
        .filter_map(|(index, statement)| {
            let HirStatementKind::Write { value, .. } = &statement.kind else {
                return None;
            };
            matches!(
                &module.expressions.expression(*value).kind,
                HirExpressionKind::TupleGet { .. }
            )
            .then_some(index)
        })
        .collect::<Vec<_>>();
    assert_eq!(tuple_reads.len(), 2);

    let writes = block
        .statements
        .iter()
        .enumerate()
        .filter_map(|(index, statement)| match &statement.kind {
            HirStatementKind::Write { target, .. }
                if matches!(target, HirWriteTarget::DefineLocal(local) if *local == declared_local)
                    || matches!(target, HirWriteTarget::AssignPlace(place) if place.root == start.params[0]) =>
            {
                Some((index, *target))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 2);
    assert!(matches!(
        writes[0].1,
        HirWriteTarget::DefineLocal(local) if local == declared_local
    ));
    assert!(matches!(
        writes[1].1,
        HirWriteTarget::AssignPlace(place) if place.root == start.params[0]
    ));
    assert!(tuple_reads.iter().all(|index| *index < writes[1].0));
}

#[test]
fn return_lowering_handles_zero_one_and_many_values() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let one_name = super::symbol("one", &mut path_fork, &mut string_table);
    let many_name = super::symbol("many", &mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let one_fn = function_node(
        one_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Return(vec![Expression::int(8, None, ValueMode::ImmutableOwned)]),
            None,
        )],
        None,
    );

    let many_fn = function_node(
        many_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::INT, builtin_type_ids::BOOL]),
        },
        vec![node(
            NodeKind::Return(vec![
                Expression::int(1, None, ValueMode::ImmutableOwned),
                Expression::bool(true, None, ValueMode::ImmutableOwned),
            ]),
            None,
        )],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn, one_fn, many_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

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
                &module.expressions.expression(*value).kind,
                HirExpressionKind::TupleConstruct { elements } if elements.len() == 0
            )
    ));

    let one_block = &module.blocks[module.functions[1].entry.0 as usize];
    assert!(matches!(
        &one_block.terminator,
        HirTerminator::Return(value)
            if matches!(
                &module.expressions.expression(*value).kind,
                HirExpressionKind::Int(8)
            )
    ));

    let many_block = &module.blocks[module.functions[2].entry.0 as usize];
    assert!(matches!(
        &many_block.terminator,
        HirTerminator::Return(value)
            if matches!(
                &module.expressions.expression(*value).kind,
                HirExpressionKind::TupleConstruct { elements } if elements.len() == 2
            )
    ));
}
