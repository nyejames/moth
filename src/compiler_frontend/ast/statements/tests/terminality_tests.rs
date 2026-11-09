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
use crate::compiler_frontend::ast::statements::never_call::{NeverCall, NeverCallTarget};
use crate::compiler_frontend::ast::statements::terminality::{
    FunctionTerminalityPolicy, NeverProofNode, never_proof_cycle, terminality_policy_for_signature,
    validate_function_body_terminality,
};
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    InvalidReturnShapeCode, InvalidReturnShapeView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::{CallableReturnShape, FunctionReturnContract};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tests::ast_fixture_support::{
    fresh_success_returns, function_body_by_name, function_node_by_name,
    function_signature_by_name, node, test_if_branch_metadata,
};
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, parse_single_file_ast, parse_single_file_ast_diagnostic,
    parse_single_file_ast_result,
};
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
        return_contract: FunctionReturnContract::Returns(fresh_success_returns(vec![
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
        ])),
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
                pattern: MatchPattern::Literal(pattern_value),
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
    let unit_like_signature = FunctionSignature {
        parameters: vec![],
        return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots([], None)),
    };
    assert_eq!(
        terminality_policy_for_signature(&unit_like_signature, false),
        FunctionTerminalityPolicy::RejectUnannotatedDivergence,
        "an omitted return contract is the only contract with no success slot and no error slot"
    );

    let error_only_signature = FunctionSignature {
        parameters: vec![],
        return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
            [],
            Some(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
        )),
    };
    assert_eq!(
        terminality_policy_for_signature(&error_only_signature, false),
        FunctionTerminalityPolicy::AllowImplicitUnit,
        "an explicitly authored error-only contract may fall through"
    );

    let (ast, path_fork, string_table) =
        parse_single_file_ast("stop || -> !:\n    assert(false, \"stop\")\n;\n");
    let never_signature = function_signature_by_name(&ast, &path_fork, &string_table, "stop");
    assert_eq!(
        terminality_policy_for_signature(never_signature, false),
        FunctionTerminalityPolicy::RequireDivergence
    );

    assert_eq!(
        terminality_policy_for_signature(&non_unit_signature(), false),
        FunctionTerminalityPolicy::RequireExplicitReturn
    );

    assert_eq!(
        terminality_policy_for_signature(&non_unit_signature(), true),
        FunctionTerminalityPolicy::EntryStartImplicitReturn
    );
    assert_eq!(
        terminality_policy_for_signature(never_signature, true),
        FunctionTerminalityPolicy::EntryStartImplicitReturn,
        "the compiler-owned entry keeps its builder-owned policy"
    );
}

/// Validate a parsed function under the policy its own signature selects.
fn validate_parsed_function(source: &str, name: &str) -> Option<DiagnosticDraft> {
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let node = function_node_by_name(&ast, &path_fork, &string_table, name);
    let NodeKind::Function(path, signature, body) = &node.kind else {
        panic!("function lookup should only return function nodes");
    };
    let policy = terminality_policy_for_signature(signature, false);
    validate_function_body_terminality(
        body,
        policy,
        None,
        path_fork.component(*path),
        &ast.expressions,
        &ast.const_values,
    )
}

#[test]
fn never_body_accepts_folded_false_assert() {
    let diagnostic =
        validate_parsed_function("stop || -> !:\n    assert(false, \"stop\")\n;\n", "stop");

    assert!(
        diagnostic.is_none(),
        "a declared Never body whose only reachable path diverges is valid"
    );
}

#[test]
fn never_body_accepts_unbroken_true_loop() {
    let diagnostic = validate_parsed_function(
        "stop || -> !:\n    loop true:\n        assert(true, \"keep going\")\n    ;\n;\n",
        "stop",
    );

    assert!(
        diagnostic.is_none(),
        "a folded-true loop with no reachable break never resumes after the loop"
    );
}

#[test]
fn never_body_rejects_dynamic_assert() {
    let failure =
        parse_single_file_ast_diagnostic("stop |flag Bool| -> !:\n    assert(flag, \"stop\")\n;\n");

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.never_body_may_complete_normally"),
    );
}

#[test]
fn never_body_rejects_reachable_break() {
    let failure = parse_single_file_ast_diagnostic(
        "stop || -> !:\n    loop true:\n        break\n    ;\n;\n",
    );

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.never_body_may_complete_normally"),
    );
}

