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
    TemplateIr, TemplateIrNode, TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
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
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView,
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
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn expression(
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value: i64,
) -> ExprId {
    expressions
        .fixed_scalar(
            const_values,
            FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("test literal should construct")
        .id()
}

/// Whether this expression is one exact `I32` literal with the expected value.
fn is_i32_literal(
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    id: ExprId,
    expected: i64,
) -> bool {
    let view = expressions.view(id).expect("test expression should exist");
    matches!(view.operation(), ExpressionOperationView::Constant(payload)
        if matches!(const_values.payload(payload), Some(ConstValuePayload::FixedScalar(value))
            if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(expected)))
}

/// Whether the mutator marked this expression with regular-division provenance.
fn division_provenance(expressions: &AstExpressionBuilder, id: ExprId) -> bool {
    expressions
        .view(id)
        .expect("test expression should exist")
        .classification()
        .contains_regular_division
}

fn dynamic_node(
    store: &mut TemplateIrStore,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value: i64,
) -> TemplateIrNodeId {
    let site_id = store.next_expression_site_id();
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: expression(expressions, const_values, value),
            origin: TemplateSegmentOrigin::Body,
            site_id,
        },
        None,
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
        None,
    ))
}

fn dynamic_site_id(store: &TemplateIrStore, node_id: TemplateIrNodeId) -> ExpressionSiteId {
    let node = store.get_node(node_id).expect("dynamic node should exist");
    let TemplateIrNodeKind::DynamicExpression { site_id, .. } = node.kind else {
        panic!("expected dynamic expression node");
    };
    site_id
}

fn collect_view(
    view: &TirView<'_>,
    expressions: &AstExpressionBuilder,
) -> Result<Vec<ExprId>, CompilerError> {
    let mut payloads = Vec::new();
    walk_tir_view_expression_payloads(view, expressions, &mut |expression| {
        payloads.push(expression.id());
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
        expressions: &mut AstExpressionBuilder,
        expression: &mut ExprId,
    ) -> Result<(), CompilerError> {
        self.count += 1;
        *expression = expressions
            .with_regular_division_provenance(expressions.own(*expression), true)
            .expect("test provenance should apply")
            .id();
        Ok(())
    }
}

#[test]
fn mutates_direct_dynamic_expression() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let mut mutator = CountingMutator::default();

    mutate_finalized_tir_body_root_expression_payloads(
        &mut store,
        &mut expressions,
        root,
        &mut mutator,
    )
    .expect("direct expression walk should succeed");

    assert_eq!(mutator.count, 1);
    let node = store.get_node(root).expect("root should exist");
    let TemplateIrNodeKind::DynamicExpression { expression, .. } = &node.kind else {
        panic!("expected dynamic expression");
    };
    assert!(
        expressions
            .view(*expression)
            .expect("mutated expression should exist")
            .classification()
            .contains_regular_division
    );
}

#[test]
fn mutates_conditional_selector_and_body_expression() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let body = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let selector = TemplateBranchSelector::Bool(expression(&mut expressions, &mut const_values, 1));
    let root = TemplateIrBuilder::new(&mut store).push_conditional_node(selector, body, None);
    let mut mutator = CountingMutator::default();

    mutate_finalized_tir_body_root_expression_payloads(
        &mut store,
        &mut expressions,
        root,
        &mut mutator,
    )
    .expect("conditional expression walk should succeed");

    assert_eq!(mutator.count, 2);
}

#[test]
fn mutates_nested_same_store_child_expression() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let child_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 3);
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
        None,
    ));
    let mut mutator = CountingMutator::default();

    mutate_finalized_tir_body_root_expression_payloads(
        &mut store,
        &mut expressions,
        child_node,
        &mut mutator,
    )
    .expect("child expression walk should succeed");

    assert_eq!(mutator.count, 1);
}

#[test]
fn structural_collection_ignores_child_expression_overlay() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let child_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let child_site_id = dynamic_site_id(&store, child_root);
    let child_template = push_template(&mut store, child_root, TemplateType::StringFunction);
    let child_expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                child_site_id,
                expression(&mut expressions, &mut const_values, 9),
            )],
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
        None,
    ));
    let parent_template = push_template(&mut store, child_node, TemplateType::StringFunction);

    let payloads = collect_tir_expression_overlay_payloads(&store, parent_template)
        .expect("structural collection should succeed");

    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].0, child_site_id);
    assert!(is_i32_literal(
        &expressions,
        &const_values,
        payloads[0].1,
        1
    ));
}

