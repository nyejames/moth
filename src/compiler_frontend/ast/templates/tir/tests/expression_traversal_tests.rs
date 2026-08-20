//! TIR expression-payload walker tests.
//!
//! WHAT: protects structural and effective-view expression traversal.
//! WHY: finalization owns one module-local TIR store, so every reachable
//! payload must be discovered through that same store and its overlays.

use super::super::expression_overlays::{
    collect_effective_tir_expression_overlay_payloads, collect_tir_expression_overlay_payloads,
    replace_expression_overlay_entries,
};
use super::super::expression_sites::{
    walk_expression_payloads_with_nested_tir_views, walk_tir_view_expression_payloads,
};
use super::super::ids::{ExpressionSiteId, TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId};
use super::super::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind,
    TemplateLoopHeaderExpressionSites,
};
use super::super::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolutionOverlayId,
    TirWrapperContextOverlayId,
};
use super::super::refs::{TemplateTirChildReference, TemplateTirReference};
use super::super::slot_plan::{
    TemplateSlotContributionSourcePlan, TemplateSlotPlan, TemplateSlotSitePlan,
};
use super::super::store::TemplateIrStore;
use super::super::summary::TemplateIrSummary;
use super::super::view::{TemplateTirPhase, TirView};
use super::builder::TemplateIrBuilder;
use super::support::{
    TirExpressionPayloadMutator, mutate_finalized_tir_body_root_expression_payloads,
};
use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeEndKind, RangeLoopSpec};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn expression(value: i32) -> Expression {
    Expression::int(value, SourceLocation::default(), ValueMode::ImmutableOwned)
}

fn dynamic_node(store: &mut TemplateIrStore, value: i32) -> TemplateIrNodeId {
    let site_id = store.next_expression_site_id();
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(expression(value)),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        SourceLocation::default(),
    ))
}

fn push_template(
    store: &mut TemplateIrStore,
    root: TemplateIrNodeId,
    kind: TemplateType,
) -> TemplateIrId {
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        kind,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    ))
}

fn dynamic_site_id(store: &TemplateIrStore, node_id: TemplateIrNodeId) -> ExpressionSiteId {
    let node = store.get_node(node_id).expect("dynamic node should exist");
    let TemplateIrNodeKind::DynamicExpression { site_id, .. } = node.kind else {
        panic!("expected dynamic expression node");
    };
    site_id
}

fn collect_view(view: &TirView<'_>) -> Result<Vec<Expression>, CompilerError> {
    let mut payloads = Vec::new();
    walk_tir_view_expression_payloads(view, &mut |expression| {
        payloads.push(expression.clone());
        Ok(())
    })?;
    Ok(payloads)
}

#[derive(Default, Debug)]
struct CountingMutator {
    count: usize,
}

impl TirExpressionPayloadMutator for CountingMutator {
    fn mutate_expression_payload(
        &mut self,
        expression: &mut Expression,
    ) -> Result<(), CompilerError> {
        self.count += 1;
        expression.contains_regular_division = true;
        Ok(())
    }
}

#[test]
fn mutates_direct_dynamic_expression() {
    let mut store = TemplateIrStore::new();
    let root = dynamic_node(&mut store, 1);
    let mut mutator = CountingMutator::default();

    mutate_finalized_tir_body_root_expression_payloads(&mut store, root, &mut mutator)
        .expect("direct expression walk should succeed");

    assert_eq!(mutator.count, 1);
    let node = store.get_node(root).expect("root should exist");
    let TemplateIrNodeKind::DynamicExpression { expression, .. } = &node.kind else {
        panic!("expected dynamic expression");
    };
    assert!(expression.contains_regular_division);
}

#[test]
fn mutates_branch_selector_and_body_expression() {
    let mut store = TemplateIrStore::new();
    let body = dynamic_node(&mut store, 2);
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(expression(1)),
        body,
        SourceLocation::default(),
        store.next_expression_site_id(),
    );
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::BranchChain {
            branches: vec![branch],
            fallback: None,
        },
        SourceLocation::default(),
    ));
    let mut mutator = CountingMutator::default();

    mutate_finalized_tir_body_root_expression_payloads(&mut store, root, &mut mutator)
        .expect("branch expression walk should succeed");

    assert_eq!(mutator.count, 2);
}

