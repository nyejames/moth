//! AST root visitor regression tests.
//!
//! WHAT: checks that the expression and place visitors reach each row of a statement tree in
//! source order, including a chain deep enough to overflow a one MiB thread under recursion.
//! WHY: the visitors walk an explicit heap stack, so nesting depth must not cost call-stack space,
//! and the order they report is the contract the freeze and rewrite passes rely on.

use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, LoopBindings, MatchExhaustiveness, NodeKind, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, PlaceId,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::expressions::parse_expression_places::place_expression_from_expression;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, PatternValue,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::tests::ast_fixture_support::{
    make_test_variable, node, test_if_branch_metadata,
};
use crate::compiler_frontend::value_mode::ValueMode;

/// Nesting depth that overflows a one MiB thread if the visitors recurse once per body.
const DEEP_IF_DEPTH: usize = 8192;
const ONE_MIB: usize = 1024 * 1024;

fn bool_expression(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: bool,
) -> ExprId {
    builder
        .bool(constants, value, None, ValueMode::ImmutableOwned)
        .expect("visitor fixture bool is valid")
        .id()
}

fn reference_expression(builder: &mut AstExpressionBuilder, mode: ValueMode) -> ExprId {
    builder
        .reference_with_type_id(
            PathId::ROOT,
            builtin_type_ids::BOOL,
            None,
            mode,
            ConstRecordState::RuntimeValue,
        )
        .expect("visitor fixture reference is valid")
        .id()
}

fn place_fixture(builder: &mut AstExpressionBuilder) -> PlaceId {
    let reference = reference_expression(builder, ValueMode::MutableReference);
    place_expression_from_expression(builder, builder.own(reference))
        .expect("visitor fixture reference is a place")
        .expect("visitor fixture reference yields a place")
}

fn expression_statement(value: ExprId) -> AstNode {
    node(NodeKind::ExpressionStatement(value), None)
}

/// Builds `depth` nested `if` statements whose innermost body assigns one place.
///
/// WHY: the fixture is built bottom-up with a loop, so building it never recurses.
/// Returns the top-level body and the expression IDs in the order the visitors must report them.
fn if_chain(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    depth: usize,
) -> (Vec<AstNode>, Vec<ExprId>) {
    let mut conditions = Vec::with_capacity(depth);
    for index in 0..depth {
        conditions.push(bool_expression(builder, constants, index % 2 == 0));
    }
    let value = bool_expression(builder, constants, true);
    let target = place_fixture(builder);

    let mut body = vec![node(NodeKind::Assignment { target, value }, None)];
    for &condition in conditions.iter().rev() {
        body = vec![node(
            NodeKind::If(condition, body, None, test_if_branch_metadata(false)),
            None,
        )];
    }

    let mut expected = conditions;
    expected.push(value);
    (body, expected)
}

/// Drops a chain built by `if_chain` one level at a time.
///
/// WHY: default drop glue recurses once per nested body, so the test tears the chain down
/// iteratively to keep the fixture itself inside the same small thread stack.
fn dismantle_if_chain(mut body: Vec<AstNode>) {
    while let Some(mut statement) = body.pop() {
        body = match &mut statement.kind {
            NodeKind::If(_, then_body, _, _) => std::mem::take(then_body),
            _ => Vec::new(),
        };
    }
}

/// Runs every root visitor over a deep `if` chain on a thread with `stack_bytes` of stack.
fn visit_if_chain_on_thread(depth: usize, stack_bytes: usize) {
    std::thread::Builder::new()
        .stack_size(stack_bytes)
        .spawn(move || {
            let mut builder = AstExpressionBuilder::new().unwrap();
            let mut constants = ConstValueStore::default();
            let (mut body, expected) = if_chain(&mut builder, &mut constants, depth);

            let mut expressions = Vec::new();
            let mut places = Vec::new();
            for statement in &body {
                statement.visit_expression_ids(&mut |id| expressions.push(id));
                statement.visit_place_ids(&mut |id| places.push(id));
            }
            assert_eq!(expressions, expected);
            assert_eq!(places.len(), 1);

            let mut mutable_expressions = Vec::new();
            let mut mutable_places = Vec::new();
            for statement in &mut body {
                statement.visit_expression_ids_mut(&mut |id| mutable_expressions.push(*id));
                statement.visit_place_ids_mut(&mut |id| mutable_places.push(*id));
            }
            assert_eq!(mutable_expressions, expected);
            assert_eq!(mutable_places.len(), 1);

            dismantle_if_chain(body);
        })
        .expect("visitor test thread spawns")
        .join()
        .expect("visitors complete without overflowing the thread stack");
}

