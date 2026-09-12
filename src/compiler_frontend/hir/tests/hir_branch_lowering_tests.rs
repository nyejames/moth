//! HIR branch (`if`/`else`) lowering regression tests.
//!
//! WHAT: checks how conditional branches lower into HIR blocks with `If` terminators and merge
//!       continuation blocks.
//! WHY: branch lowering constructs the core CFG diamond shape; errors here corrupt control flow
//!      and variable liveness across arms.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, Operator,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::{
    ProducedValues,
    types::{ValueBlock, ValueIfBlock},
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    fresh_success_returns, function_node, make_test_variable, node, param_with_type_id,
    reference_expr_with_type_id, test_if_branch_metadata,
};

use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::hir::hir_builder::{
    assert_no_placeholder_terminators, build_ast_with_registered_types, lower_ast,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    runtime_expr, runtime_function_call_item, runtime_operand_item, runtime_operator_item,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

fn blocks_with_user_function_call(module: &HirModule, function_id: FunctionId) -> Vec<BlockId> {
    module
        .blocks
        .iter()
        .filter_map(|block| {
            let has_call = block.statements.iter().any(|statement| {
                matches!(
                    statement.kind,
                    HirStatementKind::Call {
                        target: CallTarget::Local(target_id),
                        ..
                    } if target_id == function_id
                )
            });
            if has_call { Some(block.id) } else { None }
        })
        .collect()
}

fn value_block_result_assignment(
    module: &HirModule,
    block_id: BlockId,
) -> (LocalId, HirExpressionKind, BlockId) {
    let block = &module.blocks[block_id.0 as usize];
    let (result_local, value_kind) = block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Assign {
                target: HirPlace::Local(local),
                value,
            } => Some((*local, value.kind.clone())),
            _ => None,
        })
        .expect("value-block branch should assign a hidden result local");

    let merge_block = match block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("value-block branch should jump to the merge block"),
    };

    (result_local, value_kind, merge_block)
}

#[test]
fn lowers_if_to_then_else_merge_blocks() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let x = super::symbol("x", &mut path_fork, &mut string_table);
let y = super::symbol("y", &mut path_fork, &mut string_table);

let if_node = node(
    NodeKind::If(
        runtime_expr(
            vec![runtime_operand_item(Expression::bool(
                true,
                None,
                ValueMode::ImmutableOwned,
            ))],
            builtin_type_ids::BOOL,
            None,
            ValueMode::ImmutableOwned,
        ),
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                x,
                Expression::int(1, None, ValueMode::ImmutableOwned),
            )),
            None,
        )],
        Some(vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                y,
                Expression::int(2, None, ValueMode::ImmutableOwned),
            )),
            None,
        )]),
        test_if_branch_metadata(true),
    ),
    None,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![if_node],
    None,
);

let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
let (module, _type_environment) =
    lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

let start = super::start_function(&module);
let entry_block = &module.blocks[start.entry.0 as usize];

let (then_block, else_block) = match entry_block.terminator {
    HirTerminator::If {
        then_block,
        else_block,
        ..
    } => (then_block, else_block),
    _ => panic!("expected if terminator in entry block"),
};

assert!(matches!(
    module.blocks[then_block.0 as usize].terminator,
    HirTerminator::Jump { .. }
));
assert!(matches!(
    module.blocks[else_block.0 as usize].terminator,
    HirTerminator::Jump { .. }
)); }

#[test]
fn rejects_statically_decided_statement_if_at_hir_boundary() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![node(
        NodeKind::If(
            Expression::bool(true, location, ValueMode::ImmutableOwned),
            vec![],
            None,
            test_if_branch_metadata(false),
        ),
        location,
    )],
    None,
);

let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
lower_ast(ast, &mut string_table, &mut path_fork)
    .expect_err("Stage 4 must specialise statically decided statement `if` before HIR"); }

#[test]
fn short_circuit_and_keeps_rhs_call_off_always_run_path() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let rhs_name = super::symbol("rhs_and", &mut path_fork, &mut string_table);
let location = None;

