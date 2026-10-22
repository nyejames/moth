//! Value-production helper regression tests.
//!
//! WHAT: checks the all-path exit summary and reachable produced-value traversal
//! that value `if`, match and catch share.
//! WHY: mixed produce/terminate completeness and inferred result slots are hidden
//! invariants that integration output cannot fully inspect.

use super::types::BranchExitSummary;
use super::{ProducedValues, analyze_branch_exits};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, PatternValue,
};
use crate::compiler_frontend::ast::statements::value_production::completeness::validate_closed_branch_pair;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueIfBlock,
};
use crate::compiler_frontend::compiler_errors::BoundaryFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data::InvalidValueReceiverView, types_borrows_targets::TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidControlFlowStatementReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, node, test_if_branch_metadata,
};
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

    fn view(&self, id: ExprId) -> ExpressionView<'_> {
        self.builder.view(id).expect("fixture expression view")
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

fn expression_statement(fx: &mut TestExpressions, line: i32) -> AstNode {
    let value = fx.int(i64::from(line));
    node(NodeKind::ExpressionStatement(value), None)
}

fn assert_statement(fx: &mut TestExpressions, condition: bool, _line: i32) -> AstNode {
    let condition = fx.bool(condition);
    let message = fx.bool(true);
    node(NodeKind::Assert { condition, message }, None)
}

fn bool_if(
    fx: &mut TestExpressions,
    then_body: Vec<AstNode>,
    else_body: Option<Vec<AstNode>>,
    _line: i32,
) -> AstNode {
    let branch_metadata = test_if_branch_metadata(else_body.is_some());
    let condition = fx.bool(true);
    node(
        NodeKind::If(condition, then_body, else_body, branch_metadata),
        None,
    )
}

