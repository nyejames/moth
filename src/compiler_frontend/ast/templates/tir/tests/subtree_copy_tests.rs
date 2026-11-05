//! Copied TIR subtree identity tests.
//!
//! Protects the invariant that an independent subtree copy remaps every
//! expression-bearing site: dynamic expressions, conditional selectors and loop
//! headers. Overlay authority must not alias the source tree.

use super::super::copy_state::TirCopyState;
use super::super::ids::{ExpressionSiteId, TemplateIrNodeId};
use super::super::node::{TemplateIrNode, TemplateIrNodeKind, TemplateLoopHeaderExpressionSites};
use super::super::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionKind,
    TirSlotResolutionOverlay, TirWrapperApplicationMode, TirWrapperContext,
    TirWrapperContextOverlay,
};
use super::super::refs::{TemplateTirChildReference, TemplateWrapperReference};
use super::super::slot_plan::{
    TemplateSlotContributionSourcePlan, TemplateSlotPlan, TemplateSlotSitePlan,
};
use super::super::store::TemplateIrStore;
use super::super::subtree_copy::{
    CopyIdentityScope, copy_tir_subtree_with_active_slot_plan,
    copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context, record_copied_identity,
};
use super::super::summary::TemplateIrSummary;
use super::builder::TemplateIrBuilder;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, TirView};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

fn bool_expression(
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value: bool,
) -> ExprId {
    expressions
        .bool(const_values, value, None, ValueMode::ImmutableOwned)
        .expect("test bool should construct")
        .id()
}

fn text_node(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrNodeId {
    let text_id = string_table.intern(text);
    let byte_len = string_table.resolve(text_id).len();
    let mut builder = TemplateIrBuilder::new(store);
    builder.push_text_node(text_id, byte_len, TemplateSegmentOrigin::Body, None)
}

#[test]
fn copied_conditional_and_loop_expression_sites_are_independent() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();

    let conditional_body = text_node(&mut store, &mut string_table, "conditional");
    let loop_body = text_node(&mut store, &mut string_table, "loop");
    let mut builder = TemplateIrBuilder::new(&mut store);
    let conditional_root = builder.push_conditional_node(
        TemplateBranchSelector::Bool(bool_expression(&mut expressions, &mut const_values, true)),
        conditional_body,
        None,
    );
    let loop_root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: bool_expression(&mut expressions, &mut const_values, true),
        },
        loop_body,
        None,
        None,
    );
    let source_root = builder.push_sequence_node(vec![conditional_root, loop_root], None);
    let _template = builder.finish_template(
        source_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );

    let source_selector = match &store
        .get_node(conditional_root)
        .expect("source conditional should exist")
        .kind
    {
        TemplateIrNodeKind::Conditional {
            selector_site_id, ..
        } => *selector_site_id,
        other => panic!("expected Conditional, found {other:?}"),
    };
    let source_header_sites = match &store
        .get_node(loop_root)
        .expect("source loop should exist")
        .kind
    {
        TemplateIrNodeKind::Loop { header_sites, .. } => *header_sites,
        other => panic!("expected Loop, found {other:?}"),
    };

    let mut copy_state = TirCopyState::new();
    let copied_root =
        copy_tir_subtree_with_active_slot_plan(source_root, None, &mut store, &mut copy_state)
            .expect("independent subtree copy should succeed");

    let copied_children = match &store
        .get_node(copied_root)
        .expect("copied sequence should exist")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected copied Sequence, found {other:?}"),
    };
    assert_eq!(copied_children.len(), 2);

    let copied_selector = match &store
        .get_node(copied_children[0])
        .expect("copied conditional should exist")
        .kind
    {
        TemplateIrNodeKind::Conditional {
            selector_site_id, ..
        } => *selector_site_id,
        other => panic!("expected copied Conditional, found {other:?}"),
    };
    let copied_header_sites = match &store
        .get_node(copied_children[1])
        .expect("copied loop should exist")
        .kind
    {
        TemplateIrNodeKind::Loop { header_sites, .. } => *header_sites,
        other => panic!("expected copied Loop, found {other:?}"),
    };

    assert_ne!(
        copied_selector, source_selector,
        "copied conditional selector site must not alias the source site"
    );
    assert_ne!(
        copied_header_sites, source_header_sites,
        "copied loop header sites must not alias the source sites"
    );
    match (source_header_sites, copied_header_sites) {
        (
            TemplateLoopHeaderExpressionSites::Conditional {
                condition: source_condition,
            },
            TemplateLoopHeaderExpressionSites::Conditional {
                condition: copied_condition,
            },
        ) => {
            assert_ne!(
                copied_condition, source_condition,
                "copied loop condition site must not alias the source site"
            );
        }
        other => panic!("expected conditional loop header sites, found {other:?}"),
    }
}