let rhs_fn = function_node(
    rhs_name.clone(),
    FunctionSignature {
        parameters: vec![],
        returns: fresh_success_returns(vec![builtin_type_ids::BOOL]),
    },
    vec![node(
        NodeKind::Return(vec![Expression::bool(
            true,
            location,
            ValueMode::ImmutableOwned,
        )]),
        location,
    )],
    location,
);

let condition = runtime_expr(
    vec![
        runtime_operand_item(Expression::bool(false, location, ValueMode::ImmutableOwned)),
        runtime_function_call_item(rhs_name.clone(), vec![builtin_type_ids::BOOL], location),
        runtime_operator_item(Operator::And, location),
    ],
    builtin_type_ids::BOOL,
    location,
    ValueMode::MutableOwned,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![node(
        NodeKind::If(
            condition,
            vec![node(
                NodeKind::ExpressionStatement(Expression::int(
                    1,
                    location,
                    ValueMode::ImmutableOwned,
                )),
                location,
            )],
            None,
            test_if_branch_metadata(false),
        ),
        location,
    )],
    location,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![rhs_fn, start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("short-circuit and lowering should succeed");
let rhs_function_id = module
    .functions
    .iter()
    .find(|function| Some(function.id) != module.start_function)
    .expect("rhs function should be present")
    .id;
let start_function = super::start_function(&module);
let start_entry_index = start_function.entry.0 as usize;
let start_entry_block = &module.blocks[start_entry_index];

let (then_block, else_block) = match start_entry_block.terminator {
    HirTerminator::If {
        then_block,
        else_block,
        ..
    } => (then_block, else_block),
    _ => panic!("expected short-circuit dispatcher if terminator in entry block"),
};

assert!(
    start_entry_block
        .statements
        .iter()
        .all(|statement| !matches!(statement.kind, HirStatementKind::Call { .. })),
    "entry block should not eagerly execute rhs calls before short-circuit dispatch"
);

let call_blocks = blocks_with_user_function_call(&module, rhs_function_id);
assert_eq!(
    call_blocks.len(),
    1,
    "rhs function call should appear in exactly one guarded branch"
);
assert_eq!(call_blocks[0], then_block);
assert_ne!(call_blocks[0], else_block);

let rhs_branch_block = &module.blocks[then_block.0 as usize];
let short_branch_block = &module.blocks[else_block.0 as usize];
let rhs_merge_target = super::assert_branches_join_same_merge_block(
    &module,
    rhs_branch_block.id,
    short_branch_block.id,
);
let rhs_jump_args = super::assert_block_has_jump_args(&module, rhs_branch_block.id, 1);
let short_jump_args = super::assert_block_has_jump_args(&module, short_branch_block.id, 1);

super::assert_block_assigns_local(&module, rhs_branch_block.id, rhs_jump_args[0]);
super::assert_block_assigns_local(&module, short_branch_block.id, short_jump_args[0]);

let merge_block = &module.blocks[rhs_merge_target.0 as usize];
assert!(
    !merge_block.locals.is_empty(),
    "merge block should declare a destination local for branch arguments"
); }

#[test]
fn short_circuit_or_keeps_rhs_call_off_true_short_path() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let rhs_name = super::symbol("rhs_or", &mut path_fork, &mut string_table);
let location = None;

let rhs_fn = function_node(
    rhs_name.clone(),
    FunctionSignature {
        parameters: vec![],
        returns: fresh_success_returns(vec![builtin_type_ids::BOOL]),
    },
    vec![node(
        NodeKind::Return(vec![Expression::bool(
            false,
            location,
            ValueMode::ImmutableOwned,
        )]),
        location,
    )],
    location,
);

let condition = runtime_expr(
    vec![
        runtime_operand_item(Expression::bool(true, location, ValueMode::ImmutableOwned)),
        runtime_function_call_item(rhs_name.clone(), vec![builtin_type_ids::BOOL], location),
        runtime_operator_item(Operator::Or, location),
    ],
    builtin_type_ids::BOOL,
    location,
    ValueMode::MutableOwned,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![node(
        NodeKind::If(
            condition,
            vec![node(
                NodeKind::ExpressionStatement(Expression::int(
                    1,
                    location,
                    ValueMode::ImmutableOwned,
                )),
                location,
            )],
            None,
            test_if_branch_metadata(false),
        ),
        location,
    )],
    location,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![rhs_fn, start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("short-circuit or lowering should succeed");
let rhs_function_id = module
    .functions
    .iter()
    .find(|function| Some(function.id) != module.start_function)
    .expect("rhs function should be present")
    .id;
let start_function = super::start_function(&module);
let start_entry_index = start_function.entry.0 as usize;
let start_entry_block = &module.blocks[start_entry_index];

let (then_block, else_block) = match start_entry_block.terminator {
    HirTerminator::If {
        then_block,
        else_block,
        ..
    } => (then_block, else_block),
    _ => panic!("expected short-circuit dispatcher if terminator in entry block"),
};

let call_blocks = blocks_with_user_function_call(&module, rhs_function_id);
assert_eq!(
    call_blocks.len(),
    1,
    "rhs function call should appear in exactly one guarded branch"
);
assert_eq!(call_blocks[0], else_block);
assert_ne!(call_blocks[0], then_block);

let rhs_branch_block = &module.blocks[else_block.0 as usize];
let short_branch_block = &module.blocks[then_block.0 as usize];
let rhs_merge_target = super::assert_branches_join_same_merge_block(
    &module,
    rhs_branch_block.id,
    short_branch_block.id,
);
let rhs_jump_args = super::assert_block_has_jump_args(&module, rhs_branch_block.id, 1);
let short_jump_args = super::assert_block_has_jump_args(&module, short_branch_block.id, 1);

super::assert_block_assigns_local(&module, rhs_branch_block.id, rhs_jump_args[0]);
super::assert_block_assigns_local(&module, short_branch_block.id, short_jump_args[0]);

let merge_block = &module.blocks[rhs_merge_target.0 as usize];
assert!(
    !merge_block.locals.is_empty(),
    "merge block should declare a destination local for branch arguments"
); }

#[test]
fn short_circuit_place_rhs_materializes_copy_before_merge_assignment() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let lhs_name = super::symbol("lhs", &mut path_fork, &mut string_table);
let rhs_name = super::symbol("rhs", &mut path_fork, &mut string_table);
let location = None;

let condition = runtime_expr(
    vec![
        runtime_operand_item(reference_expr_with_type_id(
            lhs_name.clone(),
            builtin_type_ids::BOOL,
            location,
            ValueMode::ImmutableReference,
        )),
        runtime_operand_item(reference_expr_with_type_id(
            rhs_name.clone(),
            builtin_type_ids::BOOL,
            location,
            ValueMode::ImmutableReference,
        )),
        runtime_operator_item(Operator::And, location),
    ],
    builtin_type_ids::BOOL,
    location,
    ValueMode::MutableOwned,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                lhs_name,
                Expression::bool(false, location, ValueMode::ImmutableOwned),
            )),
            location,
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                rhs_name,
                Expression::bool(true, location, ValueMode::MutableOwned),
            )),
            location,
        ),
        node(
            NodeKind::If(
                condition,
                vec![node(
                    NodeKind::ExpressionStatement(Expression::int(
                        1,
                        location,
                        ValueMode::ImmutableOwned,
                    )),
                    location,
                )],
                None,
                test_if_branch_metadata(false),
            ),
            location,
        ),
    ],
    location,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("short-circuit place rhs lowering should succeed");
