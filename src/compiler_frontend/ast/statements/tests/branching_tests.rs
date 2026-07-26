//! Branching and match parsing regression tests.
//!
//! WHAT: validates `if`/`else` and `match`-style AST construction.
//! WHY: control-flow lowering relies on branch bodies and match arms staying structurally correct.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::MatchExhaustiveness;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, Operator,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::statements::match_exhaustiveness::MatchArmCoverageTracker;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, RelationalPatternOp,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, InvalidControlFlowStatementReason, InvalidMatchArmReason,
    InvalidMatchPatternReason, NonExhaustiveMatchReason, RuleDiagnosticKind, SyntaxDiagnosticKind,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::tests::ast_fixture_support::{start_function_body, test_location};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn parses_if_else_statements() {
    let (ast, string_table) = parse_single_file_ast(
        "flag = true\nif flag:\n    io.line([: [\"yes\"]])\nelse\n    io.line([: [\"no\"]])\n;\n",
    );

    let body = start_function_body(&ast, &string_table);

    let NodeKind::If(condition, then_block, else_block) = &body[1].kind else {
        panic!("expected if statement in start body");
    };

    assert_eq!(condition.diagnostic_type, DataType::Bool);
    assert_eq!(then_block.len(), 1);
    assert_eq!(
        else_block.as_ref().map(Vec::len),
        Some(1),
        "else block should contain one host call"
    );
}

#[test]
fn rejects_same_line_else_if_statement() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "if true:\n    io.line([: [\"selected\"]])\nelse if false:\n    io.line([: [\"unsupported\"]])\n;\n",
    );

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidControlFlowStatement {
                reason: InvalidControlFlowStatementReason::ElseIfUnsupported,
            }
        ),
        "{:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_missing_if_conditions_at_the_first_boundary() {
    let cases = [
        ("if:\n    io.line([: [\"ready\"]])\n;\n", 3),
        ("if\nio.line([: [\"ready\"]])\n;\n", 2),
        ("if;\n", 3),
        ("if then 1\n", 4),
        ("if else\n", 4),
        ("if", 2),
        ("value = if then 1 else 0\n", 12),
        ("value = if:\n    then 1\nelse\n    then 0\n;\n", 11),
        ("a, b = if then 1, 2 else 3, 4\n", 11),
    ];

    for (source, expected_column) in cases {
        let diagnostic = parse_single_file_ast_diagnostic(source);

        assert!(
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidControlFlowStatement {
                    reason: InvalidControlFlowStatementReason::ExpectedConditionAfterIf,
                }
            ),
            "expected ExpectedConditionAfterIf for {source:?}, got {:?}",
            diagnostic.payload
        );
        assert_eq!(diagnostic.primary_location.start_pos.line_number, 0);
        assert_eq!(
            diagnostic.primary_location.start_pos.char_column, expected_column,
            "unexpected boundary location for {source:?}"
        );
    }
}

fn runtime_operator_sequence(expression: &Expression) -> Vec<Operator> {
    fn collect_operators_from_rpn(rpn: &ExpressionRpn, out: &mut Vec<Operator>) {
        for item in &rpn.items {
            match item {
                ExpressionRpnItem::Operator { operator, .. } => out.push(operator.to_owned()),
                ExpressionRpnItem::Operand(Expression {
                    kind: ExpressionKind::Runtime(inner_rpn),
                    ..
                }) => collect_operators_from_rpn(inner_rpn, out),
                _ => {}
            }
        }
    }

    match &expression.kind {
        ExpressionKind::Runtime(rpn) => {
            let mut operators = Vec::new();
            collect_operators_from_rpn(rpn, &mut operators);
            operators
        }
        _ => vec![],
    }
}