#[test]
fn copied_child_remaps_retained_expression_and_slot_context() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();

    let source_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("source");
        let text_node =
            builder.push_text_node(text, "source".len(), TemplateSegmentOrigin::Body, None);
        builder.finish_template(
            text_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };

    let (child_template, source_expression_site, source_slot_occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let expression_site = builder.store.next_expression_site_id();
        let expression = builder
            .store
            .push_node(super::super::node::TemplateIrNode::new(
                TemplateIrNodeKind::DynamicExpression {
                    expression: bool_expression(&mut expressions, &mut const_values, true),
                    origin: TemplateSegmentOrigin::Body,
                    site_id: expression_site,
                },
                None,
            ));
        let slot = builder.push_slot_node(SlotKey::Default, None);
        let root = builder.push_sequence_node(vec![expression, slot], None);
        let slot_occurrence = match &builder.store.get_node(slot).expect("slot exists").kind {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected slot node, got {other:?}"),
        };
        let child_template = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        );
        (child_template, expression_site, slot_occurrence)
    };

    let child_context = {
        let override_expression = bool_expression(&mut expressions, &mut const_values, false);
        let expression_overlay = store
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(source_expression_site, override_expression)],
            })
            .expect("expression overlay should allocate");
        let slot_overlay = store
            .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
                resolutions: vec![(
                    source_slot_occurrence,
                    TirSlotResolution::resolved(SlotKey::Default, vec![source_template]),
                )],
            })
            .expect("slot overlay should allocate");
        TemplateViewContext {
            expression_overlay: Some(expression_overlay),
            slot_resolution: Some(slot_overlay),
            wrapper_context: None,
        }
    };

    let child_occurrence = store.next_child_template_occurrence_id();
    let parent_root = store.push_node(super::super::node::TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                child_template,
                TemplateTirPhase::Composed,
                child_context,
            ),
            occurrence_id: child_occurrence,
        },
        None,
    ));

    let mut copy_state = TirCopyState::new();
    let copied_root =
        copy_tir_subtree_with_active_slot_plan(parent_root, None, &mut store, &mut copy_state)
            .expect("contextual child copy should succeed");
    let (reference, child_root) = match &store
        .get_node(copied_root)
        .expect("copied root exists")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let child_root = store
                .get_template(reference.root)
                .expect("copied child template exists")
                .root;
            (reference, child_root)
        }
        other => panic!("expected copied child node, got {other:?}"),
    };
    let child_view = TirView::new(&store, reference.root, reference.phase, reference.context)
        .expect("copied child view should construct");
    let child_children = match &child_view
        .effective_node(child_root)
        .expect("child root exists")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => children,
        other => panic!("expected copied child sequence, got {other:?}"),
    };
    let copied_expression_site = match &child_view
        .effective_node(child_children[0])
        .expect("copied expression exists")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected copied dynamic expression, got {other:?}"),
    };
    let copied_slot_occurrence = match &child_view
        .effective_node(child_children[1])
        .expect("copied slot exists")
        .kind
    {
        TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
        other => panic!("expected copied slot, got {other:?}"),
    };

    assert_ne!(copied_expression_site, source_expression_site);
    assert_ne!(copied_slot_occurrence, source_slot_occurrence);
    assert!(
        child_view
            .effective_expression_for_site(&expressions, copied_expression_site)
            .expect("copied expression overlay lookup should succeed")
            .is_some()
    );
    assert!(
        child_view
            .effective_slot_resolution(copied_slot_occurrence)
            .expect("copied slot overlay lookup should succeed")
            .is_some()
    );
}

#[test]
fn copied_subtree_rebases_occurrence_keyed_wrapper_context() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let fixed_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("fixed");
        let text_node =
            builder.push_text_node(text, "fixed".len(), TemplateSegmentOrigin::Body, None);
        builder.finish_template(
            text_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };

    let source_occurrence = store.next_child_template_occurrence_id();
    let source_root = store.push_node(super::super::node::TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                fixed_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id: source_occurrence,
        },
        None,
    ));

    // The overlay also carries an occurrence from a different structural root. A copy
    // must drop it because it does not describe any node in the copied subtree.
    let foreign_occurrence = store.next_child_template_occurrence_id();
    let overlay_id = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![
                (
                    source_occurrence,
                    TirWrapperContext {
                        inherited_wrapper_set: None,
                        skip_parent_child_wrappers: true,
                        application_mode: TirWrapperApplicationMode::IfChildEmits,
                    },
                ),
                (
                    foreign_occurrence,
                    TirWrapperContext {
                        inherited_wrapper_set: None,
                        skip_parent_child_wrappers: false,
                        application_mode: TirWrapperApplicationMode::Always,
                    },
                ),
            ],
        })
        .expect("wrapper context overlay should allocate");
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(overlay_id),
    };

    let mut copy_state = TirCopyState::new();
    let (copied_root, copied_context) =
        copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
            source_root,
            None,
            context,
            None,
            &mut store,
            &mut copy_state,
        )
        .expect("context-rebasing copy should succeed");

    let copied_occurrence = match &store
        .get_node(copied_root)
        .expect("copied root should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
        other => panic!("expected copied child node, got {other:?}"),
    };
    assert_ne!(
        copied_occurrence, source_occurrence,
        "the copy must mint a fresh child occurrence identity"
    );

    let copied_overlay_id = copied_context
        .wrapper_context
        .expect("rebased context must keep the wrapper-context dimension");
    let copied_overlay = store
        .wrapper_context_overlay(copied_overlay_id)
        .expect("copied wrapper overlay should exist");
    assert_eq!(
        copied_overlay.contexts.len(),
        1,
        "entries that do not name a copied node belong to a different structural root"
    );
    assert!(
        copied_overlay
            .context_for_occurrence(source_occurrence)
            .is_none(),
        "the copied overlay must not stay keyed by the source occurrence"
    );
    assert!(
        copied_overlay
            .context_for_occurrence(foreign_occurrence)
            .is_none(),
        "foreign occurrences must not survive the rebase"
    );
    let copied_entry = copied_overlay
        .context_for_occurrence(copied_occurrence)
        .expect("the copied occurrence must inherit the source wrapper context");
    assert!(copied_entry.skip_parent_child_wrappers);
    assert_eq!(
        copied_entry.application_mode,
        TirWrapperApplicationMode::IfChildEmits
    );

    let source_overlay = store
        .wrapper_context_overlay(overlay_id)
        .expect("source overlay must remain present");
    assert!(
        source_overlay
            .context_for_occurrence(source_occurrence)
            .is_some(),
        "rebasing must not mutate the source overlay"
    );
}

