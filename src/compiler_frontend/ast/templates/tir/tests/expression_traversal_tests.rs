//! TIR expression-payload walker tests.
//!
//! WHAT: protects structural and effective-view expression traversal.
//! WHY: finalization owns one module-local TIR store, so every reachable
//! payload must be discovered through that same store and its overlays.

use super::super::expression_overlays::{
    ReachableExpressionRoot, ReachableExpressionSiteCache,
    collect_effective_tir_expression_overlay_payloads, collect_reachable_expression_sites,
    collect_tir_expression_overlay_payloads, replace_expression_overlay_entries,
};
use super::super::expression_sites::{
    walk_expression_payloads_with_nested_tir_views, walk_tir_view_expression_payloads,
};
use super::super::ids::{ExpressionSiteId, TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId};
use super::super::node::{
    TemplateIr, TemplateIrNode, TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use super::super::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolutionOverlayId, TirWrapperContext,
    TirWrapperContextOverlay, TirWrapperContextOverlayId,
};
use super::super::refs::{
    TemplateTirChildReference, TemplateTirReference, TemplateWrapperReference,
};
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
use crate::compiler_frontend::ast::expressions::eval_expression::result_type::resolved_operation;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
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
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
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
) -> Result<Vec<ExprId>, InfrastructureFailure> {
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
    ) -> Result<(), InfrastructureFailure> {
        self.count += 1;
        // WHY: the walker tests need real regular-division provenance, which only the operator
        // path creates, so self-division stands in for an authored divide.
        let operand = expressions.own(*expression);
        let operand_type = expressions
            .view_owned(operand)
            .expect("test expression should exist")
            .value_type_id()
            .expect("test expression should have one value type");
        let Ok(divide) = resolved_operation(
            Operator::Divide,
            None,
            operand_type,
            operand_type,
            Some(operand_type),
            &TypeEnvironment::new(),
        ) else {
            panic!("self-division of a test expression should resolve");
        };
        *expression = expressions
            .resolved_operation(operand, Some(operand), divide, ValueMode::ImmutableOwned)
            .expect("test division should construct")
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
            .message
            .contains("out-of-range expression-site overlay key"),
        "expected a store-owned range error, got: {}",
        error.message
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
    assert!(error.message.contains("missing child template"));
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
) -> Result<CountingMutator, InfrastructureFailure> {
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
) -> Result<Vec<ExprId>, InfrastructureFailure> {
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
fn reports_missing_runtime_slot_site_as_infrastructure_failure() {
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

    assert!(error.message.contains("missing runtime slot site"));
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
    assert!(error.message.contains("duplicate expression site"));
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
            domain: NumericScalar::Fixed(FixedScalar::I32),
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
//  Expression reachability caching coverage
// -------------------------

#[test]
fn reachability_inherits_recorded_child_summary_for_enclosing_root() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let child_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let child_site = dynamic_site_id(&store, child_dynamic);
    let child_template = push_template(&mut store, child_dynamic, TemplateType::String);

    let parent_occurrence = store.next_child_template_occurrence_id();
    let parent_child = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                child_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id: parent_occurrence,
        },
        None,
    ));
    let parent_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let parent_site = dynamic_site_id(&store, parent_dynamic);
    let parent_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![parent_child, parent_dynamic],
        },
        None,
    ));
    let parent_template = push_template(&mut store, parent_root, TemplateType::String);

    // The child root is recorded once by its own walk.
    let cache = ReachableExpressionSiteCache::default();
    let child_reach = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(child_template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("child reachability should collect");
    assert!(child_reach.contains(&child_site));
    assert_eq!(cache.cached_lookup_hits(), 0);

    // The enclosing root inherits that summary at the child edge instead of
    // walking the copied child tree again.
    let parent_reach = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(parent_template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("parent reachability should collect");
    assert!(parent_reach.contains(&parent_site));
    assert!(parent_reach.contains(&child_site));
    assert_eq!(
        cache.cached_lookup_hits(),
        1,
        "the enclosing walk must inherit the recorded child summary"
    );

    // The cached result must remain exactly the uncached result.
    let uncached_cache = ReachableExpressionSiteCache::default();
    let uncached = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(parent_template),
        TemplateViewContext::default(),
        &uncached_cache,
    )
    .expect("uncached parent reachability should collect");
    for site in [child_site, parent_site] {
        assert_eq!(
            uncached.contains(&site),
            parent_reach.contains(&site),
            "memoization must not change reachability"
        );
    }
    assert_eq!(uncached_cache.cached_lookup_hits(), 0);
}

