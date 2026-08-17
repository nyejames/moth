use super::super::node::{TemplateIr, TemplateIrNode, TemplateIrNodeKind};
use super::super::refs::TemplateTirChildReference;
use super::super::slot_plan::push_runtime_slot_contribution_source;
use super::super::store::{MalformedTirStore, TemplateIrStore};
use super::super::summary::{TemplateIrSummary, summarize_existing_root};
use super::super::view::TemplateTirPhase;
use crate::compiler_frontend::ast::templates::template::{
    Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotContributionSourceId;
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

#[test]
fn empty_summary_has_zero_counts_and_false_flags() {
    let summary = TemplateIrSummary::empty();

    assert_eq!(summary.estimated_output_bytes, 0);
    assert_eq!(summary.text_node_count, 0);
    assert_eq!(summary.text_byte_count, 0);
    assert_eq!(summary.dynamic_expression_count, 0);
    assert_eq!(summary.child_template_count, 0);
    assert_eq!(summary.head_node_count, 0);
    assert_eq!(summary.slot_count, 0);
    assert_eq!(summary.runtime_slot_site_count, 0);
    assert_eq!(summary.insert_contribution_count, 0);
    assert_eq!(summary.wrapper_count, 0);
    assert_eq!(summary.max_depth, 0);
    assert!(!summary.has_slots());
    assert_eq!(summary.insert_contribution_count, 0);
    assert!(!summary.has_control_flow);
    assert!(!summary.has_reactivity);
}

#[test]
fn record_helpers_preserve_summary_shape_contracts() {
    let mut summary = TemplateIrSummary::empty();
    summary.record_text_node(10);
    summary.record_text_node(5);
    summary.record_dynamic_expression(false);
    summary.record_dynamic_expression(true);
    summary.record_child_template();
    summary.record_child_template();
    summary.record_control_flow();
    summary.record_runtime_slot_site();
    summary.record_insert_contribution();

    assert_eq!(summary.text_node_count, 2);
    assert_eq!(summary.text_byte_count, 15);
    assert_eq!(summary.estimated_output_bytes, 15);
    assert_eq!(summary.dynamic_expression_count, 2);
    assert_eq!(summary.child_template_count, 2);
    assert_eq!(summary.runtime_slot_site_count, 1);
    assert!(!summary.has_slots());
    assert!(summary.has_control_flow);
    assert_eq!(summary.insert_contribution_count, 1);
    assert!(summary.has_reactivity);

    let mut unresolved_slot_summary = TemplateIrSummary::empty();
    unresolved_slot_summary.record_slot();
    assert_eq!(unresolved_slot_summary.slot_count, 1);
    assert!(unresolved_slot_summary.has_slots());

    let mut contribution_summary = TemplateIrSummary::empty();
    contribution_summary.record_runtime_slot_contribution_source();
    assert_eq!(
        contribution_summary.runtime_slot_contribution_source_count,
        1
    );
}

#[test]
fn contribution_source_marker_recompute_records_the_source_count() {
    let mut store = TemplateIrStore::new();
    let plan = store.push_slot_plan(super::super::slot_plan::TemplateSlotPlan {
        location: SourceLocation::default(),
        contribution_sources: vec![],
        slot_sites: vec![],
    });
    let marker = push_runtime_slot_contribution_source(
        &mut store,
        plan,
        RuntimeSlotContributionSourceId(0),
        SourceLocation::default(),
    );
    let sequence = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![marker],
        },
        SourceLocation::default(),
    ));

    let recomputed = summarize_existing_root(&store, sequence).expect("sequence is acyclic");

    assert_eq!(recomputed.runtime_slot_contribution_source_count, 1);
    assert_eq!(recomputed.runtime_slot_site_count, 0);
}

#[test]
fn summarize_existing_root_rejects_a_node_cycle() {
    let mut store = TemplateIrStore::new();
    let sequence = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        SourceLocation::default(),
    ));
    MalformedTirStore::new(&mut store).set_node_kind(
        sequence,
        TemplateIrNodeKind::Sequence {
            children: vec![sequence],
        },
    );

    let error = summarize_existing_root(&store, sequence).expect_err("cycle must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("node cycle"),
        "unexpected error: {message}"
    );
}

#[test]
fn summarize_existing_root_rejects_a_template_cycle() {
    let mut store = TemplateIrStore::new();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        SourceLocation::default(),
    ));
    let first = store.push_template(TemplateIr::new(
        child_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
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
        SourceLocation::default(),
    ));
    MalformedTirStore::new(&mut store).set_template_root(first, cycle_node);

    let error = summarize_existing_root(&store, cycle_node).expect_err("template cycle must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("template cycle"),
        "unexpected error: {message}"
    );
}

#[test]
fn nested_sequence_summary_records_real_depth() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let text = string_table.intern("x");
    let leaf = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text,
            byte_len: 1,
            origin: TemplateSegmentOrigin::Body,
        },
        SourceLocation::default(),
    ));
    let inner = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![leaf],
        },
        SourceLocation::default(),
    ));
    let outer = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![inner],
        },
        SourceLocation::default(),
    ));

    let summary = summarize_existing_root(&store, outer).expect("acyclic");
    assert_eq!(summary.max_depth, 2);
    assert_eq!(summary.text_node_count, 1);
}