#[test]
fn copied_context_preserves_overrides_of_surviving_wrapper_templates() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();

    let copied_override = bool_expression(&mut expressions, &mut const_values, false);
    let inner_override = bool_expression(&mut expressions, &mut const_values, true);
    let dropped_override = bool_expression(&mut expressions, &mut const_values, true);

    // Shared inner wrapper: still referenced by the outer wrapper's own
    // direct-child context after the receiver root is copied.
    let inner_wrapper_node;
    let inner_wrapper_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        inner_wrapper_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            inner_wrapper_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let inner_wrapper_site = match &store
        .get_node(inner_wrapper_node)
        .expect("inner wrapper dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };
    let inner_wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        inner_wrapper_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);

    // The outer wrapper owns a fixed direct child whose occurrence inherits the
    // inner wrapper set. That occurrence keys the outer wrapper's own context.
    let fixed_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("fixed");
        let text_node =
            builder.push_text_node(text, "fixed".len(), TemplateSegmentOrigin::Body, None);
        builder.finish_template(
            text_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let outer_root_node = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let reference = TemplateTirChildReference::new(
            fixed_template,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        );
        builder.push_child_template_node_with_reference(reference, None)
    };
    let outer_occurrence = match &store
        .get_node(outer_root_node)
        .expect("outer wrapper root should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
        other => panic!("expected ChildTemplate, found {other:?}"),
    };
    let outer_wrapper_context_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                outer_occurrence,
                TirWrapperContext {
                    inherited_wrapper_set: Some(inner_wrapper_set),
                    skip_parent_child_wrappers: false,
                    application_mode: TirWrapperApplicationMode::Always,
                },
            )],
        })
        .expect("outer wrapper overlay should allocate");
    let outer_wrapper_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            outer_root_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let outer_wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        outer_wrapper_template,
        TemplateTirPhase::Composed,
        TemplateViewContext {
            expression_overlay: None,
            slot_resolution: None,
            wrapper_context: Some(outer_wrapper_context_overlay),
        },
    )]);

    // A wrapper set that only a dropped occurrence names must not keep its
    // template's override alive.
    let dropped_wrapper_node;
    let dropped_wrapper_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        dropped_wrapper_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            dropped_wrapper_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let dropped_wrapper_site = match &store
        .get_node(dropped_wrapper_node)
        .expect("dropped wrapper dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };
    let dropped_wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        dropped_wrapper_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);

    let source_dynamic_node;
    let source_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        source_dynamic_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            source_dynamic_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let source_site = match &store
        .get_node(source_dynamic_node)
        .expect("source dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };
    let source_root_and_occurrence = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let source_child = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                source_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            None,
        );
        // The same template the surviving wrapper set references is also a
        // copied structural child, so one source site needs both a remapped and
        // a preserved override entry.
        let shared_child = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                inner_wrapper_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            None,
        );
        (
            builder.push_sequence_node(vec![source_child, shared_child], None),
            source_child,
        )
    };
    let (source_root, source_child_node) = source_root_and_occurrence;
    let source_occurrence = match &store
        .get_node(source_child_node)
        .expect("source child node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
        other => panic!("expected ChildTemplate, found {other:?}"),
    };
    let foreign_occurrence = store.next_child_template_occurrence_id();
    let source_wrapper_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![
                (
                    source_occurrence,
                    TirWrapperContext {
                        inherited_wrapper_set: Some(outer_wrapper_set),
                        skip_parent_child_wrappers: false,
                        application_mode: TirWrapperApplicationMode::Always,
                    },
                ),
                (
                    foreign_occurrence,
                    TirWrapperContext {
                        inherited_wrapper_set: Some(dropped_wrapper_set),
                        skip_parent_child_wrappers: false,
                        application_mode: TirWrapperApplicationMode::Always,
                    },
                ),
            ],
        })
        .expect("source wrapper overlay should allocate");
    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (source_site, copied_override),
                (inner_wrapper_site, inner_override),
                (dropped_wrapper_site, dropped_override),
            ],
        })
        .expect("expression overlay should allocate");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: None,
        wrapper_context: Some(source_wrapper_overlay),
    };

    let mut copy_state = TirCopyState::new();
    let (copied_root, copied_context) =
        copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
            source_root,
            None,
            context,
            None,
            &mut store,
            &mut copy_state,
        )
        .expect("context-rebasing copy should succeed");

    let copied_children = match &store
        .get_node(copied_root)
        .expect("copied root should exist")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected copied sequence, got {other:?}"),
    };
    assert_eq!(copied_children.len(), 2);

    let (copied_occurrence, copied_template) = match &store
        .get_node(copied_children[0])
        .expect("copied source child should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => (*occurrence_id, reference.root),
        other => panic!("expected copied child node, got {other:?}"),
    };
    let copied_shared_template = match &store
        .get_node(copied_children[1])
        .expect("copied shared child should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
        other => panic!("expected copied child node, got {other:?}"),
    };

    let copied_template_root = store
        .get_template(copied_template)
        .expect("copied child template should exist")
        .root;
    let copied_site = match &store
        .get_node(copied_template_root)
        .expect("copied dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected copied DynamicExpression, found {other:?}"),
    };
    assert_ne!(
        copied_site, source_site,
        "the copy must mint a fresh expression site for the copied tree"
    );

    let copied_shared_root = store
        .get_template(copied_shared_template)
        .expect("copied shared template should exist")
        .root;
    let copied_shared_site = match &store
        .get_node(copied_shared_root)
        .expect("copied shared dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected copied DynamicExpression, found {other:?}"),
    };
    assert_ne!(
        copied_shared_site, inner_wrapper_site,
        "the copied wrapper template use must mint a fresh site as well"
    );

    let copied_wrapper_overlay_id = copied_context
        .wrapper_context
        .expect("rebased context must keep the wrapper-context dimension");
    let copied_wrapper_overlay = store
        .wrapper_context_overlay(copied_wrapper_overlay_id)
        .expect("copied wrapper overlay should exist");
    assert_eq!(
        copied_wrapper_overlay.contexts.len(),
        1,
        "the dropped occurrence must not keep its wrapper context"
    );
    assert_eq!(
        copied_wrapper_overlay.contexts[0].0, copied_occurrence,
        "the surviving context must key the copied occurrence"
    );
    assert_eq!(
        copied_wrapper_overlay.contexts[0].1.inherited_wrapper_set,
        Some(outer_wrapper_set),
        "wrapper sets stay store-owned and unchanged across the copy"
    );

    let copied_expression_overlay_id = copied_context
        .expression_overlay
        .expect("rebased context must keep the expression dimension");
    let copied_overrides = store
        .expression_overlay(copied_expression_overlay_id)
        .expect("copied expression overlay should exist")
        .overrides
        .clone();
    let override_for = |site_id| {
        copied_overrides
            .iter()
            .find(|(candidate, _)| *candidate == site_id)
            .map(|(_, expression)| *expression)
    };

    assert_eq!(
        override_for(copied_site),
        Some(copied_override),
        "the copied site must keep its override under the copied identity"
    );
    assert_eq!(
        override_for(copied_shared_site),
        Some(inner_override),
        "the copied use of a shared wrapper template keeps the remapped override"
    );
    assert_eq!(
        override_for(inner_wrapper_site),
        Some(inner_override),
        "an unchanged wrapper set keeps the original site override for its own use"
    );
    assert!(
        override_for(source_site).is_none(),
        "the replaced source site must not survive the rebase"
    );
    assert!(
        override_for(dropped_wrapper_site).is_none(),
        "a wrapper template named only by a dropped occurrence must not keep its override"
    );
}