#[test]
fn reachability_cache_is_keyed_by_reachability_context_dimensions() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let leaf_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let leaf_site = dynamic_site_id(&store, leaf_dynamic);
    let leaf_template = push_template(&mut store, leaf_dynamic, TemplateType::String);

    let leaf_occurrence = store.next_child_template_occurrence_id();
    let child_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                leaf_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id: leaf_occurrence,
        },
        None,
    ));
    let child_template = push_template(&mut store, child_root, TemplateType::String);

    let wrapper_one_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let wrapper_one_site = dynamic_site_id(&store, wrapper_one_dynamic);
    let wrapper_one_template = push_template(&mut store, wrapper_one_dynamic, TemplateType::String);
    let wrapper_one_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        wrapper_one_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);

    let wrapper_two_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 3);
    let wrapper_two_site = dynamic_site_id(&store, wrapper_two_dynamic);
    let wrapper_two_template = push_template(&mut store, wrapper_two_dynamic, TemplateType::String);
    let wrapper_two_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        wrapper_two_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);

    let wrapper_overlay_one = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                leaf_occurrence,
                TirWrapperContext {
                    inherited_wrapper_set: Some(wrapper_one_set),
                    ..Default::default()
                },
            )],
        })
        .expect("first wrapper context overlay should allocate");
    let wrapper_overlay_two = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                leaf_occurrence,
                TirWrapperContext {
                    inherited_wrapper_set: Some(wrapper_two_set),
                    ..Default::default()
                },
            )],
        })
        .expect("second wrapper context overlay should allocate");

    let parent_one_occurrence = store.next_child_template_occurrence_id();
    let parent_one_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                child_template,
                TemplateTirPhase::Composed,
                TemplateViewContext {
                    expression_overlay: None,
                    slot_resolution: None,
                    wrapper_context: Some(wrapper_overlay_one),
                },
            ),
            occurrence_id: parent_one_occurrence,
        },
        None,
    ));
    let parent_one = push_template(&mut store, parent_one_root, TemplateType::String);

    let parent_two_occurrence = store.next_child_template_occurrence_id();
    let parent_two_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                child_template,
                TemplateTirPhase::Composed,
                TemplateViewContext {
                    expression_overlay: None,
                    slot_resolution: None,
                    wrapper_context: Some(wrapper_overlay_two),
                },
            ),
            occurrence_id: parent_two_occurrence,
        },
        None,
    ));
    let parent_two = push_template(&mut store, parent_two_root, TemplateType::String);

    let cache = ReachableExpressionSiteCache::default();
    let child_context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(wrapper_overlay_one),
    };
    let child_reach = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(child_template),
        child_context,
        &cache,
    )
    .expect("child reachability should collect");
    assert!(child_reach.contains(&leaf_site));
    assert!(child_reach.contains(&wrapper_one_site));
    assert!(!child_reach.contains(&wrapper_two_site));

    let parent_one_reach = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(parent_one),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("first enclosing reachability should collect");
    assert!(parent_one_reach.contains(&leaf_site));
    assert!(parent_one_reach.contains(&wrapper_one_site));
    assert!(!parent_one_reach.contains(&wrapper_two_site));
    assert_eq!(cache.cached_lookup_hits(), 1);

    // A different wrapper-context dimension is a different reachability key,
    // so this walk must not inherit the first summary.
    let parent_two_reach = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(parent_two),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("second enclosing reachability should collect");
    assert!(parent_two_reach.contains(&leaf_site));
    assert!(parent_two_reach.contains(&wrapper_two_site));
    assert!(!parent_two_reach.contains(&wrapper_one_site));
    assert_eq!(
        cache.cached_lookup_hits(),
        1,
        "a different reachability context must fall back to a structural walk"
    );
}

