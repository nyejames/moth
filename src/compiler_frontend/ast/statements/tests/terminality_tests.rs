//! Function-body terminality consumer tests.
//!
//! WHAT: checks the policy and fallthrough diagnostic that consume the shared control-flow exit
//! analysis. Pure exit facts are covered by `control_flow_exits_tests.rs`.
//! WHY: these rules produce the fall-through diagnostic on the AST side; keeping them in a
//! focused unit test makes the consumer contract explicit without duplicating the pure facts.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, PatternValue,
};
use crate::compiler_frontend::ast::statements::terminality::{
    FunctionTerminalityPolicy, validate_function_body_terminality,
};
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    InvalidReturnShapeCode, InvalidReturnShapeView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::tests::ast_fixture_support::{
    fresh_success_returns, function_body_by_name, node, test_if_branch_metadata,
};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// Fixture-owned expression/content pair: terminality views bind IDs here directly.
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

    fn bool(&mut self, value: bool) -> ExprId {
        self.builder
            .bool(&mut self.constants, value, None, ValueMode::ImmutableOwned)
            .expect("terminality bool fixture")
            .id()
    }

    fn int(&mut self, value: i64) -> ExprId {
        self.builder
            .fixed_scalar(
                &mut self.constants,
                FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32"),
                None,
                ValueMode::ImmutableOwned,
            )
            .expect("terminality I32 fixture")
            .id()
    }

    fn require(&self, body: &[AstNode]) -> Option<DiagnosticDraft> {
        self.require_policy(body, FunctionTerminalityPolicy::RequireExplicitReturn)
    }

    fn require_policy(
        &self,
        body: &[AstNode],
        policy: FunctionTerminalityPolicy,
    ) -> Option<DiagnosticDraft> {
        validate_function_body_terminality(
            body,
            policy,
            None,
            self.builder.store(),
            &self.constants,
        )
    }
}

fn int_return(expressions: &mut TestExpressions, line: i64) -> AstNode {
    node(NodeKind::Return(vec![expressions.int(line)]), None)
}

fn assert_bool(expressions: &mut TestExpressions, condition: bool) -> AstNode {
    let condition = expressions.bool(condition);
    let message = expressions.bool(true);
    node(NodeKind::Assert { condition, message }, None)
}

fn expression_statement(expressions: &mut TestExpressions, line: i64) -> AstNode {
    node(NodeKind::ExpressionStatement(expressions.int(line)), None)
}

fn non_unit_signature() -> FunctionSignature {
    FunctionSignature {
        parameters: vec![],
        returns: fresh_success_returns(vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)]),
    }
}

#[test]
fn allow_implicit_unit_ignores_terminality() {
    let expressions = TestExpressions::new();
    let empty_body: Vec<AstNode> = vec![];
    let diagnostic =
        expressions.require_policy(&empty_body, FunctionTerminalityPolicy::AllowImplicitUnit);

    assert!(
        diagnostic.is_none(),
        "empty-body unit function may fall through"
    );
}

#[test]
fn entry_start_implicit_return_ignores_terminality() {
    let expressions = TestExpressions::new();
    let empty_body: Vec<AstNode> = vec![];
    let diagnostic = expressions.require_policy(
        &empty_body,
        FunctionTerminalityPolicy::EntryStartImplicitReturn,
    );

    assert!(
        diagnostic.is_none(),
        "entry start must not require an explicit return"
    );
}

#[test]
fn require_explicit_return_rejects_empty_body() {
    let expressions = TestExpressions::new();
    let empty_body: Vec<AstNode> = vec![];
    let diagnostic = expressions.require(&empty_body);

    assert!(
        diagnostic.is_some(),
        "empty non-unit body should be rejected"
    );
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic.unwrap()), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    let facts = diagnostic
        .family_view::<InvalidReturnShapeView>()
        .expect("the empty body has invalid return-shape facts");
    assert_eq!(facts.reason, InvalidReturnShapeCode::FunctionMayFallThrough);
}

#[test]
fn direct_return_terminates() {
    let mut expressions = TestExpressions::new();
    let body = vec![int_return(&mut expressions, 1)];

    assert!(expressions.require(&body).is_none());
}

#[test]
fn assert_false_terminates_non_unit_function() {
    let mut expressions = TestExpressions::new();
    let body = vec![assert_bool(&mut expressions, false)];

    assert!(
        expressions.require(&body).is_none(),
        "assert(false) should be statically terminal"
    );
}

#[test]
fn assert_dynamic_does_not_terminate() {
    let mut expressions = TestExpressions::new();
    let body = vec![assert_bool(&mut expressions, true)];

    assert!(
        expressions.require(&body).is_some(),
        "assert(true) must not be treated as terminal"
    );
}

#[test]
fn terminal_statement_after_fallthrough_statement_is_terminal() {
    let mut expressions = TestExpressions::new();
    let body = vec![
        expression_statement(&mut expressions, 1),
        int_return(&mut expressions, 2),
    ];

    assert!(
        expressions.require(&body).is_none(),
        "a later terminal statement should satisfy the check"
    );
}