#[test]
fn mutates_nested_same_store_child_expression() {
    let mut store = TemplateIrStore::new();
    let child_root = dynamic_node(&mut store, 3);
    let child_template = push_template(&mut store, child_root, TemplateType::StringFunction);
    let occurrence_id = store.next_child_template_occurrence_id();
    let child_node = store.push_node(TemplateIrNode::new(
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
    let mut mutator = CountingMutator::default();

    mutate_finalized_tir_body_root_expression_payloads(&mut store, child_node, &mut mutator)
        .expect("child expression walk should succeed");

    assert_eq!(mutator.count, 1);
}

#[test]
fn structural_collection_ignores_child_expression_overlay() {
    let mut store = TemplateIrStore::new();
    let child_root = dynamic_node(&mut store, 1);
    let child_site_id = dynamic_site_id(&store, child_root);
    let child_template = push_template(&mut store, child_root, TemplateType::StringFunction);
    let child_expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(child_site_id, Box::new(expression(9)))],
        })
        .expect("test overlay allocation");
    let child_context = TemplateViewContext {
        expression_overlay: Some(child_expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };
    let occurrence_id = store.next_child_template_occurrence_id();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                child_template,
                TemplateTirPhase::Composed,
                child_context,
            ),
            occurrence_id,
        },
        SourceLocation::default(),
    ));
    let parent_template = push_template(&mut store, child_node, TemplateType::StringFunction);

    let payloads = collect_tir_expression_overlay_payloads(&store, parent_template)
        .expect("structural collection should succeed");

    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].0, child_site_id);
    assert!(matches!(payloads[0].1.kind, ExpressionKind::Int(1)));
}

#[test]
fn effective_collection_reads_same_store_child_overlay() {
    let mut store = TemplateIrStore::new();
    let child_root = dynamic_node(&mut store, 1);
    let child_site_id = dynamic_site_id(&store, child_root);
    let child_template = push_template(&mut store, child_root, TemplateType::StringFunction);
    let child_expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(child_site_id, Box::new(expression(9)))],
        })
        .expect("test overlay allocation");
    let child_view_context = TemplateViewContext {
        expression_overlay: Some(child_expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };
    let occurrence_id = store.next_child_template_occurrence_id();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                child_template,
                TemplateTirPhase::Composed,
                child_view_context,
            ),
            occurrence_id,
        },
        SourceLocation::default(),
    ));
    let parent_template = push_template(&mut store, child_node, TemplateType::StringFunction);
    let root_view_context = TemplateViewContext::default();
    let root_view = TirView::new(
        &store,
        parent_template,
        TemplateTirPhase::Finalized,
        root_view_context,
    )
    .expect("effective root view should construct");

    let payloads = collect_effective_tir_expression_overlay_payloads(&root_view)
        .expect("effective collection should succeed");
    assert!(payloads.iter().any(|(site_id, value)| {
        *site_id == child_site_id && matches!(value.kind, ExpressionKind::Int(9))
    }));
}

#[test]
fn effective_collection_preserves_outer_context_precedence_for_reused_site() {
    let mut store = TemplateIrStore::new();
    let shared_root = dynamic_node(&mut store, 1);
    let shared_site_id = dynamic_site_id(&store, shared_root);
    let shared_template = push_template(&mut store, shared_root, TemplateType::StringFunction);

    let outer_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(shared_site_id, Box::new(expression(9)))],
        })
        .expect("test overlay allocation");
    let descendant_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(shared_site_id, Box::new(expression(42)))],
        })
        .expect("test overlay allocation");
    let descendant_context = TemplateViewContext {
        expression_overlay: Some(descendant_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };

    let child_occurrence = store.next_child_template_occurrence_id();
    let child_reference = TemplateTirChildReference::new(
        shared_template,
        TemplateTirPhase::Composed,
        descendant_context,
    );
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: child_reference,
            occurrence_id: child_occurrence,
        },
        SourceLocation::default(),
    ));
    let parent_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![shared_root, child_node],
        },
        SourceLocation::default(),
    ));
    let parent_template = push_template(&mut store, parent_root, TemplateType::StringFunction);
    let outer_context = TemplateViewContext {
        expression_overlay: Some(outer_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };
    let root_view = TirView::new(
        &store,
        parent_template,
        TemplateTirPhase::Finalized,
        outer_context,
    )
    .expect("effective root view should construct");

    let payloads = collect_effective_tir_expression_overlay_payloads(&root_view)
        .expect("effective collection should succeed");

    assert_eq!(payloads.len(), 1);
    assert!(matches!(payloads[0].1.kind, ExpressionKind::Int(9)));
}

