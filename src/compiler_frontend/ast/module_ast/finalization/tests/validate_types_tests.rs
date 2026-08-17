//! Tests for final AST type-boundary validation of template expression payloads.

use super::*;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::{
    ExpressionSiteId, TemplateIr, TemplateIrBranch, TemplateIrBuilder, TemplateIrNode,
    TemplateIrStore, TemplateIrSummary, TemplateLoopHeaderExpressionSites, TemplateTirPhase,
    TemplateTirReference, TemplateViewContext, TirExpressionOverlay,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::compiler_messages::source_location::CharPosition;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
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
