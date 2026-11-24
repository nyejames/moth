//! Shared control-flow exit analysis regression tests.
//!
//! WHAT: checks branch union, reachable statement sequencing, loop-boundary consumption and the
//! folded Bool facts that ordinary function terminality and value completeness consume.
//! WHY: these facts are pure AST invariants and cannot be isolated reliably through a compiled
//! program. The tests deliberately construct bare statement bodies so each exit fact is visible.

use super::{
    ControlFlowExitSummary, analyze_control_flow_exits,
    analyze_control_flow_exits_with_never_targets,
};
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, DeclarationValue, LoopBindings, MatchExhaustiveness, NodeKind,
};
use crate::compiler_frontend::ast::const_values::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, PatternValue,
};
use crate::compiler_frontend::ast::statements::never_call::{NeverCall, NeverCallTarget};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, node, test_if_branch_metadata,
};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// Builder plus constant store for pure exit fixtures.
struct TestExpressions {
    builder: AstExpressionBuilder,
    constants: ConstValueStore,
}

impl TestExpressions {
    fn new() -> Self {
        Self {
            builder: AstExpressionBuilder::new().unwrap(),
            constants: ConstValueStore::default(),
        }
    }

    fn int(&mut self, value: i64) -> ExprId {
        self.builder
            .fixed_scalar(
                &mut self.constants,
                FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32"),
                None,
                ValueMode::ImmutableOwned,
            )
            .expect("fixture I32 literal")
            .id()
    }

    fn bool(&mut self, value: bool) -> ExprId {
        self.builder
            .bool(&mut self.constants, value, None, ValueMode::ImmutableOwned)
            .expect("fixture bool")
            .id()
    }

    /// A Bool reference with no folded binding, so the analysis keeps the condition dynamic.
    fn dynamic_bool(&mut self) -> ExprId {
        self.builder
            .reference_with_type_id(
                PathId::ROOT,
                builtin_type_ids::BOOL,
                None,
                ValueMode::ImmutableOwned,
                ConstRecordState::RuntimeValue,
            )
            .expect("fixture dynamic Bool reference")
            .id()
    }

    /// A module-constant reference, not a literal, so named folded Bools take the same path.
    fn named_bool(&mut self, value: bool) -> ExprId {
        let value_id = self
            .constants
            .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(value), None)
            .expect("fixture named Bool constant");
        self.constants
            .bind_body_local(PathId::ROOT, value_id)
            .expect("fixture named Bool binding");
        self.builder
            .reference_with_type_id(
                PathId::ROOT,
                builtin_type_ids::BOOL,
                None,
                ValueMode::ImmutableOwned,
                ConstRecordState::RuntimeValue,
            )
            .expect("fixture named Bool reference")
            .id()
    }
}

fn expression_statement(fx: &mut TestExpressions, line: i64) -> AstNode {
    node(NodeKind::ExpressionStatement(fx.int(line)), None)
}

fn return_statement(fx: &mut TestExpressions, line: i64) -> AstNode {
    node(NodeKind::Return(vec![fx.int(line)]), None)
}

fn then_statement(fx: &mut TestExpressions, line: i64) -> AstNode {
    let value = fx.int(line);
    node(
        NodeKind::ThenValue(
            crate::compiler_frontend::ast::statements::value_production::ProducedValues {
                expressions: vec![value],
                span: None,
            },
        ),
        None,
    )
}

fn assert_statement(fx: &mut TestExpressions, condition: bool) -> AstNode {
    let condition = fx.bool(condition);
    let message = fx.bool(true);
    node(NodeKind::Assert { condition, message }, None)
}

fn bool_if(
    fx: &mut TestExpressions,
    then_body: Vec<AstNode>,
    else_body: Option<Vec<AstNode>>,
) -> AstNode {
    let branch_metadata = test_if_branch_metadata(else_body.is_some());
    let condition = fx.bool(true);
    node(
        NodeKind::If(condition, then_body, else_body, branch_metadata),
        None,
    )
}