fn literal_match(
    fx: &mut TestExpressions,
    arm_body: Vec<AstNode>,
    default: Option<Vec<AstNode>>,
    line: i32,
) -> AstNode {
    let scrutinee = fx.int(i64::from(line));
    let pattern_expression = fx.int(i64::from(line));
    let pattern = MatchPattern::Literal(
        PatternValue::from_expression(fx.view(pattern_expression))
            .expect("fixture literal pattern"),
    );
    node(
        NodeKind::Match {
            scrutinee,
            arms: vec![MatchArm {
                pattern,
                guard: None,
                body: arm_body,
            }],
            default,
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    )
}

#[test]
fn branch_exits_report_direct_value_production() {
    let mut fx = TestExpressions::new();
    let first = expression_statement(&mut fx, 1);
    let second = then_value(&mut fx, 2);
    let third = expression_statement(&mut fx, 3);
    let summary = analyze_branch_exits(&[first, second, third], &fx.builder, &fx.constants);

    assert_eq!(
        summary,
        BranchExitSummary::PRODUCES,
        "statements after a producing path must not change the summary"
    );
}

#[test]
fn branch_exits_report_direct_termination() {
    let mut fx = TestExpressions::new();
    let first = expression_statement(&mut fx, 1);
    let second = return_value(&mut fx, 2);
    let third = then_value(&mut fx, 3);
    let summary = analyze_branch_exits(&[first, second, third], &fx.builder, &fx.constants);

    assert_eq!(summary, BranchExitSummary::TERMINATES);
}

#[test]
fn branch_exits_report_true_fallthrough() {
    let mut fx = TestExpressions::new();
    let first = expression_statement(&mut fx, 1);
    let second = expression_statement(&mut fx, 2);
    let summary = analyze_branch_exits(&[first, second], &fx.builder, &fx.constants);

    assert_eq!(summary, BranchExitSummary::FALLS_THROUGH);
}

#[test]
fn branch_exits_union_mixed_produce_and_terminate_alternatives() {
    let mut fx = TestExpressions::new();
    let then_body = vec![then_value(&mut fx, 2)];
    let else_body = vec![return_value(&mut fx, 3)];
    let mixed_if = bool_if(&mut fx, then_body, Some(else_body), 1);

    assert_eq!(
        analyze_branch_exits(&[mixed_if], &fx.builder, &fx.constants),
        BranchExitSummary {
            can_fall_through: false,
            produces_value: true,
            terminates: true,
        }
    );
}

#[test]
fn branch_exits_union_produce_and_fallthrough_alternatives() {
    let mut fx = TestExpressions::new();
    let then_body = vec![then_value(&mut fx, 2)];
    let else_body = vec![expression_statement(&mut fx, 3)];
    let mixed_if = bool_if(&mut fx, then_body, Some(else_body), 1);

    assert_eq!(
        analyze_branch_exits(&[mixed_if], &fx.builder, &fx.constants),
        BranchExitSummary {
            can_fall_through: true,
            produces_value: true,
            terminates: false,
        }
    );
}

#[test]
fn branch_exits_union_terminate_and_fallthrough_alternatives() {
    let mut fx = TestExpressions::new();
    let then_body = vec![return_value(&mut fx, 2)];
    let else_body = vec![expression_statement(&mut fx, 3)];
    let mixed_if = bool_if(&mut fx, then_body, Some(else_body), 1);

    assert_eq!(
        analyze_branch_exits(&[mixed_if], &fx.builder, &fx.constants),
        BranchExitSummary {
            can_fall_through: true,
            produces_value: false,
            terminates: true,
        }
    );
}

#[test]
fn branch_exits_compose_nested_if_and_match() {
    let mut fx = TestExpressions::new();
    let arm_body = vec![then_value(&mut fx, 3)];
    let match_default = Some(vec![return_value(&mut fx, 4)]);
    let inner_match = literal_match(&mut fx, arm_body, match_default, 2);
    let outer_then = vec![inner_match];
    let outer_else = Some(vec![then_value(&mut fx, 5)]);
    let nested = bool_if(&mut fx, outer_then, outer_else, 1);

    assert_eq!(
        analyze_branch_exits(&[nested], &fx.builder, &fx.constants),
        BranchExitSummary {
            can_fall_through: false,
            produces_value: true,
            terminates: true,
        }
    );
}

#[test]
fn branch_exits_sequence_statements_after_partial_fallthrough() {
    let mut fx = TestExpressions::new();
    let then_body = vec![then_value(&mut fx, 2)];
    let partial_if = bool_if(&mut fx, then_body, None, 1);
    let following = then_value(&mut fx, 3);
    let summary = analyze_branch_exits(&[partial_if, following], &fx.builder, &fx.constants);

    assert_eq!(
        summary,
        BranchExitSummary {
            can_fall_through: false,
            produces_value: true,
            terminates: false,
        }
    );
}

#[test]
fn branch_exits_ignore_statements_after_every_path_has_exited() {
    let mut fx = TestExpressions::new();
    let then_body = vec![then_value(&mut fx, 2)];
    let else_body = vec![return_value(&mut fx, 3)];
    let closed_if = bool_if(&mut fx, then_body, Some(else_body), 1);
    let following = expression_statement(&mut fx, 4);
    let summary = analyze_branch_exits(&[closed_if, following], &fx.builder, &fx.constants);

    assert_eq!(
        summary,
        BranchExitSummary {
            can_fall_through: false,
            produces_value: true,
            terminates: true,
        }
    );
}

#[test]
fn branch_exits_recurse_into_lexical_scopes() {
    let mut fx = TestExpressions::new();
    let body = vec![then_value(&mut fx, 2)];
    let scoped = node(NodeKind::LexicalScope { body }, None);

    assert_eq!(
        analyze_branch_exits(&[scoped], &fx.builder, &fx.constants),
        BranchExitSummary::PRODUCES
    );
}

#[test]
fn branch_exits_require_both_if_paths_to_exit() {
    let mut fx = TestExpressions::new();
    let producing_then = vec![then_value(&mut fx, 2)];
    let producing_else = vec![then_value(&mut fx, 3)];
    let producing_if = bool_if(&mut fx, producing_then, Some(producing_else), 1);
    let fallthrough_then = vec![then_value(&mut fx, 5)];
    let fallthrough_else = vec![expression_statement(&mut fx, 6)];
    let fallthrough_if = bool_if(&mut fx, fallthrough_then, Some(fallthrough_else), 4);

    assert_eq!(
        analyze_branch_exits(&[producing_if], &fx.builder, &fx.constants),
        BranchExitSummary::PRODUCES
    );
    assert_eq!(
        analyze_branch_exits(&[fallthrough_if], &fx.builder, &fx.constants),
        BranchExitSummary {
            can_fall_through: true,
            produces_value: true,
            terminates: false,
        }
    );
}

#[test]
fn branch_exits_combine_match_arms_and_default() {
    let mut fx = TestExpressions::new();
    let producing_arm = vec![then_value(&mut fx, 3)];
    let producing_default = Some(vec![then_value(&mut fx, 4)]);
    let producing_match = literal_match(&mut fx, producing_arm, producing_default, 1);
    let mixed_arm = vec![then_value(&mut fx, 7)];
    let mixed_default = Some(vec![return_value(&mut fx, 8)]);
    let mixed_match = literal_match(&mut fx, mixed_arm, mixed_default, 5);

    assert_eq!(
        analyze_branch_exits(&[producing_match], &fx.builder, &fx.constants),
        BranchExitSummary::PRODUCES
    );
    assert_eq!(
        analyze_branch_exits(&[mixed_match], &fx.builder, &fx.constants),
        BranchExitSummary {
            can_fall_through: false,
            produces_value: true,
            terminates: true,
        },
        "mixed produce/terminate paths are a complete all-path exit"
    );
}

#[test]
fn branch_exits_report_assert_false_as_terminal() {
    let mut fx = TestExpressions::new();
    let first = expression_statement(&mut fx, 1);
    let assertion = assert_statement(&mut fx, false, 2);
    let third = then_value(&mut fx, 3);
    let summary = analyze_branch_exits(&[first, assertion, third], &fx.builder, &fx.constants);

    assert_eq!(summary, BranchExitSummary::TERMINATES);
}

#[test]
fn branch_exits_do_not_treat_passing_assert_as_terminal() {
    let mut fx = TestExpressions::new();
    let assertion = assert_statement(&mut fx, true, 1);
    let summary = analyze_branch_exits(&[assertion], &fx.builder, &fx.constants);

    assert_eq!(summary, BranchExitSummary::FALLS_THROUGH);
}

#[test]
fn branch_exits_combine_assert_false_branches_as_terminal() {
    let mut fx = TestExpressions::new();
    let terminating_then = vec![assert_statement(&mut fx, false, 2)];
    let terminating_else = Some(vec![assert_statement(&mut fx, false, 3)]);
    let terminating_if = bool_if(&mut fx, terminating_then, terminating_else, 1);

    let partial_then = vec![assert_statement(&mut fx, false, 5)];
    let partial_if = bool_if(&mut fx, partial_then, None, 4);

    assert_eq!(
        analyze_branch_exits(&[terminating_if], &fx.builder, &fx.constants),
        BranchExitSummary::TERMINATES
    );
    assert_eq!(
        analyze_branch_exits(&[partial_if], &fx.builder, &fx.constants),
        BranchExitSummary {
            can_fall_through: true,
            produces_value: false,
            terminates: true,
        },
        "if with only one assert-false branch still has a fallthrough path"
    );
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
    let error = validate_closed_branch_pair(
        analyze_branch_exits(&[first], &fx.builder, &fx.constants),
        analyze_branch_exits(&[second], &fx.builder, &fx.constants),
        None,
    )
    .expect_err("a value-if whose every path terminates has no value to provide");
    let crate::compiler_frontend::ast::expressions::error::ExpressionParseError::Diagnostic(
        diagnostic,
    ) = error
    else {
        panic!("expected user diagnostic, found infrastructure error: {error:?}")
    };

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidControlFlowStatement {
            reason: InvalidControlFlowStatementReason::ValueIfNoProducingPath,
        }
    ));
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
fn assert_type_mismatch_failure(failure: &BoundaryFailure) {
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
