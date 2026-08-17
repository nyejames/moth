use super::super::ids::{TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId};
use super::super::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder,
};
use super::super::overlays::{
    TemplateViewContext, TirSlotResolution, TirSlotResolutionOverlay, TirWrapperContext,
    TirWrapperContextOverlay,
};
use super::super::refs::{TemplateTirChildReference, TemplateWrapperReference};
use super::super::slot_plan::{
    TemplateSlotContributionSourcePlan, TemplateSlotPlan, TemplateSlotSitePlan,
};
use super::super::store::{ControlFlowBodyKind, TemplateIrStore};
use super::super::summary::TemplateIrSummary;
use super::super::view::TemplateTirPhase;
use super::builder::TemplateIrBuilder;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateBranchSelector;
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::ast::templates::tir::copy_state::TirCopyState;
use crate::compiler_frontend::ast::templates::tir::ids::SlotOccurrenceId;
use crate::compiler_frontend::ast::templates::tir::slot_plan::convert_tir_tree_to_active_slot_plan;
use crate::compiler_frontend::ast::templates::tir::store::MalformedTirStore;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn build_finalized_tir_template(store: &mut TemplateIrStore) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let root = builder.push_sequence_node(vec![], empty_location());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    )
}

fn empty_sequence(store: &mut TemplateIrStore) -> TemplateIrNodeId {
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        empty_location(),
    ))
}

fn runtime_slot_plan(render_root: TemplateIrNodeId) -> TemplateSlotPlan {
    TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![],
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root,
            location: empty_location(),
        }],
    }
}

fn bool_selector() -> TemplateBranchSelector {
    TemplateBranchSelector::Bool(Expression {
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
    })
}

#[test]
fn store_starts_empty() {
    let store = TemplateIrStore::new();
    assert_eq!(store.template_count(), 0);
    assert_eq!(store.node_count(), 0);
}

#[test]
fn push_returns_sequential_ids_per_collection() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    // Nodes allocate sequential TemplateIrNodeIds from their own index space.
    let node_a = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern("abc"),
            byte_len: 3,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));
    let node_b = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        empty_location(),
    ));
    assert_eq!(node_a.index(), 0);
    assert_eq!(node_b.index(), 1);
    assert_eq!(store.node_count(), 2);

    // Templates allocate sequential TemplateIrIds from a separate index space.
    let template_a = store.push_template(TemplateIr::new(
        node_a,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    let template_b = store.push_template(TemplateIr::new(
        node_a,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    assert_eq!(template_a.index(), 0);
    assert_eq!(template_b.index(), 1);
    assert_eq!(store.template_count(), 2);
}

#[test]
fn typed_retrieval_returns_stored_entry() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    // Template: round-trips the root node id through get_template.
    let node_id = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern(""),
            byte_len: 0,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        node_id,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    let retrieved_template = store
        .get_template(template_id)
        .expect("template should exist");
    assert_eq!(retrieved_template.root, node_id);

    // Node: round-trips the exact node kind through get_node.
    let sequence_node_id = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        empty_location(),
    ));
    let retrieved_node = store.get_node(sequence_node_id).expect("node should exist");
    assert!(matches!(
        retrieved_node.kind,
        TemplateIrNodeKind::Sequence { .. }
    ));

    // Wrapper set: round-trips effective wrapper refs through get_wrapper_set.
    let wrapper_root = build_finalized_tir_template(&mut store);
    let wrapper_set_id = store.push_wrapper_set(super::super::store::TemplateWrapperSet {
        wrappers: vec![TemplateWrapperReference::new(
            wrapper_root,
            TemplateTirPhase::Finalized,
            TemplateViewContext::default(),
        )],
    });
    let retrieved_wrapper_set = store
        .get_wrapper_set(wrapper_set_id)
        .expect("wrapper set should exist");
    assert_eq!(retrieved_wrapper_set.wrappers.len(), 1);
    assert_eq!(retrieved_wrapper_set.wrappers[0].root, wrapper_root);

    // Slot plan: round-trips the routing plan through get_slot_plan.
    let slot_plan_id = store.push_slot_plan(runtime_slot_plan(node_id));
    let retrieved_slot_plan = store
        .get_slot_plan(slot_plan_id)
        .expect("slot plan should exist");
    assert_eq!(retrieved_slot_plan.location, empty_location());
    assert!(retrieved_slot_plan.contribution_sources.is_empty());
    assert_eq!(retrieved_slot_plan.slot_sites.len(), 1);
    assert_eq!(retrieved_slot_plan.slot_sites[0].site, RuntimeSlotSiteId(0));
    assert_eq!(retrieved_slot_plan.slot_sites[0].key, SlotKey::Default);
    assert_eq!(
        retrieved_slot_plan.slot_sites[0].render_root,
        TemplateIrNodeId::new(0)
    );
    assert_eq!(retrieved_slot_plan.slot_sites[0].location, empty_location());
}