#[test]
fn reachability_omits_sites_owned_by_templates_published_in_the_copy_pass() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    // A shared wrapper template predates the copy, so its site identity must
    // survive even though the copied template now reaches it.
    let wrapper_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let wrapper_site = dynamic_site_id(&store, wrapper_dynamic);
    let wrapper_template = push_template(&mut store, wrapper_dynamic, TemplateType::String);
    let wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        wrapper_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);

    // The copied template's own site was minted by the copy pass and therefore
    // belongs to the fresh identity domain rather than the retained one.
    let copied_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let copied_site = dynamic_site_id(&store, copied_dynamic);
    let copied_template = push_template(&mut store, copied_dynamic, TemplateType::String);
    store
        .set_conditional_child_wrapper_set(copied_template, wrapper_set)
        .expect("test wrapper set should attach");

    let cache = ReachableExpressionSiteCache::default();
    cache.register_copied_template(copied_template);
    let reach = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(copied_template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("copied-root reachability should collect");

    assert!(
        reach.contains(&wrapper_site),
        "a shared wrapper template's retained site must stay reachable"
    );
    assert!(
        !reach.contains(&copied_site),
        "a site minted by this copy pass is not a retained identity"
    );
}

#[test]
fn reachability_nested_copy_chain_inherits_retained_summaries() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    // One shared wrapper predates the copy and remains reachable from every
    // level, so every summary must carry its site exactly once.
    let wrapper_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let wrapper_site = dynamic_site_id(&store, wrapper_dynamic);
    let wrapper_template = push_template(&mut store, wrapper_dynamic, TemplateType::String);
    let wrapper_set = store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
        wrapper_template,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )]);

    let inner_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let inner_site = dynamic_site_id(&store, inner_dynamic);
    let inner_template = push_template(&mut store, inner_dynamic, TemplateType::String);
    store
        .set_conditional_child_wrapper_set(inner_template, wrapper_set)
        .expect("inner wrapper set should attach");

    let outer_occurrence = store.next_child_template_occurrence_id();
    let outer_child = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                inner_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id: outer_occurrence,
        },
        None,
    ));
    let outer_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 3);
    let outer_site = dynamic_site_id(&store, outer_dynamic);
    let outer_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![outer_child, outer_dynamic],
        },
        None,
    ));
    let outer_template = push_template(&mut store, outer_root, TemplateType::String);

    let cache = ReachableExpressionSiteCache::default();
    cache.register_copied_template(inner_template);
    cache.register_copied_template(outer_template);

    let inner_reach = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(inner_template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("inner reachability should collect");
    assert_eq!(
        inner_reach.len(),
        1,
        "only the retained shared-wrapper site survives the inner copy"
    );
    assert!(inner_reach.contains(&wrapper_site));

    let outer_reach = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(outer_template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("outer reachability should collect");
    assert_eq!(
        outer_reach.len(),
        1,
        "the enclosing copy inherits the retained summary instead of its fresh sites"
    );
    assert!(outer_reach.contains(&wrapper_site));
    assert!(!outer_reach.contains(&inner_site));
    assert!(!outer_reach.contains(&outer_site));
    assert_eq!(
        cache.cached_lookup_hits(),
        1,
        "the outer walk must not descend into the already-summarized inner root"
    );
}

#[test]
fn reachability_nested_copy_chain_shares_summaries_instead_of_flattening_sets() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    const DEPTH: usize = 8;

    // Every copied level retains one distinct wrapper template, so walking a
    // level's union would cover sites `0..=level`. Flattening those unions into
    // each cache entry would record sets of size `1..=DEPTH`.
    let mut wrapper_sites = Vec::with_capacity(DEPTH);
    let mut wrapper_sets = Vec::with_capacity(DEPTH);
    for level in 0..DEPTH {
        let wrapper_dynamic = dynamic_node(
            &mut store,
            &mut expressions,
            &mut const_values,
            i64::try_from(level).expect("test depth fits i64") + 1,
        );
        wrapper_sites.push(dynamic_site_id(&store, wrapper_dynamic));
        let wrapper_template = push_template(&mut store, wrapper_dynamic, TemplateType::String);
        wrapper_sets.push(
            store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
                wrapper_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            )]),
        );
    }

    // Build innermost first so each enclosing template references the previous
    // copied level, and register every level as published by the copy pass.
    let mut level_templates = Vec::with_capacity(DEPTH);
    for level in 0..DEPTH {
        let root = if level == 0 {
            dynamic_node(&mut store, &mut expressions, &mut const_values, 100)
        } else {
            let occurrence_id = store.next_child_template_occurrence_id();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference: TemplateTirChildReference::new(
                        level_templates[level - 1],
                        TemplateTirPhase::Composed,
                        TemplateViewContext::default(),
                    ),
                    occurrence_id,
                },
                None,
            ))
        };
        let template = push_template(&mut store, root, TemplateType::String);
        store
            .set_conditional_child_wrapper_set(template, wrapper_sets[level])
            .expect("test wrapper set should attach");
        level_templates.push(template);
    }

    let cache = ReachableExpressionSiteCache::default();
    for template in &level_templates {
        cache.register_copied_template(*template);
    }

    for (level, template) in level_templates.iter().enumerate() {
        let reach = collect_reachable_expression_sites(
            &store,
            ReachableExpressionRoot::Template(*template),
            TemplateViewContext::default(),
            &cache,
        )
        .expect("nested level reachability should collect");

        assert_eq!(
            reach.len(),
            level + 1,
            "level {level} must expose exactly its retained wrapper sites"
        );
        for (wrapper_level, wrapper_site) in wrapper_sites.iter().enumerate() {
            assert_eq!(
                reach.contains(wrapper_site),
                wrapper_level <= level,
                "level {level} reachability of wrapper site {wrapper_level}"
            );
        }
    }

    assert_eq!(
        cache.directly_recorded_site_count(),
        DEPTH,
        "each level must store only its own retained site, not a flattened union"
    );
    assert_eq!(
        cache.inherited_summary_count(),
        DEPTH - 1,
        "each enclosing level must share the previous summary by reference"
    );
    assert_eq!(
        cache.coverage_range_count(),
        DEPTH,
        "each level must index its whole nested closure as one merged range"
    );
    assert_eq!(
        cache.cached_lookup_hits(),
        DEPTH - 1,
        "each enclosing walk must inherit the child summary instead of re-walking"
    );

    // The outermost shared result must equal a cold walk of the same root.
    let uncached_cache = ReachableExpressionSiteCache::default();
    for template in &level_templates {
        uncached_cache.register_copied_template(*template);
    }
    let uncached = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(level_templates[DEPTH - 1]),
        TemplateViewContext::default(),
        &uncached_cache,
    )
    .expect("cold nested reachability should collect");
    assert_eq!(uncached.len(), DEPTH);
    for wrapper_site in &wrapper_sites {
        assert!(
            uncached.contains(wrapper_site),
            "cold nesting must reach every retained wrapper site"
        );
    }

    // The production query returns only reached candidates, so an unreachable
    // candidate must not leak into the overlay rebase's retained set.
    let queried = uncached.retained_candidates([
        wrapper_sites[0],
        wrapper_sites[DEPTH - 1],
        ExpressionSiteId::new(9_999),
    ]);
    assert_eq!(queried.len(), 2);
    assert!(queried.contains(&wrapper_sites[0]));
    assert!(queried.contains(&wrapper_sites[DEPTH - 1]));
    assert!(!queried.contains(&ExpressionSiteId::new(9_999)));
}

