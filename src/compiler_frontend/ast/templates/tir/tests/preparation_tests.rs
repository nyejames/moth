//! Exact-view preparation result tests.
//!
//! WHAT: protects the compact preparation result and its exact-view identity.
//! WHY: finalization must not reconstruct a second disposition beside the
//!      preparation owner.

use super::super::ids::{TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId, TemplateWrapperSetId};
use super::super::preparation::{
    RuntimeTemplateReason, TemplateHelperKind, TemplatePreparation, TemplatePreparationMode,
    TemplatePreparationOutcome, prepare_tir_view,
};
use super::super::slot_plan::{
    TemplateSlotContributionSourcePlan, TemplateSlotPlan, TemplateSlotSitePlan,
    push_runtime_slot_contribution_source,
};
use super::super::store::{MalformedTirStore, TemplateIrStore, TemplateWrapperSet};
use super::super::summary::TemplateIrSummary;
use super::super::{TemplateTirPhase, TemplateViewContext, TirView};
use super::builder::TemplateIrBuilder;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotKey, Style, Template, TemplateConstValueKind, TemplateSegmentOrigin,
    TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopControlKind, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotContributionSourceId;
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotSiteId;
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode,
};
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateTirReference, TemplateWrapperReference,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn runtime_expression(string_table: &mut StringTable) -> Expression {
    let scope = InternedPath::from_single_str("main.moth", string_table);
    let name = string_table.intern("runtime_text");
    Expression::new(
        ExpressionKind::FunctionCall {
            name: scope.append(name),
            args: Vec::new(),
            result_type_ids: vec![builtin_type_ids::STRING],
        },
        empty_location(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    )
}

fn prepare_root(
    kind: TemplateType,
    build_root: impl FnOnce(&mut TemplateIrBuilder<'_>, &mut StringTable) -> TemplateIrNodeId,
    mode: TemplatePreparationMode,
) -> Result<(TemplatePreparation, super::super::view::TirViewIdentity), TemplateError> {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = build_root(&mut builder, &mut string_table);
        builder.finish_template(
            root,
            Style::default(),
            kind,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )?;
    let identity = view.identity();
    let prepared = prepare_tir_view(&view, mode)?;
    Ok((prepared, identity))
}

#[test]
fn preparation_modes_return_one_identity_bound_foldable_result() {
    let (value, identity) = prepare_root(
        TemplateType::String,
        |builder, table| {
            let text = table.intern("value");
            let text_node =
                builder.push_text_node(text, 5, TemplateSegmentOrigin::Body, empty_location());
            builder.push_sequence_node(vec![text_node], empty_location())
        },
        TemplatePreparationMode::Value,
    )
    .expect("Value preparation should succeed");
    let (const_required, const_identity) = prepare_root(
        TemplateType::String,
        |builder, table| {
            let text = table.intern("value");
            let text_node =
                builder.push_text_node(text, 5, TemplateSegmentOrigin::Body, empty_location());
            builder.push_sequence_node(vec![text_node], empty_location())
        },
        TemplatePreparationMode::ConstRequired,
    )
    .expect("ConstRequired preparation should succeed");

    match (value.outcome, const_required.outcome) {
        (TemplatePreparationOutcome::Foldable, TemplatePreparationOutcome::Foldable) => {
            assert_eq!(value.identity, identity);
            assert_eq!(const_required.identity, const_identity);
            assert_eq!(value.identity, const_required.identity);
            assert!(value.facts.is_const_evaluable_shape);
            assert_eq!(
                value.facts.final_value_kind,
                TemplateConstValueKind::RenderableString
            );
        }
        (value, const_required) => panic!(
            "simple text must be exclusively foldable in both modes: {value:?} / {const_required:?}"
        ),
    }
}

#[test]
fn preparation_preserves_structural_wrapper_shape_for_value_callers() {
    let (prepared, _) = prepare_root(
        TemplateType::String,
        |builder, _| {
            let slot = builder.push_slot_node(SlotKey::Default, empty_location());
            builder.push_sequence_node(vec![slot], empty_location())
        },
        TemplatePreparationMode::Value,
    )
    .expect("unfilled wrapper preparation should succeed");

    assert!(matches!(
        prepared,
        TemplatePreparation {
            outcome: TemplatePreparationOutcome::Foldable,
            facts,
            ..
        } if facts.final_value_kind == TemplateConstValueKind::WrapperTemplate
    ));
}

#[test]
fn preparation_returns_runtime_with_exact_identity_for_runtime_expression() {
    let (prepared, identity) = prepare_root(
        TemplateType::String,
        |builder, table| {
            let expression = builder.push_dynamic_expression_node(
                runtime_expression(table),
                TemplateSegmentOrigin::Body,
                None,
                empty_location(),
            );
            builder.push_sequence_node(vec![expression], empty_location())
        },
        TemplatePreparationMode::Value,
    )
    .expect("runtime preparation should succeed");

    match prepared {
        TemplatePreparation {
            identity: prepared_identity,
            outcome: TemplatePreparationOutcome::Runtime(reason),
            ..
        } => {
            assert_eq!(prepared_identity, identity);
            assert_eq!(reason, RuntimeTemplateReason::RuntimeExpression);
        }
        other => panic!("runtime expression must be exclusively runtime: {other:?}"),
    }
}

#[test]
fn preparation_keeps_reactive_content_on_runtime_handoff() {
    let (prepared, identity) = prepare_root(
        TemplateType::String,
        |builder, table| {
            let source = ReactiveSource {
                path: InternedPath::from_single_str("main.moth/#reactive", table),
                kind: ReactiveSourceKind::Declaration,
            };
            let text = table.intern("reactive text");
            let text_node = builder.push_text_node_with_subscription(
                text,
                "reactive text".len(),
                TemplateSegmentOrigin::Body,
                Some(ReactiveSubscription {
                    source,
                    type_id: builtin_type_ids::STRING,
                    location: empty_location(),
                }),
                empty_location(),
            );
            builder.push_sequence_node(vec![text_node], empty_location())
        },
        TemplatePreparationMode::Value,
    )
    .expect("reactive preparation should succeed");

    match prepared {
        TemplatePreparation {
            identity: prepared_identity,
            outcome: TemplatePreparationOutcome::Runtime(reason),
            facts,
            ..
        } => {
            assert_eq!(prepared_identity, identity);
            assert_eq!(reason, RuntimeTemplateReason::ReactiveContent);
            assert!(facts.has_reactive_dependence);
            assert!(!facts.is_const_evaluable_shape);
        }
        other => panic!("reactive content must remain runtime: {other:?}"),
    }
}

#[test]
fn preparation_uses_structural_const_facts_for_static_string_function() {
    let (prepared, _) = prepare_root(
        TemplateType::StringFunction,
        |builder, table| {
            let text = table.intern("static function body");
            let text_node = builder.push_text_node(
                text,
                "static function body".len(),
                TemplateSegmentOrigin::Body,
                empty_location(),
            );
            builder.push_sequence_node(vec![text_node], empty_location())
        },
        TemplatePreparationMode::Value,
    )
    .expect("StringFunction preparation should succeed");

    assert!(matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Foldable
    ));
}

#[test]
fn preparation_returns_explicit_helper_results() {
    let (slot_insert, _) = prepare_root(
        TemplateType::SlotInsert(SlotKey::Default),
        |builder, table| {
            let text = table.intern("slot");
            let text_node =
                builder.push_text_node(text, 4, TemplateSegmentOrigin::Body, empty_location());
            builder.push_sequence_node(vec![text_node], empty_location())
        },
        TemplatePreparationMode::Value,
    )
    .expect("slot insert preparation should succeed");
    assert!(matches!(
        slot_insert.outcome,
        TemplatePreparationOutcome::Helper(TemplateHelperKind::SlotInsert)
    ));

    let (loop_control, _) = prepare_root(
        TemplateType::String,
        |builder, _| {
            builder.push_loop_control_node(TemplateLoopControlKind::Break, empty_location())
        },
        TemplatePreparationMode::Value,
    )
    .expect("loop-control preparation should succeed");
    assert!(matches!(
        loop_control.outcome,
        TemplatePreparationOutcome::Helper(TemplateHelperKind::LoopControl)
    ));
}

#[test]
fn preparation_mode_controls_const_required_branch_validation() {
    let build_branch = |builder: &mut TemplateIrBuilder<'_>, table: &mut StringTable| {
        let body_text = table.intern("body");
        let body =
            builder.push_text_node(body_text, 4, TemplateSegmentOrigin::Body, empty_location());
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(runtime_expression(table)),
            body,
            empty_location(),
            builder.store.next_expression_site_id(),
        );
        builder.push_branch_chain_node(vec![branch], None, empty_location())
    };

    let (value, _) = prepare_root(
        TemplateType::StringFunction,
        build_branch,
        TemplatePreparationMode::Value,
    )
    .expect("Value mode should preserve lazy branch runtime semantics");
    assert!(matches!(
        value.outcome,
        TemplatePreparationOutcome::Runtime(_)
    ));

    let const_required = prepare_root(
        TemplateType::StringFunction,
        build_branch,
        TemplatePreparationMode::ConstRequired,
    )
    .expect_err("ConstRequired mode should retain the branch diagnostic");
    let TemplateError::Diagnostic(diagnostic) = const_required else {
        panic!("ConstRequired branch rejection should remain a source diagnostic");
    };
    assert!(matches!(
        diagnostic.payload,
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidTemplateStructure {
            reason: crate::compiler_frontend::compiler_messages::InvalidTemplateStructureReason::TemplateIfConditionNotConst,
        }
    ));
}