#[test]
fn copied_context_preserves_overrides_reached_from_descendant_reference_contexts() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();

    let nested_override = bool_expression(&mut expressions, &mut const_values, false);
    let foreign_override = bool_expression(&mut expressions, &mut const_values, true);

    let nested_wrapper_node;
    let nested_wrapper_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        nested_wrapper_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            nested_wrapper_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let nested_wrapper_site = match &store
        .get_node(nested_wrapper_node)
        .expect("nested wrapper dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };
    let nested_wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        nested_wrapper_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);

    // This wrapper is allocated but never referenced, so its override must not
    // survive the rebase.
    let foreign_wrapper_node;
    let _foreign_wrapper_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        foreign_wrapper_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            foreign_wrapper_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let foreign_wrapper_site = match &store
        .get_node(foreign_wrapper_node)
        .expect("foreign wrapper dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };

    let fixed_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("fixed");
        let text_node =
            builder.push_text_node(text, "fixed".len(), TemplateSegmentOrigin::Body, None);
        builder.finish_template(
            text_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };

    // The wrapper context lives on a structural child reference and keys an
    // occurrence inside the referenced template. Only entering that reference
    // discovers the inherited wrapper set.
    let descendant_occurrence = store.next_child_template_occurrence_id();
    let descendant_wrapper_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                descendant_occurrence,
                TirWrapperContext {
                    inherited_wrapper_set: Some(nested_wrapper_set),
                    skip_parent_child_wrappers: false,
                    application_mode: TirWrapperApplicationMode::Always,
                },
            )],
        })
        .expect("descendant wrapper overlay should allocate");

    let inner_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                fixed_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id: descendant_occurrence,
        },
        None,
    ));
    let inner_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            inner_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };

    let source_occurrence = store.next_child_template_occurrence_id();
    let source_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                inner_template,
                TemplateTirPhase::Composed,
                TemplateViewContext {
                    expression_overlay: None,
                    slot_resolution: None,
                    wrapper_context: Some(descendant_wrapper_overlay),
                },
            ),
            occurrence_id: source_occurrence,
        },
        None,
    ));

    // The receiver carries no wrapper-context dimension of its own: the wrapper
    // is reachable only through the descendant reference context.
    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (nested_wrapper_site, nested_override),
                (foreign_wrapper_site, foreign_override),
            ],
        })
        .expect("expression overlay should allocate");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };

    let mut copy_state = TirCopyState::new();
    let (copied_root, copied_context) =
        copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
            source_root,
            None,
            context,
            None,
            &mut store,
            &mut copy_state,
        )
        .expect("context-rebasing copy should succeed");

    let copied_overrides = store
        .expression_overlay(
            copied_context
                .expression_overlay
                .expect("rebased context must keep the expression dimension"),
        )
        .expect("copied expression overlay should exist")
        .overrides
        .clone();
    let override_for = |site_id| {
        copied_overrides
            .iter()
            .find(|(candidate, _)| *candidate == site_id)
            .map(|(_, expression)| *expression)
    };

    assert_eq!(
        override_for(nested_wrapper_site),
        Some(nested_override),
        "a wrapper reached through a descendant reference context keeps its original site override"
    );
    assert!(
        override_for(foreign_wrapper_site).is_none(),
        "an unreachable wrapper template must not keep its override"
    );

    // The copied reference must key the descendant wrapper context to its copied
    // occurrence rather than the source occurrence.
    let copied_reference = match &store
        .get_node(copied_root)
        .expect("copied root should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference,
        other => panic!("expected copied child node, got {other:?}"),
    };
    let copied_wrapper_overlay = store
        .wrapper_context_overlay(
            copied_reference
                .context
                .wrapper_context
                .expect("copied reference must keep the wrapper-context dimension"),
        )
        .expect("copied wrapper overlay should exist");
    assert_eq!(copied_wrapper_overlay.contexts.len(), 1);
    assert_ne!(
        copied_wrapper_overlay.contexts[0].0, descendant_occurrence,
        "the copied overlay must name the copied occurrence"
    );
    assert_eq!(
        copied_wrapper_overlay.contexts[0].1.inherited_wrapper_set,
        Some(nested_wrapper_set)
    );
}

