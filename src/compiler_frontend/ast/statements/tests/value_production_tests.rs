//! Value-production consumer regression tests.
//!
//! WHAT: checks the completeness policy that consumes the shared exit summary and the
//! reachable produced-value traversal used by inference.
//! WHY: mixed producing and exiting branches, nested-loop exit consumption, invalid fallthrough
//! and inferred result slots are hidden invariants that integration output cannot fully inspect.
//! Pure exit facts are covered by `control_flow_exits_tests.rs`.

use super::ProducedValues;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::statements::control_flow_exits::{
    ControlFlowExitSummary, analyze_control_flow_exits,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, PatternValue,
};
use crate::compiler_frontend::ast::statements::never_call::{NeverCall, NeverCallTarget};
use crate::compiler_frontend::ast::statements::value_production::completeness::{
    validate_closed_branch_pair, validate_value_match_completeness,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueIfBlock,
};
use crate::compiler_frontend::compiler_errors::CompilationResult;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data::{InvalidReturnShapeView, InvalidValueReceiverView},
    choices_control_flow::InvalidControlFlowStatementView,
    types_borrows_targets::TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, InvalidControlFlowStatementReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tests::ast_fixture_support::{function_body_by_name, node};
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_first_error_type_display, parse_single_file_ast,
    parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// Builder plus constant store for value-production fixtures.
///
/// WHY: literals need the constant store, and expression constructors need the builder.
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

    /// One exact `I32` literal expression for value-production fixtures.
    ///
    /// WHY: an unconstrained whole literal defaults to `I32`, so fixtures model the completed
    ///      expression their authored source produces.
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
}

fn then_value(fx: &mut TestExpressions, _line: i32) -> AstNode {
    let value = fx.int(1);
    node(
        NodeKind::ThenValue(ProducedValues {
            expressions: vec![value],
            span: None,
        }),
        None,
    )
}

fn return_value(fx: &mut TestExpressions, line: i32) -> AstNode {
    let value = fx.int(i64::from(line));
    node(NodeKind::Return(vec![value]), None)
}

fn true_loop(fx: &mut TestExpressions, body: Vec<AstNode>) -> AstNode {
    let condition = fx.bool(true);
    node(NodeKind::WhileLoop(condition, body), None)
}

/// One standalone terminating call node; it owns no produced value.
fn never_call_statement() -> AstNode {
    node(
        NodeKind::NeverCall(Box::new(NeverCall {
            target: NeverCallTarget::Source(PathId::ROOT),
            receiver: None,
            arguments: Vec::new(),
            name: StringId::from_index(0),
            span: None,
        })),
        None,
    )
}

fn branch_exits(fx: &TestExpressions, node: &AstNode) -> ControlFlowExitSummary {
    analyze_control_flow_exits(
        std::slice::from_ref(node),
        fx.builder.store(),
        &fx.constants,
    )
}

fn literal_arm(fx: &mut TestExpressions, body: Vec<AstNode>) -> MatchArm {
    let pattern_expression = fx.int(1);
    let pattern = MatchPattern::Literal(
        PatternValue::from_expression(
            fx.builder
                .view(pattern_expression)
                .expect("fixture pattern view"),
        )
        .expect("fixture literal pattern"),
    );
    MatchArm {
        pattern,
        guard: None,
        body,
    }
}

#[test]
fn inferred_block_value_if_stores_non_empty_result_type_ids() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "choose |ready Bool| -> String:\n    label = if ready:\n        then \"ready\"\n    else\n        then \"waiting\"\n    ;\n    return label\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "choose");
    let NodeKind::VariableDeclaration(declaration) = &body[0].kind else {
        panic!("expected inferred value-if declaration");
    };
    let initializer = ast
        .expressions
        .view(
            declaration
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .expect("fixture initializer view");
    let ExpressionOperationView::ValueBlock(block) = initializer.operation() else {
        panic!("expected value block expression");
    };
    let ValueBlock::If(ValueIfBlock {
        result_type_ids, ..
    }) = block
    else {
        panic!("expected value if block");
    };

    assert_eq!(result_type_ids.as_slice(), [builtin_type_ids::STRING]);
    assert_eq!(declaration.type_id, builtin_type_ids::STRING);
    assert_eq!(initializer.success_types(), [builtin_type_ids::STRING]);
}