#[test]
fn out_of_bounds_lookup_returns_none() {
    let store = TemplateIrStore::new();
    assert!(
        store
            .get_template(super::super::ids::TemplateIrId::new(99))
            .is_none()
    );
    assert!(
        store
            .get_node(super::super::ids::TemplateIrNodeId::new(99))
            .is_none()
    );
    assert!(
        store
            .get_wrapper_set(super::super::ids::TemplateWrapperSetId::new(99))
            .is_none()
    );
    assert!(
        store
            .get_slot_plan(super::super::ids::TemplateSlotPlanId::new(99))
            .is_none()
    );
}

#[test]
fn push_or_reuse_wrapper_set_reuses_equivalent_empty_set() {
    let mut store = TemplateIrStore::new();

    let id_a = store.push_or_reuse_wrapper_set(vec![]);
    let id_b = store.push_or_reuse_wrapper_set(vec![]);

    assert_eq!(id_a, id_b, "empty wrapper vectors should be reused");
    assert_eq!(store.wrapper_set_count(), 1);
}

#[test]
fn push_or_reuse_wrapper_set_creates_new_for_different_lengths() {
    let mut store = TemplateIrStore::new();

    let wrapper_id = build_finalized_tir_template(&mut store);

    let id_a = store.push_or_reuse_wrapper_set(vec![]);
    let id_b = store.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
        wrapper_id,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )]);

    assert_ne!(
        id_a, id_b,
        "wrapper sets with different lengths should not be reused"
    );
    assert_eq!(store.wrapper_set_count(), 2);
}

#[test]
fn push_or_reuse_wrapper_set_reuses_same_template_id() {
    let mut store = TemplateIrStore::new();
    let template_id = build_finalized_tir_template(&mut store);

    let id_a = store.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
        template_id,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )]);
    let id_b = store.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
        template_id,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )]);

    assert_eq!(
        id_a, id_b,
        "wrapper sets referencing the same TemplateIrId should reuse one wrapper set"
    );
    assert_eq!(store.wrapper_set_count(), 1);
}

#[test]
fn push_or_reuse_wrapper_set_does_not_reuse_different_template_ids() {
    let mut store = TemplateIrStore::new();

    let wrapper_a = build_finalized_tir_template(&mut store);
    let wrapper_b = build_finalized_tir_template(&mut store);

    let id_a = store.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
        wrapper_a,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )]);
    let id_b = store.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
        wrapper_b,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )]);

    assert_ne!(
        id_a, id_b,
        "wrapper sets referencing different TemplateIrIds should not reuse"
    );
    assert_eq!(store.wrapper_set_count(), 2);
}

#[test]
fn reserved_slot_plan_is_invisible_until_commit() {
    let mut store = TemplateIrStore::new();
    let render_root = empty_sequence(&mut store);
    let reserved = store.reserve_slot_plan();

    assert!(
        store.get_slot_plan(reserved).is_none(),
        "a reserved plan must not be visible through ordinary lookup"
    );
    assert_eq!(store.slot_plan_index_count(), 1);

    store
        .commit_slot_plan(reserved, runtime_slot_plan(render_root))
        .expect("a reserved plan can be committed once");
    assert!(store.get_slot_plan(reserved).is_some());
}

#[test]
fn slot_plan_cannot_be_committed_twice() {
    let mut store = TemplateIrStore::new();
    let render_root = empty_sequence(&mut store);
    let reserved = store.reserve_slot_plan();
    store
        .commit_slot_plan(reserved, runtime_slot_plan(render_root))
        .expect("first commit should succeed");

    let error = store
        .commit_slot_plan(reserved, runtime_slot_plan(render_root))
        .expect_err("a committed plan cannot be committed again");
    assert!(error.msg.contains("committed more than once"));
}

#[test]
fn checked_template_kind_write_rejects_missing_id() {
    let mut store = TemplateIrStore::new();
    let error = store
        .set_template_kind(TemplateIrId::new(3), TemplateType::String)
        .expect_err("missing templates cannot receive a kind write");
    assert!(error.msg.contains("no template"));
}