#[test]
fn copied_context_preserves_overrides_of_slot_sources_reached_from_descendant_reference_contexts() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let source_override = bool_expression(&mut expressions, &mut const_values, false);
    let foreign_override = bool_expression(&mut expressions, &mut const_values, true);

    let source_node;
    let source_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        source_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            source_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let source_site = match &store
        .get_node(source_node)
        .expect("slot source dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };

    let foreign_node;
    let _foreign_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        foreign_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            foreign_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let foreign_site = match &store
        .get_node(foreign_node)
        .expect("foreign dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };

    // The slot lives inside the referenced template, while the reference context
    // resolves it to an unchanged source template.
    let (inner_template, slot_occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot = builder.push_slot_node(SlotKey::Default, None);
        let slot_occurrence = match &builder
            .store
            .get_node(slot)
            .expect("slot node should exist")
            .kind
        {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected Slot, found {other:?}"),
        };
        let template = builder.finish_template(
            slot,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        );
        (template, slot_occurrence)
    };
    let slot_resolution_overlay = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
            resolutions: vec![(
                slot_occurrence,
                TirSlotResolution::resolved(SlotKey::Default, vec![source_template]),
            )],
        })
        .expect("slot-resolution overlay should allocate");

    let source_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                inner_template,
                TemplateTirPhase::Composed,
                TemplateViewContext {
                    expression_overlay: None,
                    slot_resolution: Some(slot_resolution_overlay),
                    wrapper_context: None,
                },
            ),
            None,
        )
    };

    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (source_site, source_override),
                (foreign_site, foreign_override),
            ],
        })
        .expect("expression overlay should allocate");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };

    let mut copy_state = TirCopyState::new();
    let (copied_root, copied_context) =
        copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
            source_root,
            None,
            context,
            None,
            &mut store,
            &mut copy_state,
        )
        .expect("context-rebasing copy should succeed");

    let copied_overrides = store
        .expression_overlay(
            copied_context
                .expression_overlay
                .expect("rebased context must keep the expression dimension"),
        )
        .expect("copied expression overlay should exist")
        .overrides
        .clone();
    let override_for = |site_id| {
        copied_overrides
            .iter()
            .find(|(candidate, _)| *candidate == site_id)
            .map(|(_, expression)| *expression)
    };

    assert_eq!(
        override_for(source_site),
        Some(source_override),
        "a slot source reached through a descendant reference context keeps its original site override"
    );
    assert!(
        override_for(foreign_site).is_none(),
        "a template that no reachable slot source names must not keep its override"
    );

    // The copied reference must resolve the copied slot occurrence to the same
    // unchanged source template.
    let copied_reference = match &store
        .get_node(copied_root)
        .expect("copied root should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference,
        other => panic!("expected copied child node, got {other:?}"),
    };
    let copied_slot_overlay = store
        .slot_resolution_overlay(
            copied_reference
                .context
                .slot_resolution
                .expect("copied reference must keep the slot-resolution dimension"),
        )
        .expect("copied slot-resolution overlay should exist");
    assert_eq!(copied_slot_overlay.resolutions.len(), 1);
    assert_ne!(
        copied_slot_overlay.resolutions[0].0, slot_occurrence,
        "the copied overlay must name the copied slot occurrence"
    );
    let resolved_sources = match &copied_slot_overlay.resolutions[0].1.kind {
        TirSlotResolutionKind::Resolved { sources } => sources,
        other => panic!("expected resolved slot source, found {other:?}"),
    };
    assert_eq!(resolved_sources.as_slice(), &[source_template]);
}