let start_function = super::start_function(&module);
let start_entry_block = &module.blocks[start_function.entry.0 as usize];
let (rhs_block, _short_block) = match start_entry_block.terminator {
    HirTerminator::If {
        then_block,
        else_block,
        ..
    } => (then_block, else_block),
    _ => panic!("expected short-circuit dispatcher if terminator in entry block"),
};

let rhs_branch_block = &module.blocks[rhs_block.0 as usize];
let rhs_jump_arg_local = match &rhs_branch_block.terminator {
    HirTerminator::Jump { args, .. } if args.len() == 1 => args[0],
    _ => panic!("rhs short-circuit branch should jump with one merge argument"),
};
let rhs_jump_arg_assignment =
    super::assert_block_assigns_local(&module, rhs_branch_block.id, rhs_jump_arg_local);

assert!(
    matches!(
        rhs_jump_arg_assignment.kind,
        HirExpressionKind::Copy(HirPlace::Local(_))
    ),
    "rhs place loads should be materialized as Copy before jump-argument assignment"
); }

#[test]
fn value_if_then_place_materializes_copy_before_hidden_result_assignment() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let left_name = super::symbol("left", &mut path_fork, &mut string_table);
let right_name = super::symbol("right", &mut path_fork, &mut string_table);
let result_name = super::symbol("result", &mut path_fork, &mut string_table);
let location = None;