fn while_loop(condition: ExprId, body: Vec<AstNode>) -> AstNode {
    node(NodeKind::WhileLoop(condition, body), None)
}

fn collection_loop(fx: &mut TestExpressions, body: Vec<AstNode>) -> AstNode {
    node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            iterable: fx.int(1),
            step: None,
            body,
        },
        None,
    )
}

fn literal_match(
    fx: &mut TestExpressions,
    arms: Vec<Vec<AstNode>>,
    default: Option<Vec<AstNode>>,
    exhaustiveness: MatchExhaustiveness,
) -> AstNode {
    let scrutinee = fx.int(1);
    let pattern_expression = fx.int(1);
    let pattern = MatchPattern::Literal(
        PatternValue::from_expression(
            fx.builder
                .view(pattern_expression)
                .expect("fixture pattern view"),
        )
        .expect("fixture literal pattern"),
    );
    let arms = arms
        .into_iter()
        .map(|body| MatchArm {
            pattern: pattern.clone(),
            guard: None,
            body,
        })
        .collect();
    node(
        NodeKind::Match {
            scrutinee,
            arms,
            default,
            exhaustiveness,
        },
        None,
    )
}

fn exits_of(fx: &TestExpressions, body: &[AstNode]) -> ControlFlowExitSummary {
    analyze_control_flow_exits(body, fx.builder.store(), &fx.constants)
}

#[test]
fn sequencing_stops_at_a_non_fallthrough_statement() {
    let mut fx = TestExpressions::new();
    let body = vec![
        expression_statement(&mut fx, 1),
        return_statement(&mut fx, 2),
        then_statement(&mut fx, 3),
        expression_statement(&mut fx, 4),
    ];

    assert_eq!(
        exits_of(&fx, &body),
        ControlFlowExitSummary::RETURNS_TO_CALLER,
        "statements after the return are unreachable and cannot add value production"
    );
}

#[test]
fn producing_path_also_stops_sequencing() {
    let mut fx = TestExpressions::new();
    let body = vec![then_statement(&mut fx, 1), expression_statement(&mut fx, 2)];

    assert_eq!(exits_of(&fx, &body), ControlFlowExitSummary::PRODUCES_VALUE);
}

#[test]
fn alternative_branches_keep_independent_produce_and_return_facts() {
    let mut fx = TestExpressions::new();
    let then_body = vec![then_statement(&mut fx, 2)];
    let else_body = Some(vec![return_statement(&mut fx, 3)]);
    let branch = bool_if(&mut fx, then_body, else_body);

    assert_eq!(
        exits_of(&fx, &[branch]),
        ControlFlowExitSummary {
            can_fall_through: false,
            produces_value: true,
            returns_to_caller: true,
            diverges: false,
            breaks_current_loop: false,
            continues_current_loop: false,
        }
    );
}

#[test]
fn if_without_else_always_falls_through() {
    let mut fx = TestExpressions::new();
    let then_body = vec![return_statement(&mut fx, 1)];
    let branch = bool_if(&mut fx, then_body, None);

    assert_eq!(
        exits_of(&fx, &[branch]),
        ControlFlowExitSummary {
            can_fall_through: true,
            produces_value: false,
            returns_to_caller: true,
            diverges: false,
            breaks_current_loop: false,
            continues_current_loop: false,
        }
    );
}

#[test]
fn mixed_return_and_diverging_branches_union_their_facts() {
    let mut fx = TestExpressions::new();
    let then_body = vec![return_statement(&mut fx, 1)];
    let else_body = Some(vec![assert_statement(&mut fx, false)]);
    let branch = bool_if(&mut fx, then_body, else_body);

    assert_eq!(
        exits_of(&fx, &[branch]),
        ControlFlowExitSummary {
            can_fall_through: false,
            produces_value: false,
            returns_to_caller: true,
            diverges: true,
            breaks_current_loop: false,
            continues_current_loop: false,
        }
    );
}

