//! Focused unit tests for runtime slot-site node authority.
//!
//! WHAT: protects wrapper-reference identity, plan-qualified contribution
//!       markers and independent copied expression sites. Integration cases
//!       own visible output.
//! WHY: these facts live in the TIR store and cannot be inspected from
//!      rendered strings.

use super::super::types::RuntimeSlotContributionSourceDraft;
use super::{RuntimeSlotInjection, RuntimeWrapperSitePlanBuilder, inject_runtime_slot_fill};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::TemplateSegmentOrigin;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotContributionSourceId;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::{
    ContributionShape, ExpressionSiteId, TemplateIr, TemplateIrBuilder, TemplateIrId,
    TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore, TemplateIrSummary,
    TemplateSlotContributionSourcePlan, TemplateSlotPlan, TemplateSlotPlanId, TemplateTirPhase,
    TemplateViewContext, TirCopyState, TirSlotPlaceholder, TirSlotResolutionOverlay, TirView,
    TirWrapperApplicationMode, TirWrapperContext, TirWrapperContextOverlay,
    collect_tir_slot_layout_from_root, compose_child_decorator_wrapper,
    push_runtime_slot_contribution_source, summarize_existing_root,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::value_mode::ValueMode;

use rustc_hash::FxHashMap;

fn bool_expression(
    expressions: &mut AstExpressionBuilder,
    values: &mut ConstValueStore,
    value: bool,
) -> ExprId {
    expressions
        .bool(values, value, None, ValueMode::ImmutableOwned)
        .expect("test bool is a valid single completed expression")
        .id()
}

fn assert_authority_error(
    result: Result<TemplateIrNodeId, TemplateError>,
    context: &str,
    owner_marker: &str,
) {
    let error =
        result.expect_err(format!("{context} must surface as a broken-authority error").as_str());
    match error {
        TemplateError::Infrastructure(error) => {
            let error = &error.primary;
            assert!(
                error.message.contains(owner_marker),
                "{context} should be rejected by the {owner_marker} owner, got: {}",
                error.message,
            );
        }
        TemplateError::Diagnostic(_)
        | TemplateError::EmptyArguments(_)
        | TemplateError::NonConstantReferenceInConstant(_) => {
            panic!("{context} must be an infrastructure error, not a user diagnostic",);
        }
    }
}

fn push_slot_plan(store: &mut TemplateIrStore) -> TemplateSlotPlanId {
    store.push_slot_plan(TemplateSlotPlan {
        contribution_sources: vec![],
        slot_sites: vec![],
    })
}

fn contribution_fill(store: &mut TemplateIrStore, plan: TemplateSlotPlanId) -> TemplateIrNodeId {
    push_runtime_slot_contribution_source(
        store,
        plan,
        RuntimeSlotContributionSourceId(0),
        ContributionShape::Other,
        false,
        None,
    )
}

fn build_slot_text_wrapper(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    before: &str,
    after: &str,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let before_node = builder.push_text_node(
        string_table.intern(before),
        before.len(),
        TemplateSegmentOrigin::Body,
        None,
    );
    let slot_node = builder.push_slot_node(SlotKey::Default, None);
    let after_node = builder.push_text_node(
        string_table.intern(after),
        after.len(),
        TemplateSegmentOrigin::Body,
        None,
    );
    let root = builder.push_sequence_node(vec![before_node, slot_node, after_node], None);
    let summary = summarize_existing_root(builder.store, root).expect("valid wrapper summary");
    builder.finish_template(root, Style::default(), TemplateType::String, summary, None)
}

#[test]
fn slot_site_without_routed_source_carries_occurrence_default() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let default = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Text(string_table.intern("fallback"))),
            None,
        )
        .expect("test default inserts");

    let wrapper_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot = builder.push_slot_node_with_default(SlotKey::Default, default, None);
        builder.finish_template(
            slot,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let wrapper_root = store
        .get_template(wrapper_id)
        .expect("wrapper template exists")
        .root;
    let plan = push_slot_plan(&mut store);
    let mut copy_state = TirCopyState::new();
    let sites = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    }
    .build_slot_sites(wrapper_root)
    .expect("default-only wrapper should plan its site");

    assert_eq!(sites.len(), 1);
    match &store
        .get_node(sites[0].render_root)
        .expect("site render root exists")
        .kind
    {
        TemplateIrNodeKind::Slot { placeholder } => {
            assert_eq!(placeholder.key, SlotKey::Default);
            assert_eq!(
                placeholder.default,
                Some(default),
                "the site root must retain this occurrence's own fallback"
            );
        }
        _ => panic!("an unrouted default site must carry its occurrence fallback"),
    }
}

