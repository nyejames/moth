//! Copied TIR subtree identity tests.
//!
//! Protects the invariant that an independent subtree copy remaps every
//! expression-bearing site: dynamic expressions, branch selectors and loop
//! headers. Overlay authority must not alias the source tree.

use super::super::copy_state::TirCopyState;
use super::super::ids::TemplateIrNodeId;
use super::super::node::{TemplateIrNodeKind, TemplateLoopHeaderExpressionSites};
use super::super::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionOverlay,
};
use super::super::refs::TemplateTirChildReference;
use super::super::store::TemplateIrStore;
use super::super::subtree_copy::copy_tir_subtree_with_active_slot_plan;
use super::super::summary::TemplateIrSummary;
use super::builder::TemplateIrBuilder;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrBranch;
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, TirView};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn bool_expression(value: bool) -> Expression {
    Expression::bool(value, empty_location(), ValueMode::ImmutableOwned)
}

fn text_node(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrNodeId {
    let text_id = string_table.intern(text);
    let byte_len = string_table.resolve(text_id).len();
    let mut builder = TemplateIrBuilder::new(store);
    builder.push_text_node(
        text_id,
        byte_len,
        TemplateSegmentOrigin::Body,
        empty_location(),
    )
}

#[test]
fn copied_branch_and_loop_expression_sites_are_independent() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    let branch_body = text_node(&mut store, &mut string_table, "branch");
    let loop_body = text_node(&mut store, &mut string_table, "loop");
    let mut builder = TemplateIrBuilder::new(&mut store);
    let selector_site = builder.store.next_expression_site_id();
    let branch_root = builder.push_branch_chain_node(
        vec![TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(true)),
            branch_body,
            empty_location(),
            selector_site,
        )],
        None,
        empty_location(),
    );
    let loop_root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(bool_expression(true)),
        },
        loop_body,
        None,
        empty_location(),
    );
    let source_root = builder.push_sequence_node(vec![branch_root, loop_root], empty_location());
    let _template = builder.finish_template(
        source_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let source_selector = match &store
        .get_node(branch_root)
        .expect("source branch should exist")
        .kind
    {
        TemplateIrNodeKind::BranchChain { branches, .. } => branches[0].selector_site_id,
        other => panic!("expected BranchChain, found {other:?}"),
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
        .expect("copied branch should exist")
        .kind
    {
        TemplateIrNodeKind::BranchChain { branches, .. } => branches[0].selector_site_id,
        other => panic!("expected copied BranchChain, found {other:?}"),
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
        "copied branch selector site must not alias the source site"
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
    let mut string_table = StringTable::new();

    let source_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("source");
        let text_node = builder.push_text_node(
            text,
            "source".len(),
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        builder.finish_template(
            text_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        )
    };

    let (child_template, source_expression_site, source_slot_occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let expression_site = builder.store.next_expression_site_id();
        let expression = builder
            .store
            .push_node(super::super::node::TemplateIrNode::new(
                TemplateIrNodeKind::DynamicExpression {
                    expression: Box::new(bool_expression(true)),
                    origin: TemplateSegmentOrigin::Body,
                    reactive_subscription: None,
                    site_id: expression_site,
                },
                empty_location(),
            ));
        let slot = builder.push_slot_node(SlotKey::Default, empty_location());
        let root = builder.push_sequence_node(vec![expression, slot], empty_location());
        let slot_occurrence = match &builder.store.get_node(slot).expect("slot exists").kind {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected slot node, got {other:?}"),
        };
        let child_template = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        );
        (child_template, expression_site, slot_occurrence)
    };

    let child_context = {
        let expression_overlay = store
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(
                    source_expression_site,
                    Box::new(Expression::bool(
                        false,
                        empty_location(),
                        ValueMode::ImmutableOwned,
                    )),
                )],
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
        empty_location(),
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
            .effective_expression_for_site(copied_expression_site)
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
fn copy_depth_exceeds_the_old_u16_boundary_without_wrapping() {
    let mut state = TirCopyState::new();
    let beyond_u16 = usize::from(u16::MAX) + 1;
    for _ in 0..beyond_u16 {
        state.enter_depth();
    }
    state.record_text_node(1);
    assert_eq!(state.summary.max_depth, beyond_u16);
}