#[test]
fn parses_nested_if_else_statements() {
    let (ast, string_table) = parse_single_file_ast(
        "outer = true\ninner = false\nif outer:\n    if inner:\n        io.line([: [\"inner\"]])\n    else\n        io.line([: [\"not inner\"]])\n    ;\nelse\n    io.line([: [\"outer false\"]])\n;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::If(_, then_block, else_block) = &body[2].kind else {
        panic!("expected top-level if statement in start body");
    };
    let NodeKind::If(_, nested_then, nested_else) = &then_block[0].kind else {
        panic!("expected nested if statement in top-level then block");
    };

    assert_eq!(nested_then.len(), 1);
    assert_eq!(nested_else.as_ref().map(Vec::len), Some(1));
    assert_eq!(else_block.as_ref().map(Vec::len), Some(1));
}

// --------------------------
//  If-condition type checks
// --------------------------

#[test]
fn rejects_non_boolean_if_condition_with_type_error_metadata() {
    let diagnostic = parse_single_file_ast_diagnostic("if 1:\n    io.line([: [\"bad\"]])\n;\n");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::TypeMismatch {
                context: TypeMismatchContext::Condition,
                ..
            }
        ),
        "{:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_string_if_condition_with_type_error_metadata() {
    let diagnostic =
        parse_single_file_ast_diagnostic("if \"text\":\n    io.line([: [\"bad\"]])\n;\n");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::TypeMismatch {
                context: TypeMismatchContext::Condition,
                ..
            }
        ),
        "{:?}",
        diagnostic.payload
    );
}

// --------------------------
//  Operator precedence in conditions
// --------------------------

#[test]
fn precedence_not_binds_tighter_than_and_in_if_conditions() {
    let (ast, string_table) = parse_single_file_ast(
        "a = true\nb = false\nif not a and b:\n    io.line([: [\"x\"]])\n;\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::If(condition, _, _) = &body[2].kind else {
        panic!("expected if statement in start body");
    };

    assert_eq!(
        runtime_operator_sequence(condition),
        vec![Operator::Not, Operator::And]
    );
}

#[test]
fn precedence_and_binds_tighter_than_or_in_if_conditions() {
    let (ast, string_table) = parse_single_file_ast(
        "a = true\nb = false\nc = false\nif a or b and c:\n    io.line([: [\"x\"]])\n;\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::If(condition, _, _) = &body[3].kind else {
        panic!("expected if statement in start body");
    };

    assert_eq!(
        runtime_operator_sequence(condition),
        vec![Operator::And, Operator::Or]
    );
}

#[test]
fn parenthesized_grouping_overrides_default_logical_precedence() {
    let (ast, string_table) = parse_single_file_ast(
        "a = true\nb = false\nc = false\nif (a or b) and c:\n    io.line([: [\"x\"]])\n;\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::If(condition, _, _) = &body[3].kind else {
        panic!("expected if statement in start body");
    };

    assert_eq!(
        runtime_operator_sequence(condition),
        vec![Operator::Or, Operator::And]
    );
}

#[test]
fn comparisons_bind_tighter_than_and_in_if_conditions() {
    let (ast, string_table) = parse_single_file_ast(
        "a = 1\nb = 2\nc = 3\nd = 4\nif a < b and c < d:\n    io.line([: [\"x\"]])\n;\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::If(condition, _, _) = &body[4].kind else {
        panic!("expected if statement in start body");
    };

    assert_eq!(
        runtime_operator_sequence(condition),
        vec![Operator::LessThan, Operator::LessThan, Operator::And]
    );
}

#[test]
fn parenthesized_comparison_can_be_negated_in_if_conditions() {
    let (ast, string_table) =
        parse_single_file_ast("a = 1\nb = 2\nif not (a < b):\n    io.line([: [\"x\"]])\n;\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::If(condition, _, _) = &body[2].kind else {
        panic!("expected if statement in start body");
    };

    assert_eq!(
        runtime_operator_sequence(condition),
        vec![Operator::LessThan, Operator::Not]
    );
}

#[test]
fn equality_and_or_precedence_stays_deterministic_in_if_conditions() {
    let (ast, string_table) = parse_single_file_ast(
        "a = 1\nb = 1\nc = 2\nd = 2\nif a is b or c is d:\n    io.line([: [\"x\"]])\n;\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::If(condition, _, _) = &body[4].kind else {
        panic!("expected if statement in start body");
    };

    assert_eq!(
        runtime_operator_sequence(condition),
        vec![Operator::Equality, Operator::Equality, Operator::Or]
    );
}