#[test]
fn effective_collection_revisits_shared_root_for_a_new_context_once() {
    let mut store = TemplateIrStore::new();
    let shared_root = dynamic_node(&mut store, 1);
    let shared_site_id = dynamic_site_id(&store, shared_root);
    let shared_template = push_template(&mut store, shared_root, TemplateType::StringFunction);
    let override_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(shared_site_id, Box::new(expression(2)))],
        })
        .expect("test overlay allocation");
    let override_context = TemplateViewContext {
        expression_overlay: Some(override_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };

    let first_occurrence = store.next_child_template_occurrence_id();
    let first_child = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                shared_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id: first_occurrence,
        },
        SourceLocation::default(),
    ));
    let second_occurrence = store.next_child_template_occurrence_id();
    let second_child = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                shared_template,
                TemplateTirPhase::Composed,
                override_context,
            ),
            occurrence_id: second_occurrence,
        },
        SourceLocation::default(),
    ));
    let parent_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![first_child, second_child],
        },
        SourceLocation::default(),
    ));
    let parent_template = push_template(&mut store, parent_root, TemplateType::StringFunction);
    let root_view = TirView::new(
        &store,
        parent_template,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )
    .expect("effective root view should construct");

    let payloads = collect_effective_tir_expression_overlay_payloads(&root_view)
        .expect("effective collection should succeed");

    assert_eq!(payloads.len(), 1);
    assert!(matches!(payloads[0].1.kind, ExpressionKind::Int(2)));
}

#[test]
fn expression_overlay_rejects_unallocated_site() {
    let mut store = TemplateIrStore::new();

    let error = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(ExpressionSiteId::new(0), Box::new(expression(1)))],
        })
        .expect_err("unallocated overlay keys must be rejected at commit");
    assert!(
        error
            .msg
            .contains("out-of-range expression-site overlay key"),
        "expected a store-owned range error, got: {}",
        error.msg
    );
}

#[test]
fn view_walker_reads_expression_overlay() {
    let mut store = TemplateIrStore::new();
    let root = dynamic_node(&mut store, 1);
    let site_id = dynamic_site_id(&store, root);
    let template = push_template(&mut store, root, TemplateType::StringFunction);
    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(site_id, Box::new(expression(42)))],
        })
        .expect("test overlay allocation");
    let view_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };
    let view = TirView::with_minimum_phase(
        &store,
        template,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        view_context,
    )
    .expect("view should construct");

    let payloads = collect_view(&view).expect("view walk should succeed");
    assert!(matches!(payloads[0].kind, ExpressionKind::Int(42)));
}

#[test]
fn nested_expression_walker_enters_same_store_template_view() {
    let mut store = TemplateIrStore::new();
    let root = dynamic_node(&mut store, 7);
    let template_id = push_template(&mut store, root, TemplateType::StringFunction);
    let expression = Expression::template(
        Template {
            tir_reference: TemplateTirReference {
                root: template_id,
                phase: TemplateTirPhase::Finalized,
                context: TemplateViewContext::default(),
            },
            location: SourceLocation::default(),
        },
        ValueMode::ImmutableOwned,
    );
    let mut payloads = Vec::new();

    walk_expression_payloads_with_nested_tir_views(&expression, &store, &mut |value| {
        payloads.push(value.clone());
        Ok(())
    })
    .expect("nested expression walk should succeed");

    assert!(
        payloads
            .iter()
            .any(|value| matches!(value.kind, ExpressionKind::Int(7)))
    );
}

#[test]
fn missing_child_template_is_reported() {
    let mut store = TemplateIrStore::new();
    let occurrence_id = store.next_child_template_occurrence_id();
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                TemplateIrId::new(99),
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            ),
            occurrence_id,
        },
        SourceLocation::default(),
    ));
    let mut mutator = CountingMutator::default();

    let error = mutate_finalized_tir_body_root_expression_payloads(&mut store, root, &mut mutator)
        .expect_err("missing child should fail");
    assert!(error.msg.contains("missing child template"));
}

// -------------------------
//  Helpers for control-flow, insert, runtime-slot and nested-view cases
// -------------------------

fn bool_expression(value: bool) -> Expression {
    Expression::bool(value, SourceLocation::default(), ValueMode::ImmutableOwned)
}

fn mutate_from_root(
    store: &mut TemplateIrStore,
    root: TemplateIrNodeId,
) -> Result<CountingMutator, CompilerError> {
    let mut mutator = CountingMutator::default();
    mutate_finalized_tir_body_root_expression_payloads(store, root, &mut mutator)?;
    Ok(mutator)
}

fn branch_selector_site_id(store: &TemplateIrStore, node_id: TemplateIrNodeId) -> ExpressionSiteId {
    let node = store
        .get_node(node_id)
        .expect("branch chain node should exist");
    let TemplateIrNodeKind::BranchChain { branches, .. } = &node.kind else {
        panic!("expected branch chain node, got {:?}", node.kind);
    };
    branches[0].selector_site_id
}

fn finalized_tir_reference(
    template_id: TemplateIrId,
    context: TemplateViewContext,
) -> TemplateTirReference {
    TemplateTirReference {
        root: template_id,
        phase: TemplateTirPhase::Finalized,
        context,
    }
}

fn template_with_reference(reference: TemplateTirReference) -> Template {
    Template {
        tir_reference: reference,
        location: SourceLocation::default(),
    }
}

fn template_expression(template: Template) -> Expression {
    Expression::template(template, ValueMode::ImmutableOwned)
}