let then_body = vec![node(
    NodeKind::ThenValue(ProducedValues {
        expressions: vec![reference_expr_with_type_id(
            left_name.clone(),
            builtin_type_ids::INT,
            location,
            ValueMode::ImmutableReference,
        )],
        span: location,
    }),
    location,
)];

let else_body = vec![node(
    NodeKind::ThenValue(ProducedValues {
        expressions: vec![reference_expr_with_type_id(
            right_name.clone(),
            builtin_type_ids::INT,
            location,
            ValueMode::ImmutableReference,
        )],
        span: location,
    }),
    location,
)];

let value_if_expression = Expression::new(
    ExpressionKind::ValueBlock {
        block: Box::new(ValueBlock::If(ValueIfBlock {
            condition: runtime_expr(
                vec![runtime_operand_item(Expression::bool(
                    true,
                    location,
                    ValueMode::ImmutableOwned,
                ))],
                builtin_type_ids::BOOL,
                location,
                ValueMode::ImmutableOwned,
            ),
            then_body,
            else_body,
            then_scope: entry_path.clone(),
            else_scope: entry_path.clone(),
            span: location,
            generic_request_ranges: Default::default(),
            result_type_ids: vec![builtin_type_ids::INT],
        })),
    },
    location,
    builtin_type_ids::INT,
    DataType::Inferred,
    ValueMode::ImmutableOwned,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                left_name,
                Expression::int(1, location, ValueMode::ImmutableOwned),
            )),
            location,
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                right_name,
                Expression::int(2, location, ValueMode::ImmutableOwned),
            )),
            location,
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(result_name, value_if_expression)),
            location,
        ),
    ],
    location,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("value-if place production should lower successfully");
let start_function = super::start_function(&module);
let entry_block = &module.blocks[start_function.entry.0 as usize];
let (then_block, else_block) = match entry_block.terminator {
    HirTerminator::If {
        then_block,
        else_block,
        ..
    } => (then_block, else_block),
    _ => panic!("expected value-if dispatcher terminator"),
};

let (then_result_local, then_value_kind, then_merge) =
    value_block_result_assignment(&module, then_block);
let (else_result_local, else_value_kind, else_merge) =
    value_block_result_assignment(&module, else_block);

assert_eq!(
    then_result_local, else_result_local,
    "both branches should write the shared hidden result local"
);
assert_eq!(
    then_merge, else_merge,
    "both branches should rejoin at the shared value-if merge block"
);
assert!(
    matches!(then_value_kind, HirExpressionKind::Copy(HirPlace::Local(_))),
    "then name should be copied before assigning the hidden result local"
);
assert!(
    matches!(else_value_kind, HirExpressionKind::Copy(HirPlace::Local(_))),
    "else name should be copied before assigning the hidden result local"
); }

#[test]
fn assertion_failure_uses_message_value_block_tail() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let condition_name = super::symbol("condition", &mut path_fork, &mut string_table);
let location = None;

let message_value = Expression::new(
    ExpressionKind::ValueBlock {
        block: Box::new(ValueBlock::If(ValueIfBlock {
            condition: reference_expr_with_type_id(
                condition_name.clone(),
                builtin_type_ids::BOOL,
                location,
                ValueMode::ImmutableReference,
            ),
            then_body: vec![node(
                NodeKind::ThenValue(ProducedValues {
                    expressions: vec![Expression::string_slice(
                        string_table.intern("then"),
                        location,
                        ValueMode::ImmutableOwned,
                    )],
                    span: location,
                }),
                location,
            )],
            else_body: vec![node(
                NodeKind::ThenValue(ProducedValues {
                    expressions: vec![Expression::string_slice(
                        string_table.intern("else"),
                        location,
                        ValueMode::ImmutableOwned,
                    )],
                    span: location,
                }),
                location,
            )],
            then_scope: entry_path.clone(),
            else_scope: entry_path.clone(),
            span: location,
            generic_request_ranges: Default::default(),
            result_type_ids: vec![builtin_type_ids::STRING],
        })),
    },
    location,
    builtin_type_ids::STRING,
    DataType::StringSlice,
    ValueMode::ImmutableOwned,
);
let mut option_type_environment =
    crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();
