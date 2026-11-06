//! Slot-layout walk invariants.
//!
//! Schema keys, placeholder occurrence order, loose-fill target and structural
//! `has_slots` come from one cycle-guarded walk. Missing authority and cycles
//! are `InfrastructureFailure`.

use super::super::ids::{TemplateIrId, TemplateIrNodeId};
use super::super::node::{TemplateIr, TemplateIrNode, TemplateIrNodeKind};
use super::super::overlays::{
    TemplateViewContext, TirWrapperApplicationMode, TirWrapperContext, TirWrapperContextOverlay,
};
use super::super::refs::{TemplateTirChildReference, TemplateWrapperReference};
use super::super::slot_layout::{
    collect_tir_slot_layout, collect_tir_slot_layout_from_root,
    collect_tir_slot_layout_with_context,
};
use super::super::store::{MalformedTirStore, TemplateIrStore};
use super::super::summary::TemplateIrSummary;
use super::super::view::TemplateTirPhase;
use super::builder::TemplateIrBuilder;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::symbols::string_interning::StringTable;

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

#[test]
fn layout_includes_named_inherited_wrapper_slots_from_exact_contexts() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let row_style = string_table.intern("row_style");
    let deep_style = string_table.intern("deep_style");

    fn empty_template(store: &mut TemplateIrStore) -> TemplateIrId {
        let root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children: vec![] },
            None,
        ));
        store.push_template(TemplateIr::new(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        ))
    }

    // A wrapper nested inside another wrapper's fixed-child overlay. Its named
    // occurrence must remain addressable through the outermost reference.
    let deep_fixed_template = empty_template(&mut store);
    let deep_named_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let named = builder.push_slot_node(SlotKey::Named(deep_style), None);
        finish_string_template(&mut builder, named)
    };
    let deep_wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        deep_named_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);
    let deep_occurrence = store.next_child_template_occurrence_id();
    let deep_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                deep_occurrence,
                TirWrapperContext {
                    inherited_wrapper_set: Some(deep_wrapper_set),
                    skip_parent_child_wrappers: false,
                    application_mode: TirWrapperApplicationMode::IfChildEmits,
                },
            )],
        })
        .expect("deep wrapper-context overlay allocation");
    let deep_context = TemplateViewContext {
        wrapper_context: Some(deep_overlay),
        ..TemplateViewContext::default()
    };
    let deep_fixed_child = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                deep_fixed_template,
                TemplateTirPhase::Composed,
                deep_context,
            ),
            occurrence_id: deep_occurrence,
        },
        None,
    ));

    // The inherited wrapper declares a named target plus default and positional
    // targets. Only the named target is addressable through the inheritance.
    let decorate_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let named = builder.push_slot_node(SlotKey::Named(row_style), None);
        let default = builder.push_slot_node(SlotKey::Default, None);
        let positional = builder.push_slot_node(SlotKey::Positional(0), None);
        let root =
            builder.push_sequence_node(vec![named, default, positional, deep_fixed_child], None);
        finish_string_template(&mut builder, root)
    };
    let inherited_wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        decorate_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);

    let prefix_fixed_template = empty_template(&mut store);
    let top_occurrence = store.next_child_template_occurrence_id();
    let top_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                top_occurrence,
                TirWrapperContext {
                    inherited_wrapper_set: Some(inherited_wrapper_set),
                    skip_parent_child_wrappers: false,
                    application_mode: TirWrapperApplicationMode::Always,
                },
            )],
        })
        .expect("top wrapper-context overlay allocation");
    let top_context = TemplateViewContext {
        wrapper_context: Some(top_overlay),
        ..TemplateViewContext::default()
    };
    let prefix_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                prefix_fixed_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id: top_occurrence,
        },
        None,
    ));
    let prefix_template = store.push_template(TemplateIr::new(
        prefix_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    ));

    let layout = collect_tir_slot_layout_with_context(&store, prefix_template, top_context)
        .expect("context-aware layout should resolve");
    assert!(layout.schema.named_slots.contains(&row_style));
    assert!(
        layout.schema.named_slots.contains(&deep_style),
        "nested fixed-child wrapper contexts must contribute their named targets"
    );
    assert!(
        !layout.schema.has_default_slot,
        "an inherited wrapper's default slot stays owned by the fixed child"
    );
    assert!(
        layout.schema.positional_slots.is_empty(),
        "an inherited wrapper's positional slots stay owned by the fixed child"
    );
    assert!(
        layout.placeholders.is_empty(),
        "inherited named occurrences are not structural placeholders"
    );

    let without_context = collect_tir_slot_layout(&store, prefix_template)
        .expect("template-only layout should resolve");
    assert!(
        without_context.schema.named_slots.is_empty(),
        "template-only layout must not invent inherited context targets"
    );

    let occurrence_id = store.next_child_template_occurrence_id();
    let nested_reference = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                prefix_template,
                TemplateTirPhase::Composed,
                top_context,
            ),
            occurrence_id,
        },
        None,
    ));
    let nested_layout = collect_tir_slot_layout_from_root(&store, nested_reference)
        .expect("nested reference layout should resolve");
    assert!(
        nested_layout.schema.named_slots.contains(&row_style)
            && nested_layout.schema.named_slots.contains(&deep_style),
        "a child reference's exact context must contribute the same named targets"
    );
    assert_eq!(
        store
            .wrapper_context_overlay(top_overlay)
            .expect("top overlay must remain present")
            .contexts[0]
            .1
            .inherited_wrapper_set,
        Some(inherited_wrapper_set),
        "layout collection must not rewrite the reference context"
    );
    let preserved_deep_context = &store
        .wrapper_context_overlay(deep_overlay)
        .expect("nested overlay must remain present")
        .contexts[0]
        .1;
    assert!(
        !preserved_deep_context.skip_parent_child_wrappers
            && preserved_deep_context.application_mode == TirWrapperApplicationMode::IfChildEmits,
        "layout collection must preserve nested context flags and application mode"
    );
}