#[test]
fn mixed_multi_bind_value_block_preserves_ordered_success_shape() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "choose |ready Bool| -> String, I32:\n    first, second I32 = if ready:\n        then \"ready\", 1\n    else\n        then \"waiting\", 2\n    ;\n    return first, second\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "choose");
    let NodeKind::MultiBind { value, .. } = &body[0].kind else {
        panic!("expected multi-bind");
    };
    assert_eq!(
        ast.expressions
            .view(*value)
            .expect("multi-bind value view")
            .success_types(),
        [
            builtin_type_ids::STRING,
            builtin_type_ids::fixed_scalar(FixedScalar::I32)
        ],
    );
}

#[test]
fn mixed_multi_bind_value_block_uses_existing_slot_receiver() {
    let (ast, paths, strings) = parse_single_file_ast(
        "choose |ready Bool, first ~I32| -> I32, I32:\n\
             ~first, second = if ready:\n\
                 then 1, 2\n\
             else\n\
                 then 3, 4\n\
             ;\n\
             return first, second\n;\n",
    );
    let body = function_body_by_name(&ast, &paths, &strings, "choose");
    let NodeKind::MultiBind { targets, .. } = &body[0].kind else {
        panic!("expected mixed value-block multi-bind");
    };
    assert!(matches!(
        targets[0].kind,
        crate::compiler_frontend::ast::ast_nodes::MultiBindTargetKind::Assignment
    ));
    assert!(matches!(
        targets[1].kind,
        crate::compiler_frontend::ast::ast_nodes::MultiBindTargetKind::Declaration
    ));
}

#[test]
fn single_prefixed_receiver_does_not_parse_rhs_comma_as_a_target() {
    let failure = parse_single_file_ast_diagnostic(
        "choose |ready Bool, target ~I32|:\n\
             ~target = if ready: then 1, 2\n\
             else then 3, 4\n\
             ;\n\
         ;\n",
    );
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.too_many_return_values"),
    );
    let facts = diagnostic
        .family_view::<InvalidReturnShapeView>()
        .expect("the single receiver owns the arity diagnostic");
    assert_eq!(facts.expected_count, Some(1));
}

#[test]
fn inferred_then_rejects_zero_and_multiple_success_producers() {
    for (producer, produced_count) in [("empty()", 0), ("pair()", 2)] {
        let source = format!(
            "empty ||:\n;\npair || -> String, I32:\n    return \"name\", 1\n;\nchoose |ready Bool| -> String:\n    label = if ready:\n        then {producer}\n    else\n        then \"fallback\"\n    ;\n    return label\n;\n"
        );
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0087", None);
        let facts = diagnostic
            .family_view::<InvalidValueReceiverView>()
            .expect("the diagnostic is an invalid value receiver");
        assert_eq!(facts.receiver, ValueReceiverContext::Then, "{source}");
        assert_eq!(facts.target_count, 1, "{source}");
        assert_eq!(facts.produced_count, produced_count, "{source}");
    }
}

#[test]
fn inferred_nested_then_rejects_multiple_success_producer() {
    let failure = parse_single_file_ast_diagnostic(
        "pair || -> String, I32:\n    return \"name\", 1\n;\nchoose |ready Bool, nested Bool| -> String:\n    label = if ready:\n        if nested:\n            then \"first\"\n        else\n            then pair()\n        ;\n    else\n        then \"fallback\"\n    ;\n    return label\n;\n",
    );
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0087", None);
    let facts = diagnostic
        .family_view::<InvalidValueReceiverView>()
        .expect("the diagnostic is an invalid value receiver");
    assert_eq!(facts.receiver, ValueReceiverContext::Then);
    assert_eq!(facts.target_count, 1);
    assert_eq!(facts.produced_count, 2);
}