#[test]
fn routed_source_suppresses_slot_site_default() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let default = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Text(string_table.intern("fallback"))),
            None,
        )
        .expect("test default inserts");

    let wrapper_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot = builder.push_slot_node_with_default(SlotKey::Default, default, None);
        builder.finish_template(
            slot,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let wrapper_root = store
        .get_template(wrapper_id)
        .expect("wrapper template exists")
        .root;
    let plan = push_slot_plan(&mut store);
    let source_root = contribution_fill(&mut store, plan);
    let sources = vec![RuntimeSlotContributionSourceDraft {
        source: TemplateSlotContributionSourcePlan {
            source: RuntimeSlotContributionSourceId(0),
            target: SlotKey::Default,
            render_root: source_root,
            renders_wrapper_unconditionally: false,
            span: None,
        },
        shape: ContributionShape::Other,
        control_flow: false,
    }];
    let mut copy_state = TirCopyState::new();
    let sites = RuntimeWrapperSitePlanBuilder {
        sources: &sources,
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    }
    .build_slot_sites(wrapper_root)
    .expect("routed wrapper should plan its site");

    assert_eq!(sites.len(), 1);
    assert!(
        node_contains_contribution(sites[0].render_root, &store, plan),
        "a routed source must replace the occurrence default even when it emits empty"
    );
    assert!(
        !matches!(
            store
                .get_node(sites[0].render_root)
                .expect("site render root exists")
                .kind,
            TemplateIrNodeKind::Slot { .. }
        ),
        "the default fallback must not survive a routed contribution"
    );
}

fn build_plain_text_wrapper(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let text_node = builder.push_text_node(
        string_table.intern(text),
        text.len(),
        TemplateSegmentOrigin::Body,
        None,
    );
    builder.finish_template(
        text_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    )
}

fn node_contains_contribution(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    plan: TemplateSlotPlanId,
) -> bool {
    node_contains_runtime_source(node_id, store, plan, RuntimeSlotContributionSourceId(0))
}

fn node_contains_runtime_source(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    plan: TemplateSlotPlanId,
    source: RuntimeSlotContributionSourceId,
) -> bool {
    let Some(node) = store.get_node(node_id) else {
        return false;
    };

    match &node.kind {
        TemplateIrNodeKind::RuntimeSlotContributionSource {
            plan: marker_plan,
            source: marker_source,
            ..
        } => *marker_plan == plan && *marker_source == source,
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .any(|child| node_contains_runtime_source(*child, store, plan, source)),
        TemplateIrNodeKind::Conditional { body, .. } => {
            node_contains_runtime_source(*body, store, plan, source)
        }
        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            node_contains_runtime_source(*body, store, plan, source)
                || aggregate_wrapper.is_some_and(|wrapper| {
                    node_contains_runtime_source(wrapper, store, plan, source)
                })
        }
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            store.get_template(reference.root).is_some_and(|template| {
                node_contains_runtime_source(template.root, store, plan, source)
            })
        }
        _ => false,
    }
}

fn derived_wrapper_from_fill(
    store: &TemplateIrStore,
    fill_root: TemplateIrNodeId,
) -> (&TemplateTirChildReference, &TemplateIr) {
    let node = store
        .get_node(fill_root)
        .expect("wrapper application should produce a node");
    let TemplateIrNodeKind::ChildTemplate { reference, .. } = &node.kind else {
        panic!("wrapper application must reference a derived wrapper, got {node:?}");
    };
    let template = store
        .get_template(reference.root)
        .expect("derived wrapper template should exist");
    (reference, template)
}