#[test]
fn copied_context_preserves_overrides_of_retained_runtime_slot_plan_roots() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();

    let retained_override = bool_expression(&mut expressions, &mut const_values, false);
    let dropped_override = bool_expression(&mut expressions, &mut const_values, true);

    let source_site = store.next_expression_site_id();
    let source_root_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: bool_expression(&mut expressions, &mut const_values, true),
            origin: TemplateSegmentOrigin::Body,
            site_id: source_site,
        },
        None,
    ));
    let site_site = store.next_expression_site_id();
    let site_root_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: bool_expression(&mut expressions, &mut const_values, true),
            origin: TemplateSegmentOrigin::Body,
            site_id: site_site,
        },
        None,
    ));

    // This site names a node no reachable plan or template references.
    let dropped_site = store.next_expression_site_id();
    let _unrelated_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: bool_expression(&mut expressions, &mut const_values, true),
            origin: TemplateSegmentOrigin::Body,
            site_id: dropped_site,
        },
        None,
    ));

    // A retained plan keeps its original render roots across a copy, so their
    // sites stay addressable through the copied template.
    let slot_plan = store.push_slot_plan(TemplateSlotPlan {
        contribution_sources: vec![TemplateSlotContributionSourcePlan {
            source: RuntimeSlotContributionSourceId(0),
            target: SlotKey::Default,
            render_root: source_root_node,
            renders_wrapper_unconditionally: false,
            span: None,
        }],
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root: site_root_node,
            span: None,
        }],
    });

    let plan_owner_root = text_node(&mut store, &mut string_table, "wrapper");
    let plan_owner_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            plan_owner_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    store
        .attach_runtime_slot_plan(plan_owner_template, slot_plan)
        .expect("runtime slot plan should attach");

    let source_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                plan_owner_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            None,
        )
    };

    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (source_site, retained_override),
                (site_site, retained_override),
                (dropped_site, dropped_override),
            ],
        })
        .expect("expression overlay should allocate");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };

    let mut copy_state = TirCopyState::new();
    let (copied_root, copied_context) =
        copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
            source_root,
            None,
            context,
            None,
            &mut store,
            &mut copy_state,
        )
        .expect("context-rebasing copy should succeed");

    let copied_plan_owner = match &store
        .get_node(copied_root)
        .expect("copied root should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
        other => panic!("expected copied child node, got {other:?}"),
    };
    assert_eq!(
        store
            .get_template(copied_plan_owner)
            .expect("copied plan owner template should exist")
            .runtime_slot_plan,
        Some(slot_plan),
        "the copy must retain the shared plan and its original render roots"
    );

    let copied_overrides = store
        .expression_overlay(
            copied_context
                .expression_overlay
                .expect("rebased context must keep the expression dimension"),
        )
        .expect("copied expression overlay should exist")
        .overrides
        .clone();
    let override_for = |site_id| {
        copied_overrides
            .iter()
            .find(|(candidate, _)| *candidate == site_id)
            .map(|(_, expression)| *expression)
    };

    assert_eq!(
        override_for(source_site),
        Some(retained_override),
        "a retained contribution-source root keeps its original site override"
    );
    assert_eq!(
        override_for(site_site),
        Some(retained_override),
        "a retained slot-site root keeps its original site override"
    );
    assert!(
        override_for(dropped_site).is_none(),
        "an unreferenced site must not keep its override"
    );
}

#[test]
fn copied_context_rebases_every_copy_of_a_reused_template() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let overridden = bool_expression(&mut expressions, &mut const_values, false);

    let shared_node;
    let shared_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        shared_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            shared_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let shared_site = match &store
        .get_node(shared_node)
        .expect("shared dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };

    let source_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let first = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                shared_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            None,
        );
        let second = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                shared_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            None,
        );
        builder.push_sequence_node(vec![first, second], None)
    };

    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(shared_site, overridden)],
        })
        .expect("expression overlay should allocate");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };

    let mut copy_state = TirCopyState::new();
    let (copied_root, copied_context) =
        copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
            source_root,
            None,
            context,
            None,
            &mut store,
            &mut copy_state,
        )
        .expect("context-rebasing copy should succeed");

    let copied_children = match &store
        .get_node(copied_root)
        .expect("copied root should exist")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected copied sequence, got {other:?}"),
    };
    assert_eq!(copied_children.len(), 2);

    let copied_sites = copied_children
        .iter()
        .map(|child| {
            let copied_template = match &store
                .get_node(*child)
                .expect("copied child node should exist")
                .kind
            {
                TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
                other => panic!("expected copied child node, got {other:?}"),
            };
            let copied_template_root = store
                .get_template(copied_template)
                .expect("copied child template should exist")
                .root;
            match &store
                .get_node(copied_template_root)
                .expect("copied dynamic node should exist")
                .kind
            {
                TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
                other => panic!("expected copied DynamicExpression, found {other:?}"),
            }
        })
        .collect::<Vec<_>>();
    assert_ne!(copied_sites[0], copied_sites[1]);
    assert!(copied_sites.iter().all(|site_id| *site_id != shared_site));

    let copied_overrides = store
        .expression_overlay(
            copied_context
                .expression_overlay
                .expect("rebased context must keep the expression dimension"),
        )
        .expect("copied expression overlay should exist")
        .overrides
        .clone();
    let override_for = |site_id| {
        copied_overrides
            .iter()
            .find(|(candidate, _)| *candidate == site_id)
            .map(|(_, expression)| *expression)
    };

    for copied_site in &copied_sites {
        assert_eq!(
            override_for(*copied_site),
            Some(overridden),
            "every fresh copy of a reused source site keeps the override"
        );
    }
    assert!(
        override_for(shared_site).is_none(),
        "the replaced source site must not survive the rebase"
    );
    assert_eq!(
        copied_overrides.len(),
        2,
        "the rebased overlay must cover exactly the two fresh copies"
    );
}