#[test]
fn preparation_const_required_recurses_through_coerced_loop_condition() {
    let result = prepare_root(
        TemplateType::StringFunction,
        |builder, table| {
            let condition = Expression::bool(true, empty_location(), ValueMode::ImmutableOwned);
            let coerced_once = Expression::new(
                ExpressionKind::Coerced {
                    value: Box::new(condition),
                    to_type: builtin_type_ids::BOOL,
                },
                empty_location(),
                builtin_type_ids::BOOL,
                DataType::Bool,
                ValueMode::ImmutableOwned,
            );
            let coerced_twice = Expression::new(
                ExpressionKind::Coerced {
                    value: Box::new(coerced_once),
                    to_type: builtin_type_ids::BOOL,
                },
                empty_location(),
                builtin_type_ids::BOOL,
                DataType::Bool,
                ValueMode::ImmutableOwned,
            );
            let body_text = table.intern("body");
            let body =
                builder.push_text_node(body_text, 4, TemplateSegmentOrigin::Body, empty_location());
            builder.push_loop_node(
                TemplateLoopHeader::Conditional {
                    condition: Box::new(coerced_twice),
                },
                body,
                None,
                empty_location(),
            )
        },
        TemplatePreparationMode::ConstRequired,
    );

    let TemplateError::Diagnostic(diagnostic) =
        result.expect_err("ConstRequired mode must inspect the effective coerced condition")
    else {
        panic!("const-true loop rejection should remain a source diagnostic");
    };
    assert!(matches!(
        diagnostic.payload,
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidTemplateStructure {
            reason: crate::compiler_frontend::compiler_messages::InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue,
        }
    ));
}