fn runtime_expression(operand: Expression) -> Expression {
    Expression::new(
        ExpressionKind::Runtime(ExpressionRpn {
            items: vec![ExpressionRpnItem::Operand(operand)],
        }),
        SourceLocation::default(),
        builtin_type_ids::INT,
        DataType::Int,
        ValueMode::ImmutableOwned,
    )
}

fn coerced_expression(value: Expression) -> Expression {
    Expression::new(
        ExpressionKind::Coerced {
            value: Box::new(value),
            to_type: builtin_type_ids::STRING,
        },
        SourceLocation::default(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    )
}

fn collect_nested_expression_payloads(
    expression: &Expression,
    store: &TemplateIrStore,
) -> Result<Vec<Expression>, CompilerError> {
    let mut payloads = Vec::new();
    walk_expression_payloads_with_nested_tir_views(expression, store, &mut |payload| {
        payloads.push(payload.clone());
        Ok(())
    })?;
    Ok(payloads)
}

fn runtime_slot_plan_store(
    store: &mut TemplateIrStore,
    source_root: TemplateIrNodeId,
    site_render_root: TemplateIrNodeId,
) -> TemplateSlotPlanId {
    let contribution_source = RuntimeSlotContributionSourceId(0);
    store.push_slot_plan(TemplateSlotPlan {
        location: SourceLocation::default(),
        contribution_sources: vec![TemplateSlotContributionSourcePlan {
            source: contribution_source,
            target: SlotKey::Default,
            render_root: source_root,
            renders_wrapper_unconditionally: true,
            location: SourceLocation::default(),
        }],
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root: site_render_root,
            location: SourceLocation::default(),
        }],
    })
}

// -------------------------
//  Mutation walker coverage
// -------------------------

#[test]
fn mutates_loop_header_body_and_aggregate_wrapper_expression() {
    let mut store = TemplateIrStore::new();
    let body = dynamic_node(&mut store, 2);
    let aggregate_wrapper = dynamic_node(&mut store, 3);
    let header = TemplateLoopHeader::Conditional {
        condition: Box::new(expression(1)),
    };
    let header_sites = store.allocate_loop_header_expression_sites(&header);
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper: Some(aggregate_wrapper),
        },
        SourceLocation::default(),
    ));

    let mutator = mutate_from_root(&mut store, root).expect("walk should succeed");

    assert_eq!(mutator.count, 3);
}

#[test]
fn mutates_child_template_and_nested_child_template_expression() {
    let mut store = TemplateIrStore::new();
    let grandchild_root = dynamic_node(&mut store, 3);
    let grandchild_template =
        push_template(&mut store, grandchild_root, TemplateType::StringFunction);

    let nested_child_occurrence = store.next_child_template_occurrence_id();
    let nested_child_reference = TemplateTirChildReference::new(
        grandchild_template,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let nested_child = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: nested_child_reference,
            occurrence_id: nested_child_occurrence,
        },
        SourceLocation::default(),
    ));
    let child_template = push_template(&mut store, nested_child, TemplateType::StringFunction);

    let root_occurrence = store.next_child_template_occurrence_id();
    let root_reference = TemplateTirChildReference::new(
        child_template,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: root_reference,
            occurrence_id: root_occurrence,
        },
        SourceLocation::default(),
    ));

    let mutator = mutate_from_root(&mut store, root).expect("walk should succeed");

    assert_eq!(mutator.count, 1);
}

#[test]
fn mutates_insert_contribution_child_expression() {
    let mut store = TemplateIrStore::new();
    let insert_root = dynamic_node(&mut store, 1);
    let insert_template = push_template(&mut store, insert_root, TemplateType::StringFunction);
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::InsertContribution {
            template: insert_template,
        },
        SourceLocation::default(),
    ));

    let mutator = mutate_from_root(&mut store, root).expect("walk should succeed");

    assert_eq!(mutator.count, 1);
}

#[test]
fn mutates_runtime_slot_plan_wrapper_source_and_site_render_piece() {
    let mut store = TemplateIrStore::new();
    let wrapper_root = dynamic_node(&mut store, 1);
    let source_root = dynamic_node(&mut store, 2);
    let site_render_root = dynamic_node(&mut store, 3);
    let slot_plan_id = runtime_slot_plan_store(&mut store, source_root, site_render_root);

    let mut runtime_template = TemplateIr::new(
        wrapper_root,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );
    runtime_template.runtime_slot_plan = Some(slot_plan_id);
    let runtime_template_id = store.push_template(runtime_template);
    let occurrence_id = store.next_child_template_occurrence_id();
    let runtime_reference = TemplateTirChildReference::new(
        runtime_template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: runtime_reference,
            occurrence_id,
        },
        SourceLocation::default(),
    ));

    let mutator = mutate_from_root(&mut store, root).expect("walk should succeed");

    assert_eq!(mutator.count, 3);
}

