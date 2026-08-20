//! HIR loop lowering regression tests.
//!
//! WHAT: checks how `loop` statements lower into HIR blocks with back-edges, break/continue
//!       terminators, and optional range/collection iteration setup.
//! WHY: loop lowering is the most complex CFG construction in the frontend; targeted tests
//!      catch break-target and induction-variable regressions early.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, node, test_source_location,
};

use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::hir::hir_builder::{build_ast_with_registered_types, lower_ast};

#[test]
fn lowers_while_to_header_body_exit_shape() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let while_node = node(
        NodeKind::WhileLoop(
            Expression::bool(false, test_source_location(2), ValueMode::ImmutableOwned),
            vec![node(
                NodeKind::ExpressionStatement(Expression::int(
                    10,
                    test_source_location(2),
                    ValueMode::ImmutableOwned,
                )),
                test_source_location(2),
            )],
        ),
        test_source_location(2),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![while_node],
        test_source_location(1),
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let while_node = node(
        NodeKind::WhileLoop(
            Expression::bool(true, test_source_location(20), ValueMode::ImmutableOwned),
            vec![node(NodeKind::Break, test_source_location(21))],
        ),
        test_source_location(20),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![while_node],
        test_source_location(19),
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

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