#[test]
fn preparation_continues_after_runtime_dependence_to_malformed_authority() {
    let result = prepare_root(
        TemplateType::StringFunction,
        |builder, table| {
            let runtime = builder.push_dynamic_expression_node(
                runtime_expression(table),
                TemplateSegmentOrigin::Body,
                None,
                empty_location(),
            );
            builder.push_sequence_node(vec![runtime, TemplateIrNodeId::new(999)], empty_location())
        },
        TemplatePreparationMode::Value,
    );

    let error = result.expect_err("malformed authority must not be hidden by runtime dependence");
    let TemplateError::Infrastructure(error) = error else {
        panic!("missing TIR authority should remain an infrastructure error");
    };
    assert!(error.msg.contains("TIR preparation: node"));
}

#[test]
fn preparation_reenters_nested_template_payload_authority() {
    let mut store = TemplateIrStore::new();
    let context = TemplateViewContext::default();
    let nested_id = store.push_template(TemplateIr::new(
        TemplateIrNodeId::new(999),
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    let nested_template = Template {
        tir_reference: TemplateTirReference {
            root: nested_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        location: empty_location(),
    };
    let outer_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let dynamic = builder.push_dynamic_expression_node(
            Expression::template(nested_template, ValueMode::ImmutableOwned),
            TemplateSegmentOrigin::Body,
            None,
            empty_location(),
        );
        let root = builder.push_sequence_node(vec![dynamic], empty_location());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    let view = TirView::new(&store, outer_id, TemplateTirPhase::Composed, context)
        .expect("outer view should construct");

    let error = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect_err("nested template authority must be traversed");
    let TemplateError::Infrastructure(error) = error else {
        panic!("nested authority failure must remain infrastructure");
    };
    assert!(error.msg.contains("TIR preparation: node"));
}

#[test]
fn preparation_classifies_nested_value_cycle_as_runtime() {
    let mut store = TemplateIrStore::new();
    let context = TemplateViewContext::default();
    let nested_id = TemplateIrId::new(store.template_count());
    let nested_value = || Template {
        tir_reference: TemplateTirReference {
            root: nested_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        location: empty_location(),
    };
    {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let dynamic = builder.push_dynamic_expression_node(
            Expression::template(nested_value(), ValueMode::ImmutableOwned),
            TemplateSegmentOrigin::Body,
            None,
            empty_location(),
        );
        let root = builder.push_sequence_node(vec![dynamic], empty_location());
        assert_eq!(
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                empty_location(),
            ),
            nested_id
        );
    }
    let mut outer_builder = TemplateIrBuilder::new(&mut store);
    let outer_dynamic = outer_builder.push_dynamic_expression_node(
        Expression::template(nested_value(), ValueMode::ImmutableOwned),
        TemplateSegmentOrigin::Body,
        None,
        empty_location(),
    );
    let outer_root = outer_builder.push_sequence_node(vec![outer_dynamic], empty_location());
    let outer_id = outer_builder.finish_template(
        outer_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );
    let view = TirView::new(&store, outer_id, TemplateTirPhase::Composed, context)
        .expect("outer nested-cycle view should construct");

    let error = match prepare_tir_view(&view, TemplatePreparationMode::Value) {
        Err(error) => error,
        Ok(prepared) => panic!("nested value cycles must be CompilerError, got {prepared:?}"),
    };
    let TemplateError::Infrastructure(error) = error else {
        panic!("nested value cycles must stay on the infrastructure lane, got {error:?}");
    };
    assert_eq!(error.error_type, ErrorType::Compiler);
}

/// Exact-view child cycles are internal authority failures, not runtime values.
#[test]
fn preparation_rejects_exact_child_cycle_as_internal_error() {
    let mut store = TemplateIrStore::new();
    let template_id = TemplateIrId::new(store.template_count());
    let child = TemplateTirChildReference::new(
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    );
    let mut builder = TemplateIrBuilder::new(&mut store);
    let child_node = builder.push_child_template_node_with_reference(child, empty_location());
    let root = builder.push_sequence_node(vec![child_node], empty_location());
    let actual_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );
    assert_eq!(actual_id, template_id);
    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("cyclic view should construct");

    let error = match prepare_tir_view(&view, TemplatePreparationMode::Value) {
        Err(error) => error,
        Ok(prepared) => panic!("exact-view child cycles must be CompilerError, got {prepared:?}"),
    };
    let TemplateError::Infrastructure(error) = error else {
        panic!("exact-view child cycles must stay on the infrastructure lane, got {error:?}");
    };
    assert_eq!(error.error_type, ErrorType::Compiler);
}

#[test]
fn preparation_validates_runtime_slot_plan_authority() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("slot plan");
        let node = builder.push_text_node(text, 9, TemplateSegmentOrigin::Body, empty_location());
        builder.finish_template(
            node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    MalformedTirStore::new(&mut store)
        .set_runtime_slot_plan(template_id, Some(TemplateSlotPlanId::new(999)));
    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("view should construct before slot-plan preparation");

    let error = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect_err("missing runtime slot plan must remain authority failure");
    let TemplateError::Infrastructure(error) = error else {
        panic!("missing slot plan must remain infrastructure");
    };
    assert!(error.msg.contains("TIR preparation: slot plan"));
}

#[test]
fn preparation_publishes_runtime_plan_and_site_facts() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let render_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_text_node(
            string_table.intern("runtime site"),
            "runtime site".len(),
            TemplateSegmentOrigin::Body,
            empty_location(),
        )
    };
    let plan_id = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: Vec::new(),
        slot_sites: vec![site_plan(RuntimeSlotSiteId(0), render_root)],
    });
    let runtime_site = store.push_node(TemplateIrNode::new(
        crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind::RuntimeSlotSite {
            plan: plan_id,
            site: RuntimeSlotSiteId(0),
        },
        empty_location(),
    ));
    let root = store.push_node(TemplateIrNode::new(
        crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind::Sequence {
            children: vec![runtime_site],
        },
        empty_location(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    store
        .attach_runtime_slot_plan(template_id, plan_id)
        .expect("runtime slot plan should attach to the template");

    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("runtime-plan view should construct");
    let preparation = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("runtime-plan view should prepare");

    assert!(preparation.facts.has_runtime_slot_plan);
    assert!(preparation.facts.has_runtime_slot_sites);
    assert!(matches!(
        preparation.outcome,
        TemplatePreparationOutcome::Runtime(RuntimeTemplateReason::RuntimeSlotPlan)
            | TemplatePreparationOutcome::Runtime(RuntimeTemplateReason::RuntimeSlotSite)
    ));
}

#[test]
fn preparation_rejects_runtime_slot_site_from_a_different_plan() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let render_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_text_node(
            string_table.intern("site"),
            4,
            TemplateSegmentOrigin::Body,
            empty_location(),
        )
    };
    let owner_plan = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: Vec::new(),
        slot_sites: vec![site_plan(RuntimeSlotSiteId(0), render_root)],
    });
    let other_plan = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: Vec::new(),
        slot_sites: vec![site_plan(RuntimeSlotSiteId(0), render_root)],
    });
    let runtime_site = store.push_node(TemplateIrNode::new(
        crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind::RuntimeSlotSite {
            plan: other_plan,
            site: RuntimeSlotSiteId(0),
        },
        empty_location(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        runtime_site,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    store
        .attach_runtime_slot_plan(template_id, owner_plan)
        .expect("owner plan should attach");

    assert_preparation_authority_error(
        prepare_slot_plan_view(&store, template_id),
        "differs from the active plan",
    );
}

#[test]
fn preparation_rejects_out_of_range_runtime_slot_site() {
    let mut store = TemplateIrStore::new();
    let plan = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: Vec::new(),
        slot_sites: Vec::new(),
    });
    let runtime_site = store.push_node(TemplateIrNode::new(
        crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind::RuntimeSlotSite {
            plan,
            site: RuntimeSlotSiteId(0),
        },
        empty_location(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        runtime_site,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    store
        .attach_runtime_slot_plan(template_id, plan)
        .expect("runtime slot plan should attach");

    assert_preparation_authority_error(prepare_slot_plan_view(&store, template_id), "outside plan");
}

#[test]
fn preparation_rejects_mismatched_runtime_slot_site_identity() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let render_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_text_node(
            string_table.intern("site"),
            4,
            TemplateSegmentOrigin::Body,
            empty_location(),
        )
    };
    let plan = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: Vec::new(),
        slot_sites: vec![site_plan(RuntimeSlotSiteId(0), render_root)],
    });
    MalformedTirStore::new(&mut store)
        .replace_slot_sites(plan, vec![site_plan(RuntimeSlotSiteId(7), render_root)]);
    let runtime_site = store.push_node(TemplateIrNode::new(
        crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind::RuntimeSlotSite {
            plan,
            site: RuntimeSlotSiteId(7),
        },
        empty_location(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        runtime_site,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    store
        .attach_runtime_slot_plan(template_id, plan)
        .expect("runtime slot plan should attach");

    assert_preparation_authority_error(
        prepare_slot_plan_view(&store, template_id),
        "does not match its index",
    );
}

#[test]
fn preparation_propagates_reactive_facts_from_runtime_slot_contribution_roots() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let reactive_source = ReactiveSource {
        path: InternedPath::from_single_str("main.moth/#reactive", &mut string_table),
        kind: ReactiveSourceKind::Declaration,
    };
    let contribution_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_text_node_with_subscription(
            string_table.intern("reactive contribution"),
            "reactive contribution".len(),
            TemplateSegmentOrigin::Body,
            Some(ReactiveSubscription {
                source: reactive_source,
                type_id: builtin_type_ids::STRING,
                location: empty_location(),
            }),
            empty_location(),
        )
    };
    let plan_id = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![empty_source_plan(
            RuntimeSlotContributionSourceId(0),
            contribution_root,
        )],
        slot_sites: Vec::new(),
    });
    let wrapper_root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_text_node(
            string_table.intern("wrapper root"),
            "wrapper root".len(),
            TemplateSegmentOrigin::Body,
            empty_location(),
        )
    };
    let template_id = store.push_template(TemplateIr::new(
        wrapper_root,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    store
        .attach_runtime_slot_plan(template_id, plan_id)
        .expect("runtime slot plan should attach to the wrapper root");

    let preparation = prepare_slot_plan_view(&store, template_id)
        .expect("reactive contribution roots should be part of preparation facts");

    assert!(preparation.facts.has_runtime_slot_plan);
    assert!(preparation.facts.has_reactive_dependence);
    assert!(matches!(
        preparation.outcome,
        TemplatePreparationOutcome::Runtime(_)
    ));
}

#[test]
fn preparation_reports_missing_wrapper_root_without_a_separate_slot_layout_walk() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let missing_root = TemplateIrNodeId::new(999);
    let wrapper_template = store.push_template(TemplateIr::new(
        missing_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    ));
    let wrapper_set = store.push_wrapper_set(TemplateWrapperSet {
        wrappers: vec![TemplateWrapperReference::new(
            wrapper_template,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        )],
    });
    let outer_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_text_node(
            string_table.intern("outer"),
            "outer".len(),
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
    };
    store
        .set_conditional_child_wrapper_set(outer_template, wrapper_set)
        .expect("wrapper set should attach to the outer template");

    let error = prepare_slot_plan_view(&store, outer_template)
        .expect_err("preparation should reject the missing wrapper root");
    let TemplateError::Infrastructure(error) = error else {
        panic!("missing wrapper root should remain an infrastructure error");
    };
    assert!(
        error.msg.contains("TIR preparation: node"),
        "preparation should own missing wrapper-root validation, got: {}",
        error.msg
    );
}