#[test]
fn effective_collection_reads_same_store_child_overlay() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let child_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let child_site_id = dynamic_site_id(&store, child_root);
    let child_template = push_template(&mut store, child_root, TemplateType::StringFunction);
    let child_expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                child_site_id,
                expression(&mut expressions, &mut const_values, 9),
            )],
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
        None,
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

    let payloads = collect_effective_tir_expression_overlay_payloads(&root_view, &expressions)
        .expect("effective collection should succeed");
    assert!(payloads.iter().any(|(site_id, value)| {
        *site_id == child_site_id && is_i32_literal(&expressions, &const_values, *value, 9)
    }));
}

#[test]
fn effective_collection_preserves_outer_context_precedence_for_reused_site() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let shared_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let shared_site_id = dynamic_site_id(&store, shared_root);
    let shared_template = push_template(&mut store, shared_root, TemplateType::StringFunction);

    let outer_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                shared_site_id,
                expression(&mut expressions, &mut const_values, 9),
            )],
        })
        .expect("test overlay allocation");
    let descendant_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                shared_site_id,
                expression(&mut expressions, &mut const_values, 42),
            )],
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
        None,
    ));
    let parent_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![shared_root, child_node],
        },
        None,
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

    let payloads = collect_effective_tir_expression_overlay_payloads(&root_view, &expressions)
        .expect("effective collection should succeed");

    assert_eq!(payloads.len(), 1);
    assert!(is_i32_literal(
        &expressions,
        &const_values,
        payloads[0].1,
        9
    ));
}

#[test]
fn effective_collection_revisits_shared_root_for_a_new_context_once() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let shared_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let shared_site_id = dynamic_site_id(&store, shared_root);
    let shared_template = push_template(&mut store, shared_root, TemplateType::StringFunction);
    let override_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                shared_site_id,
                expression(&mut expressions, &mut const_values, 2),
            )],
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
        None,
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
        None,
    ));
    let parent_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![first_child, second_child],
        },
        None,
    ));
    let parent_template = push_template(&mut store, parent_root, TemplateType::StringFunction);
    let root_view = TirView::new(
        &store,
        parent_template,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )
    .expect("effective root view should construct");

    let payloads = collect_effective_tir_expression_overlay_payloads(&root_view, &expressions)
        .expect("effective collection should succeed");

    assert_eq!(payloads.len(), 1);
    assert!(is_i32_literal(
        &expressions,
        &const_values,
        payloads[0].1,
        2
    ));
}

#[test]
fn expression_overlay_rejects_unallocated_site() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let error = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                ExpressionSiteId::new(0),
                expression(&mut expressions, &mut const_values, 1),
            )],
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let site_id = dynamic_site_id(&store, root);
    let template = push_template(&mut store, root, TemplateType::StringFunction);
    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(site_id, expression(&mut expressions, &mut const_values, 42))],
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

    let payloads = collect_view(&view, &expressions).expect("view walk should succeed");
    assert!(is_i32_literal(&expressions, &const_values, payloads[0], 42));
}

#[test]
fn nested_expression_walker_enters_same_store_template_view() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let root = dynamic_node(&mut store, &mut expressions, &mut const_values, 7);
    let template_id = push_template(&mut store, root, TemplateType::StringFunction);
    let expression = expressions
        .template(
            Template {
                tir_reference: TemplateTirReference {
                    root: template_id,
                    phase: TemplateTirPhase::Finalized,
                    context: TemplateViewContext::default(),
                },
                span: None,
            },
            ValueMode::ImmutableOwned,
        )
        .expect("test template expression should construct")
        .id();
    let mut payloads = Vec::new();

    let view = expressions
        .view(expression)
        .expect("test template expression should exist");
    walk_expression_payloads_with_nested_tir_views(view, &store, &expressions, &mut |value| {
        payloads.push(value.id());
        Ok(())
    })
    .expect("nested expression walk should succeed");

    assert!(
        payloads
            .iter()
            .any(|value| is_i32_literal(&expressions, &const_values, *value, 7))
    );
}

#[test]
fn missing_child_template_is_reported() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
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
        None,
    ));
    let mut mutator = CountingMutator::default();

    let error = mutate_finalized_tir_body_root_expression_payloads(
        &mut store,
        &mut expressions,
        root,
        &mut mutator,
    )
    .expect_err("missing child should fail");
    assert!(error.msg.contains("missing child template"));
}

// -------------------------
//  Helpers for control-flow, insert, runtime-slot and nested-view cases
// -------------------------

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