// --------------------------
//  Match statements
// --------------------------

#[test]
fn parses_match_statements_with_else_arm() {
    let (ast, string_table) = parse_single_file_ast(
        "value = 42\nif value is:\n    0 => io.line([: [\"zero\"]])\n    42 => io.line([: [\"forty-two\"]])\n    else => io.line([: [\"other\"]])\n;\n",
    );

    let body = start_function_body(&ast, &string_table);

    let NodeKind::Match {
        scrutinee,
        arms,
        default: else_block,
        exhaustiveness,
    } = &body[1].kind
    else {
        panic!("expected match statement in start body");
    };

    assert_eq!(scrutinee.diagnostic_type, DataType::Int);
    assert_eq!(arms.len(), 2);
    assert!(matches!(
        arms[0].pattern,
        MatchPattern::Literal(Expression {
            kind: ExpressionKind::Int(0),
            ..
        })
    ));
    assert!(matches!(
        arms[1].pattern,
        MatchPattern::Literal(Expression {
            kind: ExpressionKind::Int(42),
            ..
        })
    ));
    assert_eq!(
        else_block.as_ref().map(Vec::len),
        Some(1),
        "match should keep the default arm body"
    );
    assert_eq!(*exhaustiveness, MatchExhaustiveness::HasDefault);
}

#[test]
fn parses_match_arm_with_boolean_guard() {
    let (ast, string_table) = parse_single_file_ast(
        "value = 42\nif value is:\n    42 if true => io.line([: [\"forty-two\"]])\n    else => io.line([: [\"other\"]])\n;\n",
    );

    let body = start_function_body(&ast, &string_table);

    let NodeKind::Match {
        arms,
        default: else_block,
        exhaustiveness,
        ..
    } = &body[1].kind
    else {
        panic!("expected match statement in start body");
    };

    assert_eq!(arms.len(), 1);
    assert!(
        matches!(
            arms[0].guard.as_ref().map(|guard| &guard.kind),
            Some(&ExpressionKind::Bool(true))
        ),
        "guard should parse as a Bool expression attached to the arm"
    );
    assert!(
        else_block.is_some(),
        "guarded literal match should still preserve explicit else body"
    );
    assert_eq!(*exhaustiveness, MatchExhaustiveness::HasDefault);
}

#[test]
fn parses_negative_match_arm_with_multiline_boolean_guard() {
    let (ast, string_table) = parse_single_file_ast(
        "value = 42\nif value is:\n    42 => io.line([: [\"forty-two\"]])\n    -42 if\n        true => io.line([: [\"negative\"]])\n    else => io.line([: [\"other\"]])\n;\n",
    );

    let body = start_function_body(&ast, &string_table);

    let NodeKind::Match { arms, .. } = &body[1].kind else {
        panic!("expected match statement in start body");
    };

    assert_eq!(arms.len(), 2);
    assert!(matches!(
        arms[1].pattern,
        MatchPattern::Literal(Expression {
            kind: ExpressionKind::Int(-42),
            ..
        })
    ));
    assert!(matches!(
        arms[1].guard.as_ref().map(|guard| &guard.kind),
        Some(&ExpressionKind::Bool(true))
    ));
}

#[test]
fn guarded_literal_coverage_only_tracks_unguarded_duplicates() {
    let literal = MatchPattern::Literal(Expression::int(
        2,
        test_location(1),
        ValueMode::ImmutableOwned,
    ));
    let guard = Expression::bool(true, test_location(2), ValueMode::ImmutableOwned);

    let mut guarded_then_unguarded = MatchArmCoverageTracker::default();
    assert!(
        !guarded_then_unguarded
            .record_arm(&literal, Some(&guard), None)
            .unreachable
    );
    assert!(
        !guarded_then_unguarded
            .record_arm(&literal, None, None)
            .unreachable
    );

    let mut unguarded_then_duplicate = MatchArmCoverageTracker::default();
    assert!(
        !unguarded_then_duplicate
            .record_arm(&literal, None, None)
            .unreachable
    );
    assert!(
        unguarded_then_duplicate
            .record_arm(&literal, Some(&guard), None)
            .unreachable
    );
}

