//! Tests for TIR slot routing, expansion and head-chain composition.
//!
//! Schema and occurrence facts are owned by `slot_layout`. These tests pin
//! routing buckets, placeholder expansion and composed child identity.

use super::super::ids::{TemplateIrId, TemplateIrNodeId, TemplateWrapperSetId};
use super::super::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder,
};
use super::super::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolutionOverlay, TirWrapperContextOverlay,
};
use super::super::refs::{TemplateTirChildReference, TemplateWrapperReference};
use super::super::slot_composition::contributions::route_tir_slot_contributions;
use super::super::slot_composition::{TirSlotContributions, compose_tir_head_chain};
use super::super::slot_layout::{TirSlotSchema, collect_tir_slot_layout, collect_tir_slot_schema};
use super::super::slot_plan::TemplateSlotPlan;
use super::super::store::{TemplateIrStore, TemplateWrapperSet};
use super::super::summary::{TemplateIrSummary, summarize_existing_root};
use super::super::view::{TemplateTirPhase, TirView};
use super::builder::TemplateIrBuilder;
use super::support::{
    expand_tir_slot_placeholders, wrap_tir_node_in_wrapper_references, wrap_tir_node_in_wrappers,
};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopControlKind, TemplateLoopHeader,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidTemplateSlotReason};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn slot_summary(slot_count: u32) -> TemplateIrSummary {
    TemplateIrSummary {
        slot_count,
        ..TemplateIrSummary::default()
    }
}

fn bool_expression(value: bool) -> Expression {
    Expression::bool(value, empty_location(), ValueMode::ImmutableOwned)
}

fn build_single_slot_template(store: &mut TemplateIrStore, key: SlotKey) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let slot_node = builder.push_slot_node(key, empty_location());
    builder.finish_template(
        slot_node,
        Style::default(),
        TemplateType::String,
        slot_summary(1),
        empty_location(),
    )
}

fn build_slot_insert_template(
    store: &mut TemplateIrStore,
    target: SlotKey,
    string_table: &mut StringTable,
) -> TemplateIrId {
    let text_id = string_table.intern("insert");
    let byte_len = string_table.resolve(text_id).len();

    let mut builder = TemplateIrBuilder::new(store);
    let root = builder.push_text_node(
        text_id,
        byte_len,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::SlotInsert(target),
        TemplateIrSummary::default(),
        empty_location(),
    )
}

fn build_text_node(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
    origin: TemplateSegmentOrigin,
) -> TemplateIrNodeId {
    let text_id = string_table.intern(text);
    let byte_len = string_table.resolve(text_id).len();

    let mut builder = TemplateIrBuilder::new(store);
    builder.push_text_node(text_id, byte_len, origin, empty_location())
}

fn build_child_template_node(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> TemplateIrNodeId {
    let text_id = string_table.intern("child");
    let byte_len = string_table.resolve(text_id).len();

    let mut builder = TemplateIrBuilder::new(store);
    let root = builder.push_text_node(
        text_id,
        byte_len,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let child_template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    builder.push_child_template_node(child_template_id, empty_location())
}

fn build_fill_template(store: &mut TemplateIrStore, nodes: Vec<TemplateIrNodeId>) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let root = builder.push_sequence_node(nodes, empty_location());

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    )
}

/// Builds a template whose root is a sequence of the given child nodes.
fn build_template_with_children(
    store: &mut TemplateIrStore,
    children: Vec<TemplateIrNodeId>,
) -> TemplateIrId {
    let child_template_count = children
        .iter()
        .filter(|&&child_id| {
            store
                .get_node(child_id)
                .is_some_and(|node| matches!(node.kind, TemplateIrNodeKind::ChildTemplate { .. }))
        })
        .count() as u32;

    let mut builder = TemplateIrBuilder::new(store);
    let root = builder.push_sequence_node(children, empty_location());

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            child_template_count,
            ..TemplateIrSummary::default()
        },
        empty_location(),
    )
}

/// Builds a `ChildTemplate` node referencing an existing template.
fn build_child_template_node_for_template(
    store: &mut TemplateIrStore,
    template_id: TemplateIrId,
) -> TemplateIrNodeId {
    let mut builder = TemplateIrBuilder::new(store);
    builder.push_child_template_node(template_id, empty_location())
}
fn build_wrapper_with_slots(store: &mut TemplateIrStore, keys: Vec<SlotKey>) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);

    let slot_nodes: Vec<TemplateIrNodeId> = keys
        .into_iter()
        .map(|key| builder.push_slot_node(key, empty_location()))
        .collect();

    let slot_count = slot_nodes.len();
    let root = builder.push_sequence_node(slot_nodes, empty_location());

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        slot_summary(u32::try_from(slot_count).unwrap_or(u32::MAX)),
        empty_location(),
    )
}

fn assert_invalid_template_slot_reason(error: &TemplateError, expected: InvalidTemplateSlotReason) {
    let TemplateError::Diagnostic(error) = error else {
        panic!("authored slot failure should remain a source diagnostic");
    };
    match &error.payload {
        DiagnosticPayload::InvalidTemplateSlot { reason, .. } => {
            assert_eq!(*reason, expected);
        }
        other => panic!("expected InvalidTemplateSlot diagnostic, got {other:?}"),
    }
}

fn assert_internal_authority_error(error: &TemplateError, expected_message: &str) {
    let TemplateError::Infrastructure(error) = error else {
        panic!("malformed composition authority should remain an infrastructure error");
    };

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains(expected_message),
        "expected infrastructure message containing {expected_message:?}, got {:?}",
        error.msg
    );
}

// -------------------------
//  Expansion Test Helpers
// -------------------------

/// Returns the root node ID of a template.
fn template_root_node_id(template_id: TemplateIrId, store: &TemplateIrStore) -> TemplateIrNodeId {
    store
        .get_template(template_id)
        .expect("template should exist in store")
        .root
}

/// Returns the text content of a single-text template's root node.
fn template_root_text(
    template_id: TemplateIrId,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> Option<String> {
    text_node_text(
        template_root_node_id(template_id, store),
        store,
        string_table,
    )
}

/// Returns the direct children of a template's root sequence.
fn root_child_node_ids(
    template_id: TemplateIrId,
    store: &TemplateIrStore,
) -> Vec<TemplateIrNodeId> {
    let template_ir = store
        .get_template(template_id)
        .expect("template should exist in store");
    let root = store
        .get_node(template_ir.root)
        .expect("template should have a root node");

    match &root.kind {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected sequence root, found {other:?}"),
    }
}

/// Returns references to the kinds of the root sequence children.
fn root_child_kinds(
    template_id: TemplateIrId,
    store: &TemplateIrStore,
) -> Vec<&TemplateIrNodeKind> {
    root_child_node_ids(template_id, store)
        .into_iter()
        .map(|node_id| {
            &store
                .get_node(node_id)
                .expect("root child node should exist")
                .kind
        })
        .collect()
}

/// Returns the text content of a Text node, if the node is a Text node.
fn text_node_text(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> Option<String> {
    let node = store.get_node(node_id)?;

    match &node.kind {
        TemplateIrNodeKind::Text { text, .. } => Some(string_table.resolve(*text).to_owned()),
        _ => None,
    }
}

/// Builds a wrapper template containing a sequence of the given slot keys.
fn build_wrapper_with_slot_sequence(
    store: &mut TemplateIrStore,
    keys: Vec<SlotKey>,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);

    let slot_nodes: Vec<TemplateIrNodeId> = keys
        .into_iter()
        .map(|key| builder.push_slot_node(key, empty_location()))
        .collect();

    let slot_count = slot_nodes.len();
    let root = builder.push_sequence_node(slot_nodes, empty_location());

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        slot_summary(u32::try_from(slot_count).unwrap_or(u32::MAX)),
        empty_location(),
    )
}

/// Builds a wrapper template with alternating Text and Slot children.
fn build_text_slot_text_wrapper(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    key: SlotKey,
) -> TemplateIrId {
    build_text_slot_text_wrapper_with_markers(store, string_table, key, "before", "after")
}

/// Builds a wrapper template with alternating Text and Slot children, using
/// caller-supplied marker text so distinct wrappers can be told apart.
///
/// WHY: identical wrappers cannot prove the innermost-to-outermost wrapping
///      order; distinct before/after markers expose which layer is innermost.
fn build_text_slot_text_wrapper_with_markers(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    key: SlotKey,
    before: &str,
    after: &str,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);

    let before_text_id = string_table.intern(before);
    let before_text_len = string_table.resolve(before_text_id).len();
    let before_text = builder.push_text_node(
        before_text_id,
        before_text_len,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );

    let slot_node = builder.push_slot_node(key, empty_location());

    let after_text_id = string_table.intern(after);
    let after_text_len = string_table.resolve(after_text_id).len();
    let after_text = builder.push_text_node(
        after_text_id,
        after_text_len,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );

    let root =
        builder.push_sequence_node(vec![before_text, slot_node, after_text], empty_location());

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        slot_summary(1),
        empty_location(),
    )
}

/// Builds a TIR template whose root is a single Text node.
fn build_single_text_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let text_id = string_table.intern(text);
    let text_len = string_table.resolve(text_id).len();
    let root = builder.push_text_node(
        text_id,
        text_len,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    )
}

#[test]
fn schema_from_default_slot_only() {
    let mut store = TemplateIrStore::new();
    let template_id = build_single_slot_template(&mut store, SlotKey::Default);

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    assert!(schema.has_default_slot);
    assert!(schema.named_slots.is_empty());
    assert!(schema.positional_slots.is_empty());
    assert!(schema.has_any_slots());
}