#[test]
fn copied_context_preserves_overrides_of_retained_template_child_wrapper_set() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();

    let overridden = bool_expression(&mut expressions, &mut const_values, false);

    let wrapper_node;
    let wrapper_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        wrapper_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            wrapper_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let wrapper_site = match &store
        .get_node(wrapper_node)
        .expect("wrapper dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };
    let wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        wrapper_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);

    // The copied root is a bare node; the receiver template's conditional
    // wrapper set is the only structure that names the overridden site.
    let source_root = text_node(&mut store, &mut string_table, "receiver");

    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(wrapper_site, overridden)],
        })
        .expect("expression overlay should allocate");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };

    let mut copy_state = TirCopyState::new();
    let (_, copied_context) = copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
        source_root,
        None,
        context,
        Some(wrapper_set),
        &mut store,
        &mut copy_state,
    )
    .expect("context-rebasing copy should succeed");

    let copied_overrides = store
        .expression_overlay(
            copied_context
                .expression_overlay
                .expect("rebased context must keep the expression dimension"),
        )
        .expect("copied expression overlay should exist")
        .overrides
        .clone();
    assert_eq!(
        copied_overrides,
        vec![(wrapper_site, overridden)],
        "the retained template-level wrapper set keeps its override reachable"
    );

    // Without the retained wrapper context the same override has no reachable
    // owner and is dropped from the exact view.
    let mut copy_state = TirCopyState::new();
    let (_, context_without_retained_wrappers) =
        copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
            source_root,
            None,
            context,
            None,
            &mut store,
            &mut copy_state,
        )
        .expect("context-rebasing copy should succeed");
    let overrides_without_retained_wrappers = store
        .expression_overlay(
            context_without_retained_wrappers
                .expression_overlay
                .expect("rebased context must keep the expression dimension"),
        )
        .expect("copied expression overlay should exist")
        .overrides
        .clone();
    assert!(
        overrides_without_retained_wrappers.is_empty(),
        "an unreachable wrapper site must not keep its override"
    );
}

#[test]
fn copy_depth_exceeds_the_old_u16_boundary_without_wrapping() {
    let mut state = TirCopyState::new();
    let beyond_u16 = usize::from(u16::MAX) + 1;
    for _ in 0..beyond_u16 {
        state.enter_depth();
    }
    state.record_text_node(1);
    assert_eq!(state.summary.max_depth, beyond_u16);
}

#[test]
fn scope_lookup_selects_record_ranges_across_nested_scopes() {
    let source = ExpressionSiteId::new(0);
    let other_source = ExpressionSiteId::new(1);
    let mut identities = FxHashMap::default();

    // A recording pass numbers every copied identity in recording order, and a
    // scope owns the half-open record range it was active for. The root records
    // before and after a nested copy and again after a sibling copy, so a shared
    // identity's destinations interleave with descendant scopes instead of
    // arriving in ascending scope order.
    let root_scope = CopyIdentityScope {
        start_record: 0,
        end_record: 6,
    };
    let first_child_scope = CopyIdentityScope {
        start_record: 1,
        end_record: 2,
    };
    let sibling_scope = CopyIdentityScope {
        start_record: 3,
        end_record: 5,
    };
    let grandchild_scope = CopyIdentityScope {
        start_record: 4,
        end_record: 5,
    };
    let empty_scope = CopyIdentityScope {
        start_record: 6,
        end_record: 7,
    };

    record_copied_identity(&mut identities, 0, source, ExpressionSiteId::new(10));
    record_copied_identity(&mut identities, 1, source, ExpressionSiteId::new(11));
    record_copied_identity(&mut identities, 2, source, ExpressionSiteId::new(12));
    record_copied_identity(&mut identities, 3, source, ExpressionSiteId::new(13));
    record_copied_identity(&mut identities, 4, source, ExpressionSiteId::new(14));
    record_copied_identity(&mut identities, 5, source, ExpressionSiteId::new(15));
    record_copied_identity(&mut identities, 2, other_source, ExpressionSiteId::new(20));

    let selected = |identity: ExpressionSiteId, scope: CopyIdentityScope| {
        identities
            .get(&identity)
            .expect("recorded source identity should have destinations")
            .iter_in_scope(scope)
            .map(ExpressionSiteId::index)
            .collect::<Vec<_>>()
    };

    assert_eq!(
        selected(source, root_scope),
        vec![10, 11, 12, 13, 14, 15],
        "a root scope must include every nested copy of a reused source identity in record order"
    );
    assert_eq!(
        selected(source, first_child_scope),
        vec![11],
        "a nested scope must select only its own copy"
    );
    assert_eq!(
        selected(source, sibling_scope),
        vec![13, 14],
        "a sibling scope must select its own copies and those of its descendants"
    );
    assert_eq!(
        selected(source, grandchild_scope),
        vec![14],
        "a grandchild scope must not select its parent's copies"
    );
    assert_eq!(
        selected(source, empty_scope),
        Vec::<usize>::new(),
        "a scope that recorded nothing must select nothing"
    );
    assert_eq!(
        selected(
            source,
            CopyIdentityScope {
                start_record: 5,
                end_record: 6,
            }
        ),
        vec![15],
        "a range start is inclusive and a range end is exclusive"
    );
    assert_eq!(
        selected(
            source,
            CopyIdentityScope {
                start_record: 6,
                end_record: 6,
            }
        ),
        Vec::<usize>::new(),
        "an empty range must select nothing"
    );

    assert_eq!(
        selected(other_source, root_scope),
        vec![20],
        "a scoped lookup must not leak destinations across source identities"
    );
    assert_eq!(
        selected(other_source, first_child_scope),
        Vec::<usize>::new(),
        "a single-destination identity recorded outside the range stays excluded"
    );
}