#[test]
fn deep_if_chain_is_visited_in_source_order_on_one_mib_stack() {
    visit_if_chain_on_thread(DEEP_IF_DEPTH, ONE_MIB);
}

#[test]
fn mixed_statement_tree_visits_rows_in_source_order() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let condition = bool_expression(&mut builder, &mut constants, true);
    let then_value = bool_expression(&mut builder, &mut constants, false);
    let assigned_value = bool_expression(&mut builder, &mut constants, true);
    let target = place_fixture(&mut builder);

    let scrutinee = bool_expression(&mut builder, &mut constants, true);
    let pattern_reference = reference_expression(&mut builder, ValueMode::ImmutableOwned);
    let pattern_value = PatternValue::from_expression(
        builder
            .view(pattern_reference)
            .expect("visitor fixture pattern reference is in the builder"),
    )
    .expect("visitor fixture pattern value is valid");
    let guard = bool_expression(&mut builder, &mut constants, true);
    let arm_value = bool_expression(&mut builder, &mut constants, false);
    let default_value = bool_expression(&mut builder, &mut constants, true);

    let item_value = bool_expression(&mut builder, &mut constants, true);
    let start = bool_expression(&mut builder, &mut constants, true);
    let end = bool_expression(&mut builder, &mut constants, true);
    let step = bool_expression(&mut builder, &mut constants, true);
    let loop_value = bool_expression(&mut builder, &mut constants, false);

    let if_statement = node(
        NodeKind::If(
            condition,
            vec![expression_statement(then_value)],
            Some(vec![node(
                NodeKind::Assignment {
                    target,
                    value: assigned_value,
                },
                None,
            )]),
            test_if_branch_metadata(true),
        ),
        None,
    );
    let match_statement = node(
        NodeKind::Match {
            scrutinee,
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(pattern_value),
                guard: Some(guard),
                body: vec![expression_statement(arm_value)],
            }],
            default: Some(vec![expression_statement(default_value)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );
    let loop_statement = node(
        NodeKind::RangeLoop {
            bindings: Box::new(LoopBindings {
                item: Some(make_test_variable(PathId::ROOT, item_value, &builder)),
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start,
                end,
                end_kind: RangeEndKind::Exclusive,
                step: Some(step),
            }),
            body: vec![expression_statement(loop_value)],
        },
        None,
    );
    let mut statements = vec![if_statement, match_statement, loop_statement];

    let expected_expressions = [
        condition,
        then_value,
        assigned_value,
        scrutinee,
        pattern_reference,
        guard,
        arm_value,
        default_value,
        item_value,
        start,
        end,
        step,
        loop_value,
    ];

    let mut expressions = Vec::new();
    for statement in &statements {
        statement.visit_expression_ids(&mut |id| expressions.push(id));
    }
    assert_eq!(expressions, expected_expressions);

    let mut places = Vec::new();
    for statement in &statements {
        statement.visit_place_ids(&mut |id| places.push(id));
    }
    assert_eq!(places, [target]);

    let mut mutable_expressions = Vec::new();
    for statement in &mut statements {
        statement.visit_expression_ids_mut(&mut |id| mutable_expressions.push(*id));
    }
    assert_eq!(mutable_expressions, expected_expressions);

    let mut mutable_places = Vec::new();
    for statement in &mut statements {
        statement.visit_place_ids_mut(&mut |id| mutable_places.push(*id));
    }
    assert_eq!(mutable_places, [target]);
}