#[test]
fn schema_from_named_slots() {
    let mut string_table = StringTable::new();
    let name_alpha = string_table.intern("alpha");
    let name_beta = string_table.intern("beta");

    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let alpha_slot = builder.push_slot_node(SlotKey::Named(name_alpha), empty_location());
    let beta_slot = builder.push_slot_node(SlotKey::Named(name_beta), empty_location());
    let root = builder.push_sequence_node(vec![alpha_slot, beta_slot], empty_location());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    assert!(!schema.has_default_slot);
    assert_eq!(
        schema.named_slots,
        [name_alpha, name_beta].into_iter().collect()
    );
    assert!(schema.positional_slots.is_empty());
    assert!(schema.has_any_slots());
}

#[test]
fn schema_from_positional_slots() {
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let slot_0 = builder.push_slot_node(SlotKey::Positional(0), empty_location());
    let slot_1 = builder.push_slot_node(SlotKey::Positional(1), empty_location());
    let slot_2 = builder.push_slot_node(SlotKey::Positional(2), empty_location());
    let root = builder.push_sequence_node(vec![slot_0, slot_1, slot_2], empty_location());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    assert!(!schema.has_default_slot);
    assert!(schema.named_slots.is_empty());
    assert_eq!(schema.positional_slots, [0, 1, 2].into_iter().collect());
    assert!(schema.has_any_slots());
}

#[test]
fn schema_from_mixed_slot_types() {
    let mut string_table = StringTable::new();
    let name_id = string_table.intern("title");

    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let default_slot = builder.push_slot_node(SlotKey::Default, empty_location());
    let named_slot = builder.push_slot_node(SlotKey::Named(name_id), empty_location());
    let positional_slot = builder.push_slot_node(SlotKey::Positional(0), empty_location());
    let root = builder.push_sequence_node(
        vec![default_slot, named_slot, positional_slot],
        empty_location(),
    );
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    assert!(schema.has_default_slot);
    assert_eq!(schema.named_slots, [name_id].into_iter().collect());
    assert_eq!(schema.positional_slots, [0].into_iter().collect());
    assert!(schema.has_any_slots());
}

#[test]
fn schema_from_nested_child_template_containing_slot() {
    let mut string_table = StringTable::new();
    let name_id = string_table.intern("child_slot");

    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let child_slot = builder.push_slot_node(SlotKey::Named(name_id), empty_location());
    let child_template_id = builder.finish_template(
        child_slot,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let child_reference = builder.push_child_template_node(child_template_id, empty_location());
    let root = builder.push_sequence_node(vec![child_reference], empty_location());
    let parent_template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let schema = collect_tir_slot_schema(&store, parent_template_id)
        .expect("schema extraction should succeed");

    assert!(!schema.has_default_slot);
    assert_eq!(schema.named_slots, [name_id].into_iter().collect());
    assert!(schema.positional_slots.is_empty());
    assert!(schema.has_any_slots());
}

#[test]
fn schema_from_branch_chain_containing_slot() {
    let mut string_table = StringTable::new();
    let name_id = string_table.intern("branch_slot");

    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let branch_body_slot = builder.push_slot_node(SlotKey::Named(name_id), empty_location());
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(bool_expression(true)),
        branch_body_slot,
        empty_location(),
        builder.store.next_expression_site_id(),
    );
    let branch_chain = builder.push_branch_chain_node(vec![branch], None, empty_location());
    let template_id = builder.finish_template(
        branch_chain,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    assert!(!schema.has_default_slot);
    assert_eq!(schema.named_slots, [name_id].into_iter().collect());
    assert!(schema.positional_slots.is_empty());
    assert!(schema.has_any_slots());
}

#[test]
fn schema_from_loop_containing_slot() {
    let mut string_table = StringTable::new();
    let name_id = string_table.intern("loop_slot");

    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let body_slot = builder.push_slot_node(SlotKey::Named(name_id), empty_location());
    let loop_node = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(bool_expression(true)),
        },
        body_slot,
        None,
        empty_location(),
    );
    let template_id = builder.finish_template(
        loop_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    assert!(!schema.has_default_slot);
    assert_eq!(schema.named_slots, [name_id].into_iter().collect());
    assert!(schema.positional_slots.is_empty());
    assert!(schema.has_any_slots());
}

#[test]
fn loose_fill_target_prefers_later_positional_slot_across_branches() {
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let default_slot = builder.push_slot_node(SlotKey::Default, empty_location());
    let positional_slot = builder.push_slot_node(SlotKey::Positional(2), empty_location());
    let branches = vec![
        TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(true)),
            default_slot,
            empty_location(),
            builder.store.next_expression_site_id(),
        ),
        TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(false)),
            positional_slot,
            empty_location(),
            builder.store.next_expression_site_id(),
        ),
    ];
    let root = builder.push_branch_chain_node(branches, None, empty_location());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    assert!(schema.has_default_slot);
    assert!(schema.named_slots.is_empty());
    assert_eq!(schema.positional_slots, [2].into_iter().collect());
    assert!(schema.has_any_slots());

    assert_eq!(
        schema.loose_fill_target_key(),
        Some(SlotKey::Positional(2)),
        "positional loose fill should win even when default appears first in branch order"
    );
}

#[test]
fn loose_fill_target_prefers_aggregate_positional_slot_after_loop_body_default() {
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let body_default_slot = builder.push_slot_node(SlotKey::Default, empty_location());
    let aggregate_positional_slot =
        builder.push_slot_node(SlotKey::Positional(1), empty_location());
    let aggregate_wrapper =
        builder.push_sequence_node(vec![aggregate_positional_slot], empty_location());
    let root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(bool_expression(true)),
        },
        body_default_slot,
        Some(aggregate_wrapper),
        empty_location(),
    );
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    assert!(schema.has_default_slot);
    assert!(schema.named_slots.is_empty());
    assert_eq!(schema.positional_slots, [1].into_iter().collect());
    assert!(schema.has_any_slots());

    assert_eq!(
        schema.loose_fill_target_key(),
        Some(SlotKey::Positional(1)),
        "aggregate-wrapper slots must participate in positional loose-fill selection"
    );
}

#[test]
fn repeated_default_slots_are_valid_replay_sites() {
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let first_default = builder.push_slot_node(SlotKey::Default, empty_location());
    let second_default = builder.push_slot_node(SlotKey::Default, empty_location());
    let root = builder.push_sequence_node(vec![first_default, second_default], empty_location());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("repeated default slots are valid");
    assert!(schema.has_default_slot);
    assert_eq!(
        schema.ordered_slot_keys(&StringTable::new()),
        vec![SlotKey::Default]
    );

    let placeholders = collect_tir_slot_layout(&store, template_id)
        .expect("placeholder layout should list every occurrence")
        .placeholders;
    assert_eq!(placeholders.len(), 2);
    assert!(
        placeholders
            .iter()
            .all(|placeholder| placeholder.key == SlotKey::Default)
    );
}

#[test]
fn ordered_slot_keys_returns_deterministic_order() {
    let mut string_table = StringTable::new();
    let name_z = string_table.intern("zulu");
    let name_a = string_table.intern("alpha");

    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);

    let named_z = builder.push_slot_node(SlotKey::Named(name_z), empty_location());
    let positional_2 = builder.push_slot_node(SlotKey::Positional(2), empty_location());
    let default_slot = builder.push_slot_node(SlotKey::Default, empty_location());
    let named_a = builder.push_slot_node(SlotKey::Named(name_a), empty_location());
    let positional_0 = builder.push_slot_node(SlotKey::Positional(0), empty_location());
    let root = builder.push_sequence_node(
        vec![named_z, positional_2, default_slot, named_a, positional_0],
        empty_location(),
    );
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    let ordered = schema.ordered_slot_keys(&string_table);
    assert_eq!(
        ordered,
        vec![
            SlotKey::Default,
            SlotKey::Positional(0),
            SlotKey::Positional(2),
            SlotKey::Named(name_a),
            SlotKey::Named(name_z),
        ]
    );
}

#[test]
fn accepts_target_validates_correctly() {
    let mut string_table = StringTable::new();
    let known_name = string_table.intern("known");

    let mut schema = TirSlotSchema {
        has_default_slot: true,
        ..TirSlotSchema::default()
    };
    schema.named_slots.insert(known_name);
    schema.positional_slots.insert(1);

    assert!(schema.accepts_target(&SlotKey::Default));
    assert!(schema.accepts_target(&SlotKey::Named(known_name)));
    assert!(schema.accepts_target(&SlotKey::Positional(1)));

    let unknown_name = string_table.intern("unknown");
    assert!(!schema.accepts_target(&SlotKey::Named(unknown_name)));
    assert!(!schema.accepts_target(&SlotKey::Positional(0)));

    let no_default_schema = TirSlotSchema::default();
    assert!(!no_default_schema.accepts_target(&SlotKey::Default));
}

#[test]
fn skipped_node_kinds_do_not_contribute_to_schema() {
    let mut string_table = StringTable::new();
    let text_id = string_table.intern("literal");

    let mut store = TemplateIrStore::new();

    // Push the aggregate node directly via the store so the builder borrow is
    // not active while mutating the store through a second path.
    let aggregate_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::AggregateOutput,
        empty_location(),
    ));

    let mut builder = TemplateIrBuilder::new(&mut store);

    let text_len = string_table.resolve(text_id).len();
    let text_node = builder.push_text_node(
        text_id,
        text_len,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let expression = Expression::string_slice(text_id, empty_location(), ValueMode::ImmutableOwned);
    let dynamic_node = builder.push_dynamic_expression_node(
        expression,
        TemplateSegmentOrigin::Body,
        None,
        empty_location(),
    );
    let loop_control_node =
        builder.push_loop_control_node(TemplateLoopControlKind::Break, empty_location());

    let root = builder.push_sequence_node(
        vec![text_node, dynamic_node, aggregate_node, loop_control_node],
        empty_location(),
    );
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let schema =
        collect_tir_slot_schema(&store, template_id).expect("schema extraction should succeed");

    assert!(!schema.has_default_slot);
    assert!(schema.named_slots.is_empty());
    assert!(schema.positional_slots.is_empty());
    assert!(!schema.has_any_slots());
}