#[test]
fn overlay_allocation_rejects_duplicate_and_out_of_range_keys() {
    let mut store = TemplateIrStore::new();
    let occurrence = store.next_child_template_occurrence_id();
    let duplicate = store.allocate_wrapper_context_overlay(TirWrapperContextOverlay {
        contexts: vec![
            (occurrence, TirWrapperContext::default()),
            (occurrence, TirWrapperContext::default()),
        ],
    });
    assert!(
        duplicate
            .expect_err("duplicate overlay keys must be rejected")
            .msg
            .contains("duplicate child-template-occurrence")
    );

    let out_of_range = store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
        resolutions: vec![(
            SlotOccurrenceId::new(0),
            TirSlotResolution::missing(SlotKey::Default),
        )],
    });
    assert!(
        out_of_range
            .expect_err("unallocated overlay keys must be rejected")
            .msg
            .contains("out-of-range slot-occurrence")
    );
}

#[test]
fn slot_placeholder_lookup_uses_the_store_not_raw_vectors() {
    let mut store = TemplateIrStore::new();
    let occurrence = store.next_slot_occurrence_id();
    let placeholder = TirSlotPlaceholder::with_wrapper_sets(
        SlotKey::Default,
        occurrence,
        empty_location(),
        None,
        None,
        false,
    );
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot { placeholder },
        empty_location(),
    ));

    let found = store
        .slot_placeholder(occurrence)
        .expect("the store must find the unique slot occurrence");
    assert_eq!(found.occurrence_id, occurrence);
    assert_eq!(found.key, SlotKey::Default);
}

#[test]
fn control_flow_body_replacement_rejects_missing_owner() {
    let mut store = TemplateIrStore::new();
    let replacement = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        empty_location(),
    ));
    let error = store
        .replace_control_flow_body(
            TemplateIrNodeId::new(99),
            ControlFlowBodyKind::LoopBody,
            replacement,
        )
        .expect_err("a missing control-flow node cannot receive a body");
    assert!(error.msg.contains("no node"));
}

#[test]
fn slot_plan_commit_rejects_source_id_that_differs_from_index() {
    let mut store = TemplateIrStore::new();
    let render_root = empty_sequence(&mut store);
    let reserved = store.reserve_slot_plan();
    let error = store
        .commit_slot_plan(
            reserved,
            TemplateSlotPlan {
                location: empty_location(),
                contribution_sources: vec![TemplateSlotContributionSourcePlan {
                    source: RuntimeSlotContributionSourceId(1),
                    target: SlotKey::Default,
                    render_root,
                    renders_wrapper_unconditionally: false,
                    location: empty_location(),
                }],
                slot_sites: vec![],
            },
        )
        .expect_err("source IDs must match their vector index");
    assert!(error.msg.contains("contribution source"));
    assert!(store.get_slot_plan(reserved).is_none());
}

#[test]
fn slot_plan_commit_rejects_site_id_that_differs_from_index() {
    let mut store = TemplateIrStore::new();
    let render_root = empty_sequence(&mut store);
    let reserved = store.reserve_slot_plan();
    let error = store
        .commit_slot_plan(
            reserved,
            TemplateSlotPlan {
                location: empty_location(),
                contribution_sources: vec![],
                slot_sites: vec![TemplateSlotSitePlan {
                    site: RuntimeSlotSiteId(1),
                    key: SlotKey::Default,
                    render_root,
                    location: empty_location(),
                }],
            },
        )
        .expect_err("site IDs must match their vector index");
    assert!(error.msg.contains("slot-plan site"));
    assert!(store.get_slot_plan(reserved).is_none());
}

#[test]
fn slot_plan_commit_rejects_missing_source_render_root() {
    let mut store = TemplateIrStore::new();
    let reserved = store.reserve_slot_plan();
    let error = store
        .commit_slot_plan(
            reserved,
            TemplateSlotPlan {
                location: empty_location(),
                contribution_sources: vec![TemplateSlotContributionSourcePlan {
                    source: RuntimeSlotContributionSourceId(0),
                    target: SlotKey::Default,
                    render_root: TemplateIrNodeId::new(99),
                    renders_wrapper_unconditionally: false,
                    location: empty_location(),
                }],
                slot_sites: vec![],
            },
        )
        .expect_err("source render roots must exist");
    assert!(error.msg.contains("missing render root"));
    assert!(store.get_slot_plan(reserved).is_none());
}

#[test]
fn slot_plan_commit_rejects_missing_site_render_root() {
    let mut store = TemplateIrStore::new();
    let reserved = store.reserve_slot_plan();
    let error = store
        .commit_slot_plan(reserved, runtime_slot_plan(TemplateIrNodeId::new(99)))
        .expect_err("site render roots must exist");
    assert!(error.msg.contains("missing render root"));
    assert!(store.get_slot_plan(reserved).is_none());
}