#[test]
fn retained_candidates_answers_absent_and_deep_sites_from_the_summary_index() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    const DEPTH: usize = 16;

    // Every copied level retains one distinct wrapper site and inherits the
    // previous level's summary, so the outermost query covers `0..=DEPTH` sites
    // while each summary records exactly one site of its own.
    let mut wrapper_sites = Vec::with_capacity(DEPTH);
    let mut wrapper_sets = Vec::with_capacity(DEPTH);
    for level in 0..DEPTH {
        let wrapper_dynamic = dynamic_node(
            &mut store,
            &mut expressions,
            &mut const_values,
            i64::try_from(level).expect("test depth fits i64") + 1,
        );
        wrapper_sites.push(dynamic_site_id(&store, wrapper_dynamic));
        let wrapper_template = push_template(&mut store, wrapper_dynamic, TemplateType::String);
        wrapper_sets.push(
            store.push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
                wrapper_template,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            )]),
        );
    }

    let mut level_templates = Vec::with_capacity(DEPTH);
    for level in 0..DEPTH {
        let root = if level == 0 {
            dynamic_node(&mut store, &mut expressions, &mut const_values, 100)
        } else {
            let occurrence_id = store.next_child_template_occurrence_id();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference: TemplateTirChildReference::new(
                        level_templates[level - 1],
                        TemplateTirPhase::Composed,
                        TemplateViewContext::default(),
                    ),
                    occurrence_id,
                },
                None,
            ))
        };
        let template = push_template(&mut store, root, TemplateType::String);
        store
            .set_conditional_child_wrapper_set(template, wrapper_sets[level])
            .expect("test wrapper set should attach");
        level_templates.push(template);
    }

    let cache = ReachableExpressionSiteCache::default();
    for template in &level_templates {
        cache.register_copied_template(*template);
    }

    // Recording every level before the outer query makes each summary inherit
    // the previous summary rather than re-walking it.
    for template in level_templates.iter().take(DEPTH - 1) {
        collect_reachable_expression_sites(
            &store,
            ReachableExpressionRoot::Template(*template),
            TemplateViewContext::default(),
            &cache,
        )
        .expect("inner level reachability should collect");
    }
    let outer = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(level_templates[DEPTH - 1]),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("outer reachability should collect");

    assert_eq!(
        cache.coverage_range_count(),
        DEPTH,
        "a nested chain must keep one merged coverage range per summary"
    );
    assert_eq!(
        cache.owned_site_count(),
        DEPTH,
        "each level must index only the site it recorded itself"
    );

    let absent = ExpressionSiteId::new(9_999);
    let candidate_sites = [
        wrapper_sites[0],
        wrapper_sites[DEPTH / 2],
        wrapper_sites[DEPTH - 1],
        absent,
    ];
    let queried = outer.retained_candidates(candidate_sites);
    assert_eq!(queried.len(), 3);
    assert!(
        queried.contains(&wrapper_sites[0]),
        "deepest site must resolve"
    );
    assert!(queried.contains(&wrapper_sites[DEPTH / 2]));
    assert!(queried.contains(&wrapper_sites[DEPTH - 1]));
    assert!(!queried.contains(&absent), "absent site must not leak");
    for site in candidate_sites {
        assert_eq!(
            queried.contains(&site),
            outer.contains(&site),
            "the summary index must agree with the descendant walk for {site}"
        );
    }
}

