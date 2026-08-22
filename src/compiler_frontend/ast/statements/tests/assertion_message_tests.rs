//! Assertion-message semantic traversal regression tests.
//!
//! WHAT: checks that assertion messages reject escaping control flow at every owned AST/TIR
//!       boundary while allowing a recovered fallible value.
//! WHY: message evaluation happens on the terminal failure edge, so each expression owner must
//!      preserve the same no-escape rule without token rescanning.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::assertion_message_effects::{
    EnclosingExitEffect, assert_message_escape_diagnostic, classify_assertion_message_effect,
};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueIfBlock,
};
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateBranch, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template::{
    Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIr, TemplateIrNode, TemplateIrNodeKind, TemplateIrStore, TemplateIrSummary,
    TemplateTirPhase, TemplateTirReference, TemplateViewContext,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidControlFlowStatementReason, InvalidFallibleHandlingReason,
    InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, function_node, node, test_source_location,
};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn propagated_expression(line: i32) -> Expression {
    let location = SourceLocation {
        start_pos: crate::compiler_frontend::tokenizer::tokens::CharPosition {
            line_number: line,
            char_column: 3,
        },
        end_pos: crate::compiler_frontend::tokenizer::tokens::CharPosition {
            line_number: line,
            char_column: 4,
        },
        ..SourceLocation::default()
    };
    Expression::option_propagation_with_type_id(
        Expression::bool(true, location.clone(), ValueMode::ImmutableOwned),
        builtin_type_ids::BOOL,
        DataType::Bool,
        location,
    )
}

fn handoff_expression(handoff: OwnedRuntimeTemplateHandoff) -> Expression {
    Expression::new(
        ExpressionKind::RuntimeTemplateHandoff(Box::new(handoff)),
        SourceLocation::default(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    )
}

fn assert_escape_location(message: Expression) -> SourceLocation {
    let diagnostic = assert_message_escape_diagnostic(&message, &TemplateIrStore::new())
        .expect("assertion-message traversal should not fail")
        .expect("message should reject escaping control flow");
    assert!(matches!(
        diagnostic.payload,
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
        }
    ));
    diagnostic.primary_location
}

#[test]
fn recovered_fallible_value_is_allowed_as_assertion_message() {
    let source = r#"
may_fail || -> String, Error!:
    return! Error("boom")
;

check || -> String:
    message = may_fail() catch then "fallback"
    assert(false, message)
    return "unreachable"
;
"#;
    let (ast, string_table) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &string_table, "check");

    assert!(
        body.iter()
            .any(|node| matches!(node.kind, NodeKind::Assert { .. })),
        "the recovered message should remain an assertion expression"
    );
}

#[test]
fn owned_runtime_handoff_checks_dynamic_selectors_and_loop_headers() {
    let dynamic_location = propagated_expression(10).location;
    let dynamic = handoff_expression(OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::DynamicExpression {
            expression: Box::new(propagated_expression(10)),
            reactive_subscription: None,
        }),
        location: SourceLocation::default(),
    });
    assert_eq!(assert_escape_location(dynamic), dynamic_location);

    let selector_location = propagated_expression(11).location;
    let selector = handoff_expression(OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::BranchChain {
            branches: vec![OwnedRuntimeTemplateBranch {
                selector: TemplateBranchSelector::Bool(propagated_expression(11)),
                body: OwnedRuntimeTemplateNode::Sequence { children: vec![] },
                location: SourceLocation::default(),
            }],
            fallback: None,
            location: SourceLocation::default(),
        }),
        location: SourceLocation::default(),
    });
    assert_eq!(assert_escape_location(selector), selector_location);

    let header_location = propagated_expression(12).location;
    let header = handoff_expression(OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Loop {
            header: TemplateLoopHeader::Conditional {
                condition: Box::new(propagated_expression(12)),
            },
            body: Box::new(OwnedRuntimeTemplateNode::Sequence { children: vec![] }),
            aggregate_wrapper: None,
            location: SourceLocation::default(),
        }),
        location: SourceLocation::default(),
    });
    assert_eq!(assert_escape_location(header), header_location);
}