fn mutate_from_root(
    store: &mut TemplateIrStore,
    expressions: &mut AstExpressionBuilder,
    root: TemplateIrNodeId,
) -> Result<CountingMutator, CompilerError> {
    let mut mutator = CountingMutator::default();
    mutate_finalized_tir_body_root_expression_payloads(store, expressions, root, &mut mutator)?;
    Ok(mutator)
}

fn conditional_selector_site_id(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> ExpressionSiteId {
    let node = store
        .get_node(node_id)
        .expect("conditional node should exist");
    let TemplateIrNodeKind::Conditional {
        selector_site_id, ..
    } = &node.kind
    else {
        panic!("expected conditional node, got {:?}", node.kind);
    };
    *selector_site_id
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
        span: None,
    }
}

fn template_expression(expressions: &mut AstExpressionBuilder, template: Template) -> ExprId {
    expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("test template expression should construct")
        .id()
}

fn runtime_expression(
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    type_environment: &TypeEnvironment,
    operand: ExprId,
) -> ExprId {
    crate::compiler_frontend::tests::type_id_fixture_support::runtime_expr(
        expressions,
        const_values,
        vec![
            ExpressionRpnItem::Operand(operand),
            ExpressionRpnItem::Operand(operand),
            ExpressionRpnItem::Operator {
                operator:
                    crate::compiler_frontend::ast::expressions::expression_kind::Operator::Equality,
                span: None,
            },
        ],
        builtin_type_ids::BOOL,
        None,
        ValueMode::ImmutableOwned,
        type_environment,
    )
}

fn coerced_expression(expressions: &mut AstExpressionBuilder, value: ExprId) -> ExprId {
    expressions
        .coerced(expressions.own(value), builtin_type_ids::STRING)
        .expect("test coerced expression should construct")
        .id()
}

fn collect_nested_expression_payloads(
    expression: ExprId,
    store: &TemplateIrStore,
    expressions: &AstExpressionBuilder,
) -> Result<Vec<ExprId>, CompilerError> {
    let mut payloads = Vec::new();
    let view = expressions
        .view(expression)
        .expect("test expression should exist");
    walk_expression_payloads_with_nested_tir_views(view, store, expressions, &mut |payload| {
        payloads.push(payload.id());
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
        contribution_sources: vec![TemplateSlotContributionSourcePlan {
            source: contribution_source,
            target: SlotKey::Default,
            render_root: source_root,
            renders_wrapper_unconditionally: true,
            span: None,
        }],
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root: site_render_root,
            span: None,
        }],
    })
}

// -------------------------
//  Mutation walker coverage
// -------------------------

#[test]
fn mutates_loop_header_body_and_aggregate_wrapper_expression() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let body = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let aggregate_wrapper = dynamic_node(&mut store, &mut expressions, &mut const_values, 3);
    let header = TemplateLoopHeader::Conditional {
        condition: expression(&mut expressions, &mut const_values, 1),
    };
    let header_sites = store.allocate_loop_header_expression_sites(&header);
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper: Some(aggregate_wrapper),
        },
        None,
    ));

    let mutator =
        mutate_from_root(&mut store, &mut expressions, root).expect("walk should succeed");

    assert_eq!(mutator.count, 3);
}

#[test]
fn mutates_child_template_and_nested_child_template_expression() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let grandchild_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 3);
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
        None,
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
        None,
    ));

    let mutator =
        mutate_from_root(&mut store, &mut expressions, root).expect("walk should succeed");

    assert_eq!(mutator.count, 1);
}

#[test]
fn mutates_insert_contribution_child_expression() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let insert_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);

    let insert_template = push_template(&mut store, insert_root, TemplateType::StringFunction);
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::InsertContribution {
            template: insert_template,
        },
        None,
    ));

    let mutator =
        mutate_from_root(&mut store, &mut expressions, root).expect("walk should succeed");

    assert_eq!(mutator.count, 1);
}

#[test]
fn mutates_runtime_slot_plan_wrapper_source_and_site_render_piece() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let wrapper_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let source_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let site_render_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 3);

    let slot_plan_id = runtime_slot_plan_store(&mut store, source_root, site_render_root);

    let mut runtime_template = TemplateIr::new(
        wrapper_root,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        None,
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
        None,
    ));

    let mutator =
        mutate_from_root(&mut store, &mut expressions, root).expect("walk should succeed");

    assert_eq!(mutator.count, 3);
}