#[test]
fn retained_candidates_respects_independent_contextual_scopes() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    // Both parents inherit the same shared child summary. The child summary is
    // therefore multi-parent, and the parents' own sites must stay out of each
    // other's retained set.
    let shared_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let shared_site = dynamic_site_id(&store, shared_dynamic);
    let shared_template = push_template(&mut store, shared_dynamic, TemplateType::String);

    let mut parent_templates = Vec::new();
    let mut parent_sites = Vec::new();
    for value in [2, 3] {
        let child_occurrence = store.next_child_template_occurrence_id();
        let child = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: TemplateTirChildReference::new(
                    shared_template,
                    TemplateTirPhase::Composed,
                    TemplateViewContext::default(),
                ),
                occurrence_id: child_occurrence,
            },
            None,
        ));
        let own_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, value);
        parent_sites.push(dynamic_site_id(&store, own_dynamic));
        let root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![child, own_dynamic],
            },
            None,
        ));
        parent_templates.push(push_template(&mut store, root, TemplateType::String));
    }

    let cache = ReachableExpressionSiteCache::default();
    let shared = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(shared_template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("shared reachability should collect");
    assert!(shared.contains(&shared_site));

    let first = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(parent_templates[0]),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("first parent reachability should collect");
    let second = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(parent_templates[1]),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("second parent reachability should collect");

    assert_eq!(cache.cached_lookup_hits(), 2);
    assert_eq!(
        cache.coverage_range_count(),
        4,
        "the shared child plus first parent is one range; the second parent leaves a hole"
    );

    let first_retained = first.retained_candidates([shared_site, parent_sites[0], parent_sites[1]]);
    assert_eq!(first_retained.len(), 2);
    assert!(first_retained.contains(&shared_site));
    assert!(first_retained.contains(&parent_sites[0]));
    assert!(!first_retained.contains(&parent_sites[1]));

    let second_retained =
        second.retained_candidates([shared_site, parent_sites[0], parent_sites[1]]);
    assert_eq!(second_retained.len(), 2);
    assert!(second_retained.contains(&shared_site));
    assert!(!second_retained.contains(&parent_sites[0]));
    assert!(second_retained.contains(&parent_sites[1]));
}