#[test]
fn raw_tir_dynamic_expression_is_checked_before_hir_handoff() {
    let mut store = TemplateIrStore::new();
    let site_id = store.next_expression_site_id();
    let location = propagated_expression(20).location;
    let node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(propagated_expression(20)),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        location.clone(),
    ));
    let root = store.push_template(TemplateIr::new(
        node,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        location.clone(),
    ));
    let template = Template {
        tir_reference: TemplateTirReference {
            root,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        location: location.clone(),
    };
    let message = Expression::new(
        ExpressionKind::Template(Box::new(template)),
        location.clone(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );

    let diagnostic = assert_message_escape_diagnostic(&message, &store)
        .expect("raw TIR traversal should not fail")
        .expect("raw TIR dynamic expressions must reject propagation");
    assert_eq!(diagnostic.primary_location, location);
}

#[test]
fn fallible_call_keeps_call_mapping_location_separate_from_postfix_effect_location() {
    let source = r#"
may_fail || -> String, Error!:
    return! Error("boom")
;

check || -> String, Error!:
    value = may_fail()!
    return value
;
"#;
    let (ast, string_table) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &string_table, "check");
    let declaration = body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::VariableDeclaration(declaration) => Some(declaration),
            _ => None,
        })
        .expect("expected the fallible value declaration");

    let propagation_location = declaration
        .value
        .propagation_location()
        .expect("parsed postfix propagation must retain its authored location");
    assert_eq!(
        propagation_location.start_pos.line_number,
        declaration.value.location.start_pos.line_number
    );
    assert!(
        propagation_location.start_pos.char_column
            > declaration.value.location.start_pos.char_column,
        "the postfix marker must be after the ordinary call location"
    );

    assert_eq!(
        classify_assertion_message_effect(&declaration.value, &TemplateIrStore::new())
            .expect("effect classification should succeed"),
        Some(EnclosingExitEffect::ErrorPropagation(
            propagation_location.clone(),
        ))
    );
}

#[test]
fn effect_classifier_respects_function_and_loop_control_boundaries() {
    let location = test_source_location(30);
    let nested_function = function_node(
        Default::default(),
        FunctionSignature::default(),
        vec![node(NodeKind::Return(vec![]), location.clone())],
        location.clone(),
    );
    let loop_with_local_break = node(
        NodeKind::WhileLoop(
            Expression::bool(true, location.clone(), ValueMode::ImmutableOwned),
            vec![node(NodeKind::Break, location.clone())],
        ),
        location.clone(),
    );
    let message = Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::If(ValueIfBlock {
                condition: Expression::bool(true, location.clone(), ValueMode::ImmutableOwned),
                then_body: vec![nested_function, loop_with_local_break],
                else_body: vec![],
                location: location.clone(),
                result_type_ids: vec![],
            })),
        },
        location.clone(),
        crate::compiler_frontend::datatypes::builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );

    assert_eq!(
        classify_assertion_message_effect(&message, &TemplateIrStore::new())
            .expect("nested local control-flow classification should succeed"),
        None
    );

    let outer_break_message = Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::If(ValueIfBlock {
                condition: Expression::bool(true, location.clone(), ValueMode::ImmutableOwned),
                then_body: vec![node(NodeKind::Break, location.clone())],
                else_body: vec![],
                location: location.clone(),
                result_type_ids: vec![],
            })),
        },
        location.clone(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );
    assert!(
        classify_assertion_message_effect(&outer_break_message, &TemplateIrStore::new()).is_err(),
        "depth-zero break must be guarded as an impossible assertion-message AST shape"
    );

    let outer_continue_message = Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::If(ValueIfBlock {
                condition: Expression::bool(true, location.clone(), ValueMode::ImmutableOwned),
                then_body: vec![node(NodeKind::Continue, location.clone())],
                else_body: vec![],
                location: location.clone(),
                result_type_ids: vec![],
            })),
        },
        location.clone(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );
    assert!(
        classify_assertion_message_effect(&outer_continue_message, &TemplateIrStore::new())
            .is_err(),
        "depth-zero continue must be guarded as an impossible assertion-message AST shape"
    );

    let enclosing_return = Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::If(ValueIfBlock {
                condition: Expression::bool(true, location.clone(), ValueMode::ImmutableOwned),
                then_body: vec![AstNode {
                    kind: NodeKind::Return(vec![]),
                    location: location.clone(),
                    scope: Default::default(),
                }],
                else_body: vec![],
                location: location.clone(),
                result_type_ids: vec![],
            })),
        },
        location.clone(),
        crate::compiler_frontend::datatypes::builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );
    assert_eq!(
        classify_assertion_message_effect(&enclosing_return, &TemplateIrStore::new())
            .expect("enclosing return classification should succeed"),
        Some(EnclosingExitEffect::FunctionReturn(location))
    );
}

#[test]
fn parser_rejects_value_block_before_outer_loop_control_can_reach_assertion_message() {
    let source = r#"
check || -> String:
    loop true:
        assert(true, if true:
            break
            then "unreachable"
        else
            then "message"
        ;
        )
        break
    ;
    return "done"
;
"#;
    let payload =
        crate::compiler_frontend::tests::parse_support::parse_single_file_ast_diagnostic(source)
            .payload;
    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidControlFlowStatement {
            reason: InvalidControlFlowStatementReason::ValueBlockOutsideReceiver,
        }
    ));
}

#[test]
fn static_true_assertion_still_reports_invalid_template_message_source() {
    let payload = crate::compiler_frontend::tests::parse_support::parse_single_file_ast_diagnostic(
        "assert(true, [: before [break] after])\n",
    )
    .payload;

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::OrphanTemplateBreak,
        }
    ));
}