let option_string = option_type_environment.intern_option(builtin_type_ids::STRING);
let message = Expression::coerced(message_value, option_string);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![param_with_type_id(
            condition_name.clone(),
            builtin_type_ids::BOOL,
            false,
            location,
        )],
        returns: vec![],
    },
    vec![node(
        NodeKind::Assert {
            condition: reference_expr_with_type_id(
                condition_name,
                builtin_type_ids::BOOL,
                location,
                ValueMode::ImmutableReference,
            ),
            message,
        },
        location,
    )],
    location,
);

let mut ast = build_ast_with_registered_types(vec![start_fn], entry_path);
assert_eq!(
    ast.type_environment.intern_option(builtin_type_ids::STRING),
    option_string,
    "the fixture environment must register the same canonical String? type"
);
let (module, _type_environment) = lower_ast(ast, &mut string_table, &mut path_fork)
    .expect("assertion with a value-block message should lower");

let assertion_block = module
    .blocks
    .iter()
    .find(|block| matches!(block.terminator, HirTerminator::AssertFailure { .. }))
    .expect("lowering should produce an assertion failure terminator");
let message_evaluation = match &assertion_block.terminator {
    HirTerminator::AssertFailure {
        message_evaluation, ..
    } => *message_evaluation,
    _ => unreachable!(),
};
assert_eq!(message_evaluation, HirAssertionMessageEvaluation::Runtime);
assert!(
    module.blocks.iter().any(|block| {
        matches!(
            block.terminator,
            HirTerminator::Jump { target, .. } if target == assertion_block.id
        )
    }),
    "the message value-block branches must converge into the assertion failure tail"
);
let failure_entry = match module.blocks[0].terminator {
    HirTerminator::If { else_block, .. } => else_block,
    ref terminator => panic!("expected dynamic assertion branch, got {terminator:?}"),
};
assert!(
    failure_entry != assertion_block.id
        && matches!(
            module.blocks[failure_entry.0 as usize].terminator,
            HirTerminator::If { .. }
        ),
    "the dynamic assertion failure edge must remain the message value-block entry"
); }

#[test]
fn statically_true_assertion_elides_runtime_message_call_and_failure_edge() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let message_name = super::symbol("runtime_message", &mut path_fork, &mut string_table);
let location = None;

let message_fn = function_node(
    message_name.clone(),
    FunctionSignature {
        parameters: vec![],
        returns: fresh_success_returns(vec![builtin_type_ids::STRING]),
    },
    vec![node(
        NodeKind::Return(vec![Expression::string_slice(
            string_table.intern("message"),
            location,
            ValueMode::ImmutableOwned,
        )]),
        location,
    )],
    location,
);
let message = runtime_expr(
    vec![runtime_function_call_item(
        message_name,
        vec![builtin_type_ids::STRING],
        location,
    )],
    builtin_type_ids::STRING,
    location,
    ValueMode::MutableOwned,
);
let mut option_type_environment =
    crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();
let option_string = option_type_environment.intern_option(builtin_type_ids::STRING);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![node(
        NodeKind::Assert {
            condition: Expression::bool(true, location, ValueMode::ImmutableOwned),
            message: Expression::coerced(message, option_string),
        },
        location,
    )],
    location,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![message_fn, start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("statically true assertion should elide its runtime message");
let message_function_id = module
    .functions
    .iter()
    .find(|function| Some(function.id) != module.start_function)
    .expect("message helper should remain a separate function")
    .id;

assert!(
    blocks_with_user_function_call(&module, message_function_id).is_empty(),
    "a statically true assertion must not lower its message call"
);
assert!(
    module.blocks.iter().all(|block| {
        !matches!(
            block.terminator,
            HirTerminator::If { .. } | HirTerminator::AssertFailure { .. }
        )
    }),
    "a statically true assertion must not retain a failure CFG or assertion message"
); }