#[test]
fn diverging_and_completing_branches_keep_the_fallthrough_path() {
    let mut fx = TestExpressions::new();
    let then_body = vec![assert_statement(&mut fx, false)];
    let else_body = Some(vec![expression_statement(&mut fx, 1)]);
    let branch = bool_if(&mut fx, then_body, else_body);

    let exits = exits_of(&fx, &[branch]);
    assert!(exits.diverges, "the false assertion branch diverges");
    assert!(
        exits.can_fall_through,
        "the completing branch still reaches the next statement"
    );
    assert!(
        !exits.diverges_on_every_path(),
        "one completing branch invalidates a declared Never body"
    );
}

#[test]
fn match_arms_and_default_union_their_exits() {
    let mut fx = TestExpressions::new();
    let arms = vec![vec![then_statement(&mut fx, 2)]];
    let default = Some(vec![return_statement(&mut fx, 3)]);
    let matched = literal_match(&mut fx, arms, default, MatchExhaustiveness::HasDefault);

    assert_eq!(
        exits_of(&fx, &[matched]),
        ControlFlowExitSummary {
            can_fall_through: false,
            produces_value: true,
            returns_to_caller: true,
            diverges: false,
            breaks_current_loop: false,
            continues_current_loop: false,
        }
    );
}

#[test]
fn default_marked_match_without_default_keeps_a_fallthrough_path() {
    let mut fx = TestExpressions::new();
    let arms = vec![vec![return_statement(&mut fx, 2)]];
    let matched = literal_match(&mut fx, arms, None, MatchExhaustiveness::HasDefault);

    assert!(
        exits_of(&fx, &[matched]).can_fall_through,
        "a missing default body leaves a path with no owned exit"
    );
}

#[test]
fn exhaustive_choice_match_uses_its_arm_exits() {
    let mut fx = TestExpressions::new();
    let arms = vec![vec![return_statement(&mut fx, 2)]];
    let matched = literal_match(&mut fx, arms, None, MatchExhaustiveness::ExhaustiveChoice);

    assert_eq!(
        exits_of(&fx, &[matched]),
        ControlFlowExitSummary::RETURNS_TO_CALLER
    );
}

#[test]
fn folded_false_assert_diverges_and_passing_assert_falls_through() {
    let mut fx = TestExpressions::new();
    let false_assertion = assert_statement(&mut fx, false);
    let true_assertion = assert_statement(&mut fx, true);

    assert_eq!(
        exits_of(&fx, &[false_assertion]),
        ControlFlowExitSummary::DIVERGES
    );
    assert_eq!(
        exits_of(&fx, &[true_assertion]),
        ControlFlowExitSummary::FALLS_THROUGH
    );
}

#[test]
fn named_folded_false_assert_diverges() {
    let mut fx = TestExpressions::new();
    let condition = fx.named_bool(false);
    let message = fx.bool(true);
    let assertion = node(NodeKind::Assert { condition, message }, None);

    assert_eq!(
        exits_of(&fx, &[assertion]),
        ControlFlowExitSummary::DIVERGES,
        "a named constant must resolve through the same folded Bool authority as a literal"
    );
}

#[test]
fn lexical_scope_propagates_loop_exits() {
    let fx = TestExpressions::new();
    let scoped = node(
        NodeKind::LexicalScope {
            body: vec![node(NodeKind::Break, None)],
        },
        None,
    );

    assert_eq!(
        exits_of(&fx, &[scoped]),
        ControlFlowExitSummary::BREAKS_CURRENT_LOOP
    );
}

#[test]
fn true_loop_converts_body_fallthrough_to_divergence() {
    let mut fx = TestExpressions::new();
    let condition = fx.bool(true);
    let loop_node = while_loop(condition, vec![expression_statement(&mut fx, 1)]);

    assert_eq!(
        exits_of(&fx, &[loop_node]),
        ControlFlowExitSummary::DIVERGES
    );
}

#[test]
fn true_loop_converts_continue_to_divergence() {
    let mut fx = TestExpressions::new();
    let condition = fx.bool(true);
    let loop_node = while_loop(condition, vec![node(NodeKind::Continue, None)]);

    assert_eq!(
        exits_of(&fx, &[loop_node]),
        ControlFlowExitSummary::DIVERGES
    );
}