#[test]
fn reports_missing_runtime_slot_site_as_compiler_error() {
    let mut store = TemplateIrStore::new();
    let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
        location: SourceLocation::default(),
        contribution_sources: vec![],
        slot_sites: vec![],
    });
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::RuntimeSlotSite {
            plan: slot_plan_id,
            site: RuntimeSlotSiteId(0),
        },
        SourceLocation::default(),
    ));

    let error = mutate_from_root(&mut store, root).expect_err("missing slot site should fail");

    assert!(error.msg.contains("missing runtime slot site"));
}

// -------------------------
//  Structural and effective collection coverage
// -------------------------

#[test]
fn collects_runtime_slot_plan_wrapper_source_and_site_render_piece_dynamic_payloads() {
    let mut store = TemplateIrStore::new();
    let wrapper_root = dynamic_node(&mut store, 1);
    let source_root = dynamic_node(&mut store, 2);
    let site_render_root = dynamic_node(&mut store, 3);
    let slot_plan_id = runtime_slot_plan_store(&mut store, source_root, site_render_root);

    let mut runtime_template = TemplateIr::new(
        wrapper_root,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );
    runtime_template.runtime_slot_plan = Some(slot_plan_id);
    let runtime_template_id = store.push_template(runtime_template);

    let payloads = collect_tir_expression_overlay_payloads(&store, runtime_template_id)
        .expect("expression overlay collection should succeed");

    assert_eq!(payloads.len(), 3);
}

#[test]
fn collects_dynamic_payloads_branch_selectors_and_loop_headers() {
    let mut store = TemplateIrStore::new();
    let branch_body = dynamic_node(&mut store, 2);
    let selector_site_id = store.next_expression_site_id();
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(expression(1)),
        branch_body,
        SourceLocation::default(),
        selector_site_id,
    );
    let branch_chain = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::BranchChain {
            branches: vec![branch],
            fallback: None,
        },
        SourceLocation::default(),
    ));

    let loop_body = dynamic_node(&mut store, 4);
    let aggregate_wrapper = dynamic_node(&mut store, 5);
    let header = TemplateLoopHeader::Conditional {
        condition: Box::new(expression(3)),
    };
    let header_sites = store.allocate_loop_header_expression_sites(&header);
    let header_condition_site_id = match header_sites {
        TemplateLoopHeaderExpressionSites::Conditional { condition } => condition,
        _ => panic!("expected conditional loop header sites"),
    };
    let loop_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body: loop_body,
            aggregate_wrapper: Some(aggregate_wrapper),
        },
        SourceLocation::default(),
    ));

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![branch_chain, loop_node],
        },
        SourceLocation::default(),
    ));
    let template_id = push_template(&mut store, root, TemplateType::StringFunction);

    let payloads = collect_tir_expression_overlay_payloads(&store, template_id)
        .expect("expression overlay collection should succeed");

    assert_eq!(
        payloads.len(),
        5,
        "collector should include dynamic nodes, branch selectors, and loop header expressions"
    );
    assert!(
        payloads
            .iter()
            .any(|(site_id, expression)| *site_id == selector_site_id
                && matches!(expression.kind, ExpressionKind::Int(1))),
        "branch selector payload should be keyed by the branch selector site ID"
    );
    assert!(
        payloads
            .iter()
            .any(|(site_id, expression)| *site_id == header_condition_site_id
                && matches!(expression.kind, ExpressionKind::Int(3))),
        "loop header payload should be keyed by the allocated header expression-site ID"
    );
}

#[test]
fn structural_collection_sorts_expression_sites_once_after_keyed_accumulation() {
    let mut store = TemplateIrStore::new();
    let first = dynamic_node(&mut store, 1);
    let first_site_id = dynamic_site_id(&store, first);
    let second = dynamic_node(&mut store, 2);
    let second_site_id = dynamic_site_id(&store, second);
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![second, first],
        },
        SourceLocation::default(),
    ));
    let template_id = push_template(&mut store, root, TemplateType::StringFunction);

    let payloads = collect_tir_expression_overlay_payloads(&store, template_id)
        .expect("structural collection should succeed");

    assert_eq!(
        payloads
            .iter()
            .map(|(site_id, _)| *site_id)
            .collect::<Vec<_>>(),
        vec![first_site_id, second_site_id]
    );
}

