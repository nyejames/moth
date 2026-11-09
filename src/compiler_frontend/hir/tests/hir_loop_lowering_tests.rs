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
use crate::compiler_frontend::datatypes::{CallableReturnShape, FunctionReturnContract};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::node;

use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use crate::compiler_frontend::tests::type_id_fixture_support::{
    build_ast_with_registered_types, lower_ast,
};

#[test]
fn lowers_while_to_header_body_exit_shape() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();

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

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![while_node],
    );

    let ast =
        build_ast_with_registered_types(vec![start_fn], builder, constants, TypeEnvironment::new());
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

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![while_node],
    );

    let ast =
        build_ast_with_registered_types(vec![start_fn], builder, constants, TypeEnvironment::new());
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

    let body_block = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected an unconditional jump into a folded-true loop body"),
    };

    let exit_block = match module.blocks[body_block.0 as usize].terminator {
        HirTerminator::Break { target } => target,
        _ => panic!("expected the loop body to break to its exit"),
    };

    assert!(
        matches!(
            module.blocks[exit_block.0 as usize].terminator,
            HirTerminator::Return(_)
        ),
        "the reachable post-loop block must remain the continuation"
    );
}

#[test]
fn folded_true_loop_without_break_has_no_exit_edge() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let condition = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
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

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![while_node],
    );

    let ast =
        build_ast_with_registered_types(vec![start_fn], builder, constants, TypeEnvironment::new());
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

    let body_block = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("folded-true loop header must jump straight into the body"),
    };

    let backedge_block = match module.blocks[body_block.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected while body to jump to the parent-region backedge"),
    };
    assert!(matches!(
        module.blocks[backedge_block.0 as usize].terminator,
        HirTerminator::Jump { target, .. } if target == header_block
    ));

    assert!(
        !module
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, HirTerminator::If { .. })),
        "a folded-true loop has no runtime false edge"
    );
    assert_eq!(
        module.blocks.len(),
        4,
        "the preallocated exit block must be removed, leaving entry/header/body/backedge"
    );
}

#[test]
fn folded_true_loop_stops_lowering_later_statements() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let condition = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("test literal fits expression store")
        .id();
    let while_node = node(NodeKind::WhileLoop(condition, vec![]), None);
    let later_break = node(NodeKind::Break, None);

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![while_node, later_break],
    );

    let ast =
        build_ast_with_registered_types(vec![start_fn], builder, constants, TypeEnvironment::new());
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("statements after a proven non-exiting loop must not be lowered");

    assert_eq!(
        module.blocks.len(),
        4,
        "an empty folded-true loop leaves entry/header/body/backedge and no continuation"
    );
}
