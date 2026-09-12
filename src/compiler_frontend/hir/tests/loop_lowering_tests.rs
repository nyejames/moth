//! Loop lowering regression tests.
//!
//! WHAT: validates range/collection loop lowering into explicit HIR CFG blocks.
//! WHY: loop header refactors must preserve control-flow semantics and loop-target routing.

use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, LoopBindings, NodeKind, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{HirNumericOp, HirNumericOperands};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    reference_expr_with_type_id, test_if_branch_metadata,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    loop_binding_with_type_id as loop_binding, runtime_expr, runtime_operand_item,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn node(kind: NodeKind, span: Option<crate::compiler_frontend::source::SourceSpan>) -> AstNode {
    AstNode {
        kind,
        span,
        scope: crate::compiler_frontend::symbols::path_interner::PathId::ROOT,
    }
}

fn function_node(
    name: PathId,
    signature: FunctionSignature,
    body: Vec<AstNode>,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> AstNode {
    node(NodeKind::Function(name, signature, body), span)
}

fn range_loop_spec(
    start: Expression,
    end: Expression,
    end_kind: RangeEndKind,
    step: Option<Expression>,
) -> RangeLoopSpec {
    RangeLoopSpec {
        start,
        end,
        end_kind,
        step,
    }
}

use crate::compiler_frontend::hir::hir_builder::{
    assert_no_placeholder_terminators, build_ast_with_registered_types, lower_ast,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

fn range_loop_cfg_blocks(module: &HirModule) -> (BlockId, BlockId, BlockId, BlockId, BlockId) {
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let step_zero_check_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected entry jump to range step-zero-check"),
    };

    let step_abs_check_block = match module.blocks[step_zero_check_block.0 as usize].terminator {
        HirTerminator::If { else_block, .. } => else_block,
        _ => panic!("expected zero-check branch"),
    };

    let direction_check_block = match module.blocks[step_abs_check_block.0 as usize].terminator {
        HirTerminator::If { else_block, .. } => else_block,
        _ => panic!("expected abs-check branch"),
    };

    let header_selector_block = match module.blocks[direction_check_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected direction branch"),
    };

    let header_ascending_block = match module.blocks[header_selector_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected header selector branch"),
    };

    (
        step_zero_check_block,
        header_selector_block,
        header_ascending_block,
        step_abs_check_block,
        direction_check_block,
    )
}

fn collection_literal(span: Option<crate::compiler_frontend::source::SourceSpan>) -> Expression {
    crate::compiler_frontend::tests::type_id_fixture_support::collection_expr(
        vec![
            Expression::int(1, span, ValueMode::ImmutableOwned),
            Expression::int(2, span, ValueMode::ImmutableOwned),
            Expression::int(3, span, ValueMode::ImmutableOwned),
        ],
        span,
        ValueMode::ImmutableOwned,
    )
}

#[test]
fn lowers_range_loop_with_new_syntax() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let range_loop = node(
    NodeKind::RangeLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        range: range_loop_spec(
            Expression::int(0, location, ValueMode::ImmutableOwned),
            Expression::int(3, location, ValueMode::ImmutableOwned),
            RangeEndKind::Exclusive,
            None,
        ),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![range_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("range loop lowering should succeed");

let (_, header_selector_block, header_ascending_block, _, _) = range_loop_cfg_blocks(&module);

let (body_block, exit_block) = match module.blocks[header_ascending_block.0 as usize].terminator
{
    HirTerminator::If {
        then_block,
        else_block,
        ..
    } => (then_block, else_block),
    _ => panic!("expected ascending header branch"),
};

let step_block = match module.blocks[body_block.0 as usize].terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected body jump to step block"),
};

assert!(matches!(
    module.blocks[step_block.0 as usize].terminator,
    HirTerminator::Jump { target, .. } if target == header_selector_block
));
assert!(matches!(
    module.blocks[exit_block.0 as usize].terminator,
    HirTerminator::Return(_)
));
assert_no_placeholder_terminators(&module); }

#[test]
fn lowers_range_loop_without_user_bindings() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let range_loop = node(
    NodeKind::RangeLoop {
        bindings: LoopBindings {
            item: None,
            index: None,
        },
        range: range_loop_spec(
            Expression::int(0, location, ValueMode::ImmutableOwned),
            Expression::int(3, location, ValueMode::ImmutableOwned),
            RangeEndKind::Exclusive,
            None,
        ),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![range_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("range loop lowering without user bindings should succeed");

let (_, _, header_ascending_block, _, _) = range_loop_cfg_blocks(&module);
let body_block = match module.blocks[header_ascending_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected ascending header branch"),
};

let body_locals = &module.blocks[body_block.0 as usize].locals;
assert!(
    body_locals.is_empty(),
    "binding-less range loops should not allocate user binding locals"
); }

#[test]
fn lowers_range_loop_with_index_binding() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let range_loop = node(
    NodeKind::RangeLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
        },
        range: range_loop_spec(
            Expression::int(0, location, ValueMode::ImmutableOwned),
            Expression::int(4, location, ValueMode::ImmutableOwned),
            RangeEndKind::Exclusive,
            None,
        ),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![range_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("range loop lowering with index should succeed");

let (_, _, header_ascending_block, _, _) = range_loop_cfg_blocks(&module);

let body_block = match module.blocks[header_ascending_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected ascending header branch"),
};

let body_statements = &module.blocks[body_block.0 as usize].statements;
let assign_count = body_statements
    .iter()
    .filter(|statement| matches!(statement.kind, HirStatementKind::Assign { .. }))
    .count();
assert_eq!(
    assign_count, 2,
    "expected value + index binding assignments"
);

let step_block = match module.blocks[body_block.0 as usize].terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected body jump to step block"),
};

let has_index_increment =
    module.blocks[step_block.0 as usize]
        .statements
        .iter()
        .any(|statement| match &statement.kind {
            HirStatementKind::NumericOp {
                op: HirNumericOp::IntAdd,
                operands: HirNumericOperands::Binary { right, .. },
                ..
            } => matches!(right.kind, HirExpressionKind::Int(1)),
            _ => false,
        });

assert!(
    has_index_increment,
    "expected explicit zero-based index increment in range step block"
); }

#[test]
fn preserves_runtime_zero_step_guard_for_dynamic_step() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let step_symbol = super::symbol("step", &mut path_fork, &mut string_table);
let step_decl = node(
    NodeKind::VariableDeclaration(Declaration {
        id: step_symbol.clone(),
        value: Expression::int(2, location, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    }),
    location,
);

let range_loop = node(
    NodeKind::RangeLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        range: range_loop_spec(
            Expression::int(0, location, ValueMode::ImmutableOwned),
            Expression::int(10, location, ValueMode::ImmutableOwned),
            RangeEndKind::Exclusive,
            Some(reference_expr_with_type_id(
                step_symbol,
                builtin_type_ids::INT,
                location,
                ValueMode::ImmutableReference,
            )),
        ),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![step_decl, range_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("dynamic-step range loop lowering should succeed");

let (step_zero_check_block, _, _, _, _) = range_loop_cfg_blocks(&module);

let panic_block = match module.blocks[step_zero_check_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected runtime zero-check branch"),
};

assert!(matches!(
    module.blocks[panic_block.0 as usize].terminator,
    HirTerminator::RuntimeFailure { .. }
)); }

#[test]
fn range_loop_nested_if_body_routes_tail_to_step_block() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let branch_value = super::symbol("branch_value", &mut path_fork, &mut string_table);
let tail_value = super::symbol("tail_value", &mut path_fork, &mut string_table);
let range_loop = node(
    NodeKind::RangeLoop {
        bindings: LoopBindings {
            item: None,
            index: None,
        },
        range: range_loop_spec(
            Expression::int(0, location, ValueMode::ImmutableOwned),
            Expression::int(4, location, ValueMode::ImmutableOwned),
            RangeEndKind::Exclusive,
            None,
        ),
        body: vec![
            node(
                NodeKind::If(
                    runtime_expr(
                        vec![runtime_operand_item(Expression::bool(
                            true,
                            location,
                            ValueMode::ImmutableOwned,
                        ))],
                        builtin_type_ids::BOOL,
                        location,
                        ValueMode::ImmutableOwned,
                    ),
                    vec![node(
                        NodeKind::VariableDeclaration(Declaration {
                            id: branch_value,
                            value: Expression::int(1, location, ValueMode::ImmutableOwned),
                            binding_span: None,
                            config_qualifier: None,
                        }),
                        location,
                    )],
                    None,
                    test_if_branch_metadata(false),
                ),
                location,
            ),
            node(
                NodeKind::VariableDeclaration(Declaration {
                    id: tail_value,
                    value: Expression::int(2, location, ValueMode::ImmutableOwned),
                    binding_span: None,
                    config_qualifier: None,
                }),
                location,
            ),
        ],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![range_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("range loop lowering with nested body control-flow should succeed");

let (_, header_selector_block, header_ascending_block, _, _) = range_loop_cfg_blocks(&module);
let body_block = match module.blocks[header_ascending_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected ascending header branch"),
};
assert!(
    matches!(
        module.blocks[body_block.0 as usize].terminator,
        HirTerminator::If { .. }
    ),
    "nested if should terminate the range-loop body entry block"
);

let step_block = module
    .blocks
    .iter()
    .find_map(|block| match block.terminator {
        HirTerminator::Jump { target, .. } if target == header_selector_block => {
            let has_index_increment = block.statements.iter().any(|statement| match &statement
                .kind
            {
                HirStatementKind::NumericOp {
                    op: HirNumericOp::IntAdd,
                    operands: HirNumericOperands::Binary { right, .. },
                    ..
                } => matches!(right.kind, HirExpressionKind::Int(1)),
                _ => false,
            });
            has_index_increment.then_some(block.id)
        }
        _ => None,
    })
    .expect("expected range-step block backedge to header selector");

let step_predecessor_ids = module
    .blocks
    .iter()
    .filter_map(|block| match block.terminator {
        HirTerminator::Jump { target, .. }
            if target == step_block && block.id != body_block =>
        {
            Some(block.id)
        }
        _ => None,
    })
    .collect::<Vec<_>>();
assert!(
    !step_predecessor_ids.is_empty(),
    "expected lowered range-loop body tail to jump into the step block"
);
assert_no_placeholder_terminators(&module); }

#[test]
fn lowers_collection_loop_to_explicit_cfg() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let collection_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        iterable: collection_literal(location),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![collection_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("collection loop lowering should succeed");

let start = &module.functions[module
    .start_function
    .expect("normal test module should have start")
    .0 as usize];
let entry_block = &module.blocks[start.entry.0 as usize];
let header_block = match entry_block.terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected jump to collection header"),
};

let (body_block, exit_block) = match module.blocks[header_block.0 as usize].terminator {
    HirTerminator::If {
        then_block,
        else_block,
        ..
    } => (then_block, else_block),
    _ => panic!("expected collection loop header conditional"),
};

let step_block = match module.blocks[body_block.0 as usize].terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected collection body jump to step block"),
};

assert!(matches!(
    module.blocks[step_block.0 as usize].terminator,
    HirTerminator::Jump { target, .. } if target == header_block
));
assert!(matches!(
    module.blocks[exit_block.0 as usize].terminator,
    HirTerminator::Return(_)
)); }

#[test]
fn lowers_collection_loop_without_user_bindings() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let collection_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: None,
            index: None,
        },
        iterable: collection_literal(location),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![collection_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("collection loop lowering without user bindings should succeed");

let start = &module.functions[module
    .start_function
    .expect("normal test module should have start")
    .0 as usize];
let entry_block = &module.blocks[start.entry.0 as usize];
let header_block = match entry_block.terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected jump to collection header"),
};
let body_block = match module.blocks[header_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected collection loop header conditional"),
};

assert!(
    module.blocks[body_block.0 as usize].locals.is_empty(),
    "binding-less collection loops should not allocate user binding locals"
); }

#[test]
fn lowers_collection_loop_item_binding_from_indexed_place() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let collection_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        iterable: collection_literal(location),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![collection_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("collection loop lowering should succeed");

let start = &module.functions[module
    .start_function
    .expect("normal test module should have start")
    .0 as usize];
let entry_block = &module.blocks[start.entry.0 as usize];
let header_block = match entry_block.terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected jump to collection header"),
};
let body_block = match module.blocks[header_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected collection loop header conditional"),
};

let has_indexed_item_assign =
    module.blocks[body_block.0 as usize]
        .statements
        .iter()
        .any(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::Assign {
                    value: crate::compiler_frontend::hir::expressions::HirExpression {
                        kind: HirExpressionKind::Load(HirPlace::Index { .. }),
                        ..
                    },
                    ..
                }
            )
        });