#[test]
fn never_body_rejects_error_propagation_to_caller() {
    let failure = parse_single_file_ast_diagnostic(
        "fail || -> Error!:\n    return! Error(\"boom\")\n;\n\nstop || -> !:\n    fail()!\n;\n",
    );

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.function_has_no_error_slot"),
    );
}

#[test]
fn never_body_rejects_option_propagation_to_caller() {
    let failure = parse_single_file_ast_diagnostic(
        "maybe |flag Bool| -> I32?:\n    if flag:\n        return 1\n    ;\n    return none\n;\n\nstop || -> !:\n    value = maybe(true)?\n    assert(false, \"stop\")\n;\n",
    );

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.function_has_no_optional_return"),
    );
}

#[test]
fn never_body_rejects_a_completing_branch() {
    let failure = parse_single_file_ast_diagnostic(
        "stop |flag Bool| -> !:\n    if flag:\n        assert(false, \"stop\")\n    else\n        assert(true, \"keep going\")\n    ;\n;\n",
    );

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.never_body_may_complete_normally"),
    );
}

#[test]
fn never_body_accepts_nested_inner_break_inside_an_outer_true_loop() {
    let diagnostic = validate_parsed_function(
        "stop || -> !:\n    loop true:\n        loop true:\n            break\n        ;\n        assert(false, \"stop\")\n    ;\n;\n",
        "stop",
    );

    assert!(
        diagnostic.is_none(),
        "a nested loop consumes its own break, so the outer true loop still diverges"
    );
}

#[test]
fn never_body_rejects_hidden_return_in_value_if() {
    let failure = parse_single_file_ast_diagnostic(
        "stop |condition Bool| -> !:\n    value = if condition:\n        then 1\n    else\n        return\n    ;\n    assert(false, \"stop\")\n;\n",
    );

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.never_body_returns_to_caller"),
    );
}

#[test]
fn never_body_rejects_return_bang_at_parse_time() {
    let failure =
        parse_single_file_ast_diagnostic("stop || -> !:\n    return! Error(\"boom\")\n;\n");

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0042",
        Some("invalid_control_flow_statement.return_bang_outside_error_function"),
    );
}

#[test]
fn explicitly_typed_placeholder_may_diverge() {
    let diagnostic = validate_parsed_function(
        "placeholder || -> I32:\n    assert(false, \"not implemented\")\n;\n",
        "placeholder",
    );

    assert!(
        diagnostic.is_none(),
        "an explicitly typed implementation may be a temporary placeholder"
    );
}

#[test]
fn unannotated_diverging_function_is_rejected() {
    let failure =
        parse_single_file_ast_diagnostic("serve ||:\n    assert(false, \"not implemented\")\n;\n");
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.unannotated_function_always_diverges"),
    );
    let facts = diagnostic
        .family_view::<InvalidReturnShapeView>()
        .expect("the diagnostic has invalid return-shape facts");

    assert_eq!(
        facts.reason,
        InvalidReturnShapeCode::UnannotatedFunctionAlwaysDiverges
    );
    assert!(
        facts.function_name.is_some(),
        "the explicitness diagnostic names the function whose contract is omitted"
    );
}

#[test]
fn unannotated_function_that_returns_is_accepted() {
    let diagnostic =
        validate_parsed_function("serve ||:\n    assert(true, \"ready\")\n;\n", "serve");

    assert!(
        diagnostic.is_none(),
        "an unannotated function with a reachable ordinary completion stays valid"
    );
}

#[test]
fn unannotated_normalised_true_loop_is_rejected() {
    let failure = parse_single_file_ast_diagnostic(
        "serve ||:\n    loop true:\n        assert(true, \"keep going\")\n    ;\n;\n",
    );

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.unannotated_function_always_diverges"),
    );
}

#[test]
fn unannotated_diverging_branches_are_rejected() {
    let failure = parse_single_file_ast_diagnostic(
        "serve |flag Bool|:\n    if flag:\n        assert(false, \"left\")\n    else\n        assert(false, \"right\")\n    ;\n;\n",
    );

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.unannotated_function_always_diverges"),
    );
}