#[test]
fn reports_missing_runtime_slot_site_as_compiler_error() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();

    let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
        contribution_sources: vec![],
        slot_sites: vec![],
    });
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::RuntimeSlotSite {
            plan: slot_plan_id,
            site: RuntimeSlotSiteId(0),
        },
        None,
    ));

    let error = mutate_from_root(&mut store, &mut expressions, root)
        .expect_err("missing slot site should fail");

    assert!(error.msg.contains("missing runtime slot site"));
}

// -------------------------
//  Structural and effective collection coverage
// -------------------------

#[test]
fn collects_runtime_slot_plan_wrapper_source_and_site_render_piece_dynamic_payloads() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let wrapper_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let source_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let site_render_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 3);
    let slot_plan_id = runtime_slot_plan_store(&mut store, source_root, site_render_root);

    let mut runtime_template = TemplateIr::new(
        wrapper_root,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        None,
    );
    runtime_template.runtime_slot_plan = Some(slot_plan_id);
    let runtime_template_id = store.push_template(runtime_template);

    let payloads = collect_tir_expression_overlay_payloads(&store, runtime_template_id)
        .expect("expression overlay collection should succeed");

    assert_eq!(payloads.len(), 3);
}

#[test]
fn collects_dynamic_payloads_conditional_selectors_and_loop_headers() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let conditional_body = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let selector = TemplateBranchSelector::Bool(expression(&mut expressions, &mut const_values, 1));
    let conditional =
        TemplateIrBuilder::new(&mut store).push_conditional_node(selector, conditional_body, None);
    let selector_site_id = conditional_selector_site_id(&store, conditional);

    let loop_body = dynamic_node(&mut store, &mut expressions, &mut const_values, 4);
    let aggregate_wrapper = dynamic_node(&mut store, &mut expressions, &mut const_values, 5);
    let header = TemplateLoopHeader::Conditional {
        condition: expression(&mut expressions, &mut const_values, 3),
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
        None,
    ));

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![conditional, loop_node],
        },
        None,
    ));
    let template_id = push_template(&mut store, root, TemplateType::StringFunction);

    let payloads = collect_tir_expression_overlay_payloads(&store, template_id)
        .expect("expression overlay collection should succeed");

    assert_eq!(
        payloads.len(),
        5,
        "collector should include dynamic nodes, conditional selectors, and loop header expressions"
    );
    assert!(
        payloads.iter().any(|(site_id, expression)| {
            *site_id == selector_site_id
                && is_i32_literal(&expressions, &const_values, *expression, 1)
        }),
        "conditional selector payload should be keyed by its selector site ID"
    );
    assert!(
        payloads.iter().any(|(site_id, expression)| {
            *site_id == header_condition_site_id
                && is_i32_literal(&expressions, &const_values, *expression, 3)
        }),
        "loop header payload should be keyed by the allocated header expression-site ID"
    );
}

#[test]
fn structural_collection_sorts_expression_sites_once_after_keyed_accumulation() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let first = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let first_site_id = dynamic_site_id(&store, first);
    let second = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let second_site_id = dynamic_site_id(&store, second);
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![second, first],
        },
        None,
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let first = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let first_site_id = dynamic_site_id(&store, first);
    let second = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let second_site_id = dynamic_site_id(&store, second);
    let existing_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (
                    first_site_id,
                    expression(&mut expressions, &mut const_values, 1),
                ),
                (
                    second_site_id,
                    expression(&mut expressions, &mut const_values, 2),
                ),
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
        vec![(
            first_site_id,
            expression(&mut expressions, &mut const_values, 9),
        )],
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
    assert!(is_i32_literal(
        &expressions,
        &const_values,
        replacement_overlay
            .expression_for_site(first_site_id)
            .expect("replaced site should exist"),
        9
    ));

    let error = replace_expression_overlay_entries(
        &mut store,
        base_context,
        vec![
            (
                first_site_id,
                expression(&mut expressions, &mut const_values, 3),
            ),
            (
                first_site_id,
                expression(&mut expressions, &mut const_values, 4),
            ),
        ],
    )
    .expect_err("duplicate replacement sites must fail before allocation");
    assert!(error.msg.contains("duplicate expression site"));
}