#[test]
fn runtime_contribution_constness_propagates_option_capture_bindings() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;
    let capture_name = string_table.intern("value");
    let capture_path = InternedPath::from_single_str("main.moth/#value", &mut string_table);
    let scrutinee = Expression::option_none_with_type_id(
        string_type_id,
        DataType::StringSlice,
        &mut type_environment,
        empty_location(),
    );
    let capture_expression = Expression::new(
        ExpressionKind::Reference(capture_path.clone()),
        empty_location(),
        string_type_id,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );
    let root = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body_expression = builder.push_dynamic_expression_node(
            capture_expression,
            TemplateSegmentOrigin::Body,
            None,
            empty_location(),
        );
        let body = builder.push_sequence_node(vec![body_expression], empty_location());
        let fallback_text = builder.push_text_node(
            string_table.intern("fallback"),
            "fallback".len(),
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let fallback = builder.push_sequence_node(vec![fallback_text], empty_location());
        let selector = TemplateBranchSelector::OptionPresentCapture {
            scrutinee,
            pattern: Box::new(MatchPattern::OptionPresentCapture {
                name: capture_name,
                binding_path: capture_path,
                inner_type_id: string_type_id,
                location: empty_location(),
                binding_location: empty_location(),
            }),
        };
        let branch = TemplateIrBranch::new(
            selector,
            body,
            empty_location(),
            builder.store.next_expression_site_id(),
        );
        builder.push_branch_chain_node(vec![branch], Some(fallback), empty_location())
    };

    assert!(
        crate::compiler_frontend::ast::templates::tir::tir_node_is_const_evaluable_value(
            &store,
            root,
            &string_table,
        )
        .expect("constness query should preserve store authority"),
        "a const option capture must make its branch binding available to the body"
    );
}

