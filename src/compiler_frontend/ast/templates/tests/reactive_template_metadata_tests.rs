//! Exact-view and owned-handoff reactive metadata traversal tests.

use super::*;
use crate::compiler_frontend::ast::expressions::expression::{ReactiveSource, ReactiveSourceKind};
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIr, TemplateIrId, TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind,
    TemplateIrStore, TemplateIrSummary, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext, TemplateWrapperReference, TemplateWrapperSet, TirExpressionOverlay,
    TirSlotPlaceholder, TirSlotResolution, TirSlotResolutionOverlay, TirView,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn location() -> SourceLocation {
    SourceLocation::default()
}

fn reactive_expression(
    string_table: &mut StringTable,
    name: &str,
) -> (Expression, ReactiveSubscription) {
    let source = ReactiveSource {
        path: InternedPath::from_single_str(name, string_table),
        kind: ReactiveSourceKind::Declaration,
    };
    let subscription = ReactiveSubscription {
        source: source.clone(),
        type_id: builtin_type_ids::INT,
        location: location(),
    };
    let expression = Expression::new(
        ExpressionKind::Reference(source.path.clone()),
        location(),
        builtin_type_ids::INT,
        DataType::Int,
        ValueMode::ImmutableReference,
    )
    .with_reactive_source(source)
    .with_reactive_template_metadata(ReactiveTemplateMetadata {
        template_backed: false,
        subscriptions: vec![subscription.clone()],
        template_value_parameters: vec![],
    });
    (expression, subscription)
}

fn template_from_node(
    store: &mut TemplateIrStore,
    node: TemplateIrNodeId,
    phase: TemplateTirPhase,
    context: TemplateViewContext,
) -> Template {
    let root = store.push_template(TemplateIr::new(
        node,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        location(),
    ));
    Template {
        tir_reference: TemplateTirReference {
            root,
            phase,
            context,
        },
        location: location(),
    }
}

fn merge(
    template: &Template,
    store: &TemplateIrStore,
) -> Result<ReactiveTemplateMetadata, CompilerError> {
    let mut metadata = ReactiveTemplateMetadata::template_backed();
    let reference = template.tir_reference;
    let view = TirView::with_minimum_phase(
        store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?;
    merge_reactive_template_metadata(&view, &mut metadata, &mut |expression| {
        Ok(expression.reactive_template.clone())
    })?;
    Ok(metadata)
}

#[test]
fn composed_view_walk_collects_dynamic_subscription_metadata() {
    let mut strings = StringTable::new();
    let (expression, subscription) = reactive_expression(&mut strings, "value");
    let mut store = TemplateIrStore::new();
    let site_id = store.next_expression_site_id();
    let node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(expression),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: Some(subscription.clone()),
            site_id,
        },
        location(),
    ));
    let template = template_from_node(
        &mut store,
        node,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    );

    let metadata = merge(&template, &store).expect("metadata walk should succeed");
    assert!(metadata.subscriptions.contains(&subscription));
}

#[test]
fn composed_view_walk_collects_text_side_table_subscription_metadata() {
    let mut strings = StringTable::new();
    let (_, subscription) = reactive_expression(&mut strings, "text-value");
    let mut store = TemplateIrStore::new();
    let text = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: strings.intern("reactive text"),
            byte_len: "reactive text".len(),
            origin: TemplateSegmentOrigin::Body,
        },
        location(),
    ));
    store
        .set_node_reactive_subscription(text, subscription.clone())
        .expect("text node should accept a subscription");
    let template = template_from_node(
        &mut store,
        text,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    );

    let metadata = merge(&template, &store).expect("text metadata walk should succeed");
    assert!(metadata.subscriptions.contains(&subscription));
}

#[test]
fn finalized_view_walk_reads_expression_overlay_metadata() {
    let mut strings = StringTable::new();
    let (structural, _) = reactive_expression(&mut strings, "structural");
    let (overlay_expression, subscription) = reactive_expression(&mut strings, "overlay");
    let mut store = TemplateIrStore::new();
    let site_id = store.next_expression_site_id();
    let node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(structural),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        location(),
    ));
    let overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(site_id, Box::new(overlay_expression))],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    let template = template_from_node(&mut store, node, TemplateTirPhase::Finalized, context);

    let mut metadata = ReactiveTemplateMetadata::template_backed();
    let view = TirView::with_minimum_phase(
        &store,
        template.tir_reference.root,
        template.tir_reference.phase,
        TemplateTirPhase::Finalized,
        template.tir_reference.context,
    )
    .expect("finalized view should be available");
    merge_reactive_template_metadata(&view, &mut metadata, &mut |expression| {
        Ok(expression.reactive_template.clone())
    })
    .expect("effective metadata walk should succeed");
    assert!(metadata.subscriptions.contains(&subscription));
}

#[test]
fn composed_view_walk_enters_parsed_structural_child() {
    let mut strings = StringTable::new();
    let (expression, subscription) = reactive_expression(&mut strings, "parsed-child");
    let mut store = TemplateIrStore::new();
    let child_site_id = store.next_expression_site_id();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(expression),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: Some(subscription.clone()),
            site_id: child_site_id,
        },
        location(),
    ));
    let child = template_from_node(
        &mut store,
        child_node,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let child_reference = TemplateTirChildReference::new(
        child.tir_reference.root,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let child_occurrence_id = store.next_child_template_occurrence_id();
    let root_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: child_reference,
            occurrence_id: child_occurrence_id,
        },
        location(),
    ));
    let root = template_from_node(
        &mut store,
        root_node,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    );

    let metadata = merge(&root, &store).expect("parsed structural child should be readable");
    assert!(metadata.subscriptions.contains(&subscription));
}