#[test]
fn conversion_failure_leaves_reserved_plan_invisible() {
    let mut store = TemplateIrStore::new();
    let occurrence = store.next_slot_occurrence_id();
    let slot = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Slot {
            placeholder: TirSlotPlaceholder::with_wrapper_sets(
                SlotKey::Default,
                occurrence,
                empty_location(),
                None,
                None,
                false,
            ),
        },
        empty_location(),
    ));
    let reserved = store.reserve_slot_plan();
    let mut copy_state = TirCopyState::new();

    convert_tir_tree_to_active_slot_plan(slot, reserved, &[], &mut store, &mut copy_state)
        .expect_err("conversion without matching sites must fail");
    assert!(store.get_slot_plan(reserved).is_none());
}

#[test]
fn active_slot_conversion_publishes_derived_child_and_rewrites_reference() {
    let mut store = TemplateIrStore::new();
    let location = empty_location();
    let (child_template_id, child_root) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let occurrence = builder.store.next_slot_occurrence_id();
        let slot = builder.push_tir_slot_placeholder_node(TirSlotPlaceholder::with_wrapper_sets(
            SlotKey::Default,
            occurrence,
            location.clone(),
            None,
            None,
            false,
        ));
        let root = builder.push_sequence_node(vec![slot], location.clone());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            location.clone(),
        );
        (template_id, root)
    };
    let wrapper_set = store.push_or_reuse_wrapper_set(Vec::new());
    store
        .set_conditional_child_wrapper_set(child_template_id, wrapper_set)
        .expect("child wrapper metadata should attach");

    let parent_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let child_node = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                child_template_id,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            location.clone(),
        );
        builder.push_sequence_node(vec![child_node], location)
    };

    let render_root = empty_sequence(&mut store);
    let reserved = store.reserve_slot_plan();
    let slot_sites = vec![TemplateSlotSitePlan {
        site: RuntimeSlotSiteId(0),
        key: SlotKey::Default,
        render_root,
        location: empty_location(),
    }];
    let mut copy_state = TirCopyState::new();

    convert_tir_tree_to_active_slot_plan(
        parent_root,
        reserved,
        &slot_sites,
        &mut store,
        &mut copy_state,
    )
    .expect("active slot conversion should publish a derived child version");

    let parent_child_template_id = match &store
        .get_node(parent_root)
        .expect("parent root exists")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => match &store
            .get_node(children[0])
            .expect("parent child node exists")
            .kind
        {
            TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
            other => panic!("expected child-template node, got {other:?}"),
        },
        other => panic!("expected parent sequence root, got {other:?}"),
    };

    assert_ne!(
        parent_child_template_id, child_template_id,
        "slot conversion should publish a derived child template"
    );
    assert_eq!(
        store
            .get_template(child_template_id)
            .expect("original child exists")
            .root,
        child_root,
        "slot conversion must not replace the original child root"
    );
    assert_eq!(
        store
            .get_template(parent_child_template_id)
            .expect("derived child exists")
            .conditional_child_wrapper_set,
        Some(wrapper_set),
        "derived child must preserve wrapper side-table links"
    );
}

#[test]
fn reserved_plan_cannot_be_attached_to_a_template() {
    let mut store = TemplateIrStore::new();
    let template_id = build_finalized_tir_template(&mut store);
    let reserved = store.reserve_slot_plan();
    let error = store
        .attach_runtime_slot_plan(template_id, reserved)
        .expect_err("reserved plans are not attachable");
    assert!(error.msg.contains("uncommitted"));
}

#[test]
fn reserved_plan_is_invisible_to_preparation_lookup() {
    let mut store = TemplateIrStore::new();
    let reserved = store.reserve_slot_plan();
    assert!(store.get_slot_plan(reserved).is_none());
    assert!(store.get_slot_plan(TemplateSlotPlanId::new(99)).is_none());
}

#[test]
fn reactive_subscription_rejects_non_text_node() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let sequence = empty_sequence(&mut store);
    let source = ReactiveSource {
        path: InternedPath::from_single_str("main.moth/#reactive", &mut string_table),
        kind: ReactiveSourceKind::Declaration,
    };
    let error = store
        .set_node_reactive_subscription(
            sequence,
            ReactiveSubscription {
                source,
                type_id: builtin_type_ids::STRING,
                location: empty_location(),
            },
        )
        .expect_err("only text nodes accept reactive subscriptions");
    assert!(error.msg.contains("non-text"));
}