#[test]
fn preparation_validates_wrapper_set_authority() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("wrapper set");
        let node = builder.push_text_node(text, 11, TemplateSegmentOrigin::Body, empty_location());
        builder.finish_template(
            node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    MalformedTirStore::new(&mut store)
        .set_conditional_child_wrapper_set(template_id, Some(TemplateWrapperSetId::new(999)));
    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("view should construct before wrapper-set preparation");

    let error = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect_err("missing wrapper set must remain authority failure");
    let TemplateError::Infrastructure(error) = error else {
        panic!("missing wrapper set must remain infrastructure");
    };
    assert!(error.msg.contains("TIR preparation: wrapper set"));
}

fn prepare_slot_plan_view(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> Result<TemplatePreparation, TemplateError> {
    let view = TirView::new(
        store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )?;
    prepare_tir_view(&view, TemplatePreparationMode::Value)
}

fn assert_preparation_authority_error(
    result: Result<TemplatePreparation, TemplateError>,
    marker: &str,
) {
    let error = result.expect_err("malformed contribution marker must be an authority error");
    let TemplateError::Infrastructure(error) = error else {
        panic!("contribution-source authority must stay infrastructure, got {error:?}");
    };
    assert!(
        error.msg.contains(marker),
        "expected {marker:?} in {}",
        error.msg
    );
}

fn empty_source_plan(
    source: RuntimeSlotContributionSourceId,
    render_root: TemplateIrNodeId,
) -> TemplateSlotContributionSourcePlan {
    TemplateSlotContributionSourcePlan {
        source,
        target: SlotKey::Default,
        render_root,
        renders_wrapper_unconditionally: true,
        location: empty_location(),
    }
}

fn site_plan(site: RuntimeSlotSiteId, render_root: TemplateIrNodeId) -> TemplateSlotSitePlan {
    TemplateSlotSitePlan {
        site,
        key: SlotKey::Default,
        render_root,
        location: empty_location(),
    }
}

#[test]
fn preparation_rejects_contribution_marker_with_wrong_plan() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let text = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_text_node(
            string_table.intern("x"),
            1,
            TemplateSegmentOrigin::Body,
            empty_location(),
        )
    };
    let owner_plan = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![empty_source_plan(RuntimeSlotContributionSourceId(0), text)],
        slot_sites: vec![],
    });
    let other_plan = TemplateSlotPlanId::new(owner_plan.index() + 1);
    let marker = push_runtime_slot_contribution_source(
        &mut store,
        other_plan,
        RuntimeSlotContributionSourceId(0),
        empty_location(),
    );
    MalformedTirStore::new(&mut store)
        .replace_slot_sites(owner_plan, vec![site_plan(RuntimeSlotSiteId(0), marker)]);
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            text,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    MalformedTirStore::new(&mut store).set_runtime_slot_plan(template_id, Some(owner_plan));

    assert_preparation_authority_error(
        prepare_slot_plan_view(&store, template_id),
        "differs from the active plan",
    );
}