#[test]
fn resolved_slot_source_contributes_metadata_through_exact_view_context() {
    let mut strings = StringTable::new();
    let (expression, subscription) = reactive_expression(&mut strings, "slot-source");
    let mut store = TemplateIrStore::new();
    let source_site_id = store.next_expression_site_id();
    let source_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(expression),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: Some(subscription.clone()),
            site_id: source_site_id,
        },
        location(),
    ));
    let source = template_from_node(
        &mut store,
        source_node,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    );
    let occurrence_id = store.next_slot_occurrence_id();
    let slot_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot {
            placeholder: TirSlotPlaceholder::new(SlotKey::Default, occurrence_id, location()),
        },
        location(),
    ));
    let slot_resolution_overlay = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
            resolutions: vec![(
                occurrence_id,
                TirSlotResolution::resolved(SlotKey::Default, vec![source.tir_reference.root]),
            )],
        })
        .expect("test overlay allocation");
    let root = template_from_node(
        &mut store,
        slot_node,
        TemplateTirPhase::Composed,
        TemplateViewContext {
            slot_resolution: Some(slot_resolution_overlay),
            ..TemplateViewContext::default()
        },
    );

    let metadata = merge(&root, &store).expect("resolved slot source should be readable");
    assert!(metadata.subscriptions.contains(&subscription));
}

#[test]
fn non_template_coercion_is_resolved_at_the_outer_expression_boundary() {
    let mut strings = StringTable::new();
    let (inner, inner_subscription) = reactive_expression(&mut strings, "coerced-inner");
    let (_, outer_subscription) = reactive_expression(&mut strings, "coerced-outer");
    let mut coerced = Expression::coerced(inner, builtin_type_ids::FLOAT);
    coerced.reactive_template = Some(ReactiveTemplateMetadata {
        template_backed: false,
        subscriptions: vec![outer_subscription.clone()],
        template_value_parameters: vec![],
    });
    let mut store = TemplateIrStore::new();
    let site_id = store.next_expression_site_id();
    let node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(coerced),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        location(),
    ));
    let template = template_from_node(
        &mut store,
        node,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    );

    let metadata = merge(&template, &store).expect("outer coercion should be resolved");
    assert!(metadata.subscriptions.contains(&outer_subscription));
    assert!(!metadata.subscriptions.contains(&inner_subscription));
}

#[test]
fn wrapper_transition_contributes_metadata_through_exact_view() {
    let mut strings = StringTable::new();
    let (expression, subscription) = reactive_expression(&mut strings, "wrapper");
    let mut store = TemplateIrStore::new();
    let wrapper_site_id = store.next_expression_site_id();
    let wrapper_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(expression),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: Some(subscription.clone()),
            site_id: wrapper_site_id,
        },
        location(),
    ));
    let wrapper = template_from_node(
        &mut store,
        wrapper_node,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    );
    let wrapper_set = store.push_wrapper_set(TemplateWrapperSet {
        wrappers: vec![TemplateWrapperReference::new(
            wrapper.tir_reference.root,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        )],
    });
    let root_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: strings.intern("root"),
            byte_len: 4,
            origin: TemplateSegmentOrigin::Body,
        },
        location(),
    ));
    let root = store.push_template({
        let mut template = TemplateIr::new(
            root_node,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            location(),
        );
        template.conditional_child_wrapper_set = Some(wrapper_set);
        template
    });
    let root_template = Template {
        tir_reference: TemplateTirReference {
            root,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        location: location(),
    };

    let metadata = merge(&root_template, &store).expect("wrapper should be readable");
    assert!(metadata.subscriptions.contains(&subscription));
}

#[test]
fn owned_runtime_handoff_metadata_is_traversed() {
    let mut strings = StringTable::new();
    let (expression, subscription) = reactive_expression(&mut strings, "handoff");
    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::DynamicExpression {
            expression: Box::new(expression),
            reactive_subscription: Some(subscription.clone()),
        }),
        location: location(),
    };

    let metadata = metadata_for_owned_runtime_template_handoff(&handoff, &mut |expression| {
        Ok(expression.reactive_template.clone())
    })
    .expect("handoff metadata walk should succeed");
    assert!(metadata.subscriptions.contains(&subscription));
}

#[test]
fn missing_composed_root_returns_compiler_error() {
    let store = TemplateIrStore::new();
    let template = Template {
        tir_reference: TemplateTirReference {
            root: TemplateIrId::new(99),
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        location: location(),
    };

    let error = merge(&template, &store).expect_err("missing root should fail");
    assert!(error.msg.contains("does not exist"));
}

#[test]
fn exact_view_cycle_is_an_internal_reactive_metadata_error() {
    let mut store = TemplateIrStore::new();
    let template_id = TemplateIrId::new(store.template_count());
    let occurrence_id = store.next_child_template_occurrence_id();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                template_id,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id,
        },
        location(),
    ));
    store.push_template(TemplateIr::new(
        child_node,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        location(),
    ));
    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        location: location(),
    };

    let error = merge(&template, &store).expect_err("active exact-view recursion must fail");
    assert!(error.msg.contains("re-entered while still active"));
}