assert!(
    has_indexed_item_assign,
    "expected collection item binding to load from indexed place"
); }

#[test]
fn lowers_collection_loop_optional_index_binding() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let collection_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
        },
        iterable: collection_literal(location),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![collection_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("collection loop lowering with index should succeed");

let start = &module.functions[module
    .start_function
    .expect("normal test module should have start")
    .0 as usize];
let entry_block = &module.blocks[start.entry.0 as usize];
let header_block = match entry_block.terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected jump to collection header"),
};
let body_block = match module.blocks[header_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected collection loop header conditional"),
};

let statements = &module.blocks[body_block.0 as usize].statements;
let has_item_assign = statements.iter().any(|statement| {
    matches!(
        statement.kind,
        HirStatementKind::Assign {
            value: crate::compiler_frontend::hir::expressions::HirExpression {
                kind: HirExpressionKind::Load(HirPlace::Index { .. }),
                ..
            },
            ..
        }
    )
});
let has_index_assign = statements.iter().any(|statement| {
    matches!(
        statement.kind,
        HirStatementKind::Assign {
            value: crate::compiler_frontend::hir::expressions::HirExpression {
                kind: HirExpressionKind::Load(HirPlace::Local(_)),
                ..
            },
            ..
        }
    )
});

assert!(has_item_assign, "expected indexed item assignment");
assert!(has_index_assign, "expected explicit user index assignment"); }