#[test]
fn true_loop_preserves_caller_returns() {
    let mut fx = TestExpressions::new();
    let condition = fx.bool(true);
    let loop_node = while_loop(condition, vec![return_statement(&mut fx, 1)]);

    assert_eq!(
        exits_of(&fx, &[loop_node]),
        ControlFlowExitSummary::RETURNS_TO_CALLER
    );
}

#[test]
fn true_loop_with_reachable_break_exposes_the_post_loop_path() {
    let mut fx = TestExpressions::new();
    let condition = fx.bool(true);
    let then_body = vec![node(NodeKind::Break, None)];
    let else_body = Some(vec![node(NodeKind::Continue, None)]);
    let loop_body = bool_if(&mut fx, then_body, else_body);
    let loop_node = while_loop(condition, vec![loop_body]);

    assert_eq!(
        exits_of(&fx, &[loop_node]),
        ControlFlowExitSummary {
            can_fall_through: true,
            produces_value: false,
            returns_to_caller: false,
            diverges: true,
            breaks_current_loop: false,
            continues_current_loop: false,
        },
        "a break resumes after the loop while continue keeps the loop divergent"
    );
}

#[test]
fn nested_inner_break_does_not_escape_an_outer_true_loop() {
    let mut fx = TestExpressions::new();
    let outer_condition = fx.bool(true);
    let inner_condition = fx.bool(true);
    let inner_loop = while_loop(inner_condition, vec![node(NodeKind::Break, None)]);
    let outer_loop = while_loop(outer_condition, vec![inner_loop]);

    let exits = exits_of(&fx, &[outer_loop]);
    assert!(
        !exits.can_fall_through,
        "the inner break resumes in the outer body, whose fallthrough starts another outer iteration"
    );
    assert!(exits.diverges);
    assert!(!exits.breaks_current_loop);
}

#[test]
fn nested_continue_is_consumed_by_the_inner_loop() {
    let mut fx = TestExpressions::new();
    let outer_condition = fx.bool(true);
    let inner_condition = fx.bool(true);
    let inner_loop = while_loop(inner_condition, vec![node(NodeKind::Continue, None)]);
    let outer_loop = while_loop(outer_condition, vec![inner_loop]);

    let exits = exits_of(&fx, &[outer_loop]);
    assert!(exits.diverges);
    assert!(!exits.continues_current_loop);
}

#[test]
fn finite_collection_loop_stays_fallthrough_but_propagates_returns() {
    let mut fx = TestExpressions::new();
    let loop_body = vec![return_statement(&mut fx, 1)];
    let loop_node = collection_loop(&mut fx, loop_body);

    assert_eq!(
        exits_of(&fx, &[loop_node]),
        ControlFlowExitSummary {
            can_fall_through: true,
            produces_value: false,
            returns_to_caller: true,
            diverges: false,
            breaks_current_loop: false,
            continues_current_loop: false,
        },
        "a collection loop may run zero times, but an iteration can still return"
    );
}

#[test]
fn folded_false_loop_does_not_analyse_its_body() {
    let mut fx = TestExpressions::new();
    let condition = fx.bool(false);
    let loop_node = while_loop(condition, vec![return_statement(&mut fx, 1)]);

    assert_eq!(
        exits_of(&fx, &[loop_node]),
        ControlFlowExitSummary::FALLS_THROUGH
    );
}

#[test]
fn dynamic_loop_keeps_fallthrough_and_inner_returns_as_possible() {
    let mut fx = TestExpressions::new();
    let condition = fx.dynamic_bool();
    let loop_node = while_loop(condition, vec![return_statement(&mut fx, 1)]);

    assert_eq!(
        exits_of(&fx, &[loop_node]),
        ControlFlowExitSummary {
            can_fall_through: true,
            produces_value: false,
            returns_to_caller: true,
            diverges: false,
            breaks_current_loop: false,
            continues_current_loop: false,
        }
    );
}

