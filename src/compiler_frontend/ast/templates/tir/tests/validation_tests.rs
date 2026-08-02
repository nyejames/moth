//! Validation tests.
//!
//! WHAT: exercises `validate_tir_store` with well-formed and malformed TIR stores
//! to confirm that structural invariants are enforced.
//! WHY: focused tests protect the malformed-store invariants that downstream
//! TIR consumers rely on.

use super::super::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId,
    TemplateSlotPlanId,
};
use super::super::node::{TemplateIr, TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder};
use super::super::refs::TemplateTirChildReference;
use super::super::refs::TemplateWrapperReference;
use super::super::slot_plan::{
    TemplateSlotPlan, TemplateSlotSitePlan, TemplateSlotSiteRenderPiece, TemplateSlotSiteRenderPlan,
};
use super::super::store::{TemplateIrStore, TemplateWrapperSet};
use super::super::summary::TemplateIrSummary;
use super::super::{TemplateTirPhase, TemplateViewContext};
use super::validation_support::validate_tir_store;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateBranchSelector;
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotSiteId;
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrBranch;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn slot_placeholder(key: SlotKey, occurrence_id: SlotOccurrenceId) -> TirSlotPlaceholder {
    TirSlotPlaceholder::new(key, occurrence_id, empty_location())
}

fn bool_expression() -> Expression {
    Expression {
        kind: ExpressionKind::Bool(true),
        type_id: builtin_type_ids::BOOL,
        diagnostic_type: DataType::Bool,
        function_receiver: None,
        value_mode: ValueMode::ImmutableOwned,
        location: empty_location(),
        reactive_source: None,
        reactive_template: None,
        const_record_state: ConstRecordState::RuntimeValue,
        contains_regular_division: false,
        synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
    }
}

fn runtime_slot_plan(site_count: usize) -> TemplateSlotPlan {
    TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![],
        slot_sites: (0..site_count)
            .map(|index| TemplateSlotSitePlan {
                site: RuntimeSlotSiteId(index),
                key: SlotKey::Default,
                render_plan: TemplateSlotSiteRenderPlan::default(),
                location: empty_location(),
            })
            .collect(),
    }
}

fn tir_slot_plan(site_count: usize) -> TemplateSlotPlan {
    let mut slot_plan = runtime_slot_plan(site_count);

    for index in 0..site_count {
        slot_plan.slot_sites.push(TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(index),
            key: SlotKey::Default,
            render_plan: TemplateSlotSiteRenderPlan::default(),
            location: empty_location(),
        });
    }

    slot_plan
}

// -------------------------
//  Well-Formed Store Tests
// -------------------------

#[test]
fn empty_store_is_valid() {
    let store = TemplateIrStore::new();
    assert!(validate_tir_store(&store).is_none());
}

#[test]
fn store_with_text_sequence_and_aggregate_output_roots_is_valid() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    // Ordinary template/root: a single text node as the template root.
    let text_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern("root"),
            byte_len: 4,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        text_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    // Sequence: a sequence root with two valid text children.
    let child_a = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern("a"),
            byte_len: 1,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));
    let child_b = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern("b"),
            byte_len: 1,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));
    let sequence_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![child_a, child_b],
        },
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        sequence_root,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    // Aggregate-output leaf: an aggregate-output marker as the template root.
    let aggregate_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::AggregateOutput,
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        aggregate_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    assert!(validate_tir_store(&store).is_none());
}

#[test]
fn loop_with_aggregate_wrapper_reference_is_valid() {
    use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;

    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    let aggregate_marker = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::AggregateOutput,
        empty_location(),
    ));

    let body = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern(""),
            byte_len: 0,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));

    let loop_header = TemplateLoopHeader::Conditional {
        condition: Box::new(bool_expression()),
    };
    let loop_header_sites = store.allocate_loop_header_expression_sites(&loop_header);
    let loop_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Loop {
            header: loop_header,
            header_sites: loop_header_sites,
            body,
            aggregate_wrapper: Some(aggregate_marker),
        },
        empty_location(),
    ));

    store.push_template(TemplateIr::new(
        loop_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    assert!(validate_tir_store(&store).is_none());
}

#[test]
fn runtime_slot_site_with_matching_plan_is_valid() {
    let mut store = TemplateIrStore::new();
    let plan_id = store.push_slot_plan(tir_slot_plan(2));

    let site_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::RuntimeSlotSite {
            plan: plan_id,
            site: RuntimeSlotSiteId(1),
        },
        empty_location(),
    ));

    let mut template = TemplateIr::new(
        site_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );
    template.runtime_slot_plan = Some(plan_id);
    store.push_template(template);

    assert!(validate_tir_store(&store).is_none());
}