#[test]
fn replacement_preserves_other_context_dimensions_and_rejects_duplicate_sites() {
    let mut store = TemplateIrStore::new();
    let first = dynamic_node(&mut store, 1);
    let first_site_id = dynamic_site_id(&store, first);
    let second = dynamic_node(&mut store, 2);
    let second_site_id = dynamic_site_id(&store, second);
    let existing_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (first_site_id, Box::new(expression(1))),
                (second_site_id, Box::new(expression(2))),
            ],
        })
        .expect("existing overlay allocation should succeed");
    let base_context = TemplateViewContext {
        expression_overlay: Some(existing_overlay),
        slot_resolution: Some(TirSlotResolutionOverlayId::new(3)),
        wrapper_context: Some(TirWrapperContextOverlayId::new(4)),
    };

    let replacement_context = replace_expression_overlay_entries(
        &mut store,
        base_context,
        vec![(first_site_id, Box::new(expression(9)))],
    )
    .expect("overlay replacement should succeed");

    assert_eq!(
        replacement_context.slot_resolution,
        base_context.slot_resolution
    );
    assert_eq!(
        replacement_context.wrapper_context,
        base_context.wrapper_context
    );
    let replacement_overlay = store
        .expression_overlay(
            replacement_context
                .expression_overlay
                .expect("replacement should allocate an expression overlay"),
        )
        .expect("replacement overlay should remain in the store");
    assert_eq!(
        replacement_overlay
            .overrides
            .iter()
            .map(|(site_id, _)| *site_id)
            .collect::<Vec<_>>(),
        vec![first_site_id, second_site_id]
    );
    assert!(matches!(
        replacement_overlay
            .expression_for_site(first_site_id)
            .expect("replaced site should exist")
            .kind,
        ExpressionKind::Int(9)
    ));

    let error = replace_expression_overlay_entries(
        &mut store,
        base_context,
        vec![
            (first_site_id, Box::new(expression(3))),
            (first_site_id, Box::new(expression(4))),
        ],
    )
    .expect_err("duplicate replacement sites must fail before allocation");
    assert!(error.msg.contains("duplicate expression site"));
}

#[test]
fn range_loop_header_positions_are_visited_by_mutation_and_collected_by_site_id() {
    let mut store = TemplateIrStore::new();
    let body = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        SourceLocation::default(),
    ));
    let header = TemplateLoopHeader::Range {
        bindings: Box::new(LoopBindings {
            item: None,
            index: None,
        }),
        range: Box::new(RangeLoopSpec {
            start: expression(1),
            end: expression(10),
            end_kind: RangeEndKind::Exclusive,
            step: Some(expression(2)),
        }),
    };
    let header_sites = store.allocate_loop_header_expression_sites(&header);
    let (start_site_id, end_site_id, step_site_id) = match header_sites {
        TemplateLoopHeaderExpressionSites::Range { start, end, step } => (
            start,
            end,
            step.expect("range step site should be allocated"),
        ),
        _ => panic!("expected range loop header sites"),
    };
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper: None,
        },
        SourceLocation::default(),
    ));
    let template_id = push_template(&mut store, root, TemplateType::StringFunction);

    // Structural collection: the start, end and step payloads are returned
    // keyed by their allocated expression-site IDs with exact payload values.
    let payloads = collect_tir_expression_overlay_payloads(&store, template_id)
        .expect("expression overlay collection should succeed");
    assert_eq!(payloads.len(), 3);
    assert!(payloads.iter().any(|(site_id, expression)| {
        *site_id == start_site_id && matches!(expression.kind, ExpressionKind::Int(1))
    }));
    assert!(payloads.iter().any(|(site_id, expression)| {
        *site_id == end_site_id && matches!(expression.kind, ExpressionKind::Int(10))
    }));
    assert!(payloads.iter().any(|(site_id, expression)| {
        *site_id == step_site_id && matches!(expression.kind, ExpressionKind::Int(2))
    }));

    // Mutation walker: the same three range header positions are visited in
    // place, so the mutation and collection rows stay truly parallel.
    let mutator = mutate_from_root(&mut store, root).expect("mutation walk should succeed");
    assert_eq!(
        mutator.count, 3,
        "mutation should visit start, end and step"
    );

    let node = store.get_node(root).expect("range loop root should exist");
    let TemplateIrNodeKind::Loop {
        header: TemplateLoopHeader::Range { range, .. },
        ..
    } = &node.kind
    else {
        panic!("expected range loop root");
    };
    assert!(range.start.contains_regular_division);
    assert!(range.end.contains_regular_division);
    assert!(
        range
            .step
            .as_ref()
            .expect("range step should remain present")
            .contains_regular_division
    );
}

// -------------------------
//  Effective view walker overlay coverage
// -------------------------