#[test]
fn range_loop_header_positions_are_visited_by_mutation_and_collected_by_site_id() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let body = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        None,
    ));
    let header = TemplateLoopHeader::Range {
        bindings: Box::new(LoopBindings {
            item: None,
            index: None,
        }),
        range: Box::new(RangeLoopSpec {
            start: expression(&mut expressions, &mut const_values, 1),
            end: expression(&mut expressions, &mut const_values, 10),
            end_kind: RangeEndKind::Exclusive,
            step: Some(expression(&mut expressions, &mut const_values, 2)),
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
        None,
    ));
    let template_id = push_template(&mut store, root, TemplateType::StringFunction);

    // Structural collection: the start, end and step payloads are returned
    // keyed by their allocated expression-site IDs with exact payload values.
    let payloads = collect_tir_expression_overlay_payloads(&store, template_id)
        .expect("expression overlay collection should succeed");
    assert_eq!(payloads.len(), 3);
    assert!(payloads.iter().any(|(site_id, expression)| {
        *site_id == start_site_id && is_i32_literal(&expressions, &const_values, *expression, 1)
    }));
    assert!(payloads.iter().any(|(site_id, expression)| {
        *site_id == end_site_id && is_i32_literal(&expressions, &const_values, *expression, 10)
    }));
    assert!(payloads.iter().any(|(site_id, expression)| {
        *site_id == step_site_id && is_i32_literal(&expressions, &const_values, *expression, 2)
    }));

    // Mutation walker: the same three range header positions are visited in
    // place, so the mutation and collection rows stay truly parallel.
    let mutator =
        mutate_from_root(&mut store, &mut expressions, root).expect("mutation walk should succeed");
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
    assert!(division_provenance(&expressions, range.start));
    assert!(division_provenance(&expressions, range.end));
    assert!(division_provenance(
        &expressions,
        range.step.expect("range step should remain present")
    ));
}

// -------------------------
//  Effective view walker overlay coverage
// -------------------------

#[test]
fn view_walker_reads_effective_conditional_selector_before_body() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let (template_id, selector_site_id) = {
        let body = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_conditional_node(
            TemplateBranchSelector::Bool(bool_expression(
                &mut expressions,
                &mut const_values,
                false,
            )),
            body,
            None,
        );
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        );
        let selector_site_id = conditional_selector_site_id(&store, root);
        (template_id, selector_site_id)
    };

    let overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                selector_site_id,
                bool_expression(&mut expressions, &mut const_values, true),
            )],
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

    let payloads = collect_view(&view, &expressions).expect("walk should succeed");

    assert_eq!(payloads.len(), 2);
    assert!(
        {
            let view = expressions
                .view(payloads[0])
                .expect("selector payload should exist");
            matches!(
                view.operation(),
                ExpressionOperationView::Constant(payload)
                    if matches!(
                        const_values.payload(payload),
                        Some(ConstValuePayload::Bool(true))
                    )
            )
        },
        "the effective selector must be visited first"
    );
    assert!(
        is_i32_literal(&expressions, &const_values, payloads[1], 2),
        "the body must follow the effective selector"
    );
}

#[test]
fn view_walker_reads_loop_header_overlay() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let (template_id, start_site_id, step_site_id) = {
        let body = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children: vec![] },
            None,
        ));
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: expression(&mut expressions, &mut const_values, 1),
                end: expression(&mut expressions, &mut const_values, 10),
                end_kind: RangeEndKind::Exclusive,
                step: Some(expression(&mut expressions, &mut const_values, 2)),
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
            None,
        ));
        let template_id = push_template(&mut store, root, TemplateType::StringFunction);
        (template_id, start_site_id, step_site_id)
    };

    let overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (
                    start_site_id,
                    expression(&mut expressions, &mut const_values, 100),
                ),
                (
                    step_site_id,
                    expression(&mut expressions, &mut const_values, 50),
                ),
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

    let payloads = collect_view(&view, &expressions).expect("walk should succeed");

    assert_eq!(payloads.len(), 3);
    assert!(
        payloads
            .iter()
            .any(|e| is_i32_literal(&expressions, &const_values, *e, 100)),
        "overlay range start should be visited"
    );
    assert!(
        payloads
            .iter()
            .any(|e| is_i32_literal(&expressions, &const_values, *e, 10)),
        "structural range end should be visited"
    );
    assert!(
        payloads
            .iter()
            .any(|e| is_i32_literal(&expressions, &const_values, *e, 50)),
        "overlay range step should be visited"
    );
    assert!(
        !payloads
            .iter()
            .any(|e| is_i32_literal(&expressions, &const_values, *e, 1)),
        "structural range start should not be visited"
    );
    assert!(
        !payloads
            .iter()
            .any(|e| is_i32_literal(&expressions, &const_values, *e, 2)),
        "structural range step should not be visited"
    );
}