#[test]
fn unannotated_return_only_function_is_accepted() {
    let diagnostic = validate_parsed_function("serve ||:\n    return\n;\n", "serve");

    assert!(
        diagnostic.is_none(),
        "a bare return is an ordinary completion, not divergence"
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

fn proof_node(index: usize, dependencies: &[usize]) -> NeverProofNode {
    NeverProofNode {
        key: PathId::try_from_index(index).expect("fixture path fits"),
        // The fixture names each node by its path index so tests can see who was reported.
        function_name: Some(StringId::from_index(index as u32)),
        span: None,
        dependencies: dependencies
            .iter()
            .map(|index| PathId::try_from_index(*index).expect("fixture path fits"))
            .collect(),
    }
}

/// The cycle diagnostic for a graph whose dependencies already name node keys.
fn never_proof_cycle_diagnostic(nodes: &[NeverProofNode]) -> Option<DiagnosticDraft> {
    never_proof_cycle(nodes, |key| key).map(|(_, diagnostic)| diagnostic)
}

fn invalid_return_shape_code(diagnostic: DiagnosticDraft) -> InvalidReturnShapeCode {
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    diagnostic
        .family_view::<InvalidReturnShapeView>()
        .expect("the diagnostic carries return-shape facts")
        .reason
}

/// The function index a cycle diagnostic names, per the `proof_node` naming scheme.
fn reported_proof_index(diagnostic: DiagnosticDraft) -> usize {
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    let view = diagnostic
        .family_view::<InvalidReturnShapeView>()
        .expect("the diagnostic carries return-shape facts");
    assert_eq!(
        view.reason,
        InvalidReturnShapeCode::NeverProofDependsOnCycle
    );
    view.function_name
        .expect("a cycle diagnostic names its function")
        .index() as usize
}

#[test]
fn never_body_accepts_a_standalone_never_call() {
    let expressions = TestExpressions::new();
    let body = vec![never_call_statement()];

    assert!(
        expressions
            .require_policy(&body, FunctionTerminalityPolicy::RequireDivergence)
            .is_none(),
        "a standalone Never call diverges on every reachable path"
    );
}

#[test]
fn never_proof_cycle_rejects_a_self_edge() {
    let diagnostic = never_proof_cycle_diagnostic(&[proof_node(10, &[10])])
        .expect("a self-recursive Never proof must be deferred");

    assert_eq!(
        invalid_return_shape_code(diagnostic),
        InvalidReturnShapeCode::NeverProofDependsOnCycle
    );
}

#[test]
fn never_proof_cycle_resolves_dependencies_before_matching_declarations() {
    // Path 99 stands for a concrete instance of template 10; only resolving it closes the cycle.
    let nodes = [proof_node(10, &[99])];
    let instance = PathId::try_from_index(99).expect("fixture path fits");
    let template = PathId::try_from_index(10).expect("fixture path fits");

    assert!(
        never_proof_cycle(&nodes, |key| key).is_none(),
        "an unresolved instance path is not a declaration in the graph"
    );
    let (index, diagnostic) =
        never_proof_cycle(&nodes, |key| if key == instance { template } else { key })
            .expect("the resolved instance call is a self-edge on its template");
    assert_eq!(index, 0);
    assert_eq!(reported_proof_index(diagnostic), 10);
}

#[test]
fn never_proof_cycle_rejects_a_mutual_group() {
    let nodes = [proof_node(10, &[11]), proof_node(11, &[10])];
    let diagnostic = never_proof_cycle_diagnostic(&nodes)
        .expect("a mutually recursive Never proof must be deferred");

    assert_eq!(
        invalid_return_shape_code(diagnostic),
        InvalidReturnShapeCode::NeverProofDependsOnCycle
    );
}

#[test]
fn never_proof_accepts_an_acyclic_group_in_either_declaration_order() {
    let nodes = [proof_node(10, &[11]), proof_node(11, &[])];

    assert!(
        never_proof_cycle_diagnostic(&nodes).is_none(),
        "a body may rely on a peer that diverges independently"
    );
}

#[test]
fn never_proof_names_the_first_declared_cycle_member_not_a_dependent_caller() {
    // Declaration order: `12` depends on the 10 <-> 11 cycle but is not part of it.
    let nodes = [
        proof_node(12, &[10]),
        proof_node(10, &[11]),
        proof_node(11, &[10]),
    ];
    let diagnostic =
        never_proof_cycle_diagnostic(&nodes).expect("the cycle itself must still be deferred");
    assert_eq!(reported_proof_index(diagnostic), 10);
}

#[test]
fn never_proof_cycle_report_follows_declaration_order() {
    // Two disjoint cycles; the reported one is whichever owns the earliest declaration,
    // independent of graph shape or traversal order.
    let forward = [
        proof_node(20, &[21]),
        proof_node(21, &[20]),
        proof_node(30, &[30]),
    ];
    let reversed = [
        proof_node(30, &[30]),
        proof_node(21, &[20]),
        proof_node(20, &[21]),
    ];
    let dependent_first = [
        proof_node(40, &[21]),
        proof_node(21, &[20]),
        proof_node(30, &[30]),
        proof_node(20, &[21]),
    ];
    // The component's Tarjan root (52) is not its earliest declaration (51).
    let root_is_not_first = [
        proof_node(50, &[52]),
        proof_node(51, &[52]),
        proof_node(52, &[51]),
    ];
    // The self-loop on 61 completes before the earlier-declared 60 <-> 62 cycle.
    let earliest_component_pops_last = [
        proof_node(60, &[61, 62]),
        proof_node(61, &[61]),
        proof_node(62, &[60]),
    ];

    let report = |nodes: &[NeverProofNode]| {
        reported_proof_index(never_proof_cycle_diagnostic(nodes).expect("cycles are rejected"))
    };
    assert_eq!(report(&forward), 20);
    assert_eq!(report(&reversed), 30);
    assert_eq!(report(&dependent_first), 21);
    assert_eq!(report(&root_is_not_first), 51);
    assert_eq!(report(&earliest_component_pops_last), 60);
}

#[test]
fn never_proof_accepts_a_long_acyclic_chain_and_rejects_its_closing_edge() {
    // Deep enough that a recursive walk would exhaust the test thread's stack.
    const LENGTH: usize = 200_000;
    let mut nodes: Vec<NeverProofNode> = (0..LENGTH)
        .map(|index| {
            let next: &[usize] = if index + 1 < LENGTH {
                &[index + 1]
            } else {
                &[]
            };
            proof_node(index, next)
        })
        .collect();
    assert!(
        never_proof_cycle_diagnostic(&nodes).is_none(),
        "a chain ending in an independent divergence is a valid proof"
    );

    nodes[LENGTH - 1] = proof_node(LENGTH - 1, &[LENGTH / 2]);
    let diagnostic =
        never_proof_cycle_diagnostic(&nodes).expect("closing the chain creates a cycle");
    assert_eq!(reported_proof_index(diagnostic), LENGTH / 2);
}

#[test]
fn never_self_recursion_is_rejected_after_specialisation() {
    let failure = parse_single_file_ast_diagnostic("stop || -> !:\n    stop()\n;\n");

    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.never_proof_depends_on_cycle"),
    );
}

#[test]
fn never_proof_rejects_cycle_through_generic_instance() {
    let failure = parse_single_file_ast_diagnostic(
        "a || -> !:\n    g(1)\n;\n\ng type T |x T| -> !:\n    a()\n;\n",
    );
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.never_proof_depends_on_cycle"),
    );
}