#[test]
fn named_folded_true_loop_diverges() {
    let mut fx = TestExpressions::new();
    let condition = fx.named_bool(true);
    let loop_node = while_loop(condition, vec![expression_statement(&mut fx, 1)]);

    assert_eq!(
        exits_of(&fx, &[loop_node]),
        ControlFlowExitSummary::DIVERGES
    );
}

#[test]
fn hidden_return_in_a_declaration_value_if_sets_return_fact() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "choose |condition Bool| -> I32:\n    value = if condition:\n        then 1\n    else\n        return 0\n    ;\n    return value\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "choose");
    let exits = analyze_control_flow_exits(
        std::slice::from_ref(&body[0]),
        &ast.expressions,
        &ast.const_values,
    );

    assert!(
        exits.can_fall_through,
        "the producing branch still reaches the next statement"
    );
    assert!(
        exits.returns_to_caller,
        "the hidden return in the else branch must stay visible to the function"
    );
    assert!(!exits.produces_value, "a declaration never produces values");
}

#[test]
fn value_block_break_inside_a_true_loop_exposes_the_loop_exit() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "keep |condition Bool| -> I32:\n    loop true:\n        value = if condition:\n            then 1\n        else\n            break\n        ;\n    ;\n    return 0\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "keep");
    let NodeKind::WhileLoop(_, loop_body) = &body[0].kind else {
        panic!("expected the folded-true loop");
    };
    let declaration_exits = analyze_control_flow_exits(
        std::slice::from_ref(&loop_body[0]),
        &ast.expressions,
        &ast.const_values,
    );

    assert!(
        declaration_exits.can_fall_through,
        "the producing branch completes the declaration"
    );
    assert!(
        declaration_exits.breaks_current_loop,
        "the value block's break targets the enclosing loop"
    );

    let loop_exits = analyze_control_flow_exits(
        std::slice::from_ref(&body[0]),
        &ast.expressions,
        &ast.const_values,
    );
    assert!(
        loop_exits.can_fall_through,
        "the reachable break exposes the post-loop path"
    );
    assert!(
        loop_exits.diverges,
        "the producing path falls off the body and starts another iteration"
    );
    assert!(
        !loop_exits.breaks_current_loop,
        "the loop boundary consumes its own break fact"
    );
}

#[test]
fn catch_on_a_later_return_slot_keeps_the_outer_expression_worklist() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "pick |value I32| -> I32, Error!:\n    return value\n;\n\nchoose |value I32| -> I32, I32:\n    return value, pick(value) catch:\n        then 0\n    ;\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "choose");
    let exits = analyze_control_flow_exits(
        std::slice::from_ref(&body[0]),
        &ast.expressions,
        &ast.const_values,
    );

    assert!(
        exits.returns_to_caller,
        "the return statement still returns after analysing the catch on its second slot"
    );
    assert!(!exits.can_fall_through, "a return never falls through");
}

/// One `-> !` call statement with a distinctive target path.
fn never_call_statement(target_index: usize) -> AstNode {
    node(
        NodeKind::NeverCall(Box::new(NeverCall {
            target: NeverCallTarget::Source(
                PathId::try_from_index(target_index).expect("fixture path fits"),
            ),
            receiver: None,
            arguments: Vec::new(),
            name: StringId::from_index(0),
            span: None,
        })),
        None,
    )
}

/// The divergence-proof dependencies one body collects.
fn proof_targets(fx: &TestExpressions, body: &[AstNode]) -> Vec<PathId> {
    analyze_control_flow_exits_with_never_targets(body, fx.builder.store(), &fx.constants).1
}

#[test]
fn never_call_diverges_and_stops_sequencing() {
    let mut fx = TestExpressions::new();
    let body = vec![never_call_statement(10), expression_statement(&mut fx, 1)];

    assert_eq!(
        exits_of(&fx, &body),
        ControlFlowExitSummary::DIVERGES,
        "a Never call has no fallthrough and hides every later statement"
    );
}

