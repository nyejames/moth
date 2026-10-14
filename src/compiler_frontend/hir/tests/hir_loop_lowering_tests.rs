//! HIR loop lowering regression tests.
//!
//! WHAT: checks how `loop` statements lower into HIR blocks with back-edges, break/continue
//!       terminators, and optional range/collection iteration setup.
//! WHY: loop lowering is the most complex CFG construction in the frontend; targeted tests
//!      catch break-target and induction-variable regressions early.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{function_node, node};

use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use crate::compiler_frontend::hir::hir_builder::{build_ast_with_registered_types, lower_ast};

#[test]
fn lowers_while_to_header_body_exit_shape() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let condition = builder
        .bool(&mut constants, false, None, ValueMode::ImmutableOwned)
        .expect("test literal fits expression store")
        .id();
    let body_value = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 10).expect("test literal fits I32"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("test literal fits expression store")
        .id();
    let while_node = node(
        NodeKind::WhileLoop(
            condition,
            vec![node(NodeKind::ExpressionStatement(body_value), None)],
        ),
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![while_node],
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

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];

    let header_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump to while header"),
    };

    let (body_block, _exit_block) = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (then_block, else_block),
        _ => panic!("expected if in while header"),
    };

    let backedge_block = match module.blocks[body_block.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected while body to jump to the parent-region backedge"),
    };

    assert!(matches!(
        module.blocks[backedge_block.0 as usize].terminator,
        HirTerminator::Jump { target, .. } if target == header_block
    ));
}

#[test]
fn break_in_while_targets_loop_exit_block() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let condition = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("test literal fits expression store")
        .id();
    let while_node = node(
        NodeKind::WhileLoop(condition, vec![node(NodeKind::Break, None)]),
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![while_node],
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

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let header_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump to while header"),
    };

    let (body_block, exit_block) = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (then_block, else_block),
        _ => panic!("expected while header conditional terminator"),
    };

    assert!(matches!(
        module.blocks[body_block.0 as usize].terminator,
        HirTerminator::Break { target } if target == exit_block
    ));
}