#[test]
fn view_walker_reads_branch_selector_overlay() {
    let mut store = TemplateIrStore::new();
    let (template_id, selector_site_id) = {
        let body = dynamic_node(&mut store, 2);
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(false)),
            body,
            SourceLocation::default(),
            store.next_expression_site_id(),
        );
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_branch_chain_node(vec![branch], None, SourceLocation::default());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );
        let selector_site_id = branch_selector_site_id(&store, root);
        (template_id, selector_site_id)
    };

    let overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(selector_site_id, Box::new(bool_expression(true)))],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };

    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("view should construct");

    let payloads = collect_view(&view).expect("walk should succeed");

    assert_eq!(payloads.len(), 2);
    assert!(
        payloads
            .iter()
            .any(|e| matches!(e.kind, ExpressionKind::Bool(true))),
        "overlay branch selector should be visited"
    );
    assert!(
        payloads
            .iter()
            .any(|e| matches!(e.kind, ExpressionKind::Int(2))),
        "body expression should be visited"
    );
    assert!(
        !payloads
            .iter()
            .any(|e| matches!(e.kind, ExpressionKind::Bool(false))),
        "structural branch selector should not be visited"
    );
}

#[test]
fn view_walker_reads_loop_header_overlay() {
    let mut store = TemplateIrStore::new();
    let (template_id, start_site_id, step_site_id) = {
        let body = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children: vec![] },
            SourceLocation::default(),
        ));
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: expression(1),
                end: expression(10),
                end_kind: RangeEndKind::Exclusive,
                step: Some(expression(2)),
            }),
        };
        let header_sites = store.allocate_loop_header_expression_sites(&header);
        let (start_site_id, step_site_id) = match header_sites {
            TemplateLoopHeaderExpressionSites::Range {
                start,
                end: _,
                step,
            } => (start, step.expect("range step site should be allocated")),
            _ => panic!("expected range loop header sites"),
        };
        let root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Loop {
                header,
                header_sites,
                body,
                aggregate_wrapper: None,
            },
            SourceLocation::default(),
        ));
        let template_id = push_template(&mut store, root, TemplateType::StringFunction);
        (template_id, start_site_id, step_site_id)
    };

    let overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (start_site_id, Box::new(expression(100))),
                (step_site_id, Box::new(expression(50))),
            ],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };

    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("view should construct");

    let payloads = collect_view(&view).expect("walk should succeed");

    assert_eq!(payloads.len(), 3);
    assert!(
        payloads
            .iter()
            .any(|e| matches!(e.kind, ExpressionKind::Int(100))),
        "overlay range start should be visited"
    );
    assert!(
        payloads
            .iter()
            .any(|e| matches!(e.kind, ExpressionKind::Int(10))),
        "structural range end should be visited"
    );
    assert!(
        payloads
            .iter()
            .any(|e| matches!(e.kind, ExpressionKind::Int(50))),
        "overlay range step should be visited"
    );
    assert!(
        !payloads
            .iter()
            .any(|e| matches!(e.kind, ExpressionKind::Int(1))),
        "structural range start should not be visited"
    );
    assert!(
        !payloads
            .iter()
            .any(|e| matches!(e.kind, ExpressionKind::Int(2))),
        "structural range step should not be visited"
    );
}

#[test]
fn view_walker_uses_parent_overlay_for_the_same_child_root() {
    let mut store = TemplateIrStore::new();
    let empty_context = TemplateViewContext::default();

    let (child_template_id, child_site_id) = {
        let child_root = dynamic_node(&mut store, 1);
        let child_site_id = dynamic_site_id(&store, child_root);
        let child_template_id = push_template(&mut store, child_root, TemplateType::StringFunction);
        (child_template_id, child_site_id)
    };

    let override_context = {
        let expression_overlay_id = store
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(child_site_id, Box::new(expression(42)))],
            })
            .expect("test overlay allocation");
        TemplateViewContext {
            expression_overlay: Some(expression_overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        }
    };

    let parent_template_id = {
        let structural_occurrence_id = store.next_child_template_occurrence_id();
        let structural_child = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: TemplateTirChildReference::new(
                    child_template_id,
                    TemplateTirPhase::Finalized,
                    empty_context,
                ),
                occurrence_id: structural_occurrence_id,
            },
            SourceLocation::default(),
        ));
        let overlaid_occurrence_id = store.next_child_template_occurrence_id();
        let overlaid_child = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: TemplateTirChildReference::new(
                    child_template_id,
                    TemplateTirPhase::Finalized,
                    override_context,
                ),
                occurrence_id: overlaid_occurrence_id,
            },
            SourceLocation::default(),
        ));
        let parent_root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![structural_child, overlaid_child],
            },
            SourceLocation::default(),
        ));
        push_template(&mut store, parent_root, TemplateType::StringFunction)
    };

    let view = TirView::new(
        &store,
        parent_template_id,
        TemplateTirPhase::Finalized,
        empty_context,
    )
    .expect("parent view should construct");

    let payloads = collect_view(&view).expect("walk should succeed");

    // Structural child transitions ignore referenced expression overlays. Both
    // occurrences therefore enter the same exact child view and are visited
    // once, retaining only the structural expression.
    assert_eq!(payloads.len(), 1);
    assert!(
        payloads
            .iter()
            .any(|expression| matches!(expression.kind, ExpressionKind::Int(1)))
    );
    assert!(
        !payloads
            .iter()
            .any(|expression| matches!(expression.kind, ExpressionKind::Int(42)))
    );
}