// -------------------------
//  Invalid Root Tests
// -------------------------

#[test]
fn template_with_out_of_bounds_root_is_invalid() {
    let mut store = TemplateIrStore::new();

    // Push a template with a root that points beyond the nodes vector.
    store.templates.push(TemplateIr::new(
        TemplateIrNodeId::new(99),
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("out of bounds"));
}

// -------------------------
//  Invalid Node Reference Tests
// -------------------------

#[test]
fn sequence_with_out_of_bounds_child_is_invalid() {
    let mut store = TemplateIrStore::new();

    // Create a sequence that references a non-existent child.
    let sequence_id = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![TemplateIrNodeId::new(99)],
        },
        empty_location(),
    ));

    store.push_template(TemplateIr::new(
        sequence_id,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("out of bounds"));
}

#[test]
fn branch_chain_with_out_of_bounds_body_is_invalid() {
    let mut store = TemplateIrStore::new();

    let branch_site_id = store.next_expression_site_id();
    let chain_id = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::BranchChain {
            branches: vec![
                TemplateIrBranch::new(
                    TemplateBranchSelector::Bool(bool_expression()),
                    TemplateIrNodeId::new(99),
                    empty_location(),
                )
                .with_selector_site_id(branch_site_id),
            ],
            fallback: None,
        },
        empty_location(),
    ));

    store.push_template(TemplateIr::new(
        chain_id,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
}

#[test]
fn loop_with_out_of_bounds_body_is_invalid() {
    use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;

    let mut store = TemplateIrStore::new();

    let loop_header = TemplateLoopHeader::Conditional {
        condition: Box::new(bool_expression()),
    };
    let loop_header_sites = store.allocate_loop_header_expression_sites(&loop_header);
    let loop_id = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Loop {
            header: loop_header,
            header_sites: loop_header_sites,
            body: TemplateIrNodeId::new(99),
            aggregate_wrapper: None,
        },
        empty_location(),
    ));

    store.push_template(TemplateIr::new(
        loop_id,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
}

#[test]
fn template_with_out_of_bounds_wrapper_set_is_invalid() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    let node_id = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern("test"),
            byte_len: 4,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));

    let mut template = TemplateIr::new(
        node_id,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );
    template.conditional_child_wrapper_set = Some(super::super::ids::TemplateWrapperSetId::new(99));
    store.push_template(template);

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("wrapper set"));
    assert!(msg.contains("out of bounds"));
}

#[test]
fn template_with_out_of_bounds_slot_plan_is_invalid() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    let node_id = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern("test"),
            byte_len: 4,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));

    let mut template = TemplateIr::new(
        node_id,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );
    template.runtime_slot_plan = Some(TemplateSlotPlanId::new(99));
    store.push_template(template);

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("slot plan"));
    assert!(msg.contains("out of bounds"));
}

#[test]
fn runtime_slot_site_with_out_of_bounds_plan_is_invalid() {
    let mut store = TemplateIrStore::new();

    let site_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::RuntimeSlotSite {
            plan: TemplateSlotPlanId::new(99),
            site: RuntimeSlotSiteId(0),
        },
        empty_location(),
    ));

    store.push_template(TemplateIr::new(
        site_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("runtime slot site"));
    assert!(msg.contains("out of bounds"));
}

