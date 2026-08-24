//! Function-body terminality helper regression tests.
//!
//! WHAT: checks the control-flow analysis used to reject non-unit functions that can fall through.
//! WHY: these rules are the AST-side replacement for the old HIR fall-through diagnostic; keeping
//! them in a focused unit test makes the contract explicit without duplicating integration coverage.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::terminality::{
    FunctionTerminalityPolicy, validate_function_body_terminality,
};
use crate::compiler_frontend::compiler_messages::InvalidReturnShapeReason;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::tests::ast_fixture_support::{
    fresh_success_returns, node, test_if_branch_metadata, test_source_location,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn int_return(line: i32) -> AstNode {
    node(
        NodeKind::Return(vec![Expression::int(
            line,
            test_source_location(line),
            ValueMode::ImmutableOwned,
        )]),
        test_source_location(line),
    )
}

fn assert_bool(condition: bool, line: i32) -> AstNode {
    node(
        NodeKind::Assert {
            condition: Expression::bool(
                condition,
                test_source_location(line),
                ValueMode::ImmutableOwned,
            ),
            // Terminality only inspects the condition; this fixture keeps a typed expression
            // placeholder because parsed assertions always carry the canonical optional value.
            message: Expression::bool(true, test_source_location(line), ValueMode::ImmutableOwned),
        },
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

fn non_unit_signature() -> FunctionSignature {
    FunctionSignature {
        parameters: vec![],
        returns: fresh_success_returns(vec![builtin_type_ids::INT]),
    }
}

#[test]
fn allow_implicit_unit_ignores_terminality() {
    let empty_body: Vec<AstNode> = vec![];
    let diagnostic = validate_function_body_terminality(
        &empty_body,
        FunctionTerminalityPolicy::AllowImplicitUnit,
        test_source_location(1),
    );

    assert!(
        diagnostic.is_none(),
        "empty-body unit function may fall through"
    );
}

#[test]
fn entry_start_implicit_return_ignores_terminality() {
    let empty_body: Vec<AstNode> = vec![];
    let diagnostic = validate_function_body_terminality(
        &empty_body,
        FunctionTerminalityPolicy::EntryStartImplicitReturn,
        test_source_location(1),
    );

    assert!(
        diagnostic.is_none(),
        "entry start must not require an explicit return"
    );
}

#[test]
fn require_explicit_return_rejects_empty_body() {
    let empty_body: Vec<AstNode> = vec![];
    let diagnostic = validate_function_body_terminality(
        &empty_body,
        FunctionTerminalityPolicy::RequireExplicitReturn,
        test_source_location(1),
    );

    assert!(
        diagnostic.is_some(),
        "empty non-unit body should be rejected"
    );
    let diagnostic = diagnostic.unwrap();
    assert!(matches!(
        diagnostic.payload,
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidReturnShape {
            reason: InvalidReturnShapeReason::FunctionMayFallThrough,
        }
    ));
}

#[test]
fn direct_return_terminates() {
    let body = vec![int_return(1)];
    let diagnostic = validate_function_body_terminality(
        &body,
        FunctionTerminalityPolicy::RequireExplicitReturn,
        test_source_location(1),
    );

    assert!(diagnostic.is_none());
}

#[test]
fn assert_false_terminates_non_unit_function() {
    let body = vec![assert_bool(false, 1)];
    let diagnostic = validate_function_body_terminality(
        &body,
        FunctionTerminalityPolicy::RequireExplicitReturn,
        test_source_location(1),
    );

    assert!(
        diagnostic.is_none(),
        "assert(false) should be statically terminal"
    );
}

#[test]
fn assert_dynamic_does_not_terminate() {
    let body = vec![assert_bool(true, 1)];
    let diagnostic = validate_function_body_terminality(
        &body,
        FunctionTerminalityPolicy::RequireExplicitReturn,
        test_source_location(1),
    );

    assert!(
        diagnostic.is_some(),
        "assert(true) must not be treated as terminal"
    );
}

#[test]
fn terminal_statement_after_fallthrough_statement_is_terminal() {
    let body = vec![expression_statement(1), int_return(2)];
    let diagnostic = validate_function_body_terminality(
        &body,
        FunctionTerminalityPolicy::RequireExplicitReturn,
        test_source_location(1),
    );

    assert!(
        diagnostic.is_none(),
        "a later terminal statement should satisfy the check"
    );
}

#[test]
fn if_requires_both_branches_to_terminate() {
    let terminal_both = node(
        NodeKind::If(
            Expression::bool(true, test_source_location(1), ValueMode::ImmutableOwned),
            vec![int_return(2)],
            Some(vec![int_return(3)]),
            test_if_branch_metadata(true),
        ),
        test_source_location(1),
    );

    let terminal_then_only = node(
        NodeKind::If(
            Expression::bool(true, test_source_location(4), ValueMode::ImmutableOwned),
            vec![int_return(5)],
            Some(vec![expression_statement(6)]),
            test_if_branch_metadata(true),
        ),
        test_source_location(4),
    );

    let no_else = node(
        NodeKind::If(
            Expression::bool(true, test_source_location(7), ValueMode::ImmutableOwned),
            vec![int_return(8)],
            None,
            test_if_branch_metadata(false),
        ),
        test_source_location(7),
    );

    assert!(
        validate_function_body_terminality(
            &[terminal_both],
            FunctionTerminalityPolicy::RequireExplicitReturn,
            test_source_location(1),
        )
        .is_none()
    );

    assert!(
        validate_function_body_terminality(
            &[terminal_then_only],
            FunctionTerminalityPolicy::RequireExplicitReturn,
            test_source_location(1),
        )
        .is_some()
    );

    assert!(
        validate_function_body_terminality(
            &[no_else],
            FunctionTerminalityPolicy::RequireExplicitReturn,
            test_source_location(1),
        )
        .is_some()
    );
}

#[test]
fn match_requires_all_arms_and_default_to_terminate() {
    let terminal_match = node(
        NodeKind::Match {
            scrutinee: Expression::int(1, test_source_location(1), ValueMode::ImmutableOwned),
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(Expression::int(
                    1,
                    test_source_location(2),
                    ValueMode::ImmutableOwned,
                )),
                guard: None,
                body: vec![int_return(3)],
            }],
            default: Some(vec![int_return(4)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_source_location(1),
    );

    let non_terminal_default = node(
        NodeKind::Match {
            scrutinee: Expression::int(1, test_source_location(5), ValueMode::ImmutableOwned),
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(Expression::int(
                    1,
                    test_source_location(6),
                    ValueMode::ImmutableOwned,
                )),
                guard: None,
                body: vec![int_return(7)],
            }],
            default: Some(vec![expression_statement(8)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_source_location(5),
    );

    assert!(
        validate_function_body_terminality(
            &[terminal_match],
            FunctionTerminalityPolicy::RequireExplicitReturn,
            test_source_location(1),
        )
        .is_none()
    );

    assert!(
        validate_function_body_terminality(
            &[non_terminal_default],
            FunctionTerminalityPolicy::RequireExplicitReturn,
            test_source_location(1),
        )
        .is_some()
    );
}

#[test]
fn exhaustive_choice_match_does_not_require_default() {
    let terminal_match = node(
        NodeKind::Match {
            scrutinee: Expression::int(1, test_source_location(1), ValueMode::ImmutableOwned),
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(Expression::int(
                    1,
                    test_source_location(2),
                    ValueMode::ImmutableOwned,
                )),
                guard: None,
                body: vec![int_return(3)],
            }],
            default: None,
            exhaustiveness: MatchExhaustiveness::ExhaustiveChoice,
        },
        test_source_location(1),
    );

    assert!(
        validate_function_body_terminality(
            &[terminal_match],
            FunctionTerminalityPolicy::RequireExplicitReturn,
            test_source_location(1),
        )
        .is_none()
    );
}

#[test]
fn match_marked_has_default_without_default_body_is_not_terminal() {
    let malformed_match = node(
        NodeKind::Match {
            scrutinee: Expression::int(1, test_source_location(1), ValueMode::ImmutableOwned),
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(Expression::int(
                    1,
                    test_source_location(2),
                    ValueMode::ImmutableOwned,
                )),
                guard: None,
                body: vec![int_return(3)],
            }],
            default: None,
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_source_location(1),
    );

    assert!(
        validate_function_body_terminality(
            &[malformed_match],
            FunctionTerminalityPolicy::RequireExplicitReturn,
            test_source_location(1),
        )
        .is_some()
    );
}

#[test]
fn scoped_block_terminates_when_body_terminates() {
    let terminal_block = node(
        NodeKind::ScopedBlock {
            body: vec![int_return(2)],
        },
        test_source_location(1),
    );
    let fallthrough_block = node(
        NodeKind::ScopedBlock {
            body: vec![expression_statement(2)],
        },
        test_source_location(1),
    );

    assert!(
        validate_function_body_terminality(
            &[terminal_block],
            FunctionTerminalityPolicy::RequireExplicitReturn,
            test_source_location(1),
        )
        .is_none()
    );

    assert!(
        validate_function_body_terminality(
            &[fallthrough_block],
            FunctionTerminalityPolicy::RequireExplicitReturn,
            test_source_location(1),
        )
        .is_some()
    );
}

#[test]
fn policy_selection_matches_signature_shape() {
    use crate::compiler_frontend::ast::statements::terminality::terminality_policy_for_signature;

    let unit_like_signature = FunctionSignature {
        parameters: vec![],
        returns: vec![],
    };
    assert_eq!(
        terminality_policy_for_signature(&unit_like_signature, false),
        FunctionTerminalityPolicy::AllowImplicitUnit
    );

    assert_eq!(
        terminality_policy_for_signature(&non_unit_signature(), false),
        FunctionTerminalityPolicy::RequireExplicitReturn
    );

    assert_eq!(
        terminality_policy_for_signature(&non_unit_signature(), true),
        FunctionTerminalityPolicy::EntryStartImplicitReturn
    );
}
