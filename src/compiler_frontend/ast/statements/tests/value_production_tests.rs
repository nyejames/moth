//! Value-production helper regression tests.
//!
//! WHAT: checks the all-path exit summary and reachable produced-value traversal
//! that value `if`, match and catch share.
//! WHY: mixed produce/terminate completeness and inferred result slots are hidden
//! invariants that integration output cannot fully inspect.

use super::types::BranchExitSummary;
use super::{ProducedValues, analyze_branch_exits};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::completeness::validate_value_if_completeness;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueIfBlock,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, InvalidControlFlowStatementReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, node, test_source_location,
};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
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
            // Branch-exit tests inspect only the condition's terminality effect.
            message: Expression::bool(true, test_source_location(line), ValueMode::ImmutableOwned),
        },
        test_source_location(line),
    )
}

fn bool_if(then_body: Vec<AstNode>, else_body: Option<Vec<AstNode>>, line: i32) -> AstNode {
    node(
        NodeKind::If(
            Expression::bool(true, test_source_location(line), ValueMode::ImmutableOwned),
            then_body,
            else_body,
        ),
        test_source_location(line),
    )
}

fn literal_match(arm_body: Vec<AstNode>, default: Option<Vec<AstNode>>, line: i32) -> AstNode {
    node(
        NodeKind::Match {
            scrutinee: Expression::int(line, test_source_location(line), ValueMode::ImmutableOwned),
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(Expression::int(
                    line,
                    test_source_location(line + 1),
                    ValueMode::ImmutableOwned,
                )),
                guard: None,
                body: arm_body,
            }],
            default,
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_source_location(line),
    )
}

#[test]
fn branch_exits_report_direct_value_production() {
    let summary = analyze_branch_exits(&[
        expression_statement(1),
        then_value(2),
        expression_statement(3),
    ]);

    assert_eq!(
        summary,
        BranchExitSummary::PRODUCES,
        "statements after a producing path must not change the summary"
    );
}

#[test]
fn branch_exits_report_direct_termination() {
    let summary = analyze_branch_exits(&[expression_statement(1), return_value(2), then_value(3)]);

    assert_eq!(summary, BranchExitSummary::TERMINATES);
}

#[test]
fn branch_exits_report_true_fallthrough() {
    let summary = analyze_branch_exits(&[expression_statement(1), expression_statement(2)]);

    assert_eq!(summary, BranchExitSummary::FALLS_THROUGH);
}

#[test]
fn branch_exits_union_mixed_produce_and_terminate_alternatives() {
    let mixed_if = bool_if(vec![then_value(2)], Some(vec![return_value(3)]), 1);

    assert_eq!(
        analyze_branch_exits(&[mixed_if]),
        BranchExitSummary {
            can_fall_through: false,
            produces_value: true,
            terminates: true,
        }
    );
}

#[test]
fn branch_exits_union_produce_and_fallthrough_alternatives() {
    let mixed_if = bool_if(vec![then_value(2)], Some(vec![expression_statement(3)]), 1);

    assert_eq!(
        analyze_branch_exits(&[mixed_if]),
        BranchExitSummary {
            can_fall_through: true,
            produces_value: true,
            terminates: false,
        }
    );
}

#[test]
fn branch_exits_union_terminate_and_fallthrough_alternatives() {
    let mixed_if = bool_if(
        vec![return_value(2)],
        Some(vec![expression_statement(3)]),
        1,
    );

    assert_eq!(
        analyze_branch_exits(&[mixed_if]),
        BranchExitSummary {
            can_fall_through: true,
            produces_value: false,
            terminates: true,
        }
    );
}

#[test]
fn branch_exits_compose_nested_if_and_match() {
    let nested = bool_if(
        vec![literal_match(
            vec![then_value(3)],
            Some(vec![return_value(4)]),
            2,
        )],
        Some(vec![then_value(5)]),
        1,
    );

    assert_eq!(
        analyze_branch_exits(&[nested]),
        BranchExitSummary {
            can_fall_through: false,
            produces_value: true,
            terminates: true,
        }
    );
}