#[test]
fn summarize_existing_root_rejects_a_missing_root_node() {
    let store = TemplateIrStore::new();
    let error = summarize_existing_root(
        &store,
        crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId::new(999),
    )
    .expect_err("missing root must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("missing node"),
        "unexpected error: {message}"
    );
}

#[test]
fn summarize_existing_root_rejects_a_missing_child_template() {
    let mut store = TemplateIrStore::new();
    let occurrence_id = store.next_child_template_occurrence_id();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId::new(99),
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            ),
            occurrence_id,
        },
        SourceLocation::default(),
    ));

    let error = summarize_existing_root(&store, child_node).expect_err("missing child must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("missing child template"),
        "unexpected error: {message}"
    );
}

#[test]
fn summarize_existing_root_rejects_a_child_template_with_a_missing_root() {
    let mut store = TemplateIrStore::new();
    let missing_root =
        crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId::new(999);
    let child_template = store.push_template(TemplateIr::new(
        missing_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    ));
    let occurrence_id = store.next_child_template_occurrence_id();
    let parent_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                child_template,
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            ),
            occurrence_id,
        },
        SourceLocation::default(),
    ));

    let error =
        summarize_existing_root(&store, parent_node).expect_err("missing child root must fail");
    let message = format!("{error:?}");
    assert!(
        message.contains("missing root node"),
        "unexpected error: {message}"
    );
}

#[test]
fn formatted_markdown_root_summary_matches_recompute() {
    use super::builder::TemplateIrBuilder;
    use crate::compiler_frontend::ast::templates::styles::markdown::markdown_formatter;
    use crate::compiler_frontend::ast::templates::tir::DerivedTemplateMetadata;
    use crate::compiler_frontend::ast::templates::tir::format_tir_template;

    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let style = Style {
        formatter: Some(markdown_formatter()),
        ..Style::default()
    };
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("Hello `code`");
        let root = builder.push_text_node(
            text,
            "Hello `code`".len(),
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        builder.finish_template(
            root,
            style.clone(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        )
    };

    let formatted_root = format_tir_template(
        &mut store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
        &style,
        &mut string_table,
    )
    .expect("markdown formatter should succeed")
    .root;
    let published = store
        .push_structurally_derived_template(
            template_id,
            formatted_root,
            DerivedTemplateMetadata::preserve_source(),
        )
        .expect("formatted root should publish");
    let stored = store
        .get_template(published)
        .expect("published formatted template exists");
    let recomputed =
        summarize_existing_root(&store, stored.root).expect("formatted root is finite");

    assert_eq!(stored.summary.text_byte_count, recomputed.text_byte_count);
    assert_eq!(stored.summary.text_node_count, recomputed.text_node_count);
    assert_eq!(stored.summary.max_depth, recomputed.max_depth);
    assert!(
        stored.summary.text_byte_count > "Hello `code`".len(),
        "markdown formatting should add output bytes that the stored summary records"
    );
}

#[test]
fn nested_child_formatter_publishes_derived_version_and_updates_reference() {
    use super::builder::TemplateIrBuilder;
    use crate::compiler_frontend::ast::templates::styles::markdown::markdown_formatter;
    use crate::compiler_frontend::ast::templates::tir::format_tir_template;

    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let child_style = Style {
        formatter: Some(markdown_formatter()),
        ..Style::default()
    };
    let (parent_id, child_id, original_child_root) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("Hello `code`");
        let child_root = builder.push_text_node(
            text,
            "Hello `code`".len(),
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let child_id = builder.finish_template(
            child_root,
            child_style.clone(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        );
        let child_node = builder.push_child_template_node(child_id, SourceLocation::default());
        let parent_id = builder.finish_template(
            child_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        );
        (parent_id, child_id, child_root)
    };

    format_tir_template(
        &mut store,
        parent_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
        &Style::default(),
        &mut string_table,
    )
    .expect("parent format should write back the child");

    let parent_root = store.get_template(parent_id).expect("parent exists").root;
    let formatted_child_id = match &store
        .get_node(parent_root)
        .expect("parent child node exists")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
        other => panic!("expected a child-template root, got {other:?}"),
    };
    assert_ne!(
        formatted_child_id, child_id,
        "nested formatter writeback must publish a derived child version"
    );

    let original_child = store.get_template(child_id).expect("original child exists");
    assert_eq!(
        original_child.root, original_child_root,
        "published child templates must not be mutated by formatter writeback"
    );

    let child = store
        .get_template(formatted_child_id)
        .expect("formatted child version exists");
    assert_ne!(
        child.root, original_child_root,
        "nested formatter writeback must replace the child root"
    );
    let recomputed = summarize_existing_root(&store, child.root).expect("child root is finite");
    assert_eq!(child.summary.text_byte_count, recomputed.text_byte_count);
    assert_eq!(child.summary.text_node_count, recomputed.text_node_count);
    assert_eq!(child.summary.max_depth, recomputed.max_depth);
}