#[test]
fn missing_same_store_child_template_is_an_authority_error() {
    let mut store = TemplateIrStore::new();
    let same_store_missing_template = TemplateTirChildReference::new(
        TemplateIrId::new(99),
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let mut builder = TemplateIrBuilder::new(&mut store);
    let child_node =
        builder.push_child_template_node_with_reference(same_store_missing_template, None);
    let plan = push_slot_plan(&mut store);
    let fill_root = contribution_fill(&mut store, plan);
    let mut copy_state = TirCopyState::new();

    let result = inject_runtime_slot_fill(
        child_node,
        &[RuntimeSlotInjection {
            key: SlotKey::Default,
            fill_root,
        }],
        None,
        None,
        None,
        &mut store,
        &mut copy_state,
    );

    assert_authority_error(
        result.map(|injection| injection.root),
        "missing same-store child template",
        "Runtime slot site planning",
    );
}

#[test]
fn missing_structural_authority_propagates_through_runtime_slot_site_planner() {
    let mut store = TemplateIrStore::new();
    let mut copy_state = TirCopyState::new();
    let fill_root = contribution_fill(&mut store, TemplateSlotPlanId::new(0));
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let string_table = StringTable::new();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: TemplateSlotPlanId::new(0),
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    };

    let result = planner.apply_wrapper_reference(
        TemplateWrapperReference::new(
            TemplateIrId::new(0),
            TemplateTirPhase::Finalized,
            TemplateViewContext::default(),
        ),
        fill_root,
    );
    let error = result.expect_err(
        "runtime slot planning must propagate missing structural authority as an error",
    );

    match error {
        TemplateError::Infrastructure(error) => {
            assert_eq!(error.primary.kind, InfrastructureFailureKind::Internal);
        }
        TemplateError::Diagnostic(_)
        | TemplateError::EmptyArguments(_)
        | TemplateError::NonConstantReferenceInConstant(_) => {
            panic!("missing structural authority must not become a user diagnostic");
        }
    }
}

#[test]
fn wrapper_application_clears_consumed_slot_resolution_context() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let wrapper_id = build_slot_text_wrapper(&mut store, &mut string_table, "before", "after");
    let overlay = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        slot_resolution: Some(overlay),
        ..TemplateViewContext::default()
    };
    let wrapper_ref = TemplateWrapperReference::new(wrapper_id, TemplateTirPhase::Parsed, context);
    let plan = push_slot_plan(&mut store);
    let fill_root = contribution_fill(&mut store, plan);
    let mut copy_state = TirCopyState::new();
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    };

    let applied = planner
        .apply_wrapper_reference(wrapper_ref, fill_root)
        .expect("wrapper with a slot-resolution context should apply");
    let (reference, derived) = derived_wrapper_from_fill(&store, applied);

    assert_eq!(reference.phase, TemplateTirPhase::Composed);
    assert_eq!(reference.context.expression_overlay, None);
    assert_eq!(reference.context.slot_resolution, None);
    assert_eq!(reference.context.wrapper_context, None);
    assert!(matches!(derived.kind, TemplateType::String));
    assert!(node_contains_contribution(applied, &store, plan));
}