#[test]
fn uncalled_generic_never_template_is_proven_at_its_declaration() {
    // No call materialises an instance, so only the declaring module's template proof can fail.
    let failure = parse_single_file_ast_diagnostic(
        "abort type T |value T| -> !:\n    io.line(\"falls through\")\n;\n",
    );
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.never_body_may_complete_normally"),
    );
}

#[test]
fn recursion_after_independent_divergence_is_accepted() {
    let diagnostic =
        parse_single_file_ast_result("stop || -> !:\n    assert(false, \"stop\")\n    stop()\n;\n");

    assert!(
        diagnostic.is_ok(),
        "an unreachable recursive call after an independent divergence is not part of the proof"
    );
}

#[test]
fn recursion_inside_a_true_loop_is_accepted() {
    let diagnostic =
        parse_single_file_ast_result("stop || -> !:\n    loop true:\n        stop()\n    ;\n;\n");

    assert!(
        diagnostic.is_ok(),
        "a folded-true loop diverges without relying on the recursive contract"
    );
}

#[test]
fn unreachable_tail_after_a_never_call_is_ignored() {
    let result = parse_single_file_ast_result(
        "fatal || -> !:\n    assert(false, \"fatal\")\n;\n\n\
         stop || -> !:\n    fatal()\n    return\n;\n",
    );

    assert!(
        result.is_ok(),
        "a statement after the standalone Never call cannot fall through and must not join the proof"
    );
}