#[test]
fn lowers_range_loop_user_bindings_as_immutable_locals() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let range_loop = node(
    NodeKind::RangeLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
        },
        range: range_loop_spec(
            Expression::int(0, location, ValueMode::ImmutableOwned),
            Expression::int(4, location, ValueMode::ImmutableOwned),
            RangeEndKind::Exclusive,
            None,
        ),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![range_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("range loop lowering should succeed");

let (_, _, header_ascending_block, _, _) = range_loop_cfg_blocks(&module);
let body_block = match module.blocks[header_ascending_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected ascending header branch"),
};

let body_locals = &module.blocks[body_block.0 as usize].locals;
assert!(
    !body_locals.is_empty() && body_locals.iter().all(|local| !local.mutable),
    "range loop user binding locals should be immutable"
); }

#[test]
fn lowers_collection_loop_user_bindings_as_immutable_locals() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let collection_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
        },
        iterable: collection_literal(location),
        body: vec![],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![collection_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("collection loop lowering should succeed");

let start = &module.functions[module
    .start_function
    .expect("normal test module should have start")
    .0 as usize];
let entry_block = &module.blocks[start.entry.0 as usize];
let header_block = match entry_block.terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected jump to collection header"),
};
let body_block = match module.blocks[header_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected collection loop header conditional"),
};