#[test]
fn rejects_non_boolean_match_guard_with_type_error_metadata() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "value = 1\nif value is:\n    1 if 7 => io.line([: [\"one\"]])\n    else => io.line([: [\"other\"]])\n;\n",
    );

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::TypeMismatch {
                context: TypeMismatchContext::Condition,
                ..
            }
        ),
        "{:?}",
        diagnostic.payload
    );
}

#[test]
fn parses_choice_match_arms_with_bare_and_qualified_variants() {
    let (ast, string_table) = parse_single_file_ast(
        "Status :: Ready, Busy;\n\
         current Status = Status::Ready\n\
         if current is:\n\
             Ready => io.line([: [\"ready\"]])\n\
             Status::Busy => io.line([: [\"busy\"]])\n\
             else => io.line([: [\"other\"]])\n\
         ;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::Match {
        scrutinee,
        arms,
        default: else_block,
        exhaustiveness,
    } = &body[1].kind
    else {
        panic!("expected match statement in start body");
    };

    assert!(
        matches!(scrutinee.diagnostic_type, DataType::Choices { .. }),
        "choice match scrutinee should preserve choice type identity"
    );
    assert_eq!(arms.len(), 2);
    assert!(
        matches!(arms[0].pattern, MatchPattern::ChoiceVariant { tag: 0, .. }),
        "expected first arm to match Ready (tag 0)"
    );
    assert!(
        matches!(arms[1].pattern, MatchPattern::ChoiceVariant { tag: 1, .. }),
        "expected second arm to match Busy (tag 1)"
    );
    assert!(
        else_block.is_some(),
        "choice match should keep explicit else default"
    );
    assert_eq!(*exhaustiveness, MatchExhaustiveness::HasDefault);
}

#[test]
fn parses_exhaustive_choice_match_without_else_marks_exhaustive_choice() {
    let (ast, string_table) = parse_single_file_ast(
        "Status :: Ready, Busy;\n\
         current Status = Status::Ready\n\
         if current is:\n\
             Ready => io.line([: [\"ready\"]])\n\
             Busy => io.line([: [\"busy\"]])\n\
         ;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::Match {
        scrutinee: _,
        arms,
        default,
        exhaustiveness,
    } = &body[1].kind
    else {
        panic!("expected match statement in start body");
    };

    assert_eq!(arms.len(), 2);
    assert!(default.is_none());
    assert_eq!(*exhaustiveness, MatchExhaustiveness::ExhaustiveChoice);
}

#[test]
fn rejects_legacy_colon_match_arm_syntax() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "value = 1\nif value is:\n    1: io.line([: [\"one\"]])\n    else => io.line([: [\"other\"]])\n;\n",
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMatchArm)
    );
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidMatchArm {
            reason: InvalidMatchArmReason::LegacyColonSyntax
        }
    );
}

