//! Tests for final AST type-boundary validation of template expression payloads.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, LoopBindings, NodeKind, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::ast::templates::tir::{
    ExpressionSiteId, TemplateIr, TemplateIrBranch, TemplateIrBuilder, TemplateIrNode,
    TemplateIrStore, TemplateIrSummary, TemplateLoopHeaderExpressionSites, TemplateTirPhase,
    TemplateTirReference, TemplateViewContext, TirExpressionOverlay,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotContributionSource, OwnedRuntimeSlotSite,
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateBranch, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

fn invalid_string_expression(span: Option<SourceSpan>) -> Expression {
    Expression::new(
        ExpressionKind::Bool(true),
        span,
        TypeId(9999),
        DataType::Bool,
        ValueMode::ImmutableOwned,
    )
}

fn orphan_bool_expression() -> Expression {
    Expression::new(
        ExpressionKind::Bool(true),
        None,
        TypeId(9999),
        DataType::Bool,
        ValueMode::ImmutableOwned,
    )
}

fn owned_render_handoff(node: OwnedRuntimeTemplateNode) -> OwnedRuntimeTemplateHandoff {
    OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(node),
        span: None,
    }
}

fn validate_owned_handoff_with_orphan_type_id(node: OwnedRuntimeTemplateNode) {
    let handoff = owned_render_handoff(node);
    let type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let context = TypeValidationContext {
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_owned_runtime_template_handoff(&handoff, &context)
        .expect_err("owned runtime handoff payloads must retain TypeId validation");
    assert!(error.msg.contains("9999"));
}

#[test]
fn owned_runtime_branch_selector_type_ids_are_validated_before_inactive_elision() {
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let selectors = vec![
        TemplateBranchSelector::Bool(orphan_bool_expression()),
        TemplateBranchSelector::OptionPresentCapture {
            scrutinee: Expression::bool(true, None, ValueMode::ImmutableOwned),
            pattern: Box::new(MatchPattern::OptionPresentCapture {
                name: strings.intern("value"),
                binding_path: PathId::ROOT,
                inner_type_id: TypeId(9999),
                span: None,
                binding_span: None,
            }),
        },
    ];

    for selector in selectors {
        validate_owned_handoff_with_orphan_type_id(OwnedRuntimeTemplateNode::BranchChain {
            branches: vec![OwnedRuntimeTemplateBranch {
                selector,
                body: OwnedRuntimeTemplateNode::Sequence {
                    children: Vec::new(),
                    span: None,
                },
                span: None,
            }],
            fallback: None,
            else_marker: None,
            span: None,
        });
    }
}
fn orphan_declaration() -> Declaration {
    Declaration {
        id: PathId::ROOT,
        value: orphan_bool_expression(),
        binding_span: None,
        config_qualifier: None,
    }
}

fn empty_loop_body() -> Box<OwnedRuntimeTemplateNode> {
    Box::new(OwnedRuntimeTemplateNode::Sequence {
        children: Vec::new(),
        span: None,
    })
}

#[test]
fn owned_runtime_slot_handoff_validates_all_expression_payload_routes() {
    let slot_handoffs = vec![
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::DynamicExpression {
                expression: Box::new(orphan_bool_expression()),
                reactive_subscription: None,
                span: None,
            },
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
            span: None,
        },
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::Sequence {
                children: Vec::new(),
                span: None,
            },
            contribution_sources: vec![OwnedRuntimeSlotContributionSource {
                source: RuntimeSlotContributionSourceId(0),
                render_root: OwnedRuntimeTemplateNode::BranchChain {
                    branches: vec![OwnedRuntimeTemplateBranch {
                        selector: TemplateBranchSelector::Bool(orphan_bool_expression()),
                        body: OwnedRuntimeTemplateNode::Sequence {
                            children: Vec::new(),
                            span: None,
                        },
                        span: None,
                    }],
                    fallback: None,
                    else_marker: None,
                    span: None,
                },
                renders_wrapper_unconditionally: false,
                span: None,
            }],
            slot_sites: Vec::new(),
            span: None,
        },
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::Sequence {
                children: Vec::new(),
                span: None,
            },
            contribution_sources: Vec::new(),
            slot_sites: vec![OwnedRuntimeSlotSite {
                site: RuntimeSlotSiteId(0),
                render_root: OwnedRuntimeTemplateNode::Loop {
                    header: TemplateLoopHeader::Range {
                        bindings: Box::new(LoopBindings {
                            item: Some(orphan_declaration()),
                            index: None,
                        }),
                        range: Box::new(RangeLoopSpec {
                            start: Expression::bool(true, None, ValueMode::ImmutableOwned),
                            end: Expression::bool(true, None, ValueMode::ImmutableOwned),
                            end_kind: RangeEndKind::Exclusive,
                            step: None,
                        }),
                    },
                    body: empty_loop_body(),
                    aggregate_wrapper: None,
                    span: None,
                },
                span: None,
            }],
            span: None,
        },
    ];

    for handoff in slot_handoffs {
        let type_environment = TypeEnvironment::new();
        let store = TemplateIrStore::new();
        let context = TypeValidationContext {
            type_environment: &type_environment,
            template_ir_store: &store,
        };
        let error = validate_owned_runtime_slot_application_handoff(&handoff, &context)
            .expect_err("slot handoff payloads must retain TypeId validation");
        assert!(error.msg.contains("9999"));
    }
}

