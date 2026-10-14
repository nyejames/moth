//! HIR branch (`if`/`else`) lowering regression tests.
//!
//! WHAT: checks how conditional branches lower into HIR blocks with `If` terminators and merge
//!       continuation blocks.
//! WHY: branch lowering constructs the core CFG diamond shape; errors here corrupt control flow
//!      and variable liveness across arms.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, SuccessShape,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::{
    ProducedValues,
    types::{ValueBlock, ValueIfBlock},
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    fresh_success_returns, function_node, make_test_variable, node, param_with_type_id,
    reference_expr_with_type_id, test_if_branch_metadata,
};

use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use crate::compiler_frontend::hir::hir_builder::{
    assert_no_placeholder_terminators, build_ast_with_registered_types, lower_ast,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    runtime_expr, runtime_function_call_item, runtime_operand_item, runtime_operator_item,
};

/// One exact `I32` literal expression.
///
/// WHY: these fixtures pin the exact `I32` width explicitly at the scalar carrier.
fn i32_literal(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    let value = FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32");
    builder
        .fixed_scalar(constants, value, span, value_mode)
        .expect("fixture scalar")
        .id()
}

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

fn value_block_result_definition(
    module: &HirModule,
    block_id: BlockId,
) -> (LocalId, HirExpression, BlockId) {
    let block = &module.blocks[block_id.0 as usize];
    let (result_local, value_kind) = block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } => Some((*local, module.expressions.expression(*value).clone())),
            _ => None,
        })
        .expect("value-block branch should define a hidden result local");

    let merge_block = match block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("value-block branch should jump to the merge block"),
    };

    (result_local, value_kind, merge_block)
}

#[test]
fn lowers_if_to_then_else_merge_blocks() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let y = super::symbol("y", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let flag = super::symbol("flag", &mut path_fork, &mut string_table);
    let flag_value = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();
    let flag_decl = node(
        NodeKind::VariableDeclaration(make_test_variable(flag, flag_value, &builder)),
        None,
    );
    let condition = reference_expr_with_type_id(
        &mut builder,
        flag,
        builtin_type_ids::BOOL,
        None,
        ValueMode::ImmutableReference,
    );
    let x_value = i32_literal(
        &mut builder,
        &mut constants,
        1,
        None,
        ValueMode::ImmutableOwned,
    );
    let x_decl = make_test_variable(x, x_value, &builder);
    let y_value = i32_literal(
        &mut builder,
        &mut constants,
        2,
        None,
        ValueMode::ImmutableOwned,
    );
    let y_decl = make_test_variable(y, y_value, &builder);
    let if_node = node(
        NodeKind::If(
            condition,
            vec![node(NodeKind::VariableDeclaration(x_decl), None)],
            Some(vec![node(NodeKind::VariableDeclaration(y_decl), None)]),
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
        vec![flag_decl, if_node],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");

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
    ));
}

#[test]
fn rejects_statically_decided_statement_if_at_hir_boundary() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let condition = builder
        .bool(&mut constants, true, location, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::If(condition, vec![], None, test_if_branch_metadata(false)),
            location,
        )],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect_err("Stage 4 must specialise statically decided statement `if` before HIR");
}