#[test]
fn slotless_runtime_wrapper_preserves_complete_reference_identity() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let wrapper_id = build_plain_text_wrapper(&mut store, &mut string_table, "prefix");
    let expression_overlay = store
        .allocate_expression_overlay(Default::default())
        .expect("test overlay allocation");
    let slot_overlay = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("test overlay allocation");
    let wrapper_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay::default())
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: Some(slot_overlay),
        wrapper_context: Some(wrapper_overlay),
    };
    let wrapper_ref = TemplateWrapperReference::new(wrapper_id, TemplateTirPhase::Parsed, context);
    let plan = push_slot_plan(&mut store);
    let fill_root = contribution_fill(&mut store, plan);
    let mut copy_state = TirCopyState::new();
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    };

    let applied = planner
        .apply_wrapper_reference(wrapper_ref, fill_root)
        .expect("slotless runtime wrapper should apply");
    let derived_wrapper = match &store.get_node(applied).expect("slotless render root").kind {
        TemplateIrNodeKind::Sequence { children } => children[0],
        other => {
            panic!("slotless wrapper application should produce a render sequence, got {other:?}")
        }
    };
    let (reference, _) = derived_wrapper_from_fill(&store, derived_wrapper);

    assert_eq!(reference.phase, wrapper_ref.phase);
    assert_eq!(reference.context, wrapper_ref.context);
    let view = TirView::new(&store, reference.root, reference.phase, reference.context)
        .expect("slotless runtime application should create an exact view");
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
fn versioned_wrapper_keeps_keyed_wrapper_context_on_nested_child() {
    let mut store = TemplateIrStore::new();
    let nested_slot = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_slot_node(SlotKey::Default, None)
    };
    let nested_template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let summary =
            summarize_existing_root(builder.store, nested_slot).expect("valid nested summary");
        builder.finish_template(
            nested_slot,
            Style::default(),
            TemplateType::String,
            summary,
            None,
        )
    };
    let nested_occurrence_id = store.next_child_template_occurrence_id();
    let nested_child = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                nested_template_id,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id: nested_occurrence_id,
        },
        None,
    ));
    let wrapper_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot = builder.push_slot_node(SlotKey::Default, None);
        let root = builder.push_sequence_node(vec![nested_child, slot], None);
        let summary = summarize_existing_root(builder.store, root).expect("valid wrapper summary");
        builder.finish_template(root, Style::default(), TemplateType::String, summary, None)
    };
    let overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                nested_occurrence_id,
                TirWrapperContext {
                    inherited_wrapper_set: None,
                    skip_parent_child_wrappers: true,
                    application_mode: TirWrapperApplicationMode::IfChildEmits,
                },
            )],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        wrapper_context: Some(overlay),
        ..TemplateViewContext::default()
    };
    let wrapper_ref =
        TemplateWrapperReference::new(wrapper_id, TemplateTirPhase::Composed, context);
    let plan = push_slot_plan(&mut store);
    let fill_root = contribution_fill(&mut store, plan);
    let mut copy_state = TirCopyState::new();
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let string_table = StringTable::new();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    };

    let applied = planner
        .apply_wrapper_reference(wrapper_ref, fill_root)
        .expect("wrapper with a wrapper-context overlay should apply");
    let (reference, _) = derived_wrapper_from_fill(&store, applied);

    assert_eq!(reference.phase, TemplateTirPhase::Composed);
    assert_eq!(reference.context.wrapper_context, Some(overlay));
    let view = TirView::new(&store, reference.root, reference.phase, reference.context)
        .expect("derived wrapper view should construct");
    let derived_root = view.root_template().expect("derived wrapper exists").root;
    let TemplateIrNodeKind::Sequence { children } = &view
        .effective_node(derived_root)
        .expect("derived wrapper root exists")
        .kind
    else {
        panic!("expected versioned wrapper sequence");
    };
    let nested_occurrence_id = match &view
        .effective_node(children[0])
        .expect("nested child exists")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
        other => panic!("expected nested child-template node, got {other:?}"),
    };
    let context = view
        .effective_wrapper_context(nested_occurrence_id)
        .expect("wrapper-context lookup should succeed")
        .expect("copied nested child should retain its keyed wrapper context");
    assert!(context.skip_parent_child_wrappers);
    assert_eq!(
        context.application_mode,
        TirWrapperApplicationMode::IfChildEmits
    );
}