#[test]
fn static_true_assertion_owned_handoff_is_validated_before_message_elision() {
    let handoff = owned_render_handoff(OwnedRuntimeTemplateNode::BranchChain {
        branches: vec![OwnedRuntimeTemplateBranch {
            selector: TemplateBranchSelector::Bool(orphan_bool_expression()),
            body: OwnedRuntimeTemplateNode::Sequence {
                children: Vec::new(),
                span: None,
            },
            span: None,
        }],
        fallback: None,
        else_marker: None,
        span: None,
    });
    let node = AstNode {
        kind: NodeKind::Assert {
            condition: Expression::bool(true, None, ValueMode::ImmutableOwned),
            message: Expression::new(
                ExpressionKind::RuntimeTemplateHandoff(Box::new(handoff)),
                None,
                builtin_type_ids::STRING,
                DataType::Template,
                ValueMode::ImmutableOwned,
            ),
        },
        span: None,
        scope: PathId::ROOT,
    };
    let type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let context = TypeValidationContext {
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_node(&node, &context)
        .expect_err("static-true owned message payload must be validated before elision");
    assert!(error.msg.contains("9999"));
}

#[test]
fn static_true_assertion_slot_handoff_is_validated_before_message_elision() {
    let slot_handoff = OwnedRuntimeSlotApplicationHandoff {
        wrapper: OwnedRuntimeTemplateNode::DynamicExpression {
            expression: Box::new(orphan_bool_expression()),
            reactive_subscription: None,
            span: None,
        },
        contribution_sources: Vec::new(),
        slot_sites: Vec::new(),
        span: None,
    };
    let node = AstNode {
        kind: NodeKind::Assert {
            condition: Expression::bool(true, None, ValueMode::ImmutableOwned),
            message: Expression::new(
                ExpressionKind::RuntimeSlotApplicationHandoff(Box::new(slot_handoff)),
                None,
                builtin_type_ids::STRING,
                DataType::Template,
                ValueMode::ImmutableOwned,
            ),
        },
        span: None,
        scope: PathId::ROOT,
    };
    let type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let context = TypeValidationContext {
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_node(&node, &context)
        .expect_err("static-true slot message payload must be validated before elision");
    assert!(error.msg.contains("9999"));
}

#[test]
fn owned_runtime_loop_header_type_ids_are_validated_before_inactive_elision() {
    let headers = [
        TemplateLoopHeader::Conditional {
            condition: Box::new(orphan_bool_expression()),
        },
        TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: orphan_bool_expression(),
                end: Expression::bool(true, None, ValueMode::ImmutableOwned),
                end_kind: RangeEndKind::Exclusive,
                step: None,
            }),
        },
        TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: Expression::bool(true, None, ValueMode::ImmutableOwned),
                end: orphan_bool_expression(),
                end_kind: RangeEndKind::Exclusive,
                step: None,
            }),
        },
        TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: Expression::bool(true, None, ValueMode::ImmutableOwned),
                end: Expression::bool(true, None, ValueMode::ImmutableOwned),
                end_kind: RangeEndKind::Exclusive,
                step: Some(orphan_bool_expression()),
            }),
        },
        TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: Some(orphan_declaration()),
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: Expression::bool(true, None, ValueMode::ImmutableOwned),
                end: Expression::bool(true, None, ValueMode::ImmutableOwned),
                end_kind: RangeEndKind::Exclusive,
                step: None,
            }),
        },
        TemplateLoopHeader::Collection {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            iterable: Box::new(orphan_bool_expression()),
        },
        TemplateLoopHeader::Collection {
            bindings: Box::new(LoopBindings {
                item: None,
                index: Some(orphan_declaration()),
            }),
            iterable: Box::new(Expression::bool(true, None, ValueMode::ImmutableOwned)),
        },
    ];

    for header in headers {
        validate_owned_handoff_with_orphan_type_id(OwnedRuntimeTemplateNode::Loop {
            header,
            body: Box::new(OwnedRuntimeTemplateNode::Sequence {
                children: Vec::new(),
                span: None,
            }),
            aggregate_wrapper: None,
            span: None,
        });
    }
}