#[test]
fn short_circuit_and_keeps_rhs_call_off_always_run_path() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let rhs_name = super::symbol("rhs_and", &mut path_fork, &mut string_table);
    let location = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let rhs_result = builder
        .bool(&mut constants, true, location, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();

    let rhs_fn = function_node(
        rhs_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::BOOL]),
        },
        vec![node(NodeKind::Return(vec![rhs_result]), location)],
        location,
    );

    let lhs_value = builder
        .bool(&mut constants, false, location, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();
    let rhs_call = runtime_function_call_item(
        &mut builder,
        rhs_name,
        vec![builtin_type_ids::BOOL],
        location,
    );
    let condition = runtime_expr(
        &mut builder,
        &mut constants,
        vec![
            runtime_operand_item(lhs_value),
            rhs_call,
            runtime_operator_item(Operator::And, location),
        ],
        builtin_type_ids::BOOL,
        location,
        ValueMode::MutableOwned,
        &TypeEnvironment::new(),
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
                    NodeKind::ExpressionStatement(i32_literal(
                        &mut builder,
                        &mut constants,
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

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![rhs_fn, start_fn],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
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

    super::assert_block_writes_local(&module, rhs_branch_block.id, rhs_jump_args[0].source);
    super::assert_block_writes_local(&module, short_branch_block.id, short_jump_args[0].source);

    let merge_block = &module.blocks[rhs_merge_target.0 as usize];
    assert!(
        !merge_block.locals.is_empty(),
        "merge block should declare a destination local for branch arguments"
    );
    assert!(
        merge_block
            .locals
            .iter()
            .any(|local| local.id == rhs_jump_args[0].destination)
    );
    assert!(
        merge_block
            .locals
            .iter()
            .any(|local| local.id == short_jump_args[0].destination)
    );
    assert_eq!(rhs_jump_args[0].destination, short_jump_args[0].destination);
}

#[test]
fn short_circuit_or_keeps_rhs_call_off_true_short_path() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let rhs_name = super::symbol("rhs_or", &mut path_fork, &mut string_table);
    let location = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let rhs_result = builder
        .bool(&mut constants, false, location, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();

    let rhs_fn = function_node(
        rhs_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::BOOL]),
        },
        vec![node(NodeKind::Return(vec![rhs_result]), location)],
        location,
    );

    let lhs_value = builder
        .bool(&mut constants, true, location, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();
    let rhs_call = runtime_function_call_item(
        &mut builder,
        rhs_name,
        vec![builtin_type_ids::BOOL],
        location,
    );
    let condition = runtime_expr(
        &mut builder,
        &mut constants,
        vec![
            runtime_operand_item(lhs_value),
            rhs_call,
            runtime_operator_item(Operator::Or, location),
        ],
        builtin_type_ids::BOOL,
        location,
        ValueMode::MutableOwned,
        &TypeEnvironment::new(),
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
                    NodeKind::ExpressionStatement(i32_literal(
                        &mut builder,
                        &mut constants,
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

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![rhs_fn, start_fn],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
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

    super::assert_block_writes_local(&module, rhs_branch_block.id, rhs_jump_args[0].source);
    super::assert_block_writes_local(&module, short_branch_block.id, short_jump_args[0].source);

    let merge_block = &module.blocks[rhs_merge_target.0 as usize];
    assert!(
        !merge_block.locals.is_empty(),
        "merge block should declare a destination local for branch arguments"
    );
    assert!(
        merge_block
            .locals
            .iter()
            .any(|local| local.id == rhs_jump_args[0].destination)
    );
    assert!(
        merge_block
            .locals
            .iter()
            .any(|local| local.id == short_jump_args[0].destination)
    );
    assert_eq!(rhs_jump_args[0].destination, short_jump_args[0].destination);
}

#[test]
fn short_circuit_place_rhs_materializes_value_load_before_edge_transfer() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let lhs_name = super::symbol("lhs", &mut path_fork, &mut string_table);
    let rhs_name = super::symbol("rhs", &mut path_fork, &mut string_table);
    let location = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let lhs_ref = reference_expr_with_type_id(
        &mut builder,
        lhs_name,
        builtin_type_ids::BOOL,
        location,
        ValueMode::ImmutableReference,
    );
    let rhs_ref = reference_expr_with_type_id(
        &mut builder,
        rhs_name,
        builtin_type_ids::BOOL,
        location,
        ValueMode::ImmutableReference,
    );
    let condition = runtime_expr(
        &mut builder,
        &mut constants,
        vec![
            runtime_operand_item(lhs_ref),
            runtime_operand_item(rhs_ref),
            runtime_operator_item(Operator::And, location),
        ],
        builtin_type_ids::BOOL,
        location,
        ValueMode::MutableOwned,
        &TypeEnvironment::new(),
    );

    let lhs_init = builder
        .bool(&mut constants, false, location, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();
    let rhs_init = builder
        .bool(&mut constants, true, location, ValueMode::MutableOwned)
        .expect("fixture bool")
        .id();
    let lhs_decl = make_test_variable(lhs_name, lhs_init, &builder);
    let rhs_decl = make_test_variable(rhs_name, rhs_init, &builder);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(NodeKind::VariableDeclaration(lhs_decl), location),
            node(NodeKind::VariableDeclaration(rhs_decl), location),
            node(
                NodeKind::If(
                    condition,
                    vec![node(
                        NodeKind::ExpressionStatement(i32_literal(
                            &mut builder,
                            &mut constants,
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

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
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
    let rhs_local = start_entry_block
        .locals
        .iter()
        .find(|local| {
            module
                .side_table
                .resolve_local_name(local.id, &path_fork, &string_table)
                == Some("rhs")
        })
        .expect("rhs source local should be registered")
        .id;
    let rhs_jump_arg = match &rhs_branch_block.terminator {
        HirTerminator::Jump { args, .. } if args.len() == 1 => args[0],
        _ => panic!("rhs short-circuit branch should jump with one merge argument"),
    };
    let rhs_jump_arg_write =
        super::assert_block_writes_local(&module, rhs_branch_block.id, rhs_jump_arg.source);

    assert!(
        matches!(
            &rhs_jump_arg_write.kind,
            HirExpressionKind::Load(place)
                if place.root == rhs_local && super::is_local_place(place)
        ),
        "rhs place loads should produce a value-form Load before the edge transfer"
    );
    assert_eq!(rhs_jump_arg_write.value_kind, ValueKind::RValue);
}

#[test]
fn value_if_then_place_materializes_value_load_before_result_definition() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let left_name = super::symbol("left", &mut path_fork, &mut string_table);
    let right_name = super::symbol("right", &mut path_fork, &mut string_table);
    let result_name = super::symbol("result", &mut path_fork, &mut string_table);
    let location = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let then_value = reference_expr_with_type_id(
        &mut builder,
        left_name,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        location,
        ValueMode::ImmutableReference,
    );
    let then_body = vec![node(
        NodeKind::ThenValue(ProducedValues {
            expressions: vec![then_value],
            span: location,
        }),
        location,
    )];

    let else_value = reference_expr_with_type_id(
        &mut builder,
        right_name,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        location,
        ValueMode::ImmutableReference,
    );
    let else_body = vec![node(
        NodeKind::ThenValue(ProducedValues {
            expressions: vec![else_value],
            span: location,
        }),
        location,
    )];

    let flag = super::symbol("flag", &mut path_fork, &mut string_table);
    let flag_value = builder
        .bool(&mut constants, true, location, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();
    let flag_decl = node(
        NodeKind::VariableDeclaration(make_test_variable(flag, flag_value, &builder)),
        location,
    );
    let condition = reference_expr_with_type_id(
        &mut builder,
        flag,
        builtin_type_ids::BOOL,
        location,
        ValueMode::ImmutableReference,
    );
    let value_if_expression = builder
        .value_block(
            ValueBlock::If(ValueIfBlock {
                condition,
                then_body,
                else_body,
                then_scope: entry_path,
                else_scope: entry_path,
                span: location,
                generic_request_ranges: Default::default(),
                result_type_ids: vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            }),
            location,
            SuccessShape::One(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
            ValueMode::ImmutableOwned,
        )
        .expect("fixture value-if block")
        .id();

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            flag_decl,
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    left_name,
                    i32_literal(
                        &mut builder,
                        &mut constants,
                        1,
                        location,
                        ValueMode::ImmutableOwned,
                    ),
                    &builder,
                )),
                location,
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    right_name,
                    i32_literal(
                        &mut builder,
                        &mut constants,
                        2,
                        location,
                        ValueMode::ImmutableOwned,
                    ),
                    &builder,
                )),
                location,
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    result_name,
                    value_if_expression,
                    &builder,
                )),
                location,
            ),
        ],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
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
        value_block_result_definition(&module, then_block);
    let (else_result_local, else_value_kind, else_merge) =
        value_block_result_definition(&module, else_block);
    let authored_local = |name| {
        module.blocks[start_function.entry.0 as usize]
            .locals
            .iter()
            .find(|local| {
                module
                    .side_table
                    .resolve_local_name(local.id, &path_fork, &string_table)
                    == Some(name)
            })
            .expect("authored branch input local should be registered")
            .id
    };
    let left_local = authored_local("left");
    let right_local = authored_local("right");

    assert_ne!(
        then_result_local, else_result_local,
        "branches define their own result temporaries"
    );
    let then_args = super::assert_block_has_jump_args(&module, then_block, 1);
    let else_args = super::assert_block_has_jump_args(&module, else_block, 1);
    assert_eq!(
        then_args[0].destination, else_args[0].destination,
        "both branches transfer to the same merge slot"
    );
    assert_eq!(then_args[0].source, then_result_local);
    assert_eq!(else_args[0].source, else_result_local);
    assert_eq!(
        then_merge, else_merge,
        "both branches should rejoin at the shared value-if merge block"
    );
    assert!(
        matches!(&then_value_kind.kind, HirExpressionKind::Load(place) if place.root == left_local && super::is_local_place(place)),
        "then name should be loaded as a value before defining the hidden result local"
    );
    assert!(
        matches!(&else_value_kind.kind, HirExpressionKind::Load(place) if place.root == right_local && super::is_local_place(place)),
        "else name should be loaded as a value before defining the hidden result local"
    );
    assert_eq!(then_value_kind.value_kind, ValueKind::RValue);
    assert_eq!(else_value_kind.value_kind, ValueKind::RValue);
}

#[test]
fn assertion_failure_uses_message_value_block_tail() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let condition_name = super::symbol("condition", &mut path_fork, &mut string_table);
    let location = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let condition = reference_expr_with_type_id(
        &mut builder,
        condition_name,
        builtin_type_ids::BOOL,
        location,
        ValueMode::ImmutableReference,
    );
    let then_string = builder
        .string_slice(
            &mut constants,
            string_table.intern("then"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string slice")
        .id();
    let else_string = builder
        .string_slice(
            &mut constants,
            string_table.intern("else"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string slice")
        .id();
    let message_value = builder
        .value_block(
            ValueBlock::If(ValueIfBlock {
                condition,
                then_body: vec![node(
                    NodeKind::ThenValue(ProducedValues {
                        expressions: vec![then_string],
                        span: location,
                    }),
                    location,
                )],
                else_body: vec![node(
                    NodeKind::ThenValue(ProducedValues {
                        expressions: vec![else_string],
                        span: location,
                    }),
                    location,
                )],
                then_scope: entry_path,
                else_scope: entry_path,
                span: location,
                generic_request_ranges: Default::default(),
                result_type_ids: vec![builtin_type_ids::STRING],
            }),
            location,
            SuccessShape::One(builtin_type_ids::STRING),
            ValueMode::ImmutableOwned,
        )
        .expect("fixture value-block message")
        .id();
    let mut option_type_environment =
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();
    let option_string = option_type_environment.intern_option(builtin_type_ids::STRING);
    let message = builder
        .coerced(builder.own(message_value), option_string)
        .expect("fixture coercion")
        .id();

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
                condition: reference_expr_with_type_id(
                    &mut builder,
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

    let mut ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    assert_eq!(
        ast.type_environment.intern_option(builtin_type_ids::STRING),
        option_string,
        "the fixture environment must register the same canonical String? type"
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
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
    );
}

#[test]
fn statically_true_assertion_elides_runtime_message_call_and_failure_edge() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let message_name = super::symbol("runtime_message", &mut path_fork, &mut string_table);
    let location = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let message_string = builder
        .string_slice(
            &mut constants,
            string_table.intern("message"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string slice")
        .id();

    let message_fn = function_node(
        message_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::STRING]),
        },
        vec![node(NodeKind::Return(vec![message_string]), location)],
        location,
    );
    let message_call = runtime_function_call_item(
        &mut builder,
        message_name,
        vec![builtin_type_ids::STRING],
        location,
    );
    let message = runtime_expr(
        &mut builder,
        &mut constants,
        vec![message_call],
        builtin_type_ids::STRING,
        location,
        ValueMode::MutableOwned,
        &TypeEnvironment::new(),
    );
    let mut option_type_environment =
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();
    let option_string = option_type_environment.intern_option(builtin_type_ids::STRING);

    let assert_condition = builder
        .bool(&mut constants, true, location, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();
    let assert_message = builder
        .coerced(builder.own(message), option_string)
        .expect("fixture coercion")
        .id();

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::Assert {
                condition: assert_condition,
                message: assert_message,
            },
            location,
        )],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![message_fn, start_fn],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
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
    );
}

#[test]
fn statically_false_assertion_keeps_cfg_producing_message_before_terminal_failure() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let condition_name = super::symbol("message_condition", &mut path_fork, &mut string_table);
    let location = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let condition = reference_expr_with_type_id(
        &mut builder,
        condition_name,
        builtin_type_ids::BOOL,
        location,
        ValueMode::ImmutableReference,
    );
    let then_string = builder
        .string_slice(
            &mut constants,
            string_table.intern("then"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string slice")
        .id();
    let else_string = builder
        .string_slice(
            &mut constants,
            string_table.intern("else"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string slice")
        .id();
    let message_value = builder
        .value_block(
            ValueBlock::If(ValueIfBlock {
                condition,
                then_body: vec![node(
                    NodeKind::ThenValue(ProducedValues {
                        expressions: vec![then_string],
                        span: location,
                    }),
                    location,
                )],
                else_body: vec![node(
                    NodeKind::ThenValue(ProducedValues {
                        expressions: vec![else_string],
                        span: location,
                    }),
                    location,
                )],
                then_scope: entry_path,
                else_scope: entry_path,
                span: location,
                generic_request_ranges: Default::default(),
                result_type_ids: vec![builtin_type_ids::STRING],
            }),
            location,
            SuccessShape::One(builtin_type_ids::STRING),
            ValueMode::ImmutableOwned,
        )
        .expect("fixture value-block message")
        .id();
    let mut option_type_environment =
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();
    let option_string = option_type_environment.intern_option(builtin_type_ids::STRING);
    let message = builder
        .coerced(builder.own(message_value), option_string)
        .expect("fixture coercion")
        .id();

    let assert_condition = builder
        .bool(&mut constants, false, location, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();

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
                condition: assert_condition,
                message,
            },
            location,
        )],
        location,
    );

    let mut ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    assert_eq!(
        ast.type_environment.intern_option(builtin_type_ids::STRING),
        option_string,
        "the fixture environment must register the same canonical String? type"
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
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
    );
}

#[test]
fn non_unit_function_with_terminal_if_does_not_report_fallthrough() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let chooser = super::symbol("chooser", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let flag = super::symbol("flag", &mut path_fork, &mut string_table);
    let flag_value = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("fixture bool")
        .id();
    let flag_decl = node(
        NodeKind::VariableDeclaration(make_test_variable(flag, flag_value, &builder)),
        None,
    );
    let condition = reference_expr_with_type_id(
        &mut builder,
        flag,
        builtin_type_ids::BOOL,
        None,
        ValueMode::ImmutableReference,
    );

    let chooser_fn = function_node(
        chooser,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)]),
        },
        vec![
            flag_decl,
            node(
                NodeKind::If(
                    condition,
                    vec![node(
                        NodeKind::Return(vec![i32_literal(
                            &mut builder,
                            &mut constants,
                            1,
                            None,
                            ValueMode::ImmutableOwned,
                        )]),
                        None,
                    )],
                    Some(vec![node(
                        NodeKind::Return(vec![i32_literal(
                            &mut builder,
                            &mut constants,
                            2,
                            None,
                            ValueMode::ImmutableOwned,
                        )]),
                        None,
                    )]),
                    test_if_branch_metadata(true),
                ),
                None,
            ),
        ],
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

    let ast = build_ast_with_registered_types(
        vec![start_fn, chooser_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("all-terminal if should not trigger fallthrough");

    let chooser_block = &module.blocks[module.functions[1].entry.0 as usize];
    assert!(matches!(chooser_block.terminator, HirTerminator::If { .. }));
    assert_no_placeholder_terminators(&module);
}