#[test]
fn preparation_rejects_contribution_marker_outside_owning_plan() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let marker = {
        let plan = store.push_slot_plan(TemplateSlotPlan {
            location: empty_location(),
            contribution_sources: vec![],
            slot_sites: vec![],
        });
        push_runtime_slot_contribution_source(
            &mut store,
            plan,
            RuntimeSlotContributionSourceId(0),
            empty_location(),
        )
    };
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("plain");
        let text_node =
            builder.push_text_node(text, 5, TemplateSegmentOrigin::Body, empty_location());
        let root = builder.push_sequence_node(vec![text_node, marker], empty_location());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };

    assert_preparation_authority_error(
        prepare_slot_plan_view(&store, template_id),
        "outside its owning plan",
    );
}

#[test]
fn preparation_rejects_out_of_range_contribution_source() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let text = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let interned = string_table.intern("src");
        builder.push_text_node(interned, 3, TemplateSegmentOrigin::Body, empty_location())
    };
    let plan = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![empty_source_plan(RuntimeSlotContributionSourceId(0), text)],
        slot_sites: vec![],
    });
    let marker = push_runtime_slot_contribution_source(
        &mut store,
        plan,
        RuntimeSlotContributionSourceId(1),
        empty_location(),
    );
    MalformedTirStore::new(&mut store)
        .replace_slot_sites(plan, vec![site_plan(RuntimeSlotSiteId(0), marker)]);
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            text,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    MalformedTirStore::new(&mut store).set_runtime_slot_plan(template_id, Some(plan));

    assert_preparation_authority_error(prepare_slot_plan_view(&store, template_id), "outside plan");
}