#[test]
fn runtime_slot_site_with_out_of_bounds_site_is_invalid() {
    let mut store = TemplateIrStore::new();
    let plan_id = store.push_slot_plan(tir_slot_plan(1));

    let site_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::RuntimeSlotSite {
            plan: plan_id,
            site: RuntimeSlotSiteId(2),
        },
        empty_location(),
    ));

    let mut template = TemplateIr::new(
        site_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );
    template.runtime_slot_plan = Some(plan_id);
    store.push_template(template);

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("runtime slot site"));
    assert!(msg.contains("site"));
}

#[test]
fn populated_slot_site_render_root_out_of_bounds_is_invalid() {
    let mut store = TemplateIrStore::new();
    let mut slot_plan = runtime_slot_plan(1);
    slot_plan.slot_sites.push(TemplateSlotSitePlan {
        site: RuntimeSlotSiteId(0),
        key: SlotKey::Default,
        render_plan: TemplateSlotSiteRenderPlan {
            pieces: vec![TemplateSlotSiteRenderPiece::Render(TemplateIrNodeId::new(
                99,
            ))],
        },
        location: empty_location(),
    });
    store.push_slot_plan(slot_plan);

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("slot plan"));
    assert!(msg.contains("render root"));
    assert!(msg.contains("out of bounds"));
}

// -------------------------
//  Occurrence and Site ID Validation Tests
// -------------------------

#[test]
fn store_with_unique_occurrence_and_site_ids_is_valid() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let text_id = string_table.intern("");

    // Two placeholder templates so child-template references resolve during
    // node reference validation.
    let placeholder_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: text_id,
            byte_len: 0,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        placeholder_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        placeholder_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    // Unique slot occurrence IDs (0 and 1) within one root.
    let slot_a = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot {
            placeholder: slot_placeholder(SlotKey::Default, SlotOccurrenceId::new(0)),
        },
        empty_location(),
    ));
    let slot_b = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot {
            placeholder: slot_placeholder(SlotKey::Default, SlotOccurrenceId::new(1)),
        },
        empty_location(),
    ));

    // Unique child-template occurrence IDs (0 and 1) referencing distinct templates.
    let child_a = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                TemplateIrId::new(0),
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            ),
            occurrence_id: ChildTemplateOccurrenceId::new(0),
        },
        empty_location(),
    ));
    let child_b = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                TemplateIrId::new(1),
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            ),
            occurrence_id: ChildTemplateOccurrenceId::new(1),
        },
        empty_location(),
    ));

    // Unique expression site IDs (0 and 1) within one root.
    let expr_a = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(bool_expression()),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: ExpressionSiteId::new(0),
        },
        empty_location(),
    ));
    let expr_b = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(bool_expression()),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: ExpressionSiteId::new(1),
        },
        empty_location(),
    ));

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![slot_a, slot_b, child_a, child_b, expr_a, expr_b],
        },
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    assert!(validate_tir_store(&store).is_none());
}

#[test]
fn store_with_duplicate_slot_occurrence_ids_is_invalid() {
    let mut store = TemplateIrStore::new();

    let slot_a = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot {
            placeholder: slot_placeholder(SlotKey::Default, SlotOccurrenceId::new(0)),
        },
        empty_location(),
    ));
    let slot_b = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot {
            placeholder: slot_placeholder(SlotKey::Default, SlotOccurrenceId::new(0)),
        },
        empty_location(),
    ));

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![slot_a, slot_b],
        },
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("duplicate slot occurrence"));
}