#[test]
fn shared_identity_recorded_after_nested_scopes_appends_in_record_order() {
    // A parent that records a shared identity after every nested copy used to
    // force an ordered insertion at the front of the destination list: the
    // parent's scope number is lower than each child's while its records arrive
    // later, so the insertion shifted every earlier destination and made the
    // pass quadratic in the number of copies. Record order is now list order, so
    // the same interleaving appends, and the root slice still covers both the
    // parent's and every child's copies.
    const NESTED_COPIES: usize = 1024;

    let source = ExpressionSiteId::new(0);
    let mut identities = FxHashMap::default();
    let mut child_scopes = Vec::with_capacity(NESTED_COPIES);
    let root_start = 0;
    let mut record = root_start;

    for index in 0..NESTED_COPIES {
        let child_start = record;
        record_copied_identity(
            &mut identities,
            record,
            source,
            ExpressionSiteId::new(index),
        );
        record += 1;
        child_scopes.push(CopyIdentityScope {
            start_record: child_start,
            end_record: record,
        });

        // The enclosing scope records the same source after the nested copy
        // finishes; under scope-sorted insertion this landed in front of every
        // earlier destination.
        record_copied_identity(
            &mut identities,
            record,
            source,
            ExpressionSiteId::new(NESTED_COPIES + index),
        );
        record += 1;
    }

    let selected = |scope: CopyIdentityScope| {
        identities
            .get(&source)
            .expect("the shared source identity should have destinations")
            .iter_in_scope(scope)
            .map(ExpressionSiteId::index)
            .collect::<Vec<_>>()
    };

    let root_scope = CopyIdentityScope {
        start_record: root_start,
        end_record: record,
    };
    let all_copies = selected(root_scope);
    assert_eq!(all_copies.len(), NESTED_COPIES * 2);
    assert!(
        all_copies.starts_with(&[0, NESTED_COPIES, 1, NESTED_COPIES + 1]),
        "the root selection stays in the order the copies were recorded"
    );
    assert_eq!(
        all_copies.last().copied(),
        Some(NESTED_COPIES * 2 - 1),
        "the root selection must include the last enclosing-scope copy"
    );

    for (index, scope) in child_scopes.iter().copied().enumerate() {
        assert_eq!(
            selected(scope),
            vec![index],
            "each nested scope must select exactly its own destination"
        );
    }
}

#[test]
fn copied_context_rebases_shared_site_across_nested_and_enclosing_scopes() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let overridden = bool_expression(&mut expressions, &mut const_values, false);

    // The source site is used inside the child template and again by the root
    // node after the child reference. Copying the root therefore records the
    // shared identity once inside the nested template scope and once again in
    // the enclosing scope afterwards.
    let shared_node;
    let shared_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        shared_node = builder.push_dynamic_expression_node(
            bool_expression(&mut expressions, &mut const_values, true),
            TemplateSegmentOrigin::Body,
            None,
        );
        builder.finish_template(
            shared_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let shared_site = match &store
        .get_node(shared_node)
        .expect("shared dynamic node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression, found {other:?}"),
    };

    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(shared_site, overridden)],
        })
        .expect("expression overlay should allocate");
    let reference_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };

    let parent_site_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: bool_expression(&mut expressions, &mut const_values, true),
            origin: TemplateSegmentOrigin::Body,
            site_id: shared_site,
        },
        None,
    ));
    let source_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let child = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                shared_template,
                TemplateTirPhase::Composed,
                reference_context,
            ),
            None,
        );
        builder.push_sequence_node(vec![child, parent_site_node], None)
    };

    let mut copy_state = TirCopyState::new();
    let (copied_root, copied_context) =
        copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
            source_root,
            None,
            reference_context,
            None,
            &mut store,
            &mut copy_state,
        )
        .expect("context-rebasing copy should succeed");

    let copied_children = match &store
        .get_node(copied_root)
        .expect("copied root should exist")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected copied sequence, got {other:?}"),
    };
    assert_eq!(copied_children.len(), 2);
    let copied_reference = match &store
        .get_node(copied_children[0])
        .expect("copied child node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => *reference,
        other => panic!("expected copied child node, got {other:?}"),
    };

    let root_overrides = store
        .expression_overlay(
            copied_context
                .expression_overlay
                .expect("rebased root context must keep the expression dimension"),
        )
        .expect("copied root expression overlay should exist")
        .overrides
        .clone();
    assert_eq!(
        root_overrides.len(),
        2,
        "the enclosing scope must rebase both the nested and the enclosing copy"
    );
    assert!(
        root_overrides
            .iter()
            .all(|(site_id, expression)| *site_id != shared_site && *expression == overridden),
        "both copies must receive fresh site identities and the source override"
    );

    let child_overrides = store
        .expression_overlay(
            copied_reference
                .context
                .expression_overlay
                .expect("copied reference context must keep the expression dimension"),
        )
        .expect("copied child expression overlay should exist")
        .overrides
        .clone();
    assert_eq!(
        child_overrides.len(),
        1,
        "the nested scope must not absorb the enclosing scope's copy"
    );
    assert_ne!(child_overrides[0].0, shared_site);
    assert_eq!(child_overrides[0].1, overridden);
    assert!(
        root_overrides
            .iter()
            .any(|(site_id, _)| *site_id == child_overrides[0].0),
        "the nested copy is also part of the enclosing scope's selection"
    );
}