#[test]
fn view_walker_reads_insert_contribution_effective_overlay() {
    let mut store = TemplateIrStore::new();
    let (parent_template_id, insert_site_id) = {
        let insert_root = dynamic_node(&mut store, 1);
        let insert_template_id =
            push_template(&mut store, insert_root, TemplateType::StringFunction);
        let insert_site_id = dynamic_site_id(&store, insert_root);

        let parent_root = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_insert_contribution_node(insert_template_id, SourceLocation::default())
        };
        let parent_template_id =
            push_template(&mut store, parent_root, TemplateType::StringFunction);
        (parent_template_id, insert_site_id)
    };

    let overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(insert_site_id, Box::new(expression(42)))],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };

    let view = TirView::with_minimum_phase(
        &store,
        parent_template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("parent view should construct");

    let payloads = collect_view(&view).expect("walk should succeed");

    assert_eq!(payloads.len(), 1);
    assert!(
        matches!(payloads[0].kind, ExpressionKind::Int(42)),
        "insert contribution should recurse through a child view and read the inherited overlay override, not the structural payload"
    );
}

#[test]
fn view_walker_reports_missing_insert_contribution_template() {
    let mut store = TemplateIrStore::new();
    let parent_template_id = {
        let parent_root = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_insert_contribution_node(TemplateIrId::new(99), SourceLocation::default())
        };
        push_template(&mut store, parent_root, TemplateType::StringFunction)
    };

    let context = TemplateViewContext::default();

    let view = TirView::new(
        &store,
        parent_template_id,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("parent view should construct");

    let error = collect_view(&view)
        .expect_err("a missing insert contribution template must fail explicitly");

    assert!(
        error.msg.contains("root_template"),
        "error must come from the required insert view root resolution: {}",
        error.msg,
    );
    assert!(
        error.msg.contains("missing"),
        "error must report the missing insert template: {}",
        error.msg
    );
}

// -------------------------
//  Nested expression-and-TIR-view walker
// -------------------------

#[test]
fn nested_walker_inspects_runtime_and_coerced_operands() {
    let mut store = TemplateIrStore::new();
    let empty_context = TemplateViewContext::default();

    let template_id = {
        let root = dynamic_node(&mut store, 99);
        push_template(&mut store, root, TemplateType::StringFunction)
    };

    let template = template_with_reference(finalized_tir_reference(template_id, empty_context));
    let template_expr = template_expression(template);
    let coerced = coerced_expression(template_expr);
    let expression = runtime_expression(coerced);

    let payloads =
        collect_nested_expression_payloads(&expression, &store).expect("walk should succeed");

    assert_eq!(payloads.len(), 1);
    assert!(
        matches!(payloads[0].kind, ExpressionKind::Int(99)),
        "TIR dynamic expression behind Coerced and Runtime should be visited"
    );
}

#[test]
fn nested_walker_shares_visited_set_between_tir_child_and_expression_template() {
    let mut store = TemplateIrStore::new();
    let empty_context = TemplateViewContext::default();

    let child_template_id = {
        let child_root = dynamic_node(&mut store, 42);
        push_template(&mut store, child_root, TemplateType::StringFunction)
    };

    let child_template_for_expr =
        template_with_reference(finalized_tir_reference(child_template_id, empty_context));

    let parent_template_id = {
        let occurrence_id = store.next_child_template_occurrence_id();
        let child_ref = TemplateTirChildReference::new(
            child_template_id,
            TemplateTirPhase::Finalized,
            empty_context,
        );
        let child_node = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: child_ref,
                occurrence_id,
            },
            SourceLocation::default(),
        ));

        let site_id = store.next_expression_site_id();
        let expr_node = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: Box::new(template_expression(child_template_for_expr)),
                origin: TemplateSegmentOrigin::Body,
                reactive_subscription: None,
                site_id,
            },
            SourceLocation::default(),
        ));

        let parent_root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![child_node, expr_node],
            },
            SourceLocation::default(),
        ));
        push_template(&mut store, parent_root, TemplateType::StringFunction)
    };

    let parent_template =
        template_with_reference(finalized_tir_reference(parent_template_id, empty_context));
    let expression = template_expression(parent_template);

    let payloads =
        collect_nested_expression_payloads(&expression, &store).expect("walk should succeed");

    let int_42_count = payloads
        .iter()
        .filter(|e| matches!(e.kind, ExpressionKind::Int(42)))
        .count();
    assert_eq!(
        int_42_count, 1,
        "shared visited set should visit child B only once across TIR child and expression template paths"
    );
}