#[test]
fn repeated_wrapper_applications_preserve_versioned_expression_site_ids() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);
    let slot_node = builder.push_slot_node(SlotKey::Default, None);
    let conditional_root = builder.push_conditional_node(
        TemplateBranchSelector::Bool(bool_expression(&mut expressions, &mut values, true)),
        slot_node,
        None,
    );
    let wrapper_id = builder.finish_template(
        conditional_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            slot_count: 1,
            has_control_flow: true,
            ..TemplateIrSummary::empty()
        },
        None,
    );
    let source_selector = match &store
        .get_node(conditional_root)
        .expect("conditional root")
        .kind
    {
        TemplateIrNodeKind::Conditional {
            selector_site_id, ..
        } => *selector_site_id,
        other => panic!("expected conditional, got {other:?}"),
    };
    let wrapper_ref = TemplateWrapperReference::new(
        wrapper_id,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    );
    let plan = push_slot_plan(&mut store);
    let first_fill = contribution_fill(&mut store, plan);
    let second_fill = contribution_fill(&mut store, plan);
    let mut copy_state = TirCopyState::new();
    let string_table = StringTable::new();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    };

    let first = planner
        .apply_wrapper_reference(wrapper_ref, first_fill)
        .expect("first wrapper application should succeed");
    let second = planner
        .apply_wrapper_reference(wrapper_ref, second_fill)
        .expect("second wrapper application should succeed");

    drop(planner);

    let first_selector = copied_conditional_selector_site(&store, first);
    let second_selector = copied_conditional_selector_site(&store, second);
    assert_eq!(first_selector, source_selector);
    assert_eq!(second_selector, source_selector);
}

fn copied_conditional_selector_site(
    store: &TemplateIrStore,
    fill_root: TemplateIrNodeId,
) -> ExpressionSiteId {
    let (_, derived) = derived_wrapper_from_fill(store, fill_root);
    match &store
        .get_node(derived.root)
        .expect("derived wrapper root")
        .kind
    {
        TemplateIrNodeKind::Conditional {
            selector_site_id, ..
        } => *selector_site_id,
        other => panic!("expected injected conditional, got {other:?}"),
    }
}

#[test]
fn wrapper_fill_injects_through_conditional_loop_and_child() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let mut store = TemplateIrStore::new();
    let plan = push_slot_plan(&mut store);
    let fill_root = contribution_fill(&mut store, plan);

    let mut builder = TemplateIrBuilder::new(&mut store);
    let conditional_slot = builder.push_slot_node(SlotKey::Default, None);
    let conditional_root = builder.push_conditional_node(
        TemplateBranchSelector::Bool(bool_expression(&mut expressions, &mut values, true)),
        conditional_slot,
        None,
    );
    let loop_slot = builder.push_slot_node(SlotKey::Default, None);
    let aggregate_slot = builder.push_slot_node(SlotKey::Default, None);
    let loop_root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: bool_expression(&mut expressions, &mut values, true),
        },
        loop_slot,
        Some(aggregate_slot),
        None,
    );
    let nested_slot = builder.push_slot_node(SlotKey::Default, None);
    let nested_id = builder.finish_template(
        nested_slot,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            slot_count: 1,
            ..TemplateIrSummary::empty()
        },
        None,
    );
    let child_node = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(
            nested_id,
            TemplateTirPhase::Parsed,
            TemplateViewContext::default(),
        ),
        None,
    );

    let mut copy_state = TirCopyState::new();
    let injected_conditional = inject_runtime_slot_fill(
        conditional_root,
        &[RuntimeSlotInjection {
            key: SlotKey::Default,
            fill_root,
        }],
        None,
        None,
        None,
        &mut store,
        &mut copy_state,
    )
    .expect("conditional body slot should accept fill")
    .root;
    let injected_loop = inject_runtime_slot_fill(
        loop_root,
        &[RuntimeSlotInjection {
            key: SlotKey::Default,
            fill_root,
        }],
        None,
        None,
        None,
        &mut store,
        &mut copy_state,
    )
    .expect("loop body and aggregate slots should accept fill")
    .root;
    let injected_child = inject_runtime_slot_fill(
        child_node,
        &[RuntimeSlotInjection {
            key: SlotKey::Default,
            fill_root,
        }],
        None,
        None,
        None,
        &mut store,
        &mut copy_state,
    )
    .expect("nested child-template slot should accept fill")
    .root;

    assert!(node_contains_contribution(
        injected_conditional,
        &store,
        plan
    ));
    assert!(node_contains_contribution(injected_loop, &store, plan));
    assert!(node_contains_contribution(injected_child, &store, plan));
    match &store.get_node(injected_child).expect("child node").kind {
        TemplateIrNodeKind::ChildTemplate { .. } => {}
        other => panic!("nested child injection must keep a ChildTemplate boundary, got {other:?}"),
    }

    match &store
        .get_node(injected_conditional)
        .expect("conditional")
        .kind
    {
        TemplateIrNodeKind::Conditional { body, .. } => {
            assert!(node_contains_contribution(*body, &store, plan));
        }
        other => panic!("expected conditional, got {other:?}"),
    }
    match &store.get_node(injected_loop).expect("loop").kind {
        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            assert!(node_contains_contribution(*body, &store, plan));
            assert!(node_contains_contribution(
                aggregate_wrapper.expect("aggregate"),
                &store,
                plan
            ));
        }
        other => panic!("expected loop, got {other:?}"),
    }
}