#[test]
fn static_true_assertion_message_reaches_type_validation_before_elision() {
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let structural = Expression::string_slice(
        strings.intern("structural"),
        None,
        ValueMode::ImmutableOwned,
    );
    let mut store = TemplateIrStore::new();
    let template = template_with_dynamic_overlay(
        &mut store,
        structural,
        invalid_string_expression(None),
        TemplateTirPhase::Finalized,
    );
    let template_expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut type_environment = TypeEnvironment::new();
    let option_string_type_id = type_environment.intern_option(builtin_type_ids::STRING);
    let message = Expression::new(
        ExpressionKind::Coerced {
            value: Box::new(template_expression),
            to_type: option_string_type_id,
        },
        None,
        option_string_type_id,
        DataType::Option(Box::new(DataType::StringSlice)),
        ValueMode::ImmutableOwned,
    );
    let node = AstNode {
        kind: NodeKind::Assert {
            condition: Expression::bool(true, None, ValueMode::ImmutableOwned),
            message,
        },
        span: None,
        scope: PathId::ROOT,
    };
    let context = TypeValidationContext {
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_node(&node, &context)
        .expect_err("static-true message payloads must be validated before elision");
    assert!(error.msg.contains("9999"));
}

fn template_with_dynamic_overlay(
    store: &mut TemplateIrStore,
    structural: Expression,
    overlay: Expression,
    phase: TemplateTirPhase,
) -> Template {
    let site_id = store.next_expression_site_id();
    let node = store.push_node(TemplateIrNode::new(
        crate::compiler_frontend::ast::templates::tir::TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(structural),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        None,
    ));
    let root = store.push_template(TemplateIr::new(
        node,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        None,
    ));
    let expression_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(site_id, Box::new(overlay))],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    Template {
        tir_reference: TemplateTirReference {
            root,
            phase,
            context,
        },
        span: None,
    }
}

#[test]
fn validation_checks_effective_dynamic_expression_overlay() {
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let structural = Expression::string_slice(
        strings.intern("structural"),
        None,
        ValueMode::ImmutableOwned,
    );
    let overlay = invalid_string_expression(None);
    let mut store = TemplateIrStore::new();
    let template =
        template_with_dynamic_overlay(&mut store, structural, overlay, TemplateTirPhase::Finalized);
    let type_environment = TypeEnvironment::new();
    let store_borrow = store;
    let context = TypeValidationContext {
        type_environment: &type_environment,
        template_ir_store: &store_borrow,
    };

    let error = validate_template_expression_payloads(&template, &context)
        .expect_err("orphan overlay type should be rejected");
    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("9999"));
}

#[test]
fn validation_rejects_non_finalized_template_reference() {
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let structural = Expression::string_slice(
        strings.intern("structural"),
        None,
        ValueMode::ImmutableOwned,
    );
    let mut store = TemplateIrStore::new();
    let template = template_with_dynamic_overlay(
        &mut store,
        structural.clone(),
        structural,
        TemplateTirPhase::Composed,
    );
    let type_environment = TypeEnvironment::new();
    let context = TypeValidationContext {
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_template_expression_payloads(&template, &context)
        .expect_err("non-finalized template should be rejected");
    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("Finalized"));
}

#[test]
fn validation_reports_missing_template_root() {
    let store = TemplateIrStore::new();
    let template = Template {
        tir_reference: TemplateTirReference {
            root: crate::compiler_frontend::ast::templates::tir::TemplateIrId::new(99),
            phase: TemplateTirPhase::Finalized,
            context: TemplateViewContext::default(),
        },
        span: None,
    };
    let type_environment = TypeEnvironment::new();
    let context = TypeValidationContext {
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_template_expression_payloads(&template, &context)
        .expect_err("missing root should be rejected");
    assert!(error.msg.contains("root"));
}

fn authored_span(start: u32) -> SourceSpan {
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(start, 1, &mut builder).expect("test span should fit");
    SourceSpan::new(SourceId::COMPILATION_ROOT, local)
}

/// Builds a finalized `Template` over `root` with one expression overlay replacing
/// the expression at `site_id` with `overlay_expression`.
fn finalized_template_with_site_overlay(
    store: &mut TemplateIrStore,
    root: crate::compiler_frontend::ast::templates::tir::TemplateIrId,
    site_id: ExpressionSiteId,
    overlay_expression: Expression,
) -> Template {
    let expression_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(site_id, Box::new(overlay_expression))],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    Template {
        tir_reference: TemplateTirReference {
            root,
            phase: TemplateTirPhase::Finalized,
            context,
        },
        span: None,
    }
}