#[test]
fn rejects_choice_match_arm_qualifier_for_other_choice() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "Status :: Ready, Busy;\n\
         OtherStatus :: Busy;\n\
         current Status = Status::Ready\n\
         if current is:\n\
             OtherStatus::Busy => io.line([: [\"busy\"]])\n\
         ;\n",
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMatchPattern)
    );
    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidMatchPattern {
                reason: InvalidMatchPatternReason::QualifierDoesNotMatchScrutinee,
                variant_name: None,
                scrutinee_name: Some(_),
            }
        ),
        "{:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_non_exhaustive_choice_match_without_else() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "Status :: Ready, Busy;\n\
         current Status = Status::Ready\n\
         if current is:\n\
             Ready => io.line([: [\"ready\"]])\n\
         ;\n",
    );

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::NonExhaustiveMatch {
                reason: NonExhaustiveMatchReason::MissingVariants,
                ref missing_variants,
                ..
            } if missing_variants.len() == 1
        ),
        "{:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_guarded_choice_match_without_else() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "Status :: Ready, Busy;\n\
         current Status = Status::Ready\n\
         if current is:\n\
             Ready if true => io.line([: [\"ready\"]])\n\
             Busy => io.line([: [\"busy\"]])\n\
         ;\n",
    );

    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::NonExhaustiveMatch {
            reason: NonExhaustiveMatchReason::GuardedArmsRequireElse,
            missing_variants: Vec::new(),
        }
    );
}

// --------------------------
//  Option present patterns
// --------------------------

#[test]
fn parses_option_present_capture_statement_condition() {
    let (ast, string_table) = parse_single_file_ast(
        "value Int? = 42\nif value is |amount|:\n    io.line([: [amount]])\n;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::Match {
        scrutinee,
        arms,
        default,
        exhaustiveness,
    } = &body[1].kind
    else {
        panic!("expected option present capture to parse as a single-arm match statement");
    };

    assert_eq!(
        ast.type_environment.option_inner_type(scrutinee.type_id),
        Some(ast.type_environment.builtins().int),
        "scrutinee should keep semantic Int? identity even when its diagnostic spelling is inferred"
    );
    assert_eq!(arms.len(), 1);
    assert!(
        matches!(arms[0].pattern, MatchPattern::OptionPresentCapture { .. }),
        "single-predicate statement form should use option present capture"
    );
    assert_eq!(arms[0].body.len(), 1);
    assert_eq!(*exhaustiveness, MatchExhaustiveness::HasDefault);
    assert!(
        default.as_ref().is_some_and(Vec::is_empty),
        "statement-only present capture should synthesize an empty default so `none` falls through"
    );
}

#[test]
fn parses_option_match_present_capture_guard_and_none_patterns() {
    let (ast, string_table) = parse_single_file_ast(
        "value Int? = 42\n\
         if value is:\n\
             |positive| if positive > 0 => io.line([: [positive]])\n\
             |fallback| => io.line([: [fallback]])\n\
             none => io.line([: [\"missing\"]])\n\
         ;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::Match { arms, default, .. } = &body[1].kind else {
        panic!("expected option full match statement");
    };

    assert_eq!(arms.len(), 3);
    assert!(
        matches!(arms[0].pattern, MatchPattern::OptionPresentCapture { .. }),
        "first arm should capture any present option value"
    );
    assert!(arms[0].guard.is_some(), "first arm should keep its guard");
    assert!(
        matches!(arms[1].pattern, MatchPattern::OptionPresentCapture { .. }),
        "second arm should be the unguarded present-value fallback"
    );
    assert!(
        matches!(arms[2].pattern, MatchPattern::OptionNone { .. }),
        "third arm should cover absence"
    );
    assert!(default.is_none(), "the source did not include an else arm");
}

// --------------------------
//  Relational match patterns
// --------------------------

#[test]
fn parses_relational_match_patterns() {
    let (ast, string_table) = parse_single_file_ast(
        "value = 1\nif value is:\n    < 0 => io.line([: [\"neg\"]])\n    else => io.line([: [\"other\"]])\n;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::Match { arms, .. } = &body[1].kind else {
        panic!("expected match statement in start body");
    };

    assert_eq!(arms.len(), 1);
    assert!(
        matches!(arms[0].pattern, MatchPattern::Relational { .. }),
        "relational pattern should parse successfully"
    );
}

#[test]
fn rejects_semicolon_between_match_arms() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "value = 1\nif value is:\n    1 => io.line([: [\"one\"]]);\n    2 => io.line([: [\"two\"]])\n    else => io.line([: [\"other\"]])\n;\n",
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMatchArm)
    );
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidMatchArm {
            reason: InvalidMatchArmReason::SemicolonDelimiter
        }
    );
}

#[test]
fn allows_semicolons_inside_nested_structures_within_match_arms() {
    let (ast, string_table) = parse_single_file_ast(
        "value = 1\n\
         if value is:\n\
             1 =>\n\
                 if true:\n\
                     io.line([: [\"nested\"]])\n\
                 ;\n\
             else => io.line([: [\"other\"]])\n\
         ;\n",
    );

    let body = start_function_body(&ast, &string_table);

    let NodeKind::Match { arms, .. } = &body[1].kind else {
        panic!("expected match statement in start body");
    };

    assert_eq!(arms.len(), 1);
    assert!(
        matches!(arms[0].body[0].kind, NodeKind::If(_, _, _)),
        "nested if body inside a match arm should parse successfully"
    );
}

