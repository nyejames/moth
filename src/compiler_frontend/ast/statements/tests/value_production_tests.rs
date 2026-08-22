//! Value-production helper regression tests.
//!
//! WHAT: checks the branch-flow helper that validates whether value-block bodies
//! produce values, terminate, or can fall through.
//! WHY: `if`, match, and catch value blocks all depend on this shared analysis;
//! regressions here otherwise surface as unrelated parser diagnostics.

use super::{BranchFlow, ProducedValues, analyze_branch_flow};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::tests::ast_fixture_support::{node, test_source_location};

use crate::compiler_frontend::value_mode::ValueMode;

fn then_value(line: i32) -> AstNode {
    node(
        NodeKind::ThenValue(ProducedValues {
            expressions: vec![Expression::int(
                line,
                test_source_location(line),
                ValueMode::ImmutableOwned,
            )],
            location: test_source_location(line),
        }),
        test_source_location(line),
    )
}

fn return_value(line: i32) -> AstNode {
    node(
        NodeKind::Return(vec![Expression::int(
            line,
            test_source_location(line),
            ValueMode::ImmutableOwned,
        )]),
        test_source_location(line),
    )
}

fn expression_statement(line: i32) -> AstNode {
    node(
        NodeKind::ExpressionStatement(Expression::int(
            line,
            test_source_location(line),
            ValueMode::ImmutableOwned,
        )),
        test_source_location(line),
    )
}

fn assert_statement(condition: Expression, line: i32) -> AstNode {
    node(
        NodeKind::Assert {
            condition,
            // Branch-flow tests inspect only the condition's terminality effect.
            message: Expression::bool(true, test_source_location(line), ValueMode::ImmutableOwned),
        },
        test_source_location(line),
    )
}

#[test]
fn branch_flow_reports_direct_value_production() {
    let flow = analyze_branch_flow(&[
        expression_statement(1),
        then_value(2),
        expression_statement(3),
    ]);

    assert_eq!(
        flow,
        BranchFlow::ProducesValue,
        "analysis should stop at the first reachable then-value"
    );
}

#[test]
fn branch_flow_requires_both_if_paths_to_produce() {
    let producing_if = node(
        NodeKind::If(
            Expression::bool(true, test_source_location(1), ValueMode::ImmutableOwned),
            vec![then_value(2)],
            Some(vec![then_value(3)]),
        ),
        test_source_location(1),
    );

    let fallthrough_if = node(
        NodeKind::If(
            Expression::bool(true, test_source_location(4), ValueMode::ImmutableOwned),
            vec![then_value(5)],
            Some(vec![expression_statement(6)]),
        ),
        test_source_location(4),
    );

    assert_eq!(
        analyze_branch_flow(&[producing_if]),
        BranchFlow::ProducesValue
    );
    assert_eq!(
        analyze_branch_flow(&[fallthrough_if]),
        BranchFlow::FallsThrough
    );
}

#[test]
fn branch_flow_combines_match_arms_and_default() {
    let producing_match = node(
        NodeKind::Match {
            scrutinee: Expression::int(1, test_source_location(1), ValueMode::ImmutableOwned),
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(Expression::int(
                    1,
                    test_source_location(2),
                    ValueMode::ImmutableOwned,
                )),
                guard: None,
                body: vec![then_value(3)],
            }],
            default: Some(vec![then_value(4)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_source_location(1),
    );

    let mixed_match = node(
        NodeKind::Match {
            scrutinee: Expression::int(1, test_source_location(5), ValueMode::ImmutableOwned),
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(Expression::int(
                    1,
                    test_source_location(6),
                    ValueMode::ImmutableOwned,
                )),
                guard: None,
                body: vec![then_value(7)],
            }],
            default: Some(vec![return_value(8)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_source_location(5),
    );

    assert_eq!(
        analyze_branch_flow(&[producing_match]),
        BranchFlow::ProducesValue
    );
    assert_eq!(
        analyze_branch_flow(&[mixed_match]),
        BranchFlow::FallsThrough,
        "mixed produce/terminate paths are not a single value-producing flow"
    );
}

#[test]
fn branch_flow_reports_assert_false_as_terminal() {
    let flow = analyze_branch_flow(&[
        expression_statement(1),
        assert_statement(
            Expression::bool(false, test_source_location(2), ValueMode::ImmutableOwned),
            2,
        ),
        then_value(3),
    ]);

    assert_eq!(flow, BranchFlow::Terminates);
}

#[test]
fn branch_flow_does_not_treat_passing_assert_as_terminal() {
    let flow = analyze_branch_flow(&[assert_statement(
        Expression::bool(true, test_source_location(1), ValueMode::ImmutableOwned),
        1,
    )]);

    assert_eq!(flow, BranchFlow::FallsThrough);
}

#[test]
fn branch_flow_combines_assert_false_branches_as_terminal() {
    let terminating_if = node(
        NodeKind::If(
            Expression::bool(true, test_source_location(1), ValueMode::ImmutableOwned),
            vec![assert_statement(
                Expression::bool(false, test_source_location(2), ValueMode::ImmutableOwned),
                2,
            )],
            Some(vec![assert_statement(
                Expression::bool(false, test_source_location(3), ValueMode::ImmutableOwned),
                3,
            )]),
        ),
        test_source_location(1),
    );

    let partial_if = node(
        NodeKind::If(
            Expression::bool(true, test_source_location(4), ValueMode::ImmutableOwned),
            vec![assert_statement(
                Expression::bool(false, test_source_location(5), ValueMode::ImmutableOwned),
                5,
            )],
            None,
        ),
        test_source_location(4),
    );

    assert_eq!(
        analyze_branch_flow(&[terminating_if]),
        BranchFlow::Terminates
    );
    assert_eq!(
        analyze_branch_flow(&[partial_if]),
        BranchFlow::FallsThrough,
        "if with only one assert-false branch still has a fallthrough path"
    );
}
