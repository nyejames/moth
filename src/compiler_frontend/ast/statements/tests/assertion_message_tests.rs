//! Assertion-message semantic traversal regression tests.
//!
//! WHAT: checks that assertion messages reject escaping control flow at every owned AST/TIR
//!       boundary while allowing a recovered fallible value.
//! WHY: message evaluation happens on the terminal failure edge, so each expression owner must
//!      preserve the same no-escape rule without token rescanning.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
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
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, function_node, node, test_source_location,
};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::value_mode::ValueMode;

fn propagated_expression(_line: i32) -> Expression {
    let span: Option<SourceSpan> = None;
    Expression::option_propagation_with_type_id(
        Expression::bool(true, span, ValueMode::ImmutableOwned),
        builtin_type_ids::BOOL,
        DataType::Bool,
        span,
    )
}

fn handoff_expression(handoff: OwnedRuntimeTemplateHandoff) -> Expression {
    Expression::new(
        ExpressionKind::RuntimeTemplateHandoff(Box::new(handoff)),
        None,
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    )
}

fn assert_escape_span(message: Expression) -> Option<SourceSpan> {
    let diagnostic = assert_message_escape_diagnostic(&message, &TemplateIrStore::new())
        .expect("assertion-message traversal should not fail")
        .expect("message should reject escaping control flow");
    assert!(matches!(
        diagnostic.payload,
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
        }
    ));
    diagnostic.primary_span
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
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");

    assert!(
        body.iter()
            .any(|node| matches!(node.kind, NodeKind::Assert { .. })),
        "the recovered message should remain an assertion expression"
    );
}

#[test]
fn owned_runtime_handoff_checks_dynamic_selectors_and_loop_headers() {
    let dynamic_span = propagated_expression(10).span;
    let dynamic = handoff_expression(OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::DynamicExpression {
            expression: Box::new(propagated_expression(10)),
            reactive_subscription: None,
            span: None,
        }),
        span: None,
    });
    assert_eq!(assert_escape_span(dynamic), dynamic_span);

    let selector_span = propagated_expression(11).span;
    let selector = handoff_expression(OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::BranchChain {
            branches: vec![OwnedRuntimeTemplateBranch {
                selector: TemplateBranchSelector::Bool(propagated_expression(11)),
                body: OwnedRuntimeTemplateNode::Sequence {
                    children: vec![],
                    span: None,
                },
                span: None,
            }],
            fallback: None,
            else_marker: None,
            span: None,
        }),
        span: None,
    });
    assert_eq!(assert_escape_span(selector), selector_span);

    let header_span = propagated_expression(12).span;
    let header = handoff_expression(OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Loop {
            header: TemplateLoopHeader::Conditional {
                condition: Box::new(propagated_expression(12)),
            },
            body: Box::new(OwnedRuntimeTemplateNode::Sequence {
                children: vec![],
                span: None,
            }),
            aggregate_wrapper: None,
            span: None,
        }),
        span: None,
    });
    assert_eq!(assert_escape_span(header), header_span);
}

#[test]
fn raw_tir_dynamic_expression_is_checked_before_hir_handoff() {
    let mut store = TemplateIrStore::new();
    let site_id = store.next_expression_site_id();
    let span = propagated_expression(20).span;
    let node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(propagated_expression(20)),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        span,
    ));
    let root = store.push_template(TemplateIr::new(
        node,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        span,
    ));
    let template = Template {
        tir_reference: TemplateTirReference {
            root,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        span,
    };
    let message = Expression::new(
        ExpressionKind::Template(Box::new(template)),
        span,
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );

    let diagnostic = assert_message_escape_diagnostic(&message, &store)
        .expect("raw TIR traversal should not fail")
        .expect("raw TIR dynamic expressions must reject propagation");
    assert_eq!(diagnostic.primary_span, span);
}

#[test]
fn fallible_call_keeps_call_mapping_span_separate_from_postfix_effect_span() {
    let source = r#"
may_fail || -> String, Error!:
    return! Error("boom")
;

check || -> String, Error!:
    value = may_fail()!
    return value
;
"#;
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let declaration = body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::VariableDeclaration(declaration) => Some(declaration),
            _ => None,
        })
        .expect("expected the fallible value declaration");

    let propagation_span = declaration
        .value
        .propagation_span()
        .expect("parsed postfix propagation must retain its authored span");
    assert_ne!(Some(propagation_span), declaration.value.span);

    assert_eq!(
        classify_assertion_message_effect(&declaration.value, &TemplateIrStore::new())
            .expect("effect classification should succeed"),
        Some(EnclosingExitEffect::ErrorPropagation(Some(
            propagation_span
        )))
    );
}

#[test]
fn effect_classifier_respects_function_and_loop_control_boundaries() {
    let span = test_source_location(30);
    let nested_function = function_node(
        PathId::ROOT,
        FunctionSignature::default(),
        vec![node(NodeKind::Return(vec![]), span)],
        span,
    );
    let loop_with_local_break = node(
        NodeKind::WhileLoop(
            Expression::bool(true, span, ValueMode::ImmutableOwned),
            vec![node(NodeKind::Break, span)],
        ),
        span,
    );
    let message = Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::If(ValueIfBlock {
                condition: Expression::bool(true, span, ValueMode::ImmutableOwned),
                then_body: vec![nested_function, loop_with_local_break],
                else_body: vec![],
                then_scope: PathId::ROOT,
                else_scope: PathId::ROOT,
                span,
                generic_request_ranges: Default::default(),
                result_type_ids: vec![],
            })),
        },
        span,
        builtin_type_ids::STRING,
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
                condition: Expression::bool(true, span, ValueMode::ImmutableOwned),
                then_body: vec![node(NodeKind::Break, span)],
                else_body: vec![],
                then_scope: PathId::ROOT,
                else_scope: PathId::ROOT,
                span,
                generic_request_ranges: Default::default(),
                result_type_ids: vec![],
            })),
        },
        span,
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
                condition: Expression::bool(true, span, ValueMode::ImmutableOwned),
                then_body: vec![node(NodeKind::Continue, span)],
                else_body: vec![],
                then_scope: PathId::ROOT,
                else_scope: PathId::ROOT,
                span,
                generic_request_ranges: Default::default(),
                result_type_ids: vec![],
            })),
        },
        span,
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
                condition: Expression::bool(true, span, ValueMode::ImmutableOwned),
                then_body: vec![node(NodeKind::Return(vec![]), span)],
                else_body: vec![],
                then_scope: PathId::ROOT,
                else_scope: PathId::ROOT,
                span,
                generic_request_ranges: Default::default(),
                result_type_ids: vec![],
            })),
        },
        span,
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );
    assert_eq!(
        classify_assertion_message_effect(&enclosing_return, &TemplateIrStore::new())
            .expect("enclosing return classification should succeed"),
        Some(EnclosingExitEffect::FunctionReturn(span))
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