let body_locals = &module.blocks[body_block.0 as usize].locals;
assert!(
    !body_locals.is_empty() && body_locals.iter().all(|local| !local.mutable),
    "collection loop user binding locals should be immutable"
); }

#[test]
fn break_targets_exit_block_in_collection_loop() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let collection_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        iterable: collection_literal(location),
        body: vec![node(
            NodeKind::If(
                runtime_expr(
                    vec![runtime_operand_item(Expression::bool(
                        true,
                        location,
                        ValueMode::ImmutableOwned,
                    ))],
                    builtin_type_ids::BOOL,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                vec![node(NodeKind::Break, location)],
                None,
                test_if_branch_metadata(false),
            ),
            location,
        )],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![collection_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("collection loop lowering should succeed");

let start = &module.functions[module
    .start_function
    .expect("normal test module should have start")
    .0 as usize];
let entry_block = &module.blocks[start.entry.0 as usize];
let header_block = match entry_block.terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected jump to collection header"),
};

let (_, exit_block) = match module.blocks[header_block.0 as usize].terminator {
    HirTerminator::If {
        then_block,
        else_block,
        ..
    } => (then_block, else_block),
    _ => panic!("expected collection loop header conditional"),
};

let break_targets_exit = module.blocks.iter().any(|block| {
    matches!(
        block.terminator,
        HirTerminator::Break { target } if target == exit_block
    )
});

assert!(
    break_targets_exit,
    "expected break terminator to target collection loop exit block"
); }