#[test]
fn duplicate_slot_occurrence_in_unreachable_history_is_valid() {
    let mut store = TemplateIrStore::new();

    let active_slot = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot {
            placeholder: slot_placeholder(SlotKey::Default, SlotOccurrenceId::new(0)),
        },
        empty_location(),
    ));

    // Append-only transforms may leave old nodes behind. A duplicate ID on an
    // unreachable historical node is not ambiguous for any active TirView.
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot {
            placeholder: slot_placeholder(SlotKey::Default, SlotOccurrenceId::new(0)),
        },
        empty_location(),
    ));

    store.push_template(TemplateIr::new(
        active_slot,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    assert!(validate_tir_store(&store).is_none());
}

#[test]
fn store_with_duplicate_child_template_occurrence_ids_is_invalid() {
    let mut store = TemplateIrStore::new();

    let mut string_table = StringTable::new();
    let text_id = string_table.intern("");

    let placeholder_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: text_id,
            byte_len: 0,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        placeholder_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let child_a_reference = TemplateTirChildReference::new(
        TemplateIrId::new(0),
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let child_a = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: child_a_reference,
            occurrence_id: ChildTemplateOccurrenceId::new(0),
        },
        empty_location(),
    ));
    let child_b_reference = TemplateTirChildReference::new(
        TemplateIrId::new(0),
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let child_b = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: child_b_reference,
            occurrence_id: ChildTemplateOccurrenceId::new(0),
        },
        empty_location(),
    ));

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![child_a, child_b],
        },
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("duplicate child-template occurrence"));
}

#[test]
fn store_with_duplicate_expression_site_ids_is_invalid() {
    let mut store = TemplateIrStore::new();

    let expr_a = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(bool_expression()),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: ExpressionSiteId::new(0),
        },
        empty_location(),
    ));
    let expr_b = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(bool_expression()),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: ExpressionSiteId::new(0),
        },
        empty_location(),
    ));

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![expr_a, expr_b],
        },
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("duplicate expression site"));
}

#[test]
fn duplicate_expression_sites_in_separate_template_roots_are_valid() {
    let mut store = TemplateIrStore::new();

    let expr_a = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(bool_expression()),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: ExpressionSiteId::new(0),
        },
        empty_location(),
    ));
    let expr_b = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(bool_expression()),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: ExpressionSiteId::new(0),
        },
        empty_location(),
    ));

    // Derived roots can preserve a site ID while the old root remains in the
    // append-only store. Each root is independently unambiguous.
    store.push_template(TemplateIr::new(
        expr_a,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        expr_b,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    assert!(validate_tir_store(&store).is_none());
}

#[test]
fn store_with_duplicate_expression_site_across_expression_and_branch_is_invalid() {
    let mut store = TemplateIrStore::new();

    let mut string_table = StringTable::new();

    // A DynamicExpression with site 0.
    let expr_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(bool_expression()),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: ExpressionSiteId::new(0),
        },
        empty_location(),
    ));

    // A branch body node.
    let branch_body = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern(""),
            byte_len: 0,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));

    // A BranchChain whose selector shares site 0 with the DynamicExpression.
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(bool_expression()),
        branch_body,
        empty_location(),
    )
    .with_selector_site_id(ExpressionSiteId::new(0));

    let branch_chain = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::BranchChain {
            branches: vec![branch],
            fallback: None,
        },
        empty_location(),
    ));

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![expr_node, branch_chain],
        },
        empty_location(),
    ));
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("duplicate expression site"));
}

// -------------------------
//  Wrapper Set Template Ref Validation Tests
// -------------------------

#[test]
fn store_with_valid_wrapper_set_refs_is_valid() {
    let mut store = TemplateIrStore::new();

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        empty_location(),
    ));
    let wrapper_template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));

    let wrapper_ref = wrapper_template_id;
    store.push_wrapper_set(TemplateWrapperSet {
        wrappers: vec![TemplateWrapperReference::new(
            wrapper_ref,
            TemplateTirPhase::Finalized,
            TemplateViewContext::default(),
        )],
    });

    assert!(validate_tir_store(&store).is_none());
}

#[test]
fn wrapper_set_with_out_of_bounds_template_ref_is_invalid() {
    let mut store = TemplateIrStore::new();

    // Create a TemplateIrId pointing to a template that does not exist.
    let stale_ref = TemplateIrId::new(99);
    store.push_wrapper_set(TemplateWrapperSet {
        wrappers: vec![TemplateWrapperReference::new(
            stale_ref,
            TemplateTirPhase::Finalized,
            TemplateViewContext::default(),
        )],
    });

    let diagnostic = validate_tir_store(&store);
    assert!(diagnostic.is_some());
    let msg = format!("{:?}", diagnostic.unwrap());
    assert!(msg.contains("out of bounds"));
}