#[test]
fn preparation_rejects_source_identity_mismatch() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let text = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let interned = string_table.intern("src");
        builder.push_text_node(interned, 3, TemplateSegmentOrigin::Body, empty_location())
    };
    let plan = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![empty_source_plan(RuntimeSlotContributionSourceId(0), text)],
        slot_sites: vec![],
    });
    MalformedTirStore::new(&mut store).replace_contribution_sources(
        plan,
        vec![empty_source_plan(RuntimeSlotContributionSourceId(7), text)],
    );
    let marker = push_runtime_slot_contribution_source(
        &mut store,
        plan,
        RuntimeSlotContributionSourceId(0),
        empty_location(),
    );
    MalformedTirStore::new(&mut store)
        .replace_slot_sites(plan, vec![site_plan(RuntimeSlotSiteId(0), marker)]);
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            text,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    MalformedTirStore::new(&mut store).set_runtime_slot_plan(template_id, Some(plan));

    assert_preparation_authority_error(
        prepare_slot_plan_view(&store, template_id),
        "does not match its index",
    );
}

#[test]
fn preparation_rejects_plan_a_source_inside_plan_b() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let text = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let interned = string_table.intern("src");
        builder.push_text_node(interned, 3, TemplateSegmentOrigin::Body, empty_location())
    };
    let plan_a = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![empty_source_plan(RuntimeSlotContributionSourceId(0), text)],
        slot_sites: vec![],
    });
    let plan_a_marker = push_runtime_slot_contribution_source(
        &mut store,
        plan_a,
        RuntimeSlotContributionSourceId(0),
        empty_location(),
    );
    let plan_b = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![empty_source_plan(RuntimeSlotContributionSourceId(0), text)],
        slot_sites: vec![site_plan(RuntimeSlotSiteId(0), plan_a_marker)],
    });
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            text,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    MalformedTirStore::new(&mut store).set_runtime_slot_plan(template_id, Some(plan_b));

    assert_preparation_authority_error(
        prepare_slot_plan_view(&store, template_id),
        "differs from the active plan",
    );
}

