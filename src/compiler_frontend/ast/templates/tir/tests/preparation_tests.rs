//! Exact-view preparation result tests.
//!
//! WHAT: protects the compact preparation result and its exact-view identity.
//! WHY: finalization must not reconstruct a second disposition beside the
//!      preparation owner.

use super::super::builder::TemplateIrBuilder;
use super::super::ids::{TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId, TemplateWrapperSetId};
use super::super::preparation::{
    PreparedTemplate, RuntimeTemplateReason, TemplateHelperKind, TemplatePreparationMode,
    prepare_tir_view,
};
use super::super::store::TemplateIrStore;
use super::super::summary::TemplateIrSummary;
use super::super::{TemplateTirPhase, TemplateViewContext, TirView};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotKey, Style, Template, TemplateConstValueKind, TemplateSegmentOrigin,
    TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopControlKind, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::node::{TemplateIr, TemplateIrBranch};
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateTirReference,
};
use crate::compiler_frontend::datatypes::DataType;
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
) -> Result<(PreparedTemplate, super::super::view::TirViewIdentity), TemplateError> {
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

    match (value, const_required) {
        (PreparedTemplate::Foldable(value), PreparedTemplate::Foldable(const_required)) => {
            assert_eq!(value.identity, identity);
            assert_eq!(const_required.identity, const_identity);
            assert_eq!(value.identity, const_required.identity);
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
        PreparedTemplate::Foldable(prepared)
            if prepared.value_kind == TemplateConstValueKind::WrapperTemplate
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
        PreparedTemplate::Runtime(runtime) => {
            assert_eq!(runtime.identity, identity);
            assert_eq!(runtime.reason, RuntimeTemplateReason::RuntimeExpression);
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
                "reactive text".len() as u32,
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
        PreparedTemplate::Runtime(runtime) => {
            assert_eq!(runtime.identity, identity);
            assert_eq!(runtime.reason, RuntimeTemplateReason::ReactiveContent);
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
                "static function body".len() as u32,
                TemplateSegmentOrigin::Body,
                empty_location(),
            );
            builder.push_sequence_node(vec![text_node], empty_location())
        },
        TemplatePreparationMode::Value,
    )
    .expect("StringFunction preparation should succeed");

    assert!(matches!(prepared, PreparedTemplate::Foldable(_)));
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
        slot_insert,
        PreparedTemplate::Helper(TemplateHelperKind::SlotInsert)
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
        loop_control,
        PreparedTemplate::Helper(TemplateHelperKind::LoopControl)
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
        );
        builder.push_branch_chain_node(vec![branch], None, empty_location())
    };

    let (value, _) = prepare_root(
        TemplateType::StringFunction,
        build_branch,
        TemplatePreparationMode::Value,
    )
    .expect("Value mode should preserve lazy branch runtime semantics");
    assert!(matches!(value, PreparedTemplate::Runtime(_)));

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
fn preparation_classifies_exact_child_cycle_as_runtime() {
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

    let prepared = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("child cycles remain valid runtime values");
    assert!(matches!(
        prepared,
        PreparedTemplate::Runtime(runtime)
            if runtime.reason == RuntimeTemplateReason::ChildTemplateCycle
    ));
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

    let prepared = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("nested value cycles remain valid runtime values");
    assert!(matches!(
        prepared,
        PreparedTemplate::Runtime(runtime)
            if runtime.reason == RuntimeTemplateReason::ChildTemplateCycle
    ));
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
    store.templates[template_id.index()].runtime_slot_plan = Some(TemplateSlotPlanId::new(999));
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
    store.templates[template_id.index()].conditional_child_wrapper_set =
        Some(TemplateWrapperSetId::new(999));
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
