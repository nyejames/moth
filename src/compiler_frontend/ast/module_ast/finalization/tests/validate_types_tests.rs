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
use crate::compiler_frontend::compiler_messages::source_location::CharPosition;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
fn invalid_string_expression(location: SourceLocation) -> Expression {
    Expression::new(
        ExpressionKind::Bool(true),
        location,
        TypeId(9999),
        DataType::Bool,
        ValueMode::ImmutableOwned,
    )
}

fn orphan_bool_expression() -> Expression {
    Expression::new(
        ExpressionKind::Bool(true),
        SourceLocation::default(),
        TypeId(9999),
        DataType::Bool,
        ValueMode::ImmutableOwned,
    )
}

fn owned_render_handoff(node: OwnedRuntimeTemplateNode) -> OwnedRuntimeTemplateHandoff {
    OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(node),
        location: SourceLocation::default(),
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
    let selectors = vec![
        TemplateBranchSelector::Bool(orphan_bool_expression()),
        TemplateBranchSelector::OptionPresentCapture {
            scrutinee: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
            pattern: Box::new(MatchPattern::OptionPresentCapture {
                name: strings.intern("value"),
                binding_path: InternedPath::new(),
                inner_type_id: TypeId(9999),
                location: SourceLocation::default(),
                binding_location: SourceLocation::default(),
            }),
        },
    ];

    for selector in selectors {
        validate_owned_handoff_with_orphan_type_id(OwnedRuntimeTemplateNode::BranchChain {
            branches: vec![OwnedRuntimeTemplateBranch {
                selector,
                body: OwnedRuntimeTemplateNode::Sequence {
                    children: Vec::new(),
                },
                location: SourceLocation::default(),
            }],
            fallback: None,
            location: SourceLocation::default(),
        });
    }
}

fn orphan_declaration() -> Declaration {
    Declaration {
        id: InternedPath::new(),
        value: orphan_bool_expression(),
        config_qualifier: None,
    }
}

fn empty_loop_body() -> Box<OwnedRuntimeTemplateNode> {
    Box::new(OwnedRuntimeTemplateNode::Sequence {
        children: Vec::new(),
    })
}

#[test]
fn owned_runtime_slot_handoff_validates_all_expression_payload_routes() {
    let slot_handoffs = vec![
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::DynamicExpression {
                expression: Box::new(orphan_bool_expression()),
                reactive_subscription: None,
            },
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
            location: SourceLocation::default(),
        },
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::Sequence {
                children: Vec::new(),
            },
            contribution_sources: vec![OwnedRuntimeSlotContributionSource {
                source: RuntimeSlotContributionSourceId(0),
                render_root: OwnedRuntimeTemplateNode::BranchChain {
                    branches: vec![OwnedRuntimeTemplateBranch {
                        selector: TemplateBranchSelector::Bool(orphan_bool_expression()),
                        body: OwnedRuntimeTemplateNode::Sequence {
                            children: Vec::new(),
                        },
                        location: SourceLocation::default(),
                    }],
                    fallback: None,
                    location: SourceLocation::default(),
                },
                renders_wrapper_unconditionally: false,
                location: SourceLocation::default(),
            }],
            slot_sites: Vec::new(),
            location: SourceLocation::default(),
        },
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::Sequence {
                children: Vec::new(),
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
                            start: Expression::bool(
                                true,
                                SourceLocation::default(),
                                ValueMode::ImmutableOwned,
                            ),
                            end: Expression::bool(
                                true,
                                SourceLocation::default(),
                                ValueMode::ImmutableOwned,
                            ),
                            end_kind: RangeEndKind::Exclusive,
                            step: None,
                        }),
                    },
                    body: empty_loop_body(),
                    aggregate_wrapper: None,
                    location: SourceLocation::default(),
                },
                location: SourceLocation::default(),
            }],
            location: SourceLocation::default(),
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
            },
            location: SourceLocation::default(),
        }],
        fallback: None,
        location: SourceLocation::default(),
    });
    let node = AstNode {
        kind: NodeKind::Assert {
            condition: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
            message: Expression::new(
                ExpressionKind::RuntimeTemplateHandoff(Box::new(handoff)),
                SourceLocation::default(),
                builtin_type_ids::STRING,
                DataType::Template,
                ValueMode::ImmutableOwned,
            ),
        },
        location: SourceLocation::default(),
        scope: InternedPath::new(),
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
        },
        contribution_sources: Vec::new(),
        slot_sites: Vec::new(),
        location: SourceLocation::default(),
    };
    let node = AstNode {
        kind: NodeKind::Assert {
            condition: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
            message: Expression::new(
                ExpressionKind::RuntimeSlotApplicationHandoff(Box::new(slot_handoff)),
                SourceLocation::default(),
                builtin_type_ids::STRING,
                DataType::Template,
                ValueMode::ImmutableOwned,
            ),
        },
        location: SourceLocation::default(),
        scope: InternedPath::new(),
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
                end: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
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
                start: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
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
                start: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
                end: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
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
                start: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
                end: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
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
            iterable: Box::new(Expression::bool(
                true,
                SourceLocation::default(),
                ValueMode::ImmutableOwned,
            )),
        },
    ];

    for header in headers {
        validate_owned_handoff_with_orphan_type_id(OwnedRuntimeTemplateNode::Loop {
            header,
            body: Box::new(OwnedRuntimeTemplateNode::Sequence {
                children: Vec::new(),
            }),
            aggregate_wrapper: None,
            location: SourceLocation::default(),
        });
    }
}