// -------------------------
//  Routing Tests
// -------------------------

/// Exercises every explicit-insert target branch and its exact routed bucket.
#[test]
fn route_explicit_insert_to_each_target_slot_key() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("title");
    let mut store = TemplateIrStore::new();

    // Named target: the body node fills the named bucket and leaves the
    // default and positional buckets untouched.
    {
        let wrapper = build_single_slot_template(&mut store, SlotKey::Named(name));
        let insert_template =
            build_slot_insert_template(&mut store, SlotKey::Named(name), &mut string_table);
        let insert_node = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_insert_contribution_node(insert_template, empty_location())
        };
        let fill = build_fill_template(&mut store, vec![insert_node]);

        let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
            .expect("named-target routing should succeed");

        let insert_body_node = template_root_node_id(insert_template, &store);
        assert_eq!(
            routed.nodes_for_slot(&SlotKey::Named(name)),
            &[insert_body_node]
        );
        assert!(
            routed.nodes_for_slot(&SlotKey::Default).is_empty(),
            "named insert must not spill into the default bucket"
        );
        assert!(
            routed.nodes_for_slot(&SlotKey::Positional(0)).is_empty(),
            "named insert must not spill into a positional bucket"
        );
        assert_eq!(
            collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .named_slots,
            [name].into_iter().collect()
        );
        assert!(
            collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .positional_slots
                .is_empty()
        );
        assert!(
            !collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .accepts_target(&SlotKey::Default)
        );
        assert!(
            !collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .has_default_slot
        );
    }

    // Default target: the body node fills the default bucket and leaves the
    // named and positional buckets untouched.
    {
        let wrapper = build_single_slot_template(&mut store, SlotKey::Default);
        let insert_template =
            build_slot_insert_template(&mut store, SlotKey::Default, &mut string_table);
        let insert_node = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_insert_contribution_node(insert_template, empty_location())
        };
        let fill = build_fill_template(&mut store, vec![insert_node]);

        let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
            .expect("default-target routing should succeed");

        let insert_body_node = template_root_node_id(insert_template, &store);
        assert_eq!(
            routed.nodes_for_slot(&SlotKey::Default),
            &[insert_body_node]
        );
        assert!(
            routed.named_nodes.is_empty(),
            "default insert must not spill into a named bucket"
        );
        assert!(
            routed.positional_nodes.is_empty(),
            "default insert must not spill into a positional bucket"
        );
        assert!(
            collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .has_default_slot
        );
        assert!(
            collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .named_slots
                .is_empty()
        );
        assert!(
            collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .positional_slots
                .is_empty()
        );
    }

    // Positional target: the body node fills positional slot 0 and leaves the
    // default bucket untouched. Positional slot 1 is not declared by this
    // wrapper, so it rejects that target.
    {
        let wrapper = build_single_slot_template(&mut store, SlotKey::Positional(0));
        let insert_template =
            build_slot_insert_template(&mut store, SlotKey::Positional(0), &mut string_table);
        let insert_node = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_insert_contribution_node(insert_template, empty_location())
        };
        let fill = build_fill_template(&mut store, vec![insert_node]);

        let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
            .expect("positional-target routing should succeed");

        let insert_body_node = template_root_node_id(insert_template, &store);
        assert_eq!(
            routed.nodes_for_slot(&SlotKey::Positional(0)),
            &[insert_body_node]
        );
        assert!(
            routed.nodes_for_slot(&SlotKey::Default).is_empty(),
            "positional insert must not spill into the default bucket"
        );
        assert!(
            !collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .has_default_slot
        );
        assert!(
            collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .named_slots
                .is_empty()
        );
        assert_eq!(
            collect_tir_slot_schema(&store, wrapper)
                .expect("wrapper schema")
                .positional_slots,
            [0].into_iter().collect()
        );
    }
}

#[test]
fn route_missing_loose_node_produces_internal_error() {
    let string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let wrapper = build_single_slot_template(&mut store, SlotKey::Default);
    let missing_node = TemplateIrNodeId::new(store.node_count() + 100);
    let fill = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_sequence_node(vec![missing_node], empty_location());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };

    let error = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect_err("missing loose nodes must not be treated as ordinary content");

    assert_internal_authority_error(&error, "fill template child node ID");
}

#[test]
fn route_missing_insert_body_node_produces_internal_error() {
    let string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let wrapper = build_single_slot_template(&mut store, SlotKey::Default);
    let missing_node = TemplateIrNodeId::new(store.node_count() + 100);
    let insert_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_sequence_node(vec![missing_node], empty_location());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    let insert_node = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_insert_contribution_node(insert_template, empty_location())
    };
    let fill = build_fill_template(&mut store, vec![insert_node]);

    let error = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect_err("missing insert body nodes must fail routing");

    assert_internal_authority_error(&error, "insert contribution child node ID");
}

#[test]
fn route_loose_content_to_positional_slots() {
    let mut string_table = StringTable::new();

    let mut store = TemplateIrStore::new();
    let wrapper = build_wrapper_with_slots(
        &mut store,
        vec![SlotKey::Positional(0), SlotKey::Positional(1)],
    );

    let first_child = build_child_template_node(&mut store, &mut string_table);
    let second_child = build_child_template_node(&mut store, &mut string_table);
    let fill = build_fill_template(&mut store, vec![first_child, second_child]);

    let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect("routing should succeed");

    // Two authored child contributions fill two positional slots in authored
    // order. The default and named buckets stay empty because the wrapper
    // declares only positional slots.
    assert_eq!(
        routed.nodes_for_slot(&SlotKey::Positional(0)),
        &[first_child]
    );
    assert_eq!(
        routed.nodes_for_slot(&SlotKey::Positional(1)),
        &[second_child]
    );
    assert!(
        routed.nodes_for_slot(&SlotKey::Default).is_empty(),
        "a positional-only wrapper has no default bucket"
    );
    assert!(routed.named_nodes.is_empty());
    assert!(
        !collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .has_default_slot
    );
    assert_eq!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .positional_slots,
        [0, 1].into_iter().collect()
    );
}

#[test]
fn route_loose_content_to_default_slot_after_positional_exhaustion() {
    let mut string_table = StringTable::new();

    let mut store = TemplateIrStore::new();
    let wrapper =
        build_wrapper_with_slots(&mut store, vec![SlotKey::Positional(0), SlotKey::Default]);

    let leading_text = build_text_node(
        &mut store,
        &mut string_table,
        "before",
        TemplateSegmentOrigin::Body,
    );
    let child = build_child_template_node(&mut store, &mut string_table);
    let trailing_text = build_text_node(
        &mut store,
        &mut string_table,
        "after",
        TemplateSegmentOrigin::Body,
    );
    let fill = build_fill_template(&mut store, vec![leading_text, child, trailing_text]);

    let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect("routing should succeed");

    // Positional slots fill first in authored order; once they are exhausted
    // the remaining chunks flow into the default slot, preserving order.
    assert_eq!(
        routed.nodes_for_slot(&SlotKey::Positional(0)),
        &[leading_text]
    );
    assert_eq!(
        routed.nodes_for_slot(&SlotKey::Default),
        &[child, trailing_text]
    );
    assert!(routed.named_nodes.is_empty());
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .has_default_slot
    );
    assert_eq!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .positional_slots,
        [0].into_iter().collect()
    );
}

#[test]
fn route_loose_whitespace_after_head_fill_to_next_child_contribution() {
    let mut string_table = StringTable::new();

    let mut store = TemplateIrStore::new();
    let wrapper =
        build_wrapper_with_slots(&mut store, vec![SlotKey::Positional(0), SlotKey::Default]);

    let head_fill = build_text_node(
        &mut store,
        &mut string_table,
        "head fill",
        TemplateSegmentOrigin::Head,
    );
    let separator = build_text_node(
        &mut store,
        &mut string_table,
        " ",
        TemplateSegmentOrigin::Body,
    );
    let child = build_child_template_node(&mut store, &mut string_table);
    let fill = build_fill_template(&mut store, vec![head_fill, separator, child]);

    let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect("routing should succeed");

    // A head-origin contribution opens a new positional chunk, so a following
    // body whitespace separator is carried with the next body contribution and
    // never absorbed as trailing positional content.
    assert_eq!(
        routed.nodes_for_slot(&SlotKey::Positional(0)),
        &[head_fill],
        "whitespace after a head contribution must not become trailing positional content"
    );
    assert_eq!(
        routed.nodes_for_slot(&SlotKey::Default),
        &[separator, child],
        "the separator before the body child belongs to the body contribution"
    );
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .has_default_slot
    );
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .named_slots
            .is_empty()
    );
    assert_eq!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .positional_slots,
        [0].into_iter().collect()
    );
}

#[test]
fn route_loose_content_to_default_slot_only() {
    let mut string_table = StringTable::new();

    let mut store = TemplateIrStore::new();
    let wrapper = build_single_slot_template(&mut store, SlotKey::Default);

    let text = build_text_node(
        &mut store,
        &mut string_table,
        "body",
        TemplateSegmentOrigin::Body,
    );
    let child = build_child_template_node(&mut store, &mut string_table);
    let fill = build_fill_template(&mut store, vec![text, child]);

    let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect("routing should succeed");

    // With only a default slot, every authored loose chunk flows straight into
    // the default bucket in authored order.
    assert_eq!(routed.nodes_for_slot(&SlotKey::Default), &[text, child]);
    assert!(
        routed.positional_nodes.is_empty(),
        "a default-only wrapper has no positional buckets"
    );
    assert!(routed.named_nodes.is_empty());
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .has_default_slot
    );
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .positional_slots
            .is_empty()
    );
}