#[test]
fn wrap_site_fill_applies_wrapper_set_innermost_to_outermost() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let inner_wrapper =
        build_slot_text_wrapper(&mut store, &mut string_table, "inner-before", "inner-after");
    let outer_wrapper =
        build_slot_text_wrapper(&mut store, &mut string_table, "outer-before", "outer-after");
    let inner_ref = TemplateWrapperReference::new(
        inner_wrapper,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    );
    let outer_ref = TemplateWrapperReference::new(
        outer_wrapper,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    );
    let wrapper_set_id = store.push_or_reuse_wrapper_set(&[inner_ref, outer_ref]);
    let plan = push_slot_plan(&mut store);
    let fill_root = contribution_fill(&mut store, plan);
    let mut copy_state = TirCopyState::new();
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    };

    let wrapped = planner
        .wrap_site_fill_with_tir_child_wrappers(fill_root, wrapper_set_id)
        .expect("nested wrapper application should succeed");
    let (outer_reference, outer_derived) = derived_wrapper_from_fill(&store, wrapped);
    assert_eq!(outer_reference.phase, TemplateTirPhase::Composed);
    assert!(node_contains_contribution(outer_derived.root, &store, plan));
}

/// Builds a wrapper template whose named slot is the plan's named target.
///
/// WHY: a wrapper set that declares the receiving application's named slot is the
///      case where resolving the set against named markers derives a new template,
///      so repeated resolution is observable in the published wrapper-set identity.
fn build_named_slot_wrapper(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    name: StringId,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let open = builder.push_text_node(
        string_table.intern("<li>"),
        4,
        TemplateSegmentOrigin::Body,
        None,
    );
    let slot = builder.push_slot_node(SlotKey::Named(name), None);
    let close = builder.push_text_node(
        string_table.intern("</li>"),
        5,
        TemplateSegmentOrigin::Body,
        None,
    );
    let root = builder.push_sequence_node(vec![open, slot, close], None);
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            slot_count: 1,
            ..TemplateIrSummary::empty()
        },
        None,
    )
}