#[test]
fn view_walker_uses_parent_overlay_for_the_same_child_root() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let empty_context = TemplateViewContext::default();

    let (child_template_id, child_site_id) = {
        let child_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
        let child_site_id = dynamic_site_id(&store, child_root);
        let child_template_id = push_template(&mut store, child_root, TemplateType::StringFunction);
        (child_template_id, child_site_id)
    };

    let override_context = {
        let expression_overlay_id = store
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(
                    child_site_id,
                    expression(&mut expressions, &mut const_values, 42),
                )],
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
            None,
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
            None,
        ));
        let parent_root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![structural_child, overlaid_child],
            },
            None,
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

    let payloads = collect_view(&view, &expressions).expect("walk should succeed");

    // Structural child transitions ignore referenced expression overlays. Both
    // occurrences therefore enter the same exact child view and are visited
    // once, retaining only the structural expression.
    assert_eq!(payloads.len(), 1);
    assert!(payloads.iter().any(|expression| is_i32_literal(
        &expressions,
        &const_values,
        *expression,
        1
    )));
    assert!(!payloads.iter().any(|expression| is_i32_literal(
        &expressions,
        &const_values,
        *expression,
        42
    )));
}

#[test]
fn view_walker_reads_insert_contribution_effective_overlay() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let (parent_template_id, insert_site_id) = {
        let insert_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
        let insert_template_id =
            push_template(&mut store, insert_root, TemplateType::StringFunction);
        let insert_site_id = dynamic_site_id(&store, insert_root);

        let parent_root = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_insert_contribution_node(insert_template_id, None)
        };
        let parent_template_id =
            push_template(&mut store, parent_root, TemplateType::StringFunction);
        (parent_template_id, insert_site_id)
    };

    let overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                insert_site_id,
                expression(&mut expressions, &mut const_values, 42),
            )],
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

    let payloads = collect_view(&view, &expressions).expect("walk should succeed");

    assert_eq!(payloads.len(), 1);
    assert!(
        is_i32_literal(&expressions, &const_values, payloads[0], 42),
        "insert contribution should recurse through a child view and read the inherited overlay override, not the structural payload"
    );
}

#[test]
fn view_walker_reports_missing_insert_contribution_template() {
    let mut store = TemplateIrStore::new();
    let expressions = AstExpressionBuilder::new().unwrap();
    let parent_template_id = {
        let parent_root = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_insert_contribution_node(TemplateIrId::new(99), None)
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

    let error = collect_view(&view, &expressions)
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let empty_context = TemplateViewContext::default();

    let template_id = {
        let root = dynamic_node(&mut store, &mut expressions, &mut const_values, 99);
        push_template(&mut store, root, TemplateType::StringFunction)
    };

    let template = template_with_reference(finalized_tir_reference(template_id, empty_context));
    let template_expr = template_expression(&mut expressions, template);
    let coerced = coerced_expression(&mut expressions, template_expr);
    let expression = runtime_expression(
        &mut expressions,
        &mut const_values,
        &type_environment,
        coerced,
    );

    let payloads = collect_nested_expression_payloads(expression, &store, &expressions)
        .expect("walk should succeed");

    assert_eq!(payloads.len(), 1);
    assert!(
        is_i32_literal(&expressions, &const_values, payloads[0], 99),
        "TIR dynamic expression behind Coerced and Runtime should be visited"
    );
}

#[test]
fn nested_walker_shares_visited_set_between_tir_child_and_expression_template() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let empty_context = TemplateViewContext::default();

    let child_template_id = {
        let child_root = dynamic_node(&mut store, &mut expressions, &mut const_values, 42);
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
            None,
        ));

        let site_id = store.next_expression_site_id();
        let expr_node = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: template_expression(&mut expressions, child_template_for_expr),
                origin: TemplateSegmentOrigin::Body,
                site_id,
            },
            None,
        ));

        let parent_root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![child_node, expr_node],
            },
            None,
        ));
        push_template(&mut store, parent_root, TemplateType::StringFunction)
    };

    let parent_template =
        template_with_reference(finalized_tir_reference(parent_template_id, empty_context));
    let expression = template_expression(&mut expressions, parent_template);

    let payloads = collect_nested_expression_payloads(expression, &store, &expressions)
        .expect("walk should succeed");

    let int_42_count = payloads
        .iter()
        .filter(|e| is_i32_literal(&expressions, &const_values, **e, 42))
        .count();
    assert_eq!(
        int_42_count, 1,
        "shared visited set should visit child B only once across TIR child and expression template paths"
    );
}
