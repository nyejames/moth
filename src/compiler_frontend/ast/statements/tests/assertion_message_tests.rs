//! Assertion-message semantic traversal regression tests.
//!
//! WHAT: checks that assertion messages reject escaping control flow at every owned AST/TIR
//!       boundary while allowing a recovered fallible value.
//! WHY: message evaluation happens on the terminal failure edge, so each expression owner must
//!      preserve the same no-escape rule without token rescanning.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::assertion_message_effects::{
    assert_message_escape_diagnostic, classify_assertion_message_effect,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionView, SuccessShape,
};
use crate::compiler_frontend::ast::expressions::failure_classification::EnclosingExitEffect;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueIfBlock,
};
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
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
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, function_node, node, runtime_template_handoff_expr, test_source_location,
};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;

/// Builder plus constant store for assertion-message fixtures.
///
/// WHY: literals need the constant store, and expression constructors need the builder.
struct TestExpressions {
    builder: AstExpressionBuilder,
    constants: ConstValueStore,
}

impl Default for TestExpressions {
    fn default() -> Self {
        Self {
            builder: AstExpressionBuilder::new().unwrap(),
            constants: ConstValueStore::default(),
        }
    }
}

impl TestExpressions {
    fn bool(&mut self, value: bool) -> ExprId {
        self.builder
            .bool(&mut self.constants, value, None, ValueMode::ImmutableOwned)
            .expect("fixture bool")
            .id()
    }

    /// Propagating `Bool` payload: the escaping shape assertion messages must reject.
    fn propagated(&mut self) -> ExprId {
        let value = self.bool(true);
        self.builder
            .option_propagation_with_type_id(self.builder.own(value), builtin_type_ids::BOOL, None)
            .expect("fixture propagation")
            .id()
    }

    /// Builds a runtime-template handoff as a string-typed operand via the shared fixture.
    fn runtime_handoff(&mut self, handoff: OwnedRuntimeTemplateHandoff) -> ExprId {
        runtime_template_handoff_expr(&mut self.builder, handoff, None, ValueMode::ImmutableOwned)
    }

    fn value_block_message(&mut self, block: ValueBlock, span: Option<SourceSpan>) -> ExprId {
        self.builder
            .value_block(
                block,
                span,
                SuccessShape::One(builtin_type_ids::STRING),
                ValueMode::ImmutableOwned,
            )
            .expect("fixture value-block message")
            .id()
    }

    fn view(&self, id: ExprId) -> ExpressionView<'_> {
        self.builder.view(id).expect("fixture expression view")
    }

    fn span_of(&self, id: ExprId) -> Option<SourceSpan> {
        self.view(id).span()
    }
}

/// Builds an `if` value block with the given condition and then-branch body.
fn if_value_block(
    condition: ExprId,
    then_body: Vec<AstNode>,
    span: Option<SourceSpan>,
) -> ValueBlock {
    ValueBlock::If(ValueIfBlock {
        condition,
        then_body,
        else_body: vec![],
        then_scope: PathId::ROOT,
        else_scope: PathId::ROOT,
        span,
        generic_request_ranges: Default::default(),
        result_type_ids: vec![],
    })
}

fn assert_escape_span(fx: &TestExpressions, message: ExprId) -> Option<SourceSpan> {
    let diagnostic =
        assert_message_escape_diagnostic(fx.view(message), &TemplateIrStore::new(), &fx.builder)
            .expect("assertion-message traversal should not fail")
            .expect("message should reject escaping control flow");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
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
    let mut fx = TestExpressions::default();

    let dynamic_expression = fx.propagated();
    let dynamic_span = fx.span_of(dynamic_expression);
    let dynamic_node = OwnedRuntimeTemplateNode::DynamicExpression {
        expression: dynamic_expression,
        span: None,
    };
    let dynamic = fx.runtime_handoff(OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(dynamic_node),
    });
    assert_eq!(assert_escape_span(&fx, dynamic), dynamic_span);

    let selector_condition = fx.propagated();
    let selector_span = fx.span_of(selector_condition);
    let selector_node = OwnedRuntimeTemplateNode::Conditional {
        selector: Box::new(TemplateBranchSelector::Bool(selector_condition)),
        body: Box::new(OwnedRuntimeTemplateNode::Sequence {
            children: vec![],
            span: None,
        }),
        span: None,
    };
    let selector = fx.runtime_handoff(OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(selector_node),
    });
    assert_eq!(assert_escape_span(&fx, selector), selector_span);

    let header_condition = fx.propagated();
    let header_span = fx.span_of(header_condition);
    let header_node = OwnedRuntimeTemplateNode::Loop {
        header: TemplateLoopHeader::Conditional {
            condition: header_condition,
        },
        body: Box::new(OwnedRuntimeTemplateNode::Sequence {
            children: vec![],
            span: None,
        }),
        aggregate_wrapper: None,
        span: None,
    };
    let header = fx.runtime_handoff(OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(header_node),
    });
    assert_eq!(assert_escape_span(&fx, header), header_span);
}