#[test]
fn shared_validator_rejects_all_terminating_value_if() {
    let mut fx = TestExpressions::new();
    let first = return_value(&mut fx, 1);
    let second = return_value(&mut fx, 2);
    let error =
        validate_closed_branch_pair(branch_exits(&fx, &first), branch_exits(&fx, &second), None)
            .expect_err("a value-if whose every path terminates has no value to provide");
    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected user diagnostic, found infrastructure error: {error:?}")
    };

    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    let facts = diagnostic
        .family_view::<InvalidControlFlowStatementView>()
        .expect("the diagnostic is an invalid control-flow statement");
    assert_eq!(
        facts.reason,
        InvalidControlFlowStatementReason::ValueIfNoProducingPath
    );
}

#[test]
fn shared_validator_accepts_mixed_produce_and_terminate_branches() {
    let mut fx = TestExpressions::new();
    let producing = then_value(&mut fx, 1);
    let returning = return_value(&mut fx, 2);

    validate_closed_branch_pair(
        branch_exits(&fx, &producing),
        branch_exits(&fx, &returning),
        None,
    )
    .expect("a producing branch and a terminating branch complete the value site");
}

#[test]
fn shared_validator_accepts_loop_exits_targeting_an_outside_loop() {
    for loop_exit in [NodeKind::Break, NodeKind::Continue] {
        let mut fx = TestExpressions::new();
        let exit = node(loop_exit, None);
        let producing = then_value(&mut fx, 1);

        validate_closed_branch_pair(
            branch_exits(&fx, &exit),
            branch_exits(&fx, &producing),
            None,
        )
        .expect("break and continue leave the value block for an enclosing loop");
    }
}

#[test]
fn shared_validator_rejects_a_nested_loop_consuming_its_own_break() {
    let mut fx = TestExpressions::new();
    let nested_break = true_loop(&mut fx, vec![node(NodeKind::Break, None)]);
    let producing = then_value(&mut fx, 1);

    let error = validate_closed_branch_pair(
        branch_exits(&fx, &nested_break),
        branch_exits(&fx, &producing),
        None,
    )
    .expect_err("the nested loop consumes its break and then falls through the branch");
    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected user diagnostic, found infrastructure error: {error:?}")
    };

    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    let facts = diagnostic
        .family_view::<InvalidControlFlowStatementView>()
        .expect("the diagnostic is an invalid control-flow statement");
    assert_eq!(
        facts.reason,
        InvalidControlFlowStatementReason::ValueIfBranchFallsThrough
    );
}

#[test]
fn shared_validator_rejects_a_real_fallthrough_branch() {
    let mut fx = TestExpressions::new();
    let falling_through = node(NodeKind::ExpressionStatement(fx.int(1)), None);
    let producing = then_value(&mut fx, 2);

    let error = validate_closed_branch_pair(
        branch_exits(&fx, &falling_through),
        branch_exits(&fx, &producing),
        None,
    )
    .expect_err("a branch that reaches its end still falls through");
    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected user diagnostic, found infrastructure error: {error:?}")
    };

    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    let facts = diagnostic
        .family_view::<InvalidControlFlowStatementView>()
        .expect("the diagnostic is an invalid control-flow statement");
    assert_eq!(
        facts.reason,
        InvalidControlFlowStatementReason::ValueIfBranchFallsThrough
    );
}

#[test]
fn value_match_arms_accept_loop_exits_targeting_an_outside_loop() {
    for loop_exit in [NodeKind::Break, NodeKind::Continue] {
        let mut fx = TestExpressions::new();
        let exit = node(loop_exit, None);
        let producing = then_value(&mut fx, 1);
        let arms = vec![
            literal_arm(&mut fx, vec![exit]),
            literal_arm(&mut fx, vec![producing]),
        ];

        validate_value_match_completeness(&arms, None, None, &fx.builder, &fx.constants)
            .expect("match arms may leave the value block for an enclosing loop");
    }
}