#[test]
fn true_loop_without_break_satisfies_a_typed_function() {
    let mut expressions = TestExpressions::new();
    let condition = expressions.bool(true);
    let body = vec![node(
        NodeKind::WhileLoop(condition, vec![expression_statement(&mut expressions, 1)]),
        None,
    )];

    assert!(
        expressions.require(&body).is_none(),
        "a folded-true loop with no reachable break cannot fall through"
    );
}

#[test]
fn true_loop_with_reachable_break_still_falls_through() {
    let mut expressions = TestExpressions::new();
    let condition = expressions.bool(true);
    let loop_body = node(NodeKind::Break, None);
    let body = vec![node(NodeKind::WhileLoop(condition, vec![loop_body]), None)];

    assert!(
        expressions.require(&body).is_some(),
        "a reachable self-targeting break exposes the post-loop path"
    );
}

#[test]
fn if_requires_both_branches_to_terminate() {
    let mut expressions = TestExpressions::new();
    let terminal_both = node(
        NodeKind::If(
            expressions.bool(true),
            vec![int_return(&mut expressions, 2)],
            Some(vec![int_return(&mut expressions, 3)]),
            test_if_branch_metadata(true),
        ),
        None,
    );
    let terminal_then_only = node(
        NodeKind::If(
            expressions.bool(true),
            vec![int_return(&mut expressions, 5)],
            Some(vec![expression_statement(&mut expressions, 6)]),
            test_if_branch_metadata(true),
        ),
        None,
    );
    let no_else = node(
        NodeKind::If(
            expressions.bool(true),
            vec![int_return(&mut expressions, 8)],
            None,
            test_if_branch_metadata(false),
        ),
        None,
    );

    assert!(expressions.require(&[terminal_both]).is_none());
    assert!(expressions.require(&[terminal_then_only]).is_some());
    assert!(expressions.require(&[no_else]).is_some());
}

#[test]
fn match_requires_all_arms_and_default_to_terminate() {
    let mut expressions = TestExpressions::new();
    let scrutinee = expressions.int(1);
    let pattern_value = PatternValue::from_expression(
        expressions
            .builder
            .view(scrutinee)
            .expect("terminality scrutinee"),
    )
    .expect("terminality pattern value");
    let terminal_match = node(
        NodeKind::Match {
            scrutinee,
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(pattern_value.clone()),
                guard: None,
                body: vec![int_return(&mut expressions, 3)],
            }],
            default: Some(vec![int_return(&mut expressions, 4)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );
    let non_terminal_default = node(
        NodeKind::Match {
            scrutinee,
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(pattern_value),
                guard: None,
                body: vec![int_return(&mut expressions, 7)],
            }],
            default: Some(vec![expression_statement(&mut expressions, 8)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );

    assert!(expressions.require(&[terminal_match]).is_none());
    assert!(expressions.require(&[non_terminal_default]).is_some());
}

#[test]
fn exhaustive_choice_match_does_not_require_default() {
    let mut expressions = TestExpressions::new();
    let scrutinee = expressions.int(1);
    let pattern_value = PatternValue::from_expression(
        expressions
            .builder
            .view(scrutinee)
            .expect("terminality scrutinee"),
    )
    .expect("terminality pattern value");
    let terminal_match = node(
        NodeKind::Match {
            scrutinee,
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(pattern_value),
                guard: None,
                body: vec![int_return(&mut expressions, 3)],
            }],
            default: None,
            exhaustiveness: MatchExhaustiveness::ExhaustiveChoice,
        },
        None,
    );

    assert!(expressions.require(&[terminal_match]).is_none());
}

#[test]
fn lexical_scope_terminates_when_body_terminates() {
    let mut expressions = TestExpressions::new();
    let terminal_block = node(
        NodeKind::LexicalScope {
            body: vec![int_return(&mut expressions, 2)],
        },
        None,
    );
    let fallthrough_block = node(
        NodeKind::LexicalScope {
            body: vec![expression_statement(&mut expressions, 2)],
        },
        None,
    );

    assert!(expressions.require(&[terminal_block]).is_none());
    assert!(expressions.require(&[fallthrough_block]).is_some());
}

#[test]
fn policy_selection_matches_signature_shape() {
    use crate::compiler_frontend::ast::statements::terminality::terminality_policy_for_signature;

    let unit_like_signature = FunctionSignature {
        parameters: vec![],
        returns: CallableReturnShape::from_slots([], None),
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

#[test]
fn loop_true_with_hidden_catch_break_may_fall_through() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "fail || -> Error!:\n    return! Error(\"boom\")\n;\n\nrecover || -> I32:\n    loop true:\n        fail() catch:\n            break\n        ;\n    ;\n    return 1\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let diagnostic = validate_function_body_terminality(
        std::slice::from_ref(&body[0]),
        FunctionTerminalityPolicy::RequireExplicitReturn,
        None,
        &ast.expressions,
        &ast.const_values,
    )
    .expect("the reachable catch break leaves the loop, so the function may fall through");

    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    let facts = diagnostic
        .family_view::<InvalidReturnShapeView>()
        .expect("the diagnostic has invalid return-shape facts");
    assert_eq!(facts.reason, InvalidReturnShapeCode::FunctionMayFallThrough);
}
