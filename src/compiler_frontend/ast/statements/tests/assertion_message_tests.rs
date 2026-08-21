//! Assertion-message semantic traversal regression tests.
//!
//! WHAT: checks that assertion messages reject escaping control flow at every owned AST/TIR
//!       boundary while allowing a recovered fallible value.
//! WHY: message evaluation happens on the terminal failure edge, so each expression owner must
//!      preserve the same no-escape rule without token rescanning.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
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
use crate::compiler_frontend::compiler_messages::InvalidFallibleHandlingReason;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
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
    let diagnostic = super::assert_message_escape_diagnostic(&message, &TemplateIrStore::new())
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

    let diagnostic = super::assert_message_escape_diagnostic(&message, &store)
        .expect("raw TIR traversal should not fail")
        .expect("raw TIR dynamic expressions must reject propagation");
    assert_eq!(diagnostic.primary_location, location);
}