#[test]
fn unknown_insert_target_produces_diagnostic() {
    let mut string_table = StringTable::new();
    let unknown_name = string_table.intern("unknown");

    let mut store = TemplateIrStore::new();
    let wrapper = build_single_slot_template(&mut store, SlotKey::Default);
    let insert_template =
        build_slot_insert_template(&mut store, SlotKey::Named(unknown_name), &mut string_table);

    let mut builder = TemplateIrBuilder::new(&mut store);
    let insert_node = builder.push_insert_contribution_node(insert_template, empty_location());
    let fill = build_fill_template(&mut store, vec![insert_node]);

    // The wrapper has a valid default slot, so only the unknown insert target
    // can own this rejection.
    let error = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect_err("unknown insert target should produce an error");

    assert_invalid_template_slot_reason(
        &error,
        InvalidTemplateSlotReason::InsertTargetsUnknownNamedSlot,
    );
}

#[test]
fn loose_content_without_default_or_positional_slots_produces_diagnostic() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("only_named");

    let mut store = TemplateIrStore::new();
    let wrapper = build_single_slot_template(&mut store, SlotKey::Named(name));

    let text = build_text_node(
        &mut store,
        &mut string_table,
        "loose",
        TemplateSegmentOrigin::Body,
    );
    let fill = build_fill_template(&mut store, vec![text]);

    // Meaningful loose content with no default or positional target is a
    // diagnostic rather than discardable formatting whitespace.
    let error = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect_err("loose content with no default slot should produce an error");

    assert_invalid_template_slot_reason(
        &error,
        InvalidTemplateSlotReason::LooseContentWithoutDefaultSlot,
    );
}

#[test]
fn named_only_slots_discard_loose_whitespace_around_insert_contributions() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("title");

    let mut store = TemplateIrStore::new();
    let wrapper = build_single_slot_template(&mut store, SlotKey::Named(name));
    let insert_template =
        build_slot_insert_template(&mut store, SlotKey::Named(name), &mut string_table);

    let leading_whitespace = build_text_node(
        &mut store,
        &mut string_table,
        "\n    ",
        TemplateSegmentOrigin::Body,
    );
    let trailing_whitespace = build_text_node(
        &mut store,
        &mut string_table,
        "\n",
        TemplateSegmentOrigin::Body,
    );
    let mut builder = TemplateIrBuilder::new(&mut store);
    let insert_node = builder.push_insert_contribution_node(insert_template, empty_location());
    let fill = build_fill_template(
        &mut store,
        vec![leading_whitespace, insert_node, trailing_whitespace],
    );

    let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect("formatting whitespace should not require a default slot");

    // A named-only wrapper discards formatting whitespace around an explicit
    // insert and routes only the insert body, without needing a default slot.
    let insert_body_node = template_root_node_id(insert_template, &store);
    assert_eq!(
        routed.nodes_for_slot(&SlotKey::Named(name)),
        &[insert_body_node]
    );
    assert!(
        routed.nodes_for_slot(&SlotKey::Default).is_empty(),
        "the discarded whitespace must not create a default bucket"
    );
    assert!(
        routed.nodes_for_slot(&SlotKey::Positional(0)).is_empty(),
        "the discarded whitespace must not create a positional bucket"
    );
    assert!(
        !collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .has_default_slot
    );
    assert_eq!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .named_slots,
        [name].into_iter().collect()
    );
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .positional_slots
            .is_empty()
    );
}

#[test]
fn extra_loose_content_beyond_positional_capacity_produces_diagnostic() {
    let mut string_table = StringTable::new();

    let mut store = TemplateIrStore::new();
    let wrapper = build_single_slot_template(&mut store, SlotKey::Positional(0));

    let first_child = build_child_template_node(&mut store, &mut string_table);
    let second_child = build_child_template_node(&mut store, &mut string_table);
    let fill = build_fill_template(&mut store, vec![first_child, second_child]);

    // One positional slot accepts the first chunk, leaving the second chunk
    // without a target or default fallback.
    let error = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect_err("extra loose content should produce an error");

    assert_invalid_template_slot_reason(
        &error,
        InvalidTemplateSlotReason::ExtraLooseContentWithoutDefaultSlot,
    );
}

#[test]
fn empty_fill_template_with_slots_wrapper_routes_to_empty_contributions() {
    let string_table = StringTable::new();

    let mut store = TemplateIrStore::new();
    let wrapper = build_single_slot_template(&mut store, SlotKey::Default);
    let fill = build_fill_template(&mut store, vec![]);

    let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect("routing should succeed");

    // An empty fill against a default-slot wrapper discovers the slot schema
    // and routes nothing into every bucket.
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .has_default_slot
    );
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .named_slots
            .is_empty()
    );
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .positional_slots
            .is_empty()
    );
    assert!(routed.nodes_for_slot(&SlotKey::Default).is_empty());
    assert!(routed.named_nodes.is_empty());
    assert!(routed.positional_nodes.is_empty());
}

#[test]
fn mixed_explicit_inserts_and_loose_content_are_bucketed() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("title");

    let mut store = TemplateIrStore::new();
    let wrapper =
        build_wrapper_with_slots(&mut store, vec![SlotKey::Named(name), SlotKey::Default]);

    let insert_template =
        build_slot_insert_template(&mut store, SlotKey::Named(name), &mut string_table);

    let mut builder = TemplateIrBuilder::new(&mut store);
    let insert_node = builder.push_insert_contribution_node(insert_template, empty_location());
    let loose_text = build_text_node(
        &mut store,
        &mut string_table,
        "body",
        TemplateSegmentOrigin::Body,
    );
    let loose_child = build_child_template_node(&mut store, &mut string_table);
    let fill = build_fill_template(&mut store, vec![insert_node, loose_text, loose_child]);

    // Explicit inserts and loose content in one fill are routed to disjoint
    // buckets: the insert body fills its named target, and authored loose
    // content fills the default slot in authored order.
    let routed = route_tir_slot_contributions(&store, wrapper, fill, &string_table)
        .expect("routing should succeed");

    let insert_body_node = template_root_node_id(insert_template, &store);
    assert_eq!(
        routed.nodes_for_slot(&SlotKey::Named(name)),
        &[insert_body_node]
    );
    assert_eq!(
        routed.nodes_for_slot(&SlotKey::Default),
        &[loose_text, loose_child]
    );
    assert!(
        routed.positional_nodes.is_empty(),
        "a named+default wrapper has no positional buckets"
    );
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .has_default_slot
    );
    assert_eq!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .named_slots,
        [name].into_iter().collect()
    );
    assert!(
        collect_tir_slot_schema(&store, wrapper)
            .expect("wrapper schema")
            .positional_slots
            .is_empty()
    );
}

// -------------------------
//  Expansion Tests
// -------------------------

/// Exercises all three slot-key replacement branches in declaration order.
#[test]
fn expand_routes_each_slot_key_branch_to_its_contribution() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("title");
    let mut store = TemplateIrStore::new();

    let wrapper = build_wrapper_with_slot_sequence(
        &mut store,
        vec![
            SlotKey::Default,
            SlotKey::Named(name),
            SlotKey::Positional(0),
        ],
    );

    let default_contribution = build_single_text_template(&mut store, &mut string_table, "filled");
    let named_contribution = build_single_text_template(&mut store, &mut string_table, "heading");
    let positional_contribution =
        build_single_text_template(&mut store, &mut string_table, "first");
    let default_node = template_root_node_id(default_contribution, &store);
    let named_node = template_root_node_id(named_contribution, &store);
    let positional_node = template_root_node_id(positional_contribution, &store);

    let routed = TirSlotContributions {
        default_nodes: vec![default_node],
        named_nodes: [(name, vec![named_node])].into_iter().collect(),
        positional_nodes: [(0, vec![positional_node])].into_iter().collect(),
    };

    let expanded_root = expand_tir_slot_placeholders(&mut store, wrapper, &routed, &string_table)
        .expect("expansion should succeed");

    let children = root_child_node_ids_for_node(expanded_root, &store);
    assert_eq!(
        children,
        vec![default_node, named_node, positional_node],
        "each slot must splice only its exact contribution in declaration order"
    );
}

#[test]
fn missing_slot_renders_as_empty() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("missing");
    let mut store = TemplateIrStore::new();

    let wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Named(name)]);

    let routed = TirSlotContributions::default();

    let expanded_root = expand_tir_slot_placeholders(&mut store, wrapper, &routed, &string_table)
        .expect("expansion should succeed");

    // A named slot with no routed contributions expands to a fresh empty
    // Sequence node, not the original placeholder and not a dropped child.
    match &store
        .get_node(expanded_root)
        .expect("expanded root should exist")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => {
            assert!(
                children.is_empty(),
                "missing slot should expand to an empty Sequence node"
            );
        }
        other => panic!("expected empty Sequence root for a missing slot, found {other:?}"),
    }
}

#[test]
fn expansion_missing_wrapper_set_produces_internal_error() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let placeholder = TirSlotPlaceholder::with_wrapper_sets(
        SlotKey::Default,
        store.next_slot_occurrence_id(),
        empty_location(),
        None,
        Some(TemplateWrapperSetId::new(0)),
        false,
    );
    let wrapper = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot_node = builder.push_tir_slot_placeholder_node(placeholder);
        builder.finish_template(
            slot_node,
            Style::default(),
            TemplateType::String,
            slot_summary(1),
            empty_location(),
        )
    };
    let child_template = build_single_text_template(&mut store, &mut string_table, "child");
    let child_node = build_child_template_node_for_template(&mut store, child_template);
    let routed = TirSlotContributions {
        default_nodes: vec![child_node],
        ..TirSlotContributions::default()
    };

    let error = expand_tir_slot_placeholders(&mut store, wrapper, &routed, &string_table)
        .expect_err("missing wrapper sets must fail slot expansion");

    assert_internal_authority_error(&error, "placeholder referenced a missing wrapper set");
}