#[test]
fn parses_relational_int_patterns() {
    let (ast, string_table) = parse_single_file_ast(
        "value = 5\nif value is:\n    < 0 => io.line([: [\"negative\"]])\n    >= 0 => io.line([: [\"non-negative\"]])\n    else => io.line([: [\"fallback\"]])\n;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::Match { arms, .. } = &body[1].kind else {
        panic!("expected match statement in start body");
    };

    assert_eq!(arms.len(), 2);
    assert!(
        matches!(
            arms[0].pattern,
            MatchPattern::Relational {
                op: RelationalPatternOp::LessThan,
                ..
            }
        ),
        "first arm should be < pattern"
    );
    assert!(
        matches!(
            arms[1].pattern,
            MatchPattern::Relational {
                op: RelationalPatternOp::GreaterThanOrEqual,
                ..
            }
        ),
        "second arm should be >= pattern"
    );
}

#[test]
fn parses_relational_arm_after_single_line_assignment_body() {
    let (ast, string_table) = parse_single_file_ast(
        "value = 5\n\
         result ~= \"\"\n\
         if value is:\n\
             < 0 => result = \"negative\"\n\
             0 => result = \"zero\"\n\
             <= 10 => result = \"small\"\n\
             else => result = \"fallback\"\n\
         ;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::Match { arms, .. } = &body[2].kind else {
        panic!("expected match statement in start body");
    };

    assert_eq!(arms.len(), 3);
    assert!(
        matches!(
            arms[2].pattern,
            MatchPattern::Relational {
                op: RelationalPatternOp::LessThanOrEqual,
                ..
            }
        ),
        "assignment bodies must not consume the next relational arm header"
    );
}

#[test]
fn relational_patterns_without_default_are_not_exhaustive() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "value = 5\nif value is:\n    < 0 => io.line([: [\"negative\"]])\n    >= 0 => io.line([: [\"non-negative\"]])\n;\n",
    );

    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::NonExhaustiveMatch {
            reason: NonExhaustiveMatchReason::MissingElseArm,
            missing_variants: Vec::new(),
        }
    );
}

#[test]
fn relational_pattern_rejects_bool() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "value = true\nif value is:\n    < true => io.line([: [\"bad\"]])\n    else => io.line([: [\"fallback\"]])\n;\n",
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMatchPattern)
    );
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidMatchPattern {
            reason: InvalidMatchPatternReason::ScrutineeTypeUnsupportedForRelational,
            variant_name: None,
            scrutinee_name: None,
        }
    );
}

#[test]
fn relational_pattern_accepts_string() {
    let (ast, string_table) = parse_single_file_ast(
        "value = \"abc\"\nif value is:\n    < \"def\" => io.line([: [\"before\"]])\n    else => io.line([: [\"fallback\"]])\n;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::Match { arms, .. } = &body[1].kind else {
        panic!("expected match statement in start body");
    };

    assert_eq!(arms.len(), 1);
    assert!(
        matches!(
            arms[0].pattern,
            MatchPattern::Relational {
                op: RelationalPatternOp::LessThan,
                ..
            }
        ),
        "string relational pattern should parse successfully"
    );
}

#[test]
fn parses_multi_statement_match_arm_body_delimited_by_next_arm() {
    let (ast, string_table) = parse_single_file_ast(
        "value = 1\n\
         result ~= \"unset\"\n\
         if value is:\n\
             1 =>\n\
                 result = \"one\"\n\
                 io.line([: [result]])\n\
             2 =>\n\
                 result = \"two\"\n\
             else => result = \"other\"\n\
         ;\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::Match {
        arms,
        default: else_block,
        ..
    } = &body[2].kind
    else {
        panic!("expected match statement in start body");
    };

    assert_eq!(arms.len(), 2, "should have two pattern arms");
    assert_eq!(
        arms[0].body.len(),
        2,
        "first arm should have two statements"
    );
    assert_eq!(
        arms[1].body.len(),
        1,
        "second arm should have one statement"
    );
    assert!(else_block.is_some(), "should have an else default arm");
}