fn invalid_bool_expression(value: bool, span: Option<SourceSpan>) -> Expression {
    Expression::new(
        ExpressionKind::Bool(value),
        span,
        TypeId(9999),
        DataType::Bool,
        ValueMode::ImmutableOwned,
    )
}

#[test]
fn finalized_tir_view_branch_selector_payload_validates_effective_overlay_span() {
    let type_environment = TypeEnvironment::new();
    let mut store = TemplateIrStore::new();

    let structural_span = Some(authored_span(10));
    let structural_selector = Expression::bool(true, structural_span, ValueMode::ImmutableOwned);

    let overlay_span = Some(authored_span(20));
    let overlay_selector = invalid_bool_expression(true, overlay_span);

    let (template_id, selector_site_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let branch_body = builder.push_sequence_node(Vec::new(), None);
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(structural_selector),
            branch_body,
            structural_span,
            builder.store.next_expression_site_id(),
        );
        let branch_chain_node_id = builder.push_branch_chain_node(vec![branch], None, None, None);
        let template_id = builder.finish_template(
            branch_chain_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        );
        let selector_site_id = match &store
            .get_node(branch_chain_node_id)
            .expect("branch chain node should exist")
            .kind
        {
            crate::compiler_frontend::ast::templates::tir::TemplateIrNodeKind::BranchChain {
                branches,
                ..
            } => branches[0].selector_site_id,
            other => panic!("expected branch chain node, got {other:?}"),
        };
        (template_id, selector_site_id)
    };

    let template = finalized_template_with_site_overlay(
        &mut store,
        template_id,
        selector_site_id,
        overlay_selector,
    );
    let context = TypeValidationContext {
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_template_expression_payloads(&template, &context).expect_err(
        "finalized TirView path should detect orphan TypeId on effective overlay selector",
    );

    assert_eq!(
        error.source_span, overlay_span,
        "error span must point to the effective overlay selector, not the structural selector"
    );
    assert!(error.msg.contains("9999"));
}

#[test]
fn finalized_tir_view_loop_header_payload_validates_effective_overlay_span() {
    let type_environment = TypeEnvironment::new();
    let mut store = TemplateIrStore::new();

    let structural_span = Some(authored_span(10));
    let structural_condition = Expression::bool(false, structural_span, ValueMode::ImmutableOwned);

    let overlay_span = Some(authored_span(30));
    let overlay_condition = invalid_bool_expression(false, overlay_span);

    let (template_id, condition_site_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let loop_body = builder.push_sequence_node(Vec::new(), None);
        let header = TemplateLoopHeader::Conditional {
            condition: Box::new(structural_condition),
        };
        let loop_node_id = builder.push_loop_node(header, loop_body, None, structural_span);
        let template_id = builder.finish_template(
            loop_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        );
        let condition_site_id = match &store
            .get_node(loop_node_id)
            .expect("loop node should exist")
            .kind
        {
            crate::compiler_frontend::ast::templates::tir::TemplateIrNodeKind::Loop {
                header_sites,
                ..
            } => match header_sites {
                TemplateLoopHeaderExpressionSites::Conditional { condition } => *condition,
                other => panic!("expected conditional loop header sites, got {other:?}"),
            },
            other => panic!("expected loop node, got {other:?}"),
        };
        (template_id, condition_site_id)
    };

    let template = finalized_template_with_site_overlay(
        &mut store,
        template_id,
        condition_site_id,
        overlay_condition,
    );
    let context = TypeValidationContext {
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_template_expression_payloads(&template, &context).expect_err(
        "finalized TirView path should detect orphan TypeId on effective overlay loop header",
    );

    assert_eq!(
        error.source_span, overlay_span,
        "error span must point to the effective overlay loop header, not the structural header"
    );
    assert!(error.msg.contains("9999"));
}