#[test]
fn expansion_missing_conditional_wrapper_set_produces_internal_error() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let placeholder = TirSlotPlaceholder::with_wrapper_sets(
        SlotKey::Default,
        store.next_slot_occurrence_id(),
        empty_location(),
        None,
        Some(TemplateWrapperSetId::new(0)),
        false,
    );
    let wrapper = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot_node = builder.push_tir_slot_placeholder_node(placeholder);
        builder.finish_template(
            slot_node,
            Style::default(),
            TemplateType::String,
            slot_summary(1),
            empty_location(),
        )
    };
    let branch_node = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_text_node(
            string_table.intern("branch"),
            "branch".len(),
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(true)),
            body,
            empty_location(),
            builder.store.next_expression_site_id(),
        );
        builder.push_branch_chain_node(vec![branch], None, empty_location())
    };
    let routed = TirSlotContributions {
        default_nodes: vec![branch_node],
        ..TirSlotContributions::default()
    };

    let error = expand_tir_slot_placeholders(&mut store, wrapper, &routed, &string_table)
        .expect_err("missing conditional wrapper sets must fail slot expansion");

    assert_internal_authority_error(&error, "conditional child wrapper set ID");
}

#[test]
fn repeated_slot_replays_same_contributions() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let wrapper =
        build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Default, SlotKey::Default]);
    let contribution = build_single_text_template(&mut store, &mut string_table, "shared");
    let contribution_node_id = template_root_node_id(contribution, &store);

    let routed = TirSlotContributions {
        default_nodes: vec![contribution_node_id],
        ..TirSlotContributions::default()
    };

    let expanded_root = expand_tir_slot_placeholders(&mut store, wrapper, &routed, &string_table)
        .expect("expansion should succeed");

    let root_children = root_child_node_ids_for_node(expanded_root, &store);
    assert_eq!(
        root_children.len(),
        2,
        "two default slots should splice the same contribution twice"
    );

    for child_id in root_children {
        assert_eq!(
            text_node_text(child_id, &store, &string_table),
            Some("shared".to_owned())
        );
        assert_eq!(child_id, contribution_node_id);
    }
}

/// Proves the nested child-template clone/reuse decision in one parent tree.
#[test]
fn nested_child_template_clone_vs_reuse_by_slot_presence() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("inner");
    let mut store = TemplateIrStore::new();

    let slot_bearing_child =
        build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Named(name)]);
    let slot_less_child = build_single_text_template(&mut store, &mut string_table, "no slots");

    let parent_wrapper = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot_child_ref = builder.push_child_template_node(slot_bearing_child, empty_location());
        let no_slot_child_ref = builder.push_child_template_node(slot_less_child, empty_location());
        let root =
            builder.push_sequence_node(vec![slot_child_ref, no_slot_child_ref], empty_location());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };

    let contribution = build_single_text_template(&mut store, &mut string_table, "inner text");
    let contribution_node_id = template_root_node_id(contribution, &store);

    let routed = TirSlotContributions {
        named_nodes: [(name, vec![contribution_node_id])].into_iter().collect(),
        ..TirSlotContributions::default()
    };

    let expanded_root =
        expand_tir_slot_placeholders(&mut store, parent_wrapper, &routed, &string_table)
            .expect("expansion should succeed");

    let parent_children = root_child_node_ids_for_node(expanded_root, &store);
    assert_eq!(
        parent_children.len(),
        2,
        "parent should keep both child template references in order"
    );

    // With-slots child: expansion clones it into a new expanded template entry
    // and splices the contribution into the cloned root.
    let expanded_slot_child_template_id = match &store
        .get_node(parent_children[0])
        .expect("slot-bearing child node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
        other => panic!("expected ChildTemplate node for slot-bearing child, found {other:?}"),
    };
    assert_ne!(
        expanded_slot_child_template_id, slot_bearing_child,
        "slot-bearing child should clone into a new expanded template entry"
    );
    let expanded_slot_child_children = root_child_kinds(expanded_slot_child_template_id, &store);
    assert_eq!(expanded_slot_child_children.len(), 1);
    assert!(
        matches!(
            expanded_slot_child_children[0],
            TemplateIrNodeKind::Text { .. }
        ),
        "inner slot contribution should be spliced into the cloned child root"
    );
    assert_eq!(
        text_node_text(
            root_child_node_ids(expanded_slot_child_template_id, &store)[0],
            &store,
            &string_table
        ),
        Some("inner text".to_owned())
    );

    // Without-slots child: expansion leaves the reference unchanged because
    // the child has no slot composition work.
    let unchanged_child_template_id = match &store
        .get_node(parent_children[1])
        .expect("slot-less child node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
        other => panic!("expected ChildTemplate node for slot-less child, found {other:?}"),
    };
    assert_eq!(
        unchanged_child_template_id, slot_less_child,
        "slot-less child should keep its original template ID"
    );
}

/// Nested slot expansion must keep the child's complete view identity.
#[test]
fn nested_child_slot_expansion_preserves_phase_context_wrapper_set_and_slot_plan() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("inner");
    let mut store = TemplateIrStore::new();

    let slot_bearing_child =
        build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Named(name)]);
    let wrapper_set_id = store.push_wrapper_set(TemplateWrapperSet { wrappers: vec![] });
    let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![],
        slot_sites: vec![],
    });
    {
        let child_template = store
            .template_mut(slot_bearing_child)
            .expect("slot-bearing child should exist");
        child_template.conditional_child_wrapper_set = Some(wrapper_set_id);
        child_template.runtime_slot_plan = Some(slot_plan_id);
    }

    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay::default())
        .expect("test overlay allocation");
    let slot_overlay = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("test overlay allocation");
    let wrapper_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay::default())
        .expect("test overlay allocation");
    let source_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: Some(slot_overlay),
        wrapper_context: Some(wrapper_overlay),
    };
    let source_phase = TemplateTirPhase::Composed;

    let parent_wrapper = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let child_ref = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(slot_bearing_child, source_phase, source_context),
            empty_location(),
        );
        let root = builder.push_sequence_node(vec![child_ref], empty_location());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };

    let contribution = build_single_text_template(&mut store, &mut string_table, "inner text");
    let contribution_node_id = template_root_node_id(contribution, &store);
    let routed = TirSlotContributions {
        named_nodes: [(name, vec![contribution_node_id])].into_iter().collect(),
        ..TirSlotContributions::default()
    };

    let expanded_root =
        expand_tir_slot_placeholders(&mut store, parent_wrapper, &routed, &string_table)
            .expect("expansion should succeed");
    let parent_children = root_child_node_ids_for_node(expanded_root, &store);
    assert_eq!(parent_children.len(), 1);

    let expanded_reference = match &store
        .get_node(parent_children[0])
        .expect("expanded child node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => *reference,
        other => panic!("expected ChildTemplate after expansion, found {other:?}"),
    };
    let expanded_template = store
        .get_template(expanded_reference.root)
        .expect("expanded child template should exist");

    assert_eq!(
        expanded_reference.phase, source_phase,
        "derived child must keep the source phase"
    );
    assert_eq!(
        expanded_reference.context, source_context,
        "derived child must keep the complete source context"
    );
    assert_eq!(
        expanded_template.conditional_child_wrapper_set,
        Some(wrapper_set_id),
        "derived child must keep the source wrapper set"
    );
    assert_eq!(
        expanded_template.runtime_slot_plan,
        Some(slot_plan_id),
        "derived child must keep the source runtime slot plan"
    );
}

#[test]
fn wrapper_application_preserves_only_authorised_structural_context() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Default]);
    let child = build_single_text_template(&mut store, &mut string_table, "child");
    let child_node = build_child_template_node_for_template(&mut store, child);
    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay::default())
        .expect("expression overlay");
    let slot_overlay = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("slot overlay");
    let wrapper_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay::default())
        .expect("wrapper overlay");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: Some(slot_overlay),
        wrapper_context: Some(wrapper_overlay),
    };

    let parsed_reference =
        TemplateWrapperReference::new(wrapper, TemplateTirPhase::Parsed, context);
    let parsed_result = wrap_tir_node_in_wrapper_references(
        &mut store,
        child_node,
        &[parsed_reference],
        &string_table,
    )
    .expect("parsed wrapper should compose");
    let parsed_child_reference = expect_child_template_reference(parsed_result, &store);
    assert_eq!(parsed_child_reference.phase, TemplateTirPhase::Composed);
    assert_eq!(parsed_child_reference.context.expression_overlay, None);
    assert_eq!(parsed_child_reference.context.slot_resolution, None);
    assert_eq!(parsed_child_reference.context.wrapper_context, None);
    let parsed_view = TirView::new(
        &store,
        parsed_child_reference.root,
        parsed_child_reference.phase,
        parsed_child_reference.context,
    )
    .expect("parsed application should create an exact view");
    assert!(
        parsed_view
            .slot_resolution_overlay()
            .expect("slot overlay lookup")
            .is_none()
    );
    assert!(
        parsed_view
            .wrapper_context_overlay()
            .expect("wrapper overlay lookup")
            .is_none()
    );

    let composed_reference = TemplateWrapperReference::new(
        wrapper,
        TemplateTirPhase::Composed,
        TemplateViewContext {
            expression_overlay: Some(expression_overlay),
            slot_resolution: Some(slot_overlay),
            wrapper_context: Some(wrapper_overlay),
        },
    );
    let composed_result = wrap_tir_node_in_wrapper_references(
        &mut store,
        child_node,
        &[composed_reference],
        &string_table,
    )
    .expect("composed wrapper should compose");
    let composed_child_reference = expect_child_template_reference(composed_result, &store);
    assert_eq!(composed_child_reference.phase, TemplateTirPhase::Composed);
    assert_eq!(composed_child_reference.context.expression_overlay, None);
    assert_eq!(composed_child_reference.context.slot_resolution, None);
    assert_eq!(
        composed_child_reference.context.wrapper_context,
        Some(wrapper_overlay)
    );
}