#[test]
fn raw_tir_dynamic_expression_is_checked_before_hir_handoff() {
    let mut fx = TestExpressions::default();
    let mut store = TemplateIrStore::new();
    let site_id = store.next_expression_site_id();
    let expression = fx.propagated();
    let span = fx.span_of(expression);
    let node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression,
            origin: TemplateSegmentOrigin::Body,
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
    let message = fx
        .builder
        .template(template, ValueMode::ImmutableOwned)
        .expect("fixture template message")
        .id();

    let diagnostic = assert_message_escape_diagnostic(fx.view(message), &store, &fx.builder)
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

    let initializer = declaration
        .value
        .initializer()
        .expect("propagated declaration initializer");
    let initializer_view = ast
        .expressions
        .view(initializer)
        .expect("propagated declaration view");
    let propagation_span = initializer_view
        .propagation_span()
        .expect("parsed postfix propagation must retain its authored span");
    assert_ne!(Some(propagation_span), initializer_view.span());

    assert_eq!(
        classify_assertion_message_effect(
            initializer_view,
            &TemplateIrStore::new(),
            &ast.expressions
        )
        .expect("effect classification should succeed"),
        Some(EnclosingExitEffect::ErrorPropagation(Some(
            propagation_span
        )))
    );
}

#[test]
fn effect_classifier_respects_function_and_loop_control_boundaries() {
    let mut fx = TestExpressions::default();
    let span = test_source_location(30);
    let nested_function = function_node(
        PathId::ROOT,
        FunctionSignature::default(),
        vec![node(NodeKind::Return(vec![]), span)],
        span,
    );
    let loop_condition = fx.bool(true);
    let loop_with_local_break = node(
        NodeKind::WhileLoop(loop_condition, vec![node(NodeKind::Break, span)]),
        span,
    );
    let condition = fx.bool(true);
    let block = if_value_block(
        condition,
        vec![nested_function, loop_with_local_break],
        span,
    );
    let message = fx.value_block_message(block, span);

    assert_eq!(
        classify_assertion_message_effect(fx.view(message), &TemplateIrStore::new(), &fx.builder)
            .expect("nested local control-flow classification should succeed"),
        None
    );

    let condition = fx.bool(true);
    let block = if_value_block(condition, vec![node(NodeKind::Break, span)], span);
    let outer_break_message = fx.value_block_message(block, span);
    assert!(
        classify_assertion_message_effect(
            fx.view(outer_break_message),
            &TemplateIrStore::new(),
            &fx.builder
        )
        .is_err(),
        "depth-zero break must be guarded as an impossible assertion-message AST shape"
    );

    let condition = fx.bool(true);
    let block = if_value_block(condition, vec![node(NodeKind::Continue, span)], span);
    let outer_continue_message = fx.value_block_message(block, span);
    assert!(
        classify_assertion_message_effect(
            fx.view(outer_continue_message),
            &TemplateIrStore::new(),
            &fx.builder
        )
        .is_err(),
        "depth-zero continue must be guarded as an impossible assertion-message AST shape"
    );

    let condition = fx.bool(true);
    let block = if_value_block(condition, vec![node(NodeKind::Return(vec![]), span)], span);
    let enclosing_return = fx.value_block_message(block, span);
    assert_eq!(
        classify_assertion_message_effect(
            fx.view(enclosing_return),
            &TemplateIrStore::new(),
            &fx.builder
        )
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
fn static_true_assertion_still_validates_invalid_template_message_source() {
    let diagnostic =
        crate::compiler_frontend::tests::parse_support::parse_single_file_ast_diagnostic(
            "assert(true, [: before [break] after])\n",
        );

    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0002");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken { .. }
    ));
}
