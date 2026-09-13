//! Value-producing block lowering shape tests.
//!
//! WHAT: pins HIR CFG shape for value-if, value-match, and `ThenValue` lowering
//!       that integration tests cannot observe directly.
//! WHY: borrow validation and backend lowering depend on consistent result-local
//!      allocation, merge-block targeting, and active-value-block-target protocol.

use crate::compiler_frontend::ast::ast_nodes::{MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::{
    ProducedValues,
    types::{ValueBlock, ValueMatchBlock},
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node, param_with_type_id, reference_expr_with_type_id,
};

use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::hir::hir_builder::{
    assert_no_placeholder_terminators, build_ast_with_registered_types, lower_ast,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

/// Extracts the result-local assignment and merge target from a value-block arm block.
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

    super::assert_block_has_jump_args(module, block_id, 0);
    let HirTerminator::Jump { target, .. } = block.terminator else {
        panic!("value-block branch should jump to the merge block");
    };

    (result_local, value_kind, target)
}

#[test]
fn value_match_lowering_uses_shared_result_local_and_merge_block() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);
let x = super::symbol("x", &mut path_fork, &mut string_table);
let result_name = super::symbol("result", &mut path_fork, &mut string_table);

let arm_a = MatchArm {
    pattern: MatchPattern::Literal(Expression::int(1, None, ValueMode::ImmutableOwned)),
    guard: None,
    body: vec![node(
        NodeKind::ThenValue(ProducedValues {
            expressions: vec![Expression::int(10, None, ValueMode::ImmutableOwned)],
            span: None,
        }),
        None,
    )],
};

let arm_b = MatchArm {
    pattern: MatchPattern::Literal(Expression::int(2, None, ValueMode::ImmutableOwned)),
    guard: None,
    body: vec![node(
        NodeKind::ThenValue(ProducedValues {
            expressions: vec![Expression::int(20, None, ValueMode::ImmutableOwned)],
            span: None,
        }),
        None,
    )],
};

let default_body = vec![node(
    NodeKind::ThenValue(ProducedValues {
        expressions: vec![Expression::int(0, None, ValueMode::ImmutableOwned)],
        span: None,
    }),
    None,
)];

let value_match_expr = Expression::new(
    ExpressionKind::ValueBlock {
        block: Box::new(ValueBlock::Match(ValueMatchBlock {
            scrutinee: reference_expr_with_type_id(
                x.clone(),
                builtin_type_ids::INT,
                None,
                ValueMode::ImmutableReference,
            ),
            arms: vec![arm_a, arm_b],
            default: Some(default_body),
            exhaustiveness: MatchExhaustiveness::HasDefault,
            span: None,
            result_type_ids: vec![builtin_type_ids::INT],
        })),
    },
    None,
    builtin_type_ids::INT,
    DataType::Inferred,
    ValueMode::ImmutableOwned,
);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![param_with_type_id(x, builtin_type_ids::INT, false, None)],
        returns: vec![],
    },
    vec![node(
        NodeKind::VariableDeclaration(make_test_variable(result_name, value_match_expr)),
        None,
    )],
    None,
);

let (module, _type_environment) = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect("value-match lowering should succeed");

let start = super::start_function(&module);
let entry_block = &module.blocks[start.entry.0 as usize];

let match_terminator = match &entry_block.terminator {
    HirTerminator::Match { arms, .. } => {
        assert_eq!(
            arms.len(),
            3,
            "two literal arms plus default wildcard should produce three HIR arms"
        );
        arms
    }
    other => panic!("expected match terminator, got {other:?}"),
};

let first_arm_block_id = match_terminator[0].body;
let second_arm_block_id = match_terminator[1].body;
let default_arm_block_id = match_terminator[2].body;

let (result_local_1, _, merge_1) = value_block_result_assignment(&module, first_arm_block_id);
let (result_local_2, _, merge_2) = value_block_result_assignment(&module, second_arm_block_id);
let (result_local_3, _, merge_3) = value_block_result_assignment(&module, default_arm_block_id);

assert_eq!(
    result_local_1, result_local_2,
    "all match arms should assign to the same shared result local"
);
assert_eq!(
    result_local_2, result_local_3,
    "all match arms should assign to the same shared result local"
);
assert_eq!(
    merge_1, merge_2,
    "all match arms should jump to the same shared merge block"
);
assert_eq!(
    merge_2, merge_3,
    "all match arms should jump to the same shared merge block"
);

assert_no_placeholder_terminators(&module); }

#[test]
fn then_value_without_active_target_is_hir_invariant_failure() { let mut path_fork = super::PathInternerFork::empty(); let mut string_table = StringTable::new();
let (entry_path, start_name) = super::entry_path_and_start_name(&mut path_fork, &mut string_table);

let start_fn = function_node(
    start_name,
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    },
    vec![node(
        NodeKind::ThenValue(ProducedValues {
            expressions: vec![Expression::int(1, None, ValueMode::ImmutableOwned)],
            span: None,
        }),
        None,
    )],
    None,
);

let err = lower_ast(build_ast_with_registered_types(vec![start_fn], entry_path), &mut string_table, &mut path_fork)
.expect_err("ThenValue outside active value-block target should fail HIR lowering");

let error = err
    .infrastructure_error()
    .expect("HIR lowering failure should be wrapped for rendering");
assert_eq!(&error.error_type, &ErrorType::HirTransformation);
assert!(
    error.msg.contains("active value block target"),
    "unexpected error message: {}",
    error.msg,
); }