#[test]
fn retained_candidates_handles_sites_recorded_by_multiple_summaries() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    // The shared template is never collected on its own, so both parents walk
    // it directly and each records the same site in its own summary. The site
    // therefore has two owner summaries.
    let shared_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let shared_site = dynamic_site_id(&store, shared_dynamic);
    let shared_template = push_template(&mut store, shared_dynamic, TemplateType::String);

    let mut parent_templates = Vec::new();
    for _ in 0..2 {
        let occurrence_id = store.next_child_template_occurrence_id();
        let root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: TemplateTirChildReference::new(
                    shared_template,
                    TemplateTirPhase::Composed,
                    TemplateViewContext::default(),
                ),
                occurrence_id,
            },
            None,
        ));
        parent_templates.push(push_template(&mut store, root, TemplateType::String));
    }

    // This root inherits only the first parent, so the shared site must resolve
    // through the reachable owner even though the site has two owners.
    let enclosing_occurrence = store.next_child_template_occurrence_id();
    let enclosing_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                parent_templates[0],
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id: enclosing_occurrence,
        },
        None,
    ));
    let enclosing_template = push_template(&mut store, enclosing_root, TemplateType::String);

    let unrelated_dynamic = dynamic_node(&mut store, &mut expressions, &mut const_values, 2);
    let unrelated_site = dynamic_site_id(&store, unrelated_dynamic);
    let unrelated_template = push_template(&mut store, unrelated_dynamic, TemplateType::String);

    let cache = ReachableExpressionSiteCache::default();
    for template in [parent_templates[0], parent_templates[1], unrelated_template] {
        let reach = collect_reachable_expression_sites(
            &store,
            ReachableExpressionRoot::Template(template),
            TemplateViewContext::default(),
            &cache,
        )
        .expect("template reachability should collect");
        if template == unrelated_template {
            assert!(reach.contains(&unrelated_site));
        } else {
            assert!(reach.contains(&shared_site));
        }
    }

    let enclosing = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(enclosing_template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("enclosing reachability should collect");

    assert_eq!(
        cache.owned_site_count(),
        2,
        "the shared site must have one owner entry even with several owning summaries"
    );
    assert_eq!(
        cache.coverage_range_count(),
        5,
        "both parents and the unrelated root are single ranges; the enclosing root leaves a hole"
    );

    let retained =
        enclosing.retained_candidates([shared_site, unrelated_site, ExpressionSiteId::new(9_999)]);
    assert_eq!(retained.len(), 1);
    assert!(
        retained.contains(&shared_site),
        "the reachable owner must keep the multiply-recorded site"
    );
    assert!(
        !retained.contains(&unrelated_site),
        "an owner outside the coverage must not leak into the retained set"
    );
}

