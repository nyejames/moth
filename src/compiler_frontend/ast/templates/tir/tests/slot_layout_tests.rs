//! Slot-layout walk invariants.
//!
//! Schema keys, placeholder occurrence order, loose-fill target and structural
//! `has_slots` come from one cycle-guarded walk. Missing authority and cycles
//! are `CompilerError`.

use super::super::ids::{TemplateIrId, TemplateIrNodeId};
use super::super::node::{TemplateIr, TemplateIrNode, TemplateIrNodeKind};
use super::super::overlays::TemplateViewContext;
use super::super::refs::TemplateTirChildReference;
use super::super::slot_layout::{collect_tir_slot_layout, collect_tir_slot_layout_from_root};
use super::super::store::{MalformedTirStore, TemplateIrStore};
use super::super::summary::TemplateIrSummary;
use super::super::view::TemplateTirPhase;
use super::builder::TemplateIrBuilder;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};

fn finish_string_template(
    builder: &mut TemplateIrBuilder<'_>,
    root: TemplateIrNodeId,
) -> super::super::ids::TemplateIrId {
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    )
}

#[test]
fn layout_rejects_a_missing_root_node() {
    let store = TemplateIrStore::new();
    let missing = TemplateIrNodeId::new(42);

    let error =
        collect_tir_slot_layout_from_root(&store, missing).expect_err("missing root must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("missing node"),
        "unexpected error: {message}"
    );
}

#[test]
fn layout_rejects_a_missing_child_template() {
    let mut store = TemplateIrStore::new();
    let reference = TemplateTirChildReference::new(
        TemplateIrId::new(99),
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let occurrence_id = store.next_child_template_occurrence_id();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        },
        None,
    ));
    let parent = store.push_template(TemplateIr::new(
        child_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    ));

    let error =
        collect_tir_slot_layout(&store, parent).expect_err("missing child template must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("missing child template"),
        "unexpected error: {message}"
    );
}

#[test]
fn layout_rejects_a_child_template_with_a_missing_root() {
    let mut store = TemplateIrStore::new();
    let missing_root = TemplateIrNodeId::new(99);
    let child_template = store.push_template(TemplateIr::new(
        missing_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    ));
    let reference = TemplateTirChildReference::new(
        child_template,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let occurrence_id = store.next_child_template_occurrence_id();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        },
        None,
    ));
    let parent = store.push_template(TemplateIr::new(
        child_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    ));

    let error = collect_tir_slot_layout(&store, parent).expect_err("missing child root must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("missing root node"),
        "unexpected error: {message}"
    );
}

#[test]
fn layout_rejects_a_node_cycle() {
    let mut store = TemplateIrStore::new();
    let sequence = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        None,
    ));
    MalformedTirStore::new(&mut store).set_node_kind(
        sequence,
        TemplateIrNodeKind::Sequence {
            children: vec![sequence],
        },
    );
    let template = store.push_template(TemplateIr::new(
        sequence,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    ));

    let error = collect_tir_slot_layout(&store, template).expect_err("node cycle must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("node cycle"),
        "unexpected error: {message}"
    );
}

#[test]
fn layout_rejects_a_template_cycle() {
    let mut store = TemplateIrStore::new();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        None,
    ));
    let first = store.push_template(TemplateIr::new(
        child_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    ));
    let reference = TemplateTirChildReference::new(
        first,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let occurrence_id = store.next_child_template_occurrence_id();
    let cycle_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        },
        None,
    ));
    MalformedTirStore::new(&mut store).set_template_root(first, cycle_node);

    let error = collect_tir_slot_layout(&store, first).expect_err("template cycle must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("template cycle"),
        "unexpected error: {message}"
    );
}

#[test]
fn layout_records_unique_keys_every_occurrence() {
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let first = builder.push_slot_node(SlotKey::Default, None);
    let second = builder.push_slot_node(SlotKey::Default, None);
    let root = builder.push_sequence_node(vec![first, second], None);
    let template_id = finish_string_template(&mut builder, root);

    let layout = collect_tir_slot_layout(&store, template_id).expect("legal layout");

    assert!(layout.schema.has_any_slots());
    assert!(layout.schema.has_default_slot);
    assert_eq!(
        layout.schema.loose_fill_target_key(),
        Some(SlotKey::Default)
    );
    assert_eq!(layout.placeholders.len(), 2);
    assert_eq!(layout.placeholders[0].key, SlotKey::Default);
    assert_eq!(layout.placeholders[1].key, SlotKey::Default);
    assert_eq!(layout.placeholders[0].span, None);
    assert_eq!(layout.placeholders[1].span, None);
    assert_ne!(
        layout.placeholders[0].occurrence_id,
        layout.placeholders[1].occurrence_id
    );
}

#[test]
fn layout_allows_shared_child_templates_as_a_dag() {
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let child_slot = builder.push_slot_node(SlotKey::Default, None);
    let child_template = finish_string_template(&mut builder, child_slot);
    let first_ref = builder.push_child_template_node(child_template, None);
    let second_ref = builder.push_child_template_node(child_template, None);
    let root = builder.push_sequence_node(vec![first_ref, second_ref], None);
    let parent = finish_string_template(&mut builder, root);

    let layout = collect_tir_slot_layout(&store, parent).expect("shared child is a DAG");

    assert!(layout.schema.has_any_slots());
    assert_eq!(
        layout.placeholders.len(),
        2,
        "each child reference is a replay occurrence"
    );
}