#[test]
fn value_match_rejects_an_arm_whose_nested_loop_consumes_its_break() {
    let mut fx = TestExpressions::new();
    let nested_break = true_loop(&mut fx, vec![node(NodeKind::Break, None)]);
    let producing = then_value(&mut fx, 1);
    let arms = vec![
        literal_arm(&mut fx, vec![nested_break]),
        literal_arm(&mut fx, vec![producing]),
    ];

    let error = validate_value_match_completeness(&arms, None, None, &fx.builder, &fx.constants)
        .expect_err("the nested loop consumes its break and then falls through the arm");
    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected user diagnostic, found infrastructure error: {error:?}")
    };

    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    let facts = diagnostic
        .family_view::<InvalidControlFlowStatementView>()
        .expect("the diagnostic is an invalid control-flow statement");
    assert_eq!(
        facts.reason,
        InvalidControlFlowStatementReason::ValueIfBranchFallsThrough
    );
}

#[test]
fn never_call_branch_completes_a_value_producer_without_producing() {
    let mut fx = TestExpressions::new();
    let then_body = [then_value(&mut fx, 1)];
    let never_body = [never_call_statement()];

    let never_exits = branch_exits(&fx, &never_body[0]);
    assert!(never_exits.diverges);
    assert!(!never_exits.produces_value);
    assert!(!never_exits.can_fall_through);

    validate_closed_branch_pair(branch_exits(&fx, &then_body[0]), never_exits, None)
        .expect("a Never branch diverges instead of falling through or producing a value");
}

#[test]
fn shared_validator_still_requires_one_producing_path_when_others_diverge() {
    let mut fx = TestExpressions::new();
    let diversifying = true_loop(&mut fx, vec![node(NodeKind::Continue, None)]);
    let returning = return_value(&mut fx, 2);

    let error = validate_closed_branch_pair(
        branch_exits(&fx, &diversifying),
        branch_exits(&fx, &returning),
        None,
    )
    .expect_err("a value-if needs at least one path that produces a value");
    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected user diagnostic, found infrastructure error: {error:?}")
    };

    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    let facts = diagnostic
        .family_view::<InvalidControlFlowStatementView>()
        .expect("the diagnostic is an invalid control-flow statement");
    assert_eq!(
        facts.reason,
        InvalidControlFlowStatementReason::ValueIfNoProducingPath
    );
}

#[test]
fn inferred_block_value_if_rejects_later_nested_produced_type_conflict() {
    let failure = parse_single_file_ast_diagnostic(
        "choose |ready Bool, use_fallback Bool| -> String:\n    label = if ready:\n        if use_fallback:\n            then \"fallback\"\n        else\n            then 1\n        ;\n    else\n        then \"waiting\"\n    ;\n    return label\n;\n",
    );

    assert_type_mismatch_failure(&failure);
}

#[test]
fn inferred_multi_bind_rejects_later_nested_produced_type_conflict() {
    let failure = parse_single_file_ast_diagnostic(
        "choose |ready Bool, use_fallback Bool| -> String, I32:\n    first, second I32 = if ready:\n        if use_fallback:\n            then \"fallback\", 1\n        else\n            then 2, 1\n        ;\n    else\n        then \"waiting\", 2\n    ;\n    return first, second\n;\n",
    );

    assert_type_mismatch_failure(&failure);
}

/// Asserts that a nested producer conflict keeps its stable code and exact expected/found types.
fn assert_type_mismatch_failure(failure: &CompilationResult<()>) {
    let diagnostic = assert_ast_diagnostic(failure, "MOTH-TYPE-0001", None);
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("the diagnostic is a type mismatch");
    assert_eq!(
        ast_first_error_type_display(failure, facts.expected),
        "String"
    );
    assert_eq!(ast_first_error_type_display(failure, facts.found), "I32");
}