#[test]
fn decorator_composite_reuses_prepared_named_edges_for_each_fill() {
    let mut string_table = StringTable::new();
    let row_style = string_table.intern("row_style");
    let mut store = TemplateIrStore::new();

    let prefix = build_named_slot_wrapper(&mut store, &mut string_table, row_style);
    let suffix = build_plain_text_wrapper(&mut store, &mut string_table, "</tr>");
    let composite = compose_child_decorator_wrapper(
        &mut store,
        Some(TemplateWrapperReference::new(
            prefix,
            TemplateTirPhase::Parsed,
            TemplateViewContext::default(),
        )),
        Some(TemplateWrapperReference::new(
            suffix,
            TemplateTirPhase::Parsed,
            TemplateViewContext::default(),
        )),
    )
    .expect("the decorator composite should build");

    let plan = push_slot_plan(&mut store);
    let name_marker = push_runtime_slot_contribution_source(
        &mut store,
        plan,
        RuntimeSlotContributionSourceId(1),
        ContributionShape::Other,
        false,
        None,
    );
    let sources = [RuntimeSlotContributionSourceDraft {
        source: TemplateSlotContributionSourcePlan {
            source: RuntimeSlotContributionSourceId(1),
            target: SlotKey::Named(row_style),
            render_root: name_marker,
            renders_wrapper_unconditionally: false,
            span: None,
        },
        shape: ContributionShape::Other,
        control_flow: false,
    }];
    let first_fill = contribution_fill(&mut store, plan);
    let second_fill = contribution_fill(&mut store, plan);

    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut copy_state = TirCopyState::new();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &sources,
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    };

    let first = planner
        .apply_wrapper_reference(composite, first_fill)
        .expect("the first decorator fill should apply");
    let second = planner
        .apply_wrapper_reference(composite, second_fill)
        .expect("the second decorator fill should apply");
    drop(planner);

    let (first_reference, first_template) = derived_wrapper_from_fill(&store, first);
    let (second_reference, second_template) = derived_wrapper_from_fill(&store, second);
    assert_eq!(first_reference.phase, TemplateTirPhase::Composed);
    assert_eq!(second_reference.phase, TemplateTirPhase::Composed);
    assert_eq!(first_reference.context, second_reference.context);
    assert_eq!(
        first_reference.context,
        TemplateViewContext::default(),
        "the composed decorator application must keep its exact empty view context"
    );

    let sequence_children = |node_id: TemplateIrNodeId| match &store
        .get_node(node_id)
        .expect("the derived decorator root should exist")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected a decorator sequence root, got {other:?}"),
    };
    let first_children = sequence_children(first_template.root);
    let second_children = sequence_children(second_template.root);

    assert_eq!(first_children.len(), 3);
    assert_eq!(second_children.len(), 3);
    assert!(
        matches!(
            store
                .get_node(first_children[0])
                .expect("prefix edge should exist")
                .kind,
            TemplateIrNodeKind::ChildTemplate { .. }
        ),
        "the prefix must stay an exact child reference"
    );
    assert!(
        matches!(
            store
                .get_node(first_children[2])
                .expect("suffix edge should exist")
                .kind,
            TemplateIrNodeKind::ChildTemplate { .. }
        ),
        "the suffix must stay an exact child reference"
    );
    assert_eq!(
        first_children[0], second_children[0],
        "every fill must reuse the prepared prefix edge rather than derive it again"
    );
    assert_eq!(
        first_children[2], second_children[2],
        "every fill must reuse the prepared suffix edge rather than derive it again"
    );
    assert_ne!(
        first_children[1], second_children[1],
        "the implicit default occurrence must stay per fill"
    );

    assert!(node_contains_runtime_source(
        first_children[0],
        &store,
        plan,
        RuntimeSlotContributionSourceId(1)
    ));
    assert!(node_contains_runtime_source(
        second_children[0],
        &store,
        plan,
        RuntimeSlotContributionSourceId(1)
    ));
    assert!(node_contains_runtime_source(
        first_children[1],
        &store,
        plan,
        RuntimeSlotContributionSourceId(0)
    ));
    assert!(node_contains_runtime_source(
        second_children[1],
        &store,
        plan,
        RuntimeSlotContributionSourceId(0)
    ));
}

#[test]
fn prepared_wrapper_injects_target_through_nested_child_path() {
    let mut string_table = StringTable::new();
    let unfilled = string_table.intern("unfilled");
    let mut store = TemplateIrStore::new();

    let nested_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let named = builder.push_slot_node(SlotKey::Named(unfilled), None);
        let target = builder.push_slot_node(SlotKey::Default, None);
        let root = builder.push_sequence_node(vec![named, target], None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary {
                slot_count: 2,
                ..TemplateIrSummary::empty()
            },
            None,
        )
    };
    let nested_child = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                nested_id,
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            ),
            None,
        )
    };
    let wrapper_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            nested_child,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary {
                child_template_count: 1,
                ..TemplateIrSummary::empty()
            },
            None,
        )
    };
    let wrapper_ref = TemplateWrapperReference::new(
        wrapper_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );

    let plan = push_slot_plan(&mut store);
    let fill = contribution_fill(&mut store, plan);
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut copy_state = TirCopyState::new();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    };

    let applied = planner
        .apply_wrapper_reference(wrapper_ref, fill)
        .expect("a nested loose-fill occurrence should still apply");
    drop(planner);

    let (_, wrapper_template) = derived_wrapper_from_fill(&store, applied);
    let nested_derived_root = match &store
        .get_node(wrapper_template.root)
        .expect("the derived wrapper root should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            store
                .get_template(reference.root)
                .expect("the derived nested template should exist")
                .root
        }
        other => panic!("expected the wrapper root to stay a child reference, got {other:?}"),
    };
    let children = match &store
        .get_node(nested_derived_root)
        .expect("the derived nested root should exist")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected a nested sequence root, got {other:?}"),
    };

    assert_eq!(children.len(), 2);
    assert!(
        matches!(
            &store
                .get_node(children[0])
                .expect("the unfilled named occurrence should exist")
                .kind,
            TemplateIrNodeKind::Sequence { children } if children.is_empty()
        ),
        "a named occurrence without a routed source must resolve to empty output"
    );
    assert!(node_contains_runtime_source(
        children[1],
        &store,
        plan,
        RuntimeSlotContributionSourceId(0)
    ));
}