#[test]
fn reactive_subscription_read_rejects_truncated_side_table() {
    let mut store = TemplateIrStore::new();
    let node = empty_sequence(&mut store);
    MalformedTirStore::new(&mut store).truncate_reactive_side_table();

    let error = store
        .node_reactive_subscription(node)
        .expect_err("a missing aligned side-table entry is malformed store state");
    assert!(error.msg.contains("reactive side table is missing"));
}

#[test]
fn control_flow_lookup_reports_missing_node_as_error() {
    let store = TemplateIrStore::new();
    let error = store
        .control_flow_node_id_in_subtree(TemplateIrNodeId::new(99))
        .expect_err("missing nodes are compiler errors");
    assert!(error.msg.contains("no node"));
}

#[test]
fn control_flow_lookup_reports_missing_forwarding_template_as_error() {
    let mut store = TemplateIrStore::new();
    let occurrence_id = store.next_child_template_occurrence_id();
    let child = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                TemplateIrId::new(99),
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            ),
            occurrence_id,
        },
        empty_location(),
    ));
    let sequence = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![child],
        },
        empty_location(),
    ));
    let error = store
        .control_flow_node_id_in_subtree(sequence)
        .expect_err("missing forwarding templates are compiler errors");
    assert!(error.msg.contains("no forwarding template"));
}

#[test]
fn control_flow_lookup_reports_cycle_as_error() {
    let mut store = TemplateIrStore::new();
    let sequence = empty_sequence(&mut store);
    MalformedTirStore::new(&mut store).set_node_kind(
        sequence,
        TemplateIrNodeKind::Sequence {
            children: vec![sequence],
        },
    );
    let error = store
        .control_flow_node_id_in_subtree(sequence)
        .expect_err("cycles are compiler errors");
    assert!(error.msg.contains("cycle"));
}

#[test]
fn derived_publication_rejects_unknown_source() {
    let mut store = TemplateIrStore::new();
    let root = empty_sequence(&mut store);
    let error = store
        .push_structurally_derived_template(
            TemplateIrId::new(99),
            root,
            crate::compiler_frontend::ast::templates::tir::DerivedTemplateMetadata::preserve_source(
            ),
        )
        .expect_err("unknown sources cannot be derivation authority");
    assert!(error.msg.contains("unknown source"));
}

#[test]
fn derived_publication_preserves_source_metadata() {
    let mut store = TemplateIrStore::new();
    let mut style = Style::default();
    style.skip_parent_child_wrappers = true;
    let source_root = empty_sequence(&mut store);
    let source = store.push_template(TemplateIr::new(
        source_root,
        style.clone(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    let wrapper_set = store.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
        source,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )]);
    store
        .set_conditional_child_wrapper_set(source, wrapper_set)
        .expect("wrapper set should attach");
    let plan = store.push_slot_plan(runtime_slot_plan(source_root));
    store
        .attach_runtime_slot_plan(source, plan)
        .expect("committed plan should attach");

    let new_root = empty_sequence(&mut store);
    let derived = store
        .push_structurally_derived_template(
            source,
            new_root,
            crate::compiler_frontend::ast::templates::tir::DerivedTemplateMetadata::preserve_source(
            ),
        )
        .expect("known sources can be derived");
    let derived_template = store
        .get_template(derived)
        .expect("derived template exists");

    assert_eq!(derived_template.root, new_root);
    assert_eq!(derived_template.summary.text_byte_count, 0);
    assert!(derived_template.style.skip_parent_child_wrappers);
    assert_eq!(derived_template.kind, TemplateType::StringFunction);
    assert_eq!(derived_template.location, empty_location());
    assert_eq!(
        derived_template.conditional_child_wrapper_set,
        Some(wrapper_set)
    );
    assert_eq!(derived_template.runtime_slot_plan, Some(plan));
}

#[test]
fn branch_construction_requires_an_allocated_selector_site() {
    let mut store = TemplateIrStore::new();
    let body = empty_sequence(&mut store);
    let site = store.next_expression_site_id();
    let branch = TemplateIrBranch::new(bool_selector(), body, empty_location(), site);
    assert_eq!(branch.selector_site_id, site);

    let mut builder = TemplateIrBuilder::new(&mut store);
    let node = builder.push_branch_chain_node(vec![branch], None, empty_location());
    let TemplateIrNodeKind::BranchChain { branches, .. } = &store.get_node(node).unwrap().kind
    else {
        panic!("expected a branch chain");
    };
    assert_eq!(branches[0].selector_site_id, site);
}
