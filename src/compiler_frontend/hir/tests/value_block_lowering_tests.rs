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
    function_node, make_test_variable, node, test_source_location,
};

use crate::compiler_frontend::tests::type_id_fixture_support::{
    param_with_type_id, reference_expr,
};
use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::hir::hir_builder::{
    assert_no_placeholder_terminators, build_ast, lower_ast,
};

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
fn value_match_lowering_uses_shared_result_local_and_merge_block() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);
    let result_name = super::symbol("result", &mut string_table);

    let arm_a = MatchArm {
        pattern: MatchPattern::Literal(Expression::int(
            1,
            test_source_location(3),
            ValueMode::ImmutableOwned,
        )),
        guard: None,
        body: vec![node(
            NodeKind::ThenValue(ProducedValues {
                expressions: vec![Expression::int(
                    10,
                    test_source_location(3),
                    ValueMode::ImmutableOwned,
                )],
                location: test_source_location(3),
            }),
            test_source_location(3),
        )],
    };

    let arm_b = MatchArm {
        pattern: MatchPattern::Literal(Expression::int(
            2,
            test_source_location(4),
            ValueMode::ImmutableOwned,
        )),
        guard: None,
        body: vec![node(
            NodeKind::ThenValue(ProducedValues {
                expressions: vec![Expression::int(
                    20,
                    test_source_location(4),
                    ValueMode::ImmutableOwned,
                )],
                location: test_source_location(4),
            }),
            test_source_location(4),
        )],
    };

    let default_body = vec![node(
        NodeKind::ThenValue(ProducedValues {
            expressions: vec![Expression::int(
                0,
                test_source_location(5),
                ValueMode::ImmutableOwned,
            )],
            location: test_source_location(5),
        }),
        test_source_location(5),
    )];

    let value_match_expr = Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::Match(ValueMatchBlock {
                scrutinee: reference_expr(
                    x.clone(),
                    builtin_type_ids::INT,
                    test_source_location(2),
                    ValueMode::ImmutableReference,
                ),
                arms: vec![arm_a, arm_b],
                default: Some(default_body),
                exhaustiveness: MatchExhaustiveness::HasDefault,
                location: test_source_location(2),
                result_type_ids: vec![builtin_type_ids::INT],
            })),
        },
        test_source_location(2),
        builtin_type_ids::INT,
        DataType::Inferred,
        ValueMode::ImmutableOwned,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::INT,
                false,
                test_source_location(1),
            )],
            returns: vec![],
        },
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(result_name, value_match_expr)),
            test_source_location(2),
        )],
        test_source_location(1),
    );

    let (module, _type_environment) =
        lower_ast(build_ast(vec![start_fn], entry_path), &mut string_table)
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

    assert_no_placeholder_terminators(&module);
}

#[test]
fn then_value_without_active_target_is_hir_invariant_failure() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::ThenValue(ProducedValues {
                expressions: vec![Expression::int(
                    1,
                    test_source_location(2),
                    ValueMode::ImmutableOwned,
                )],
                location: test_source_location(2),
            }),
            test_source_location(2),
        )],
        test_source_location(1),
    );

    let err = lower_ast(build_ast(vec![start_fn], entry_path), &mut string_table)
        .expect_err("ThenValue outside active value-block target should fail HIR lowering");

    let (error_type, message, _location) = err
        .first_infrastructure_error_for_tests()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(error_type, &ErrorType::HirTransformation);
    assert!(
        message.contains("active value block target"),
        "unexpected error message: {message}"
    );
}