#[test]
fn site_render_root_reuses_named_filled_conditional_wrapper_set() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("body");
    let mut store = TemplateIrStore::new();

    let named_wrapper = build_named_slot_wrapper(&mut store, &mut string_table, name);
    let child_wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        named_wrapper,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )]);

    let occurrence_id = store.next_slot_occurrence_id();
    let wrapper_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_tir_slot_placeholder_node(
            TirSlotPlaceholder::with_wrapper_sets(
                SlotKey::Named(name),
                occurrence_id,
                None,
                None,
                Some(child_wrapper_set),
                false,
            ),
            None,
        )
    };

    let layout = collect_tir_slot_layout_from_root(&store, wrapper_root)
        .expect("the wrapper layout should collect");
    let placeholder = layout
        .placeholders
        .into_iter()
        .next()
        .expect("the wrapper should declare one slot occurrence");

    let plan = push_slot_plan(&mut store);
    let render_root = contribution_fill(&mut store, plan);
    let shape = ContributionShape::Child {
        skips_parent_child_wrappers: false,
    };
    let sources = vec![
        RuntimeSlotContributionSourceDraft {
            source: TemplateSlotContributionSourcePlan {
                source: RuntimeSlotContributionSourceId(0),
                target: SlotKey::Named(name),
                render_root,
                renders_wrapper_unconditionally: false,
                span: None,
            },
            shape,
            control_flow: true,
        },
        RuntimeSlotContributionSourceDraft {
            source: TemplateSlotContributionSourcePlan {
                source: RuntimeSlotContributionSourceId(1),
                target: SlotKey::Named(name),
                render_root,
                renders_wrapper_unconditionally: false,
                span: None,
            },
            shape,
            control_flow: true,
        },
    ];

    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut copy_state = TirCopyState::new();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &sources,
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
        expressions: &expressions,
        const_values: &const_values,
        string_table: &string_table,
        resolved_wrappers: FxHashMap::default(),
    };

    let site_root = planner
        .build_site_render_root(&placeholder, None)
        .expect("the site render root should build");

    let site_children = match &planner.store.get_node(site_root).expect("site root").kind {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected a sequence site root, got {other:?}"),
    };
    assert_eq!(
        site_children.len(),
        2,
        "each sibling source must keep its own render root"
    );

    let published_store: &TemplateIrStore = &*planner.store;
    let conditional_set = |node_id: TemplateIrNodeId| {
        let TemplateIrNodeKind::ChildTemplate { reference, .. } = &published_store
            .get_node(node_id)
            .expect("contribution root should exist")
            .kind
        else {
            panic!("expected a conditional contribution child, got {node_id:?}");
        };
        published_store
            .get_template(reference.root)
            .expect("the conditional contribution template should exist")
            .conditional_child_wrapper_set
    };
    let first_set = conditional_set(site_children[0]);
    let second_set = conditional_set(site_children[1]);

    assert!(
        first_set.is_some(),
        "a control-flow source must carry the receiving slot's wrapper set"
    );
    assert_eq!(
        first_set, second_set,
        "sibling sources must share one named-filled wrapper set instead of publishing a copy each"
    );
}