#[test]
fn slotless_wrapper_application_preserves_complete_reference_identity() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let wrapper = build_single_text_template(&mut store, &mut string_table, "prefix");
    let child = build_single_text_template(&mut store, &mut string_table, "child");
    let child_node = build_child_template_node_for_template(&mut store, child);
    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay::default())
        .expect("expression overlay");
    let slot_overlay = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("slot overlay");
    let wrapper_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay::default())
        .expect("wrapper overlay");
    let reference = TemplateWrapperReference::new(
        wrapper,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: Some(expression_overlay),
            slot_resolution: Some(slot_overlay),
            wrapper_context: Some(wrapper_overlay),
        },
    );

    let wrapped =
        wrap_tir_node_in_wrapper_references(&mut store, child_node, &[reference], &string_table)
            .expect("slotless wrapper should apply");
    let derived_reference = expect_child_template_reference(wrapped, &store);
    assert_ne!(derived_reference.root, wrapper);
    assert_eq!(derived_reference.phase, reference.phase);
    assert_eq!(derived_reference.context, reference.context);
    let view = TirView::new(
        &store,
        derived_reference.root,
        derived_reference.phase,
        derived_reference.context,
    )
    .expect("slotless application should create an exact view");
    assert!(
        view.expression_overlay()
            .expect("expression overlay lookup")
            .is_some()
    );
    assert!(
        view.slot_resolution_overlay()
            .expect("slot overlay lookup")
            .is_some()
    );
    assert!(
        view.wrapper_context_overlay()
            .expect("wrapper overlay lookup")
            .is_some()
    );
}

#[test]
fn branch_chain_with_slots_in_body_is_expanded() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let occurrence_id = store.next_slot_occurrence_id();
    let placeholder = TirSlotPlaceholder::new(SlotKey::Default, occurrence_id, empty_location());
    let body_slot = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot { placeholder },
        empty_location(),
    ));

    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(bool_expression(true)),
        body_slot,
        empty_location(),
        store.next_expression_site_id(),
    );

    let mut builder = TemplateIrBuilder::new(&mut store);
    let branch_chain = builder.push_branch_chain_node(vec![branch], None, empty_location());
    let wrapper = builder.finish_template(
        branch_chain,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let contribution = build_single_text_template(&mut store, &mut string_table, "branch body");

    let routed = TirSlotContributions {
        default_nodes: vec![template_root_node_id(contribution, &store)],
        ..TirSlotContributions::default()
    };

    let expanded_root = expand_tir_slot_placeholders(&mut store, wrapper, &routed, &string_table)
        .expect("expansion should succeed");

    let expanded_branch_chain = match &store
        .get_node(expanded_root)
        .expect("root should exist")
        .kind
    {
        TemplateIrNodeKind::BranchChain { branches, .. } => {
            assert_eq!(branches.len(), 1);
            &branches[0]
        }
        other => panic!("expected BranchChain root, found {other:?}"),
    };

    let branch_body_children = root_child_kinds_for_node(expanded_branch_chain.body, &store);
    assert_eq!(branch_body_children.len(), 1);
    assert!(
        matches!(branch_body_children[0], TemplateIrNodeKind::Text { .. }),
        "branch body slot should be spliced into the body sequence"
    );
    assert_eq!(
        text_node_text(
            root_child_node_ids_for_node(expanded_branch_chain.body, &store)[0],
            &store,
            &string_table
        ),
        Some("branch body".to_owned())
    );
}

#[test]
fn loop_with_slots_in_body_is_expanded() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let mut builder = TemplateIrBuilder::new(&mut store);
    let body_slot = builder.push_slot_node(SlotKey::Default, empty_location());
    let loop_node = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(bool_expression(true)),
        },
        body_slot,
        None,
        empty_location(),
    );
    let wrapper = builder.finish_template(
        loop_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let contribution = build_single_text_template(&mut store, &mut string_table, "iteration");

    let routed = TirSlotContributions {
        default_nodes: vec![template_root_node_id(contribution, &store)],
        ..TirSlotContributions::default()
    };

    let expanded_root = expand_tir_slot_placeholders(&mut store, wrapper, &routed, &string_table)
        .expect("expansion should succeed");

    let expanded_loop = match &store
        .get_node(expanded_root)
        .expect("root should exist")
        .kind
    {
        TemplateIrNodeKind::Loop { body, .. } => *body,
        other => panic!("expected Loop root, found {other:?}"),
    };

    let body_children = root_child_kinds_for_node(expanded_loop, &store);
    assert_eq!(body_children.len(), 1);
    assert!(
        matches!(body_children[0], TemplateIrNodeKind::Text { .. }),
        "loop body slot should be spliced into the body sequence"
    );
    assert_eq!(
        text_node_text(
            root_child_node_ids_for_node(expanded_loop, &store)[0],
            &store,
            &string_table
        ),
        Some("iteration".to_owned())
    );
}

#[test]
fn no_slots_in_wrapper_returns_original_root() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let wrapper = build_single_text_template(&mut store, &mut string_table, "plain");
    let original_root = store
        .get_template(wrapper)
        .expect("wrapper should exist")
        .root;

    let routed = TirSlotContributions::default();

    let expanded_root = expand_tir_slot_placeholders(&mut store, wrapper, &routed, &string_table)
        .expect("expansion should succeed");

    assert_eq!(
        expanded_root, original_root,
        "wrapper with no slots should return the original root node ID"
    );
}

#[test]
fn expand_preserves_non_slot_nodes() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let text_id = string_table.intern("literal");
    let text_len = string_table.resolve(text_id).len();

    let text_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: text_id,
            byte_len: text_len,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));

    let aggregate_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::AggregateOutput,
        empty_location(),
    ));

    let loop_control_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::LoopControl {
            kind: TemplateLoopControlKind::Break,
        },
        empty_location(),
    ));

    let expression = Expression::string_slice(text_id, empty_location(), ValueMode::ImmutableOwned);
    let site_id = store.next_expression_site_id();
    let dynamic_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(expression),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        empty_location(),
    ));

    let runtime_slot_plan_id = store.push_slot_plan(
        crate::compiler_frontend::ast::templates::tir::slot_plan::TemplateSlotPlan {
            location: empty_location(),
            contribution_sources: vec![],
            slot_sites: vec![],
        },
    );

    let runtime_slot_site_id =
        crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotSiteId(0);
    let runtime_slot_site = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::RuntimeSlotSite {
            plan: runtime_slot_plan_id,
            site: runtime_slot_site_id,
        },
        empty_location(),
    ));

    let occurrence_id = store.next_slot_occurrence_id();
    let placeholder = TirSlotPlaceholder::new(SlotKey::Default, occurrence_id, empty_location());
    let slot_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot { placeholder },
        empty_location(),
    ));

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![
                text_node,
                aggregate_node,
                slot_node,
                loop_control_node,
                dynamic_node,
                runtime_slot_site,
            ],
        },
        empty_location(),
    ));

    let wrapper = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let contribution = build_single_text_template(&mut store, &mut string_table, "only slot");
    let contribution_node = template_root_node_id(contribution, &store);

    let routed = TirSlotContributions {
        default_nodes: vec![contribution_node],
        ..TirSlotContributions::default()
    };

    let expanded_root = expand_tir_slot_placeholders(&mut store, wrapper, &routed, &string_table)
        .expect("expansion should succeed");

    let child_kinds = root_child_kinds_for_node(expanded_root, &store);
    let child_ids = root_child_node_ids_for_node(expanded_root, &store);
    assert_eq!(
        child_ids,
        vec![
            text_node,
            aggregate_node,
            contribution_node,
            loop_control_node,
            dynamic_node,
            runtime_slot_site,
        ],
        "expansion must preserve every non-slot node identity and splice the exact contribution"
    );
    assert_eq!(child_kinds.len(), 6);
    assert!(matches!(child_kinds[0], TemplateIrNodeKind::Text { .. }));
    assert!(matches!(
        child_kinds[1],
        TemplateIrNodeKind::AggregateOutput
    ));
    assert!(
        matches!(child_kinds[2], TemplateIrNodeKind::Text { .. }),
        "slot contribution should be spliced between surrounding non-slot nodes without a nested sequence"
    );
    assert!(matches!(
        child_kinds[3],
        TemplateIrNodeKind::LoopControl { .. }
    ));
    assert!(matches!(
        child_kinds[4],
        TemplateIrNodeKind::DynamicExpression { .. }
    ));
    assert!(matches!(
        child_kinds[5],
        TemplateIrNodeKind::RuntimeSlotSite { .. }
    ));
    assert_eq!(
        text_node_text(child_ids[2], &store, &string_table),
        Some("only slot".to_owned())
    );
}

// -------------------------
//  Head-Chain Composition Tests
// -------------------------

