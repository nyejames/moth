//! Focused unit tests for runtime slot-site node authority.
//!
//! WHAT: protects wrapper-reference identity, plan-qualified contribution
//!       markers and independent copied expression sites. Integration cases
//!       own visible output.
//! WHY: these facts live in the TIR store and cannot be inspected from
//!      rendered strings.

use super::{RuntimeWrapperSitePlanBuilder, inject_runtime_slot_fill};
use crate::compiler_frontend::ast::expressions::expression::Expression;
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
    ExpressionSiteId, TemplateIr, TemplateIrBranch, TemplateIrBuilder, TemplateIrId,
    TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore, TemplateIrSummary,
    TemplateSlotPlan, TemplateSlotPlanId, TemplateTirPhase, TemplateViewContext, TirCopyState,
    TirSlotResolutionOverlay, TirView, TirWrapperApplicationMode, TirWrapperContext,
    TirWrapperContextOverlay, push_runtime_slot_contribution_source,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn bool_expression(value: bool) -> Expression {
    Expression::bool(value, SourceLocation::default(), ValueMode::ImmutableOwned)
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
            assert!(
                error.msg.contains(owner_marker),
                "{context} should be rejected by the {owner_marker} owner, got: {}",
                error.msg,
            );
        }
        TemplateError::Diagnostic(_) => {
            panic!("{context} must be an infrastructure error, not a user diagnostic",);
        }
    }
}

fn push_slot_plan(store: &mut TemplateIrStore) -> TemplateSlotPlanId {
    store.push_slot_plan(TemplateSlotPlan {
        location: SourceLocation::default(),
        contribution_sources: vec![],
        slot_sites: vec![],
    })
}

fn contribution_fill(store: &mut TemplateIrStore, plan: TemplateSlotPlanId) -> TemplateIrNodeId {
    push_runtime_slot_contribution_source(
        store,
        plan,
        RuntimeSlotContributionSourceId(0),
        SourceLocation::default(),
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
        SourceLocation::default(),
    );
    let slot_node = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let after_node = builder.push_text_node(
        string_table.intern(after),
        after.len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let root = builder.push_sequence_node(
        vec![before_node, slot_node, after_node],
        SourceLocation::default(),
    );
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            slot_count: 1,
            ..TemplateIrSummary::empty()
        },
        SourceLocation::default(),
    )
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
        SourceLocation::default(),
    );
    builder.finish_template(
        text_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    )
}

fn node_contains_contribution(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    plan: TemplateSlotPlanId,
) -> bool {
    let Some(node) = store.get_node(node_id) else {
        return false;
    };

    match &node.kind {
        TemplateIrNodeKind::RuntimeSlotContributionSource {
            plan: marker_plan,
            source: RuntimeSlotContributionSourceId(0),
        } => *marker_plan == plan,
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .any(|child| node_contains_contribution(*child, store, plan)),
        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            branches
                .iter()
                .any(|branch| node_contains_contribution(branch.body, store, plan))
                || fallback
                    .is_some_and(|fallback| node_contains_contribution(fallback, store, plan))
        }
        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            node_contains_contribution(*body, store, plan)
                || aggregate_wrapper
                    .is_some_and(|wrapper| node_contains_contribution(wrapper, store, plan))
        }
        TemplateIrNodeKind::ChildTemplate { reference, .. } => store
            .get_template(reference.root)
            .is_some_and(|template| node_contains_contribution(template.root, store, plan)),
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
    let child_node = builder.push_child_template_node_with_reference(
        same_store_missing_template,
        SourceLocation::default(),
    );
    let plan = push_slot_plan(&mut store);
    let fill_root = contribution_fill(&mut store, plan);
    let mut copy_state = TirCopyState::new();

    let result = inject_runtime_slot_fill(
        child_node,
        fill_root,
        &SlotKey::Default,
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
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: TemplateSlotPlanId::new(0),
        store: &mut store,
        copy_state: &mut copy_state,
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
            assert_eq!(error.error_type, ErrorType::Compiler);
        }
        TemplateError::Diagnostic(_) => {
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
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
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
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
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
        builder.push_slot_node(SlotKey::Default, SourceLocation::default())
    };
    let nested_template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            nested_slot,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
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
        SourceLocation::default(),
    ));
    let wrapper_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
        let root = builder.push_sequence_node(vec![nested_child, slot], SourceLocation::default());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        )
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
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
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
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);
    let slot_node = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let selector_site = builder.store.next_expression_site_id();
    let branch_root = builder.push_branch_chain_node(
        vec![TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(true)),
            slot_node,
            SourceLocation::default(),
            selector_site,
        )],
        None,
        SourceLocation::default(),
    );
    let wrapper_id = builder.finish_template(
        branch_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            slot_count: 1,
            has_control_flow: true,
            ..TemplateIrSummary::empty()
        },
        SourceLocation::default(),
    );
    let source_selector = match &store.get_node(branch_root).expect("branch root").kind {
        TemplateIrNodeKind::BranchChain { branches, .. } => branches[0].selector_site_id,
        other => panic!("expected branch chain, got {other:?}"),
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
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
    };

    let first = planner
        .apply_wrapper_reference(wrapper_ref, first_fill)
        .expect("first wrapper application should succeed");
    let second = planner
        .apply_wrapper_reference(wrapper_ref, second_fill)
        .expect("second wrapper application should succeed");

    let first_selector = copied_branch_selector_site(&store, first);
    let second_selector = copied_branch_selector_site(&store, second);
    assert_eq!(first_selector, source_selector);
    assert_eq!(second_selector, source_selector);
}