#[test]
fn case_is_valid_as_normal_identifier() {
    let (ast, string_table) = parse_single_file_ast("case = 42\nio.line([: [case]])\n");

    let body = start_function_body(&ast, &string_table);
    assert_eq!(
        body.len(),
        2,
        "should parse declaration and call without treating `case` as a keyword"
    );

    assert!(
        matches!(&body[0].kind, NodeKind::VariableDeclaration { .. }),
        "should parse `case = 42` as a normal variable declaration"
    );
}

// Inline value-if missing-else routing.

/// Asserts the first diagnostic is the requested inline control-flow reason.
fn assert_inline_control_flow_reason(source: &str, expected: InvalidControlFlowStatementReason) {
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidControlFlowStatement),
        "expected MOTH-RULE-0042 for {source:?}, got {:?}",
        diagnostic.kind,
    );
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidControlFlowStatement { reason } if *reason == expected,
        ),
        "expected {expected:?} for {source:?}, got {:?}",
        diagnostic.payload,
    );
}

#[test]
fn routes_missing_inline_else_at_declaration_boundary_through_value_if_missing_else() {
    for source in ["value = if true then 1", "value = if true then 1\n"] {
        let diagnostic = parse_single_file_ast_diagnostic(source);

        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidControlFlowStatement {
                reason: InvalidControlFlowStatementReason::ValueIfMissingElse,
            }
        ));
        assert_eq!(diagnostic.primary_location.start_pos.line_number, 0);
        assert_eq!(diagnostic.primary_location.start_pos.char_column, 22);
    }
}

#[test]
fn routes_missing_inline_else_at_assignment_boundary_through_value_if_missing_else() {
    // The assignment receiver keeps a real newline before the next statement,
    // so the absent `else` must be routed through the structured reason.
    assert_inline_control_flow_reason(
        "choose |condition Bool| -> Int:\n    result ~= 0\n    result = if condition then 10\n    return result\n;\nvalue = choose(false)\n",
        InvalidControlFlowStatementReason::ValueIfMissingElse,
    );
}

#[test]
fn routes_missing_inline_else_at_return_boundary_through_value_if_missing_else() {
    // The return receiver keeps a real newline before the block close, so the
    // absent `else` must be routed through the structured reason.
    assert_inline_control_flow_reason(
        "choose |condition Bool| -> Int:\n    return if condition then 10\n;\nvalue = choose(false)\n",
        InvalidControlFlowStatementReason::ValueIfMissingElse,
    );
}

#[test]
fn preserves_inline_multiline_when_newline_precedes_an_authored_else() {
    // A real newline before an authored `else` stays the multiline inline form
    // at both assignment and return receivers, not a missing-`else` diagnostic.
    assert_inline_control_flow_reason(
        "choose |condition Bool| -> Int:\n    result ~= 0\n    result = if condition then 10\nelse 0\n    return result\n;\nvalue = choose(false)\n",
        InvalidControlFlowStatementReason::InlineValueIfMultiline,
    );
    assert_inline_control_flow_reason(
        "choose |condition Bool| -> Int:\n    return if condition then 10\nelse 0\n;\nvalue = choose(false)\n",
        InvalidControlFlowStatementReason::InlineValueIfMultiline,
    );
    assert_inline_control_flow_reason(
        "value = if true then 1\nelse 0\n",
        InvalidControlFlowStatementReason::InlineValueIfMultiline,
    );
}

#[test]
fn ignores_else_owned_by_a_later_statement() {
    assert_inline_control_flow_reason(
        "choose |condition Bool| -> Int:\n    result ~= 0\n    result = if condition then 10\n    if condition:\n        result = 1\n    else\n        result = 2\n    ;\n    return result\n;\nvalue = choose(false)\n",
        InvalidControlFlowStatementReason::ValueIfMissingElse,
    );
}