#[test]
fn statically_false_assertion_keeps_cfg_producing_message_before_terminal_failure() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let condition_name = super::symbol("message_condition", &mut path_fork, &mut string_table);
let location = None;

let message_value = Expression::new(
    ExpressionKind::ValueBlock {
        block: Box::new(ValueBlock::If(ValueIfBlock {
            condition: reference_expr_with_type_id(
                condition_name.clone(),
                builtin_type_ids::BOOL,
                location,
                ValueMode::ImmutableReference,
            ),
            then_body: vec![node(
                NodeKind::ThenValue(ProducedValues {
                    expressions: vec![Expression::string_slice(
                        string_table.intern("then"),
                        location,
                        ValueMode::ImmutableOwned,
                    )],
                    span: location,
                }),
                location,
            )],
            else_body: vec![node(
                NodeKind::ThenValue(ProducedValues {
                    expressions: vec![Expression::string_slice(
                        string_table.intern("else"),
                        location,
                        ValueMode::ImmutableOwned,
                    )],
                    span: location,
                }),
                location,
            )],
            then_scope: entry_path.clone(),
            else_scope: entry_path.clone(),
            span: location,
            generic_request_ranges: Default::default(),
            result_type_ids: vec![builtin_type_ids::STRING],
        })),
    },
    location,
    builtin_type_ids::STRING,
    DataType::StringSlice,
    ValueMode::ImmutableOwned,
);
let mut option_type_environment =
    crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();
let option_string = option_type_environment.intern_option(builtin_type_ids::STRING);
let message = Expression::coerced(message_value, option_string);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![param_with_type_id(
            condition_name,
            builtin_type_ids::BOOL,
            false,
            location,
        )],
        returns: vec![],
    },
    vec![node(
        NodeKind::Assert {
            condition: Expression::bool(false, location, ValueMode::ImmutableOwned),
            message,
        },
        location,
    )],
    location,
);

let mut ast = build_ast_with_registered_types(vec![start_fn], entry_path);
assert_eq!(
    ast.type_environment.intern_option(builtin_type_ids::STRING),
    option_string,
    "the fixture environment must register the same canonical String? type"
);
let (module, _type_environment) = lower_ast(ast, &mut string_table, &mut path_fork)
    .expect("statically false assertion with a value-block message should lower");

assert_no_placeholder_terminators(&module);
assert_eq!(
    module
        .blocks
        .iter()
        .filter(|block| matches!(block.terminator, HirTerminator::If { .. }))
        .count(),
    1,
    "the message CFG should remain, but the statically false assertion must not add a pass branch"
);
assert!(
    module
        .blocks
        .iter()
        .any(|block| matches!(block.terminator, HirTerminator::AssertFailure { .. })),
    "the message CFG must converge into a terminal assertion failure"
); }

#[test]
fn non_unit_function_with_terminal_if_does_not_report_fallthrough() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let chooser = super::symbol("chooser", &mut path_fork, &mut string_table);

let chooser_fn = function_node(
    chooser,
    FunctionSignature {
        parameters: vec![],
        returns: fresh_success_returns(vec![builtin_type_ids::INT]),
    },
    vec![node(
        NodeKind::If(
            runtime_expr(
                vec![runtime_operand_item(Expression::bool(
                    true,
                    None,
                    ValueMode::ImmutableOwned,
                ))],
                builtin_type_ids::BOOL,
                None,
                ValueMode::ImmutableOwned,
            ),
            vec![node(
                NodeKind::Return(vec![Expression::int(1, None, ValueMode::ImmutableOwned)]),
                None,
            )],
            Some(vec![node(
                NodeKind::Return(vec![Expression::int(2, None, ValueMode::ImmutableOwned)]),
                None,
            )]),
            test_if_branch_metadata(true),
        ),
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
    vec![],
    None,
);

let ast = build_ast_with_registered_types(vec![start_fn, chooser_fn], entry_path);
let (module, _type_environment) =
    lower_ast(ast, &mut string_table, &mut path_fork).expect("all-terminal if should not trigger fallthrough");

let chooser_block = &module.blocks[module.functions[1].entry.0 as usize];
assert!(matches!(chooser_block.terminator, HirTerminator::If { .. }));
assert_no_placeholder_terminators(&module); }