#[test]
fn direct_break_in_collection_loop_does_not_leave_unreachable_step_block() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let collection_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        iterable: collection_literal(location),
        body: vec![node(NodeKind::Break, location)],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![collection_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("direct break collection loop lowering should succeed");

assert_no_placeholder_terminators(&module); }

#[test]
fn direct_break_in_range_loop_does_not_leave_unreachable_step_block() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let range_loop = node(
    NodeKind::RangeLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        range: range_loop_spec(
            Expression::int(0, location, ValueMode::ImmutableOwned),
            Expression::int(3, location, ValueMode::ImmutableOwned),
            RangeEndKind::Exclusive,
            None,
        ),
        body: vec![node(NodeKind::Break, location)],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![range_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("direct break range loop lowering should succeed");

assert_no_placeholder_terminators(&module); }

#[test]
fn continue_targets_step_block_in_collection_loop() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let collection_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        iterable: collection_literal(location),
        body: vec![node(NodeKind::Continue, location)],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![collection_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("collection loop lowering should succeed");

let start = &module.functions[module
    .start_function
    .expect("normal test module should have start")
    .0 as usize];
let entry_block = &module.blocks[start.entry.0 as usize];
let header_block = match entry_block.terminator {
    HirTerminator::Jump { target, .. } => target,
    _ => panic!("expected jump to collection header"),
};

let body_block = match module.blocks[header_block.0 as usize].terminator {
    HirTerminator::If { then_block, .. } => then_block,
    _ => panic!("expected collection loop header conditional"),
};

let step_block = match module.blocks[body_block.0 as usize].terminator {
    HirTerminator::Continue { target } => target,
    _ => panic!("expected continue terminator in collection body"),
};

assert!(matches!(
    module.blocks[step_block.0 as usize].terminator,
    HirTerminator::Jump { target, .. } if target == header_block
)); }

#[test]
fn nested_loop_targets_remain_correct() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let location = None;

let inner_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        iterable: collection_literal(location),
        body: vec![node(NodeKind::Continue, location)],
    },
    location,
);

let outer_loop = node(
    NodeKind::CollectionLoop {
        bindings: LoopBindings {
            item: Some(loop_binding(PathId::ROOT, builtin_type_ids::INT)),
            index: None,
        },
        iterable: collection_literal(location),
        body: vec![inner_loop, node(NodeKind::Continue, location)],
    },
    location,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![outer_loop],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("nested collection loop lowering should succeed");

let continue_targets = module
    .blocks
    .iter()
    .filter_map(|block| match block.terminator {
        HirTerminator::Continue { target } => Some(target),
        _ => None,
    })
    .collect::<Vec<_>>();

assert!(
    continue_targets.len() >= 2,
    "expected at least one continue for each nested loop"
);

let unique_targets = continue_targets
    .iter()
    .map(|target| target.0)
    .collect::<std::collections::BTreeSet<_>>();
assert!(
    unique_targets.len() >= 2,
    "nested loops should continue to distinct step blocks"
);

for target in continue_targets {
    assert!(matches!(
        module.blocks[target.0 as usize].terminator,
        HirTerminator::Jump { .. }
    ));
} }