/// A single receiver routes both head-origin and body-origin fill into its
/// default slot, preserving cross-origin authored order (head fill before body
/// fill). Proves the head-children and body-children routing branches of
/// `build_tir_chain_graph` in one labelled tree.
#[test]
fn single_receiver_routes_head_and_body_fill_in_authored_order() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Default]);
    let wrapper_node = build_child_template_node_for_template(&mut store, wrapper);
    let head_fill = build_text_node(
        &mut store,
        &mut string_table,
        "head fill",
        TemplateSegmentOrigin::Head,
    );
    let body_fill = build_text_node(
        &mut store,
        &mut string_table,
        "body fill",
        TemplateSegmentOrigin::Body,
    );

    let template_id =
        build_template_with_children(&mut store, vec![wrapper_node, head_fill, body_fill]);

    let composed_root = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("composition should succeed");

    let composed_children = root_child_node_ids_for_node(composed_root, &store);
    assert_eq!(
        composed_children.len(),
        1,
        "composed root should contain only the resolved wrapper"
    );

    let resolved_wrapper_template_id = expect_child_template_id(composed_children[0], &store);
    let resolved_wrapper_children = root_child_node_ids(resolved_wrapper_template_id, &store);
    assert_eq!(
        resolved_wrapper_children,
        vec![head_fill, body_fill],
        "wrapper default slot should receive the exact head and body fill nodes in authored order"
    );
    assert_eq!(
        text_node_text(resolved_wrapper_children[0], &store, &string_table),
        Some("head fill".to_owned()),
        "head-origin fill is routed through the head-children loop into the active receiver"
    );
    assert_eq!(
        text_node_text(resolved_wrapper_children[1], &store, &string_table),
        Some("body fill".to_owned()),
        "body-origin fill is routed through the body-children loop into the deepest active receiver"
    );
}

#[cfg(feature = "benchmark_counters")]
#[test]
fn head_chain_receiver_collects_one_slot_layout() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, reset_ast_counters, test_read_ast_counter,
    };

    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Default]);
    let wrapper_node = build_child_template_node_for_template(&mut store, wrapper);
    let fill = build_text_node(
        &mut store,
        &mut string_table,
        "fill",
        TemplateSegmentOrigin::Body,
    );
    let template_id = build_template_with_children(&mut store, vec![wrapper_node, fill]);

    reset_ast_counters();
    compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("head-chain composition should succeed");

    assert_eq!(
        test_read_ast_counter(AstCounter::TirSlotSchemaWalks),
        1,
        "one receiver should collect its slot layout once"
    );
}

#[test]
fn composed_wrapper_published_summary_matches_the_expanded_tree() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let wrapper = build_text_slot_text_wrapper(&mut store, &mut string_table, SlotKey::Default);
    let wrapper_node = build_child_template_node_for_template(&mut store, wrapper);
    let fill = build_text_node(
        &mut store,
        &mut string_table,
        "fill body",
        TemplateSegmentOrigin::Body,
    );
    let template_id = build_template_with_children(&mut store, vec![wrapper_node, fill]);

    let composed_root = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("composition should succeed");
    let composed_children = root_child_node_ids_for_node(composed_root, &store);
    let resolved_wrapper_id = expect_child_template_id(composed_children[0], &store);
    let published = store
        .get_template(resolved_wrapper_id)
        .expect("composed wrapper exists");
    let recomputed =
        summarize_existing_root(&store, published.root).expect("composed wrapper root is finite");

    assert_eq!(
        published.summary.text_byte_count,
        recomputed.text_byte_count
    );
    assert_eq!(
        published.summary.text_node_count,
        recomputed.text_node_count
    );
    assert_eq!(published.summary.max_depth, recomputed.max_depth);
    assert_eq!(published.summary.slot_count, 0);
    assert!(
        published.summary.text_byte_count >= "fill body".len(),
        "composed wrapper summary must include contributed fill text"
    );
    assert!(published.summary.max_depth >= 1);
}

#[test]
fn head_chain_missing_root_authority_remains_infrastructure_error() {
    let string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let missing_root = TemplateIrNodeId::new(99);
    let template_id = store.push_template(TemplateIr::new(
        missing_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            child_template_count: 1,
            ..TemplateIrSummary::default()
        },
        empty_location(),
    ));

    let error = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect_err("missing head-chain root authority must fail composition");

    assert_internal_authority_error(&error, "root node ID was not present");
}

/// Nested head-origin receivers resolve bottom-up: the inner receiver resolves
/// first and becomes the outer receiver's fill, landing at the outer's slot
/// between its surrounding text. Multiple body fills accumulate in the deepest
/// active layer in authored order. Proves the complete nested/multiple receiver
/// chain-order matrix in one labelled tree.
#[test]
fn nested_receivers_route_multiple_fills_in_chain_order() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let inner_wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Default]);
    let inner_wrapper_node = build_child_template_node_for_template(&mut store, inner_wrapper);

    let outer_wrapper =
        build_text_slot_text_wrapper(&mut store, &mut string_table, SlotKey::Default);
    let outer_wrapper_children = root_child_node_ids(outer_wrapper, &store);
    let outer_wrapper_node = build_child_template_node_for_template(&mut store, outer_wrapper);

    let first_fill = build_text_node(
        &mut store,
        &mut string_table,
        "first fill",
        TemplateSegmentOrigin::Body,
    );
    let second_fill = build_text_node(
        &mut store,
        &mut string_table,
        "second fill",
        TemplateSegmentOrigin::Body,
    );

    let template_id = build_template_with_children(
        &mut store,
        vec![
            outer_wrapper_node,
            inner_wrapper_node,
            first_fill,
            second_fill,
        ],
    );

    let composed_root = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("composition should succeed");

    let composed_children = root_child_node_ids_for_node(composed_root, &store);
    assert_eq!(
        composed_children.len(),
        1,
        "outer wrapper should be the only root child"
    );

    let outer_resolved_id = expect_child_template_id(composed_children[0], &store);
    let outer_children = root_child_node_ids(outer_resolved_id, &store);
    assert_eq!(
        outer_children.len(),
        3,
        "outer wrapper should keep before/after text around the slot holding the resolved inner"
    );
    assert_eq!(
        outer_children[0], outer_wrapper_children[0],
        "outer wrapper preserves its exact leading text node"
    );
    let inner_resolved_id = expect_child_template_id(outer_children[1], &store);
    assert_eq!(
        outer_children[2], outer_wrapper_children[2],
        "outer wrapper preserves its exact trailing text node"
    );

    let inner_children = root_child_node_ids(inner_resolved_id, &store);
    assert_eq!(
        inner_children,
        vec![first_fill, second_fill],
        "inner receiver collects the exact body fill nodes in authored order"
    );
    assert_eq!(
        text_node_text(inner_children[0], &store, &string_table),
        Some("first fill".to_owned()),
        "body fills keep authored order in the deepest receiver"
    );
    assert_eq!(
        text_node_text(inner_children[1], &store, &string_table),
        Some("second fill".to_owned()),
        "body fills keep authored order in the deepest receiver"
    );
}

#[test]
fn receiver_with_no_fill_stays_unresolved() {
    let string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Default]);
    let wrapper_node = build_child_template_node_for_template(&mut store, wrapper);

    let template_id = build_template_with_children(&mut store, vec![wrapper_node]);
    let original_root = template_root_node_id(template_id, &store);

    let composed_root = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("composition should succeed");

    assert_eq!(
        composed_root, original_root,
        "receiver with no fill should keep the original root unchanged"
    );
}

#[test]
fn receiver_with_named_slots() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("title");

    let mut store = TemplateIrStore::new();

    let wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Named(name)]);
    let wrapper_node = build_child_template_node_for_template(&mut store, wrapper);

    let insert_template =
        build_slot_insert_template(&mut store, SlotKey::Named(name), &mut string_table);
    let insert_body = template_root_node_id(insert_template, &store);
    let mut builder = TemplateIrBuilder::new(&mut store);
    let insert_node = builder.push_insert_contribution_node(insert_template, empty_location());

    let template_id = build_template_with_children(&mut store, vec![wrapper_node, insert_node]);

    let composed_root = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("composition should succeed");

    let composed_children = root_child_node_ids_for_node(composed_root, &store);
    assert_eq!(composed_children.len(), 1);

    let resolved_wrapper_template_id = expect_child_template_id(composed_children[0], &store);
    let resolved_wrapper_children = root_child_node_ids(resolved_wrapper_template_id, &store);
    assert_eq!(
        resolved_wrapper_children,
        vec![insert_body],
        "named-slot routing should splice the exact insert body node"
    );

    // Slot expansion places the insert helper's body content directly into the
    // wrapper's slot; the InsertContribution marker is resolved during routing.
    assert_eq!(
        text_node_text(insert_body, &store, &string_table),
        Some("insert".to_owned())
    );
}

#[test]
fn mixed_head_text_and_receiver() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let head_text = build_text_node(
        &mut store,
        &mut string_table,
        "head text",
        TemplateSegmentOrigin::Head,
    );

    let wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Default]);
    let wrapper_node = build_child_template_node_for_template(&mut store, wrapper);

    let body_text = build_text_node(
        &mut store,
        &mut string_table,
        "body fill",
        TemplateSegmentOrigin::Body,
    );

    let template_id =
        build_template_with_children(&mut store, vec![head_text, wrapper_node, body_text]);

    let composed_root = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("composition should succeed");

    let composed_children = root_child_node_ids_for_node(composed_root, &store);
    assert_eq!(
        composed_children.len(),
        2,
        "head text and resolved wrapper should remain"
    );

    assert_eq!(
        composed_children[0], head_text,
        "the exact head text node should appear before the resolved wrapper"
    );

    let resolved_wrapper_template_id = expect_child_template_id(composed_children[1], &store);
    let resolved_wrapper_children = root_child_node_ids(resolved_wrapper_template_id, &store);
    assert_eq!(
        resolved_wrapper_children,
        vec![body_text],
        "the exact body node should fill the wrapper after the root head text"
    );
}