#[test]
fn retained_candidates_remains_exact_across_repeated_cyclic_summaries() {
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let site_node = dynamic_node(&mut store, &mut expressions, &mut const_values, 1);
    let site = dynamic_site_id(&store, site_node);

    // The template references its own future id, so a second collection must
    // inherit the first summary instead of recursing through the cycle.
    let cycle_template_id = TemplateIrId::new(store.template_count());
    let occurrence_id = store.next_child_template_occurrence_id();
    let cycle_reference = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                cycle_template_id,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id,
        },
        None,
    ));
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![site_node, cycle_reference],
        },
        None,
    ));
    let template = push_template(&mut store, root, TemplateType::String);
    assert_eq!(template, cycle_template_id);

    let cache = ReachableExpressionSiteCache::default();
    let first = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("cyclic reachability should collect");
    assert_eq!(first.len(), 1);

    let absent = ExpressionSiteId::new(9_999);
    let first_retained = first.retained_candidates([site, absent]);
    assert_eq!(first_retained.len(), 1);
    assert!(first_retained.contains(&site));
    assert!(!first_retained.contains(&absent));

    let second = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("repeated cyclic reachability should collect");
    assert_eq!(
        cache.cached_lookup_hits(),
        1,
        "the second walk must inherit the cyclic summary instead of descending"
    );
    assert_eq!(
        cache.coverage_range_count(),
        1,
        "the repeated summary merges into one contiguous coverage range"
    );

    let second_retained = second.retained_candidates([site, absent]);
    assert_eq!(second_retained.len(), 1);
    assert!(second_retained.contains(&site));
    assert!(!second_retained.contains(&absent));
}

#[test]
fn reachability_cyclic_child_reference_terminates_with_shared_summaries() {
    let mut store = TemplateIrStore::new();

    // The template references its own future id, so the reachability walk must
    // close the cycle through its visited-view guard instead of recursing.
    let cycle_template_id = TemplateIrId::new(store.template_count());
    let occurrence_id = store.next_child_template_occurrence_id();
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: TemplateTirChildReference::new(
                cycle_template_id,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            occurrence_id,
        },
        None,
    ));
    let template = push_template(&mut store, root, TemplateType::String);
    assert_eq!(template, cycle_template_id);

    let cache = ReachableExpressionSiteCache::default();
    let reach = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("cyclic reachability should collect without recursing");
    assert_eq!(reach.len(), 0);

    // The shared summary recorded for the cycle must not make a later walk or
    // query recurse either.
    let again = collect_reachable_expression_sites(
        &store,
        ReachableExpressionRoot::Template(template),
        TemplateViewContext::default(),
        &cache,
    )
    .expect("cyclic reachability should remain finite with a cached summary");
    assert_eq!(again.len(), 0);
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
                domain: NumericScalar::Fixed(FixedScalar::I32),
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
        error.message.contains("root_template"),
        "error must come from the required insert view root resolution: {}",
        error.message,
    );
    assert!(
        error.message.contains("missing"),
        "error must report the missing insert template: {}",
        error.message
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