#[test]
fn preparation_keeps_nested_plans_with_local_source_zero_independent() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let inner_text = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let interned = string_table.intern("inner");
        builder.push_text_node(interned, 5, TemplateSegmentOrigin::Body, empty_location())
    };
    let inner_marker = {
        let inner_plan = store.push_slot_plan(TemplateSlotPlan {
            location: empty_location(),
            contribution_sources: vec![empty_source_plan(
                RuntimeSlotContributionSourceId(0),
                inner_text,
            )],
            slot_sites: vec![],
        });
        let marker = push_runtime_slot_contribution_source(
            &mut store,
            inner_plan,
            RuntimeSlotContributionSourceId(0),
            empty_location(),
        );
        MalformedTirStore::new(&mut store)
            .replace_slot_sites(inner_plan, vec![site_plan(RuntimeSlotSiteId(0), marker)]);
        let mut inner_template = TemplateIr::new(
            inner_text,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        );
        inner_template.runtime_slot_plan = Some(inner_plan);
        store.push_template(inner_template)
    };
    let outer_text = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let interned = string_table.intern("outer");
        builder.push_text_node(interned, 5, TemplateSegmentOrigin::Body, empty_location())
    };
    let child_node = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                inner_marker,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            empty_location(),
        )
    };
    let outer_plan = store.push_slot_plan(TemplateSlotPlan {
        location: empty_location(),
        contribution_sources: vec![empty_source_plan(
            RuntimeSlotContributionSourceId(0),
            child_node,
        )],
        slot_sites: vec![],
    });
    let outer_marker = push_runtime_slot_contribution_source(
        &mut store,
        outer_plan,
        RuntimeSlotContributionSourceId(0),
        empty_location(),
    );
    MalformedTirStore::new(&mut store).replace_slot_sites(
        outer_plan,
        vec![site_plan(RuntimeSlotSiteId(0), outer_marker)],
    );
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(
            outer_text,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            empty_location(),
        )
    };
    MalformedTirStore::new(&mut store).set_runtime_slot_plan(template_id, Some(outer_plan));

    prepare_slot_plan_view(&store, template_id).expect("nested plans may both use local source 0");
}