#[test]
fn no_receiver_returns_original_root_with_head_and_body_in_order() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let head_text = build_text_node(
        &mut store,
        &mut string_table,
        "head text",
        TemplateSegmentOrigin::Head,
    );
    let body_text = build_text_node(
        &mut store,
        &mut string_table,
        "body text",
        TemplateSegmentOrigin::Body,
    );

    let template_id = build_template_with_children(&mut store, vec![head_text, body_text]);
    let original_root = template_root_node_id(template_id, &store);

    let composed_root = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("composition should succeed");

    assert_eq!(
        composed_root, original_root,
        "a template without child references should keep its original root"
    );

    let composed_children = root_child_node_ids_for_node(composed_root, &store);
    assert_eq!(
        composed_children,
        vec![head_text, body_text],
        "head and body nodes should remain at the root in authored order"
    );
}

#[test]
fn compose_preserves_non_receiver_head_child_templates() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let non_receiver = build_single_text_template(&mut store, &mut string_table, "no slots");
    let non_receiver_node = build_child_template_node_for_template(&mut store, non_receiver);
    let body_text = build_text_node(
        &mut store,
        &mut string_table,
        "body text",
        TemplateSegmentOrigin::Body,
    );

    let template_id = build_template_with_children(&mut store, vec![non_receiver_node, body_text]);
    let original_root = template_root_node_id(template_id, &store);

    let composed_root = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("composition should succeed");

    assert_eq!(
        composed_root, original_root,
        "a slot-less head child should keep the original root"
    );

    let composed_children = root_child_node_ids_for_node(composed_root, &store);
    assert_eq!(
        composed_children,
        vec![non_receiver_node, body_text],
        "the receiver scan fast path should preserve both child node identities"
    );

    let preserved_child_id = expect_child_template_id(composed_children[0], &store);
    assert_eq!(
        preserved_child_id, non_receiver,
        "non-receiver head child template should keep its original template ID"
    );
}

/// Asserts that a node is a `ChildTemplate` reference and returns the template ID.
fn expect_child_template_id(node_id: TemplateIrNodeId, store: &TemplateIrStore) -> TemplateIrId {
    expect_child_template_reference(node_id, store).root
}

fn expect_child_template_reference(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
) -> TemplateTirChildReference {
    let node = store.get_node(node_id).expect("node should exist");

    match &node.kind {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => *reference,
        other => panic!("expected ChildTemplate node, found {other:?}"),
    }
}

// -------------------------
//  Expansion Test Inspection Helpers
// -------------------------

/// Returns the direct children of any Sequence node.
fn children_of_sequence_node(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
) -> Vec<TemplateIrNodeId> {
    let node = store.get_node(node_id).expect("node should exist");

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected Sequence node, found {other:?}"),
    }
}

/// Returns the direct children of a node that is expected to be a Sequence root.
fn root_child_node_ids_for_node(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
) -> Vec<TemplateIrNodeId> {
    children_of_sequence_node(node_id, store)
}

/// Returns references to the kinds of the children of a Sequence node.
fn root_child_kinds_for_node(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
) -> Vec<&TemplateIrNodeKind> {
    root_child_node_ids_for_node(node_id, store)
        .into_iter()
        .map(|node_id| {
            &store
                .get_node(node_id)
                .expect("child node should exist")
                .kind
        })
        .collect()
}

// -------------------------
//  Per-Child Wrapper Composition
// -------------------------
// These tests exercise `wrap_tir_node_in_wrappers`, the owned wrapper-tree
// builder used by slot expansion. They pin the three distinct wrapper-expansion
// shapes: slot-bearing wrapper expansion, slot-less wrapper prepend, and
// multi-wrapper nesting/order. Direct-child inherited wrappers are owned by
// wrapper-context overlays.

#[test]
fn wrap_direct_child_in_single_wrapper_with_default_slot() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Default]);
    let child = build_single_text_template(&mut store, &mut string_table, "child");
    let child_node = build_child_template_node_for_template(&mut store, child);
    let wrapped_child_node_id =
        wrap_tir_node_in_wrappers(&mut store, child_node, &[wrapper], &string_table)
            .expect("wrapper application should succeed");

    let wrapped_template_id = expect_child_template_id(wrapped_child_node_id, &store);
    let wrapped_children = root_child_node_ids(wrapped_template_id, &store);
    assert_eq!(
        wrapped_children,
        vec![child_node],
        "default-slot expansion should splice the original child node exactly once"
    );

    let resolved_child_id = expect_child_template_id(wrapped_children[0], &store);
    assert_eq!(
        resolved_child_id, child,
        "wrapper with a single default slot should contain the original child template"
    );
    assert_eq!(
        template_root_text(resolved_child_id, &store, &string_table),
        Some("child".to_owned())
    );
}

#[test]
fn shared_child_dag_replays_one_application_at_each_structural_occurrence() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let slot_wrapper = build_wrapper_with_slot_sequence(&mut store, vec![SlotKey::Default]);
    let first_slot_wrapper_node = build_child_template_node_for_template(&mut store, slot_wrapper);
    let second_slot_wrapper_node = build_child_template_node_for_template(&mut store, slot_wrapper);
    let shared_child_wrapper = build_template_with_children(
        &mut store,
        vec![first_slot_wrapper_node, second_slot_wrapper_node],
    );
    let contribution = build_single_text_template(&mut store, &mut string_table, "shared");
    let contribution_node = build_child_template_node_for_template(&mut store, contribution);

    let wrapped = wrap_tir_node_in_wrapper_references(
        &mut store,
        contribution_node,
        &[TemplateWrapperReference::new(
            shared_child_wrapper,
            TemplateTirPhase::Parsed,
            TemplateViewContext::default(),
        )],
        &string_table,
    )
    .expect("shared child DAG application should succeed");
    let applied_reference = expect_child_template_reference(wrapped, &store);
    assert_eq!(applied_reference.context.slot_resolution, None);
    let applied_children = root_child_node_ids(applied_reference.root, &store);
    assert_eq!(applied_children.len(), 2);
    for child_id in applied_children {
        let child_reference = expect_child_template_reference(child_id, &store);
        assert_eq!(
            root_child_node_ids(child_reference.root, &store),
            vec![contribution_node],
            "one routed contribution must replay at every shared structural occurrence"
        );
    }
}

#[test]
fn wrap_direct_child_in_wrapper_without_slots() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let wrapper = build_single_text_template(&mut store, &mut string_table, "prefix");
    let child = build_single_text_template(&mut store, &mut string_table, "child");
    let child_node = build_child_template_node_for_template(&mut store, child);
    let wrapped_child_node_id =
        wrap_tir_node_in_wrappers(&mut store, child_node, &[wrapper], &string_table)
            .expect("wrapper application should succeed");

    let combined_template_id = expect_child_template_id(wrapped_child_node_id, &store);
    let combined_children = root_child_node_ids(combined_template_id, &store);
    assert_eq!(combined_children.len(), 2);
    assert_eq!(
        combined_children[1], child_node,
        "slot-less wrapping should preserve the original child node identity"
    );

    let wrapper_child_id = expect_child_template_id(combined_children[0], &store);
    assert_eq!(
        wrapper_child_id, wrapper,
        "slot-less wrapper should prepend its content before the original child"
    );
    assert_eq!(
        template_root_text(wrapper_child_id, &store, &string_table),
        Some("prefix".to_owned())
    );

    let inner_child_id = expect_child_template_id(combined_children[1], &store);
    assert_eq!(
        inner_child_id, child,
        "slot-less wrapper should keep the original child after the wrapper content"
    );
}

#[test]
fn wrap_direct_child_in_multiple_wrappers() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let inner_wrapper = build_text_slot_text_wrapper_with_markers(
        &mut store,
        &mut string_table,
        SlotKey::Default,
        "inner-before",
        "inner-after",
    );
    let outer_wrapper = build_text_slot_text_wrapper_with_markers(
        &mut store,
        &mut string_table,
        SlotKey::Default,
        "outer-before",
        "outer-after",
    );

    let child = build_single_text_template(&mut store, &mut string_table, "child");
    let child_node = build_child_template_node_for_template(&mut store, child);
    // Wrappers are stored innermost-first; forward iteration applies the
    // innermost wrapper directly around the child and each later wrapper
    // around the previous result, so the final nesting is outer(inner(child)).
    let wrapper_ids = vec![inner_wrapper, outer_wrapper];
    let wrapped_child_node_id =
        wrap_tir_node_in_wrappers(&mut store, child_node, &wrapper_ids, &string_table)
            .expect("wrapper application should succeed");

    let outer_resolved_id = expect_child_template_id(wrapped_child_node_id, &store);
    let outer_children = root_child_node_ids(outer_resolved_id, &store);
    assert_eq!(outer_children.len(), 3);

    assert_eq!(
        text_node_text(outer_children[0], &store, &string_table),
        Some("outer-before".to_owned())
    );

    let inner_resolved_id = expect_child_template_id(outer_children[1], &store);
    let inner_children = root_child_node_ids(inner_resolved_id, &store);
    assert_eq!(inner_children.len(), 3);
    assert_eq!(
        text_node_text(inner_children[0], &store, &string_table),
        Some("inner-before".to_owned())
    );
    assert_eq!(
        text_node_text(inner_children[2], &store, &string_table),
        Some("inner-after".to_owned())
    );

    let innermost_child_id = expect_child_template_id(inner_children[1], &store);
    assert_eq!(innermost_child_id, child);

    assert_eq!(
        text_node_text(outer_children[2], &store, &string_table),
        Some("outer-after".to_owned())
    );
}

#[test]
fn head_chain_returns_the_original_root_when_no_receiver_exists() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let body_text = build_text_node(
        &mut store,
        &mut string_table,
        "body",
        TemplateSegmentOrigin::Body,
    );
    let template_id = build_template_with_children(&mut store, vec![body_text]);
    let original_root = store.get_template(template_id).expect("template").root;

    let composed_root = compose_tir_head_chain(&mut store, template_id, &string_table, false)
        .expect("head-chain composition should succeed");

    assert_eq!(
        composed_root, original_root,
        "template with no receivers should return the original root"
    );
}