fn copied_branch_selector_site(
    store: &TemplateIrStore,
    fill_root: TemplateIrNodeId,
) -> ExpressionSiteId {
    let (_, derived) = derived_wrapper_from_fill(store, fill_root);
    match &store
        .get_node(derived.root)
        .expect("derived wrapper root")
        .kind
    {
        TemplateIrNodeKind::BranchChain { branches, .. } => branches[0].selector_site_id,
        other => panic!("expected injected branch chain, got {other:?}"),
    }
}

#[test]
fn wrapper_fill_injects_through_branch_fallback_loop_and_child() {
    let mut store = TemplateIrStore::new();
    let plan = push_slot_plan(&mut store);
    let fill_root = contribution_fill(&mut store, plan);

    let mut builder = TemplateIrBuilder::new(&mut store);
    let then_slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let fallback_slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let selector_site = builder.store.next_expression_site_id();
    let branch_root = builder.push_branch_chain_node(
        vec![TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(true)),
            then_slot,
            SourceLocation::default(),
            selector_site,
        )],
        Some(fallback_slot),
        SourceLocation::default(),
    );
    let loop_slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let aggregate_slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let loop_root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(bool_expression(true)),
        },
        loop_slot,
        Some(aggregate_slot),
        SourceLocation::default(),
    );
    let nested_slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let nested_id = builder.finish_template(
        nested_slot,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            slot_count: 1,
            ..TemplateIrSummary::empty()
        },
        SourceLocation::default(),
    );
    let child_node = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(
            nested_id,
            TemplateTirPhase::Parsed,
            TemplateViewContext::default(),
        ),
        SourceLocation::default(),
    );

    let mut copy_state = TirCopyState::new();
    let injected_branch = inject_runtime_slot_fill(
        branch_root,
        fill_root,
        &SlotKey::Default,
        &mut store,
        &mut copy_state,
    )
    .expect("branch and fallback slots should accept fill")
    .root;
    let injected_loop = inject_runtime_slot_fill(
        loop_root,
        fill_root,
        &SlotKey::Default,
        &mut store,
        &mut copy_state,
    )
    .expect("loop body and aggregate slots should accept fill")
    .root;
    let injected_child = inject_runtime_slot_fill(
        child_node,
        fill_root,
        &SlotKey::Default,
        &mut store,
        &mut copy_state,
    )
    .expect("nested child-template slot should accept fill")
    .root;

    assert!(node_contains_contribution(injected_branch, &store, plan));
    assert!(node_contains_contribution(injected_loop, &store, plan));
    assert!(node_contains_contribution(injected_child, &store, plan));
    match &store.get_node(injected_child).expect("child node").kind {
        TemplateIrNodeKind::ChildTemplate { .. } => {}
        other => panic!("nested child injection must keep a ChildTemplate boundary, got {other:?}"),
    }

    match &store.get_node(injected_branch).expect("branch").kind {
        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            assert!(node_contains_contribution(branches[0].body, &store, plan));
            assert!(node_contains_contribution(
                fallback.expect("fallback"),
                &store,
                plan
            ));
        }
        other => panic!("expected branch chain, got {other:?}"),
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
    let wrapper_set_id = store.push_or_reuse_wrapper_set(vec![inner_ref, outer_ref]);
    let plan = push_slot_plan(&mut store);
    let fill_root = contribution_fill(&mut store, plan);
    let mut copy_state = TirCopyState::new();
    let mut planner = RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: plan,
        store: &mut store,
        copy_state: &mut copy_state,
    };

    let wrapped = planner
        .wrap_site_fill_with_tir_child_wrappers(fill_root, wrapper_set_id)
        .expect("nested wrapper application should succeed");
    let (outer_reference, outer_derived) = derived_wrapper_from_fill(&store, wrapped);
    assert_eq!(outer_reference.phase, TemplateTirPhase::Composed);
    assert!(node_contains_contribution(outer_derived.root, &store, plan));
}