#[test]
fn proof_targets_stop_at_independent_divergence() {
    let mut fx = TestExpressions::new();
    let body = vec![assert_statement(&mut fx, false), never_call_statement(10)];

    assert!(
        proof_targets(&fx, &body).is_empty(),
        "a call after an independent divergence is unreachable and cannot be part of the proof"
    );
}

#[test]
fn proof_targets_skip_true_loop_bodies() {
    let mut fx = TestExpressions::new();
    let condition = fx.bool(true);
    let body = vec![while_loop(condition, vec![never_call_statement(10)])];

    let (summary, targets) =
        analyze_control_flow_exits_with_never_targets(&body, fx.builder.store(), &fx.constants);

    assert!(
        summary.diverges,
        "the folded-true loop diverges structurally"
    );
    assert!(
        targets.is_empty(),
        "a call inside the loop body does not prove the enclosing body's divergence"
    );
}

#[test]
fn proof_targets_follow_reachable_branches_only() {
    let mut fx = TestExpressions::new();
    let branch = bool_if(
        &mut fx,
        vec![never_call_statement(10)],
        Some(vec![never_call_statement(11)]),
    );

    assert_eq!(
        proof_targets(&fx, &[branch]),
        vec![
            PathId::try_from_index(10).expect("fixture path fits"),
            PathId::try_from_index(11).expect("fixture path fits"),
        ],
        "both reachable branches contribute their own dependency in source order"
    );
}

/// The initializer expression of the fixture body's only declaration.
fn declaration_initializer(statement: &AstNode) -> ExprId {
    let NodeKind::VariableDeclaration(declaration) = &statement.kind else {
        panic!("the fixture statement must be a declaration");
    };
    let DeclarationValue::Initializer(expression) = declaration.value else {
        panic!("the fixture declaration must hold an authored initializer");
    };
    expression
}

/// One Never call statement whose single argument is `argument`.
fn never_call_statement_with_argument(argument: ExprId) -> AstNode {
    node(
        NodeKind::NeverCall(Box::new(NeverCall {
            target: NeverCallTarget::Source(PathId::try_from_index(30).expect("fixture path fits")),
            receiver: None,
            arguments: vec![CallArgument::positional(
                argument,
                CallAccessMode::Shared,
                None,
            )],
            name: StringId::from_index(0),
            span: None,
        })),
        None,
    )
}

#[test]
fn never_call_argument_value_block_return_reaches_the_function_body() {
    // A value block is not legal as authored call syntax, so the fixture takes the parsed
    // initializer expression and places it in the Never call's argument slot directly.
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "choose |condition Bool| -> I32:\n    value = if condition:\n        then 1\n    else\n        return 0\n    ;\n    return value\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "choose");
    let statement = never_call_statement_with_argument(declaration_initializer(&body[0]));
    let exits = analyze_control_flow_exits(
        std::slice::from_ref(&statement),
        &ast.expressions,
        &ast.const_values,
    );

    assert!(
        exits.diverges,
        "the standalone Never call still diverges on its own path"
    );
    assert!(
        exits.returns_to_caller,
        "a `return` inside the argument's value block must reach the enclosing function body"
    );
    assert!(
        !exits.can_fall_through,
        "a Never call never falls through to a later statement"
    );
}

#[test]
fn never_call_argument_value_block_break_reaches_the_enclosing_loop() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "keep |condition Bool| -> I32:\n    loop true:\n        value = if condition:\n            then 1\n        else\n            break\n        ;\n    ;\n    return 0\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "keep");
    let NodeKind::WhileLoop(_, loop_body) = &body[0].kind else {
        panic!("the fixture body must start with the authored loop");
    };
    let statement = never_call_statement_with_argument(declaration_initializer(&loop_body[0]));
    let exits = analyze_control_flow_exits(
        std::slice::from_ref(&statement),
        &ast.expressions,
        &ast.const_values,
    );

    assert!(exits.diverges, "the Never call diverges");
    assert!(
        exits.breaks_current_loop,
        "a `break` inside the argument's value block must reach the enclosing loop"
    );
}