#[test]
fn static_true_assertion_message_reaches_type_validation_before_elision() {
    let mut strings = StringTable::new();
    let structural = Expression::string_slice(
        strings.intern("structural"),
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
    );
    let mut store = TemplateIrStore::new();
    let template = template_with_dynamic_overlay(
        &mut store,
        structural,
        invalid_string_expression(SourceLocation::default()),
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
        SourceLocation::default(),
        option_string_type_id,
        DataType::Option(Box::new(DataType::StringSlice)),
        ValueMode::ImmutableOwned,
    );
    let node = AstNode {
        kind: NodeKind::Assert {
            condition: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
            message,
        },
        location: SourceLocation::default(),
        scope: InternedPath::new(),
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
        SourceLocation::default(),
    ));
    let root = store.push_template(TemplateIr::new(
        node,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        SourceLocation::default(),
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
        location: SourceLocation::default(),
    }
}

#[test]
fn validation_checks_effective_dynamic_expression_overlay() {
    let mut strings = StringTable::new();
    let structural = Expression::string_slice(
        strings.intern("structural"),
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
    );
    let overlay = invalid_string_expression(SourceLocation::default());
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
    let structural = Expression::string_slice(
        strings.intern("structural"),
        SourceLocation::default(),
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
        location: SourceLocation::default(),
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

/// Builds a deterministic source location for test assertions.
fn location_at(line: i32, column: i32) -> SourceLocation {
    SourceLocation::new(
        InternedPath::default(),
        CharPosition {
            line_number: line,
            char_column: column,
        },
        CharPosition {
            line_number: line,
            char_column: column,
        },
    )
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
        location: SourceLocation::default(),
    }
}

fn invalid_bool_expression(value: bool, location: SourceLocation) -> Expression {
    Expression::new(
        ExpressionKind::Bool(value),
        location,
        TypeId(9999),
        DataType::Bool,
        ValueMode::ImmutableOwned,
    )
}

#[test]
fn finalized_tir_view_branch_selector_payload_validates_effective_overlay_expression_location() {
    let type_environment = TypeEnvironment::new();
    let mut store = TemplateIrStore::new();

    let structural_location = location_at(10, 5);
    let structural_selector =
        Expression::bool(true, structural_location.clone(), ValueMode::ImmutableOwned);

    let overlay_location = location_at(20, 7);
    let overlay_selector = invalid_bool_expression(true, overlay_location.clone());

    let (template_id, selector_site_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let branch_body = builder.push_sequence_node(Vec::new(), SourceLocation::default());
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(structural_selector),
            branch_body,
            structural_location,
            builder.store.next_expression_site_id(),
        );
        let branch_chain_node_id =
            builder.push_branch_chain_node(vec![branch], None, SourceLocation::default());
        let template_id = builder.finish_template(
            branch_chain_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
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
        error.location, overlay_location,
        "error location must point to the effective overlay selector, not the structural selector"
    );
    assert!(error.msg.contains("9999"));
}

#[test]
fn finalized_tir_view_loop_header_payload_validates_effective_overlay_expression_location() {
    let type_environment = TypeEnvironment::new();
    let mut store = TemplateIrStore::new();

    let structural_location = location_at(10, 5);
    let structural_condition = Expression::bool(
        false,
        structural_location.clone(),
        ValueMode::ImmutableOwned,
    );

    let overlay_location = location_at(30, 9);
    let overlay_condition = invalid_bool_expression(false, overlay_location.clone());

    let (template_id, condition_site_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let loop_body = builder.push_sequence_node(Vec::new(), SourceLocation::default());
        let header = TemplateLoopHeader::Conditional {
            condition: Box::new(structural_condition),
        };
        let loop_node_id = builder.push_loop_node(header, loop_body, None, structural_location);
        let template_id = builder.finish_template(
            loop_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
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
        error.location, overlay_location,
        "error location must point to the effective overlay loop header, not the structural header"
    );
    assert!(error.msg.contains("9999"));
}