#[test]
fn branch_exits_sequence_statements_after_partial_fallthrough() {
    let partial_if = bool_if(vec![then_value(2)], None, 1);
    let summary = analyze_branch_exits(&[partial_if, then_value(3)]);

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
    let closed_if = bool_if(vec![then_value(2)], Some(vec![return_value(3)]), 1);
    let summary = analyze_branch_exits(&[closed_if, expression_statement(4)]);

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
fn branch_exits_recurse_into_scoped_blocks() {
    let scoped = node(
        NodeKind::ScopedBlock {
            body: vec![then_value(2)],
        },
        test_source_location(1),
    );

    assert_eq!(analyze_branch_exits(&[scoped]), BranchExitSummary::PRODUCES);
}

#[test]
fn branch_exits_require_both_if_paths_to_exit() {
    let producing_if = bool_if(vec![then_value(2)], Some(vec![then_value(3)]), 1);
    let fallthrough_if = bool_if(vec![then_value(5)], Some(vec![expression_statement(6)]), 4);

    assert_eq!(
        analyze_branch_exits(&[producing_if]),
        BranchExitSummary::PRODUCES
    );
    assert_eq!(
        analyze_branch_exits(&[fallthrough_if]),
        BranchExitSummary {
            can_fall_through: true,
            produces_value: true,
            terminates: false,
        }
    );
}

#[test]
fn branch_exits_combine_match_arms_and_default() {
    let producing_match = literal_match(vec![then_value(3)], Some(vec![then_value(4)]), 1);
    let mixed_match = literal_match(vec![then_value(7)], Some(vec![return_value(8)]), 5);

    assert_eq!(
        analyze_branch_exits(&[producing_match]),
        BranchExitSummary::PRODUCES
    );
    assert_eq!(
        analyze_branch_exits(&[mixed_match]),
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
    let summary = analyze_branch_exits(&[
        expression_statement(1),
        assert_statement(
            Expression::bool(false, test_source_location(2), ValueMode::ImmutableOwned),
            2,
        ),
        then_value(3),
    ]);

    assert_eq!(summary, BranchExitSummary::TERMINATES);
}

#[test]
fn branch_exits_do_not_treat_passing_assert_as_terminal() {
    let summary = analyze_branch_exits(&[assert_statement(
        Expression::bool(true, test_source_location(1), ValueMode::ImmutableOwned),
        1,
    )]);

    assert_eq!(summary, BranchExitSummary::FALLS_THROUGH);
}

#[test]
fn branch_exits_combine_assert_false_branches_as_terminal() {
    let terminating_if = bool_if(
        vec![assert_statement(
            Expression::bool(false, test_source_location(2), ValueMode::ImmutableOwned),
            2,
        )],
        Some(vec![assert_statement(
            Expression::bool(false, test_source_location(3), ValueMode::ImmutableOwned),
            3,
        )]),
        1,
    );

    let partial_if = bool_if(
        vec![assert_statement(
            Expression::bool(false, test_source_location(5), ValueMode::ImmutableOwned),
            5,
        )],
        None,
        4,
    );

    assert_eq!(
        analyze_branch_exits(&[terminating_if]),
        BranchExitSummary::TERMINATES
    );
    assert_eq!(
        analyze_branch_exits(&[partial_if]),
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
    let (ast, string_table) = parse_single_file_ast(
        "choose |ready Bool| -> String:\n    label = if ready:\n        then \"ready\"\n    else\n        then \"waiting\"\n    ;\n    return label\n;\n",
    );
    let body = function_body_by_name(&ast, &string_table, "choose");
    let NodeKind::VariableDeclaration(declaration) = &body[0].kind else {
        panic!("expected inferred value-if declaration");
    };
    let ExpressionKind::ValueBlock { block } = &declaration.value.kind else {
        panic!("expected value block expression");
    };
    let ValueBlock::If(ValueIfBlock {
        result_type_ids, ..
    }) = block.as_ref()
    else {
        panic!("expected value if block");
    };

    assert_eq!(result_type_ids.as_slice(), [builtin_type_ids::STRING]);
    assert_eq!(declaration.value.type_id, builtin_type_ids::STRING);
}

#[test]
fn shared_validator_rejects_all_terminating_value_if() {
    let error = validate_value_if_completeness(
        &[return_value(1)],
        &[return_value(2)],
        &test_source_location(1),
    )
    .expect_err("a value-if whose every path terminates has no value to provide");
    let diagnostic = CompilerDiagnostic::from(error);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidControlFlowStatement {
            reason: InvalidControlFlowStatementReason::ValueIfNoProducingPath,
        }
    ));
}

#[test]
fn inferred_block_value_if_rejects_later_nested_produced_type_conflict() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "choose |ready Bool, use_fallback Bool| -> String:\n    label = if ready:\n        if use_fallback:\n            then \"fallback\"\n        else\n            then 1\n        ;\n    else\n        then \"waiting\"\n    ;\n    return label\n;\n",
    );

    let DiagnosticPayload::TypeMismatch { context, .. } = &diagnostic.payload else {
        panic!(
            "expected type mismatch from a later nested ThenValue, got {:?}",
            diagnostic.payload
        );
    };
    assert_eq!(*context, TypeMismatchContext::Declaration);
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 5);
}

#[test]
fn inferred_multi_bind_rejects_later_nested_produced_type_conflict() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "choose |ready Bool, use_fallback Bool| -> String, Int:\n    first, second Int = if ready:\n        if use_fallback:\n            then \"fallback\", 1\n        else\n            then 2, 1\n        ;\n    else\n        then \"waiting\", 2\n    ;\n    return first, second\n;\n",
    );

    let DiagnosticPayload::TypeMismatch { context, .. } = &diagnostic.payload else {
        panic!(
            "expected type mismatch from a later nested multi-bind ThenValue, got {:?}",
            diagnostic.payload
        );
    };
    assert_eq!(*context, TypeMismatchContext::Assignment);
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 5);
}
