//! TIR classification tests.
//!
//! WHAT: proves TIR structural and effective-view classification behavior.
//!
//! WHY: structural walkers and effective views own production classification.
//! Tests target those owners directly instead of preserving compatibility
//! materialization entry points for detached fixtures.

use super::super::builder::TemplateIrBuilder;
use super::super::classification::{
    TirTemplateClassification, classify_effective_tir_view_template,
    tir_node_is_const_evaluable_value_with_bindings,
};
use super::super::contribution_shape::classify_tir_contribution_node;
use super::super::node::{TemplateIrNode, TemplateIrNodeKind};
use super::super::store::TemplateIrStore;
use super::super::summary::TemplateIrSummary;
use super::super::{TemplateTirPhase, TirView};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotKey, Style, TemplateConstValueKind, TemplateSegmentOrigin,
    TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopControlKind;
use crate::compiler_frontend::ast::templates::tir::ids::{
    SlotOccurrenceId, TemplateIrId, TemplateIrNodeId,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionOverlay,
    TirWrapperContextOverlay,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

// -------------------------
//  Test helpers
// -------------------------

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn string_expression(string_table: &mut StringTable, text: &str) -> Expression {
    Expression::string_slice(
        string_table.intern(text),
        empty_location(),
        ValueMode::ImmutableOwned,
    )
}

fn string_function_call_expression(string_table: &mut StringTable, name: &str) -> Expression {
    let scope = InternedPath::from_single_str("main.moth", string_table);
    let name_id = string_table.intern(name);

    Expression::new(
        ExpressionKind::FunctionCall {
            name: scope.append(name_id),
            args: Vec::new(),
            result_type_ids: vec![builtin_type_ids::STRING],
        },
        empty_location(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    )
}

fn string_function_child_id_with_runtime_head(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> TemplateIrId {
    let runtime_head = string_function_call_expression(string_table, "wrapper");
    let mut builder = TemplateIrBuilder::new(store);
    let dynamic_head = builder.push_dynamic_expression_node(
        runtime_head,
        TemplateSegmentOrigin::Head,
        None,
        empty_location(),
    );
    let root = builder.push_sequence_node(vec![dynamic_head], empty_location());

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary {
            dynamic_expression_count: 1,
            max_depth: 1,
            is_const_evaluable_shape: false,
            ..TemplateIrSummary::empty()
        },
        empty_location(),
    )
}

fn classify_store_view_template(
    string_table: &mut StringTable,
    template_kind: TemplateType,
    build_root: impl FnOnce(
        &mut TemplateIrBuilder<'_>,
        &mut StringTable,
    ) -> super::super::ids::TemplateIrNodeId,
) -> TemplateConstValueKind {
    classify_store_view_template_result(string_table, template_kind, build_root)
        .expect("view classification should succeed")
        .const_value_kind
}

fn classify_store_view_template_result(
    string_table: &mut StringTable,
    template_kind: TemplateType,
    build_root: impl FnOnce(
        &mut TemplateIrBuilder<'_>,
        &mut StringTable,
    ) -> super::super::ids::TemplateIrNodeId,
) -> Result<TirTemplateClassification, TemplateError> {
    let mut store = TemplateIrStore::new();
    let context = TemplateViewContext::default();

    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = build_root(&mut builder, string_table);

        builder.finish_template(
            root,
            Style::default(),
            template_kind,
            TemplateIrSummary::empty(),
            empty_location(),
        )
    };

    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateTirPhase::Composed,
        context,
    )
    .expect("test view should resolve");

    classify_effective_tir_view_template(&view)
}

fn assert_compiler_infrastructure_error<T>(result: Result<T, TemplateError>, context: &str) {
    let error = match result {
        Ok(_) => panic!("{context} should fail through the compiler-error lane"),
        Err(error) => error,
    };

    let TemplateError::Infrastructure(error) = error else {
        panic!("{context} should fail as infrastructure/compiler error");
    };
    assert_eq!(
        error.error_type,
        ErrorType::Compiler,
        "{context} should use the compiler error type"
    );
}

#[test]
fn effective_classification_rejects_missing_structural_node_authority() {
    let mut string_table = StringTable::new();
    let result = classify_store_view_template_result(
        &mut string_table,
        TemplateType::String,
        |builder, _| builder.push_sequence_node(vec![TemplateIrNodeId::new(99)], empty_location()),
    );

    assert_compiler_infrastructure_error(result, "missing structural node authority");
}

#[test]
fn effective_classification_rejects_missing_same_store_child_template() {
    let mut string_table = StringTable::new();
    let result = classify_store_view_template_result(
        &mut string_table,
        TemplateType::String,
        |builder, _| builder.push_child_template_node(TemplateIrId::new(99), empty_location()),
    );

    assert_compiler_infrastructure_error(result, "missing same-store child template authority");
}

#[test]
fn effective_classification_rejects_missing_insert_template() {
    let mut string_table = StringTable::new();
    let result = classify_store_view_template_result(
        &mut string_table,
        TemplateType::String,
        |builder, _| {
            let insert =
                builder.push_insert_contribution_node(TemplateIrId::new(99), empty_location());
            builder.push_sequence_node(vec![insert], empty_location())
        },
    );

    assert_compiler_infrastructure_error(result, "missing insert-template authority");
}

#[test]
fn contribution_shape_rejects_missing_node_authority() {
    let store = TemplateIrStore::new();

    let error = classify_tir_contribution_node(&store, TemplateIrNodeId::new(0))
        .expect_err("missing contribution nodes must fail classification");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(error.msg.contains("contribution node ID"));
}

#[test]
fn contribution_shape_rejects_missing_same_store_child_template() {
    let mut store = TemplateIrStore::new();
    let child_node = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_child_template_node(TemplateIrId::new(0), empty_location())
    };

    let error = classify_tir_contribution_node(&store, child_node)
        .expect_err("missing same-store child templates must fail classification");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(error.msg.contains("child template ID"));
}

#[test]
fn contribution_shape_rejects_missing_insert_template() {
    let mut store = TemplateIrStore::new();
    let insert_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::InsertContribution {
            template: TemplateIrId::new(0),
        },
        empty_location(),
    ));

    let error = classify_tir_contribution_node(&store, insert_node)
        .expect_err("missing insert templates must fail classification");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(error.msg.contains("insert contribution template ID"));
}

#[test]
fn const_body_evaluation_treats_string_function_children_as_structural() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let child_template = string_function_child_id_with_runtime_head(&mut store, &mut string_table);

    let mut builder = TemplateIrBuilder::new(&mut store);
    let child_node = builder.push_child_template_node(child_template, empty_location());
    let root = builder.push_sequence_node(vec![child_node], empty_location());

    assert!(
        tir_node_is_const_evaluable_value_with_bindings(&store, root, &[], &string_table),
        "StringFunction children are wrapper values in const-required body validation"
    );
}

// -------------------------
//  TirView classification
// -------------------------

#[test]
fn tir_view_classification_returns_renderable_string_for_text_root() {
    let mut string_table = StringTable::new();

    let const_kind =
        classify_store_view_template(&mut string_table, TemplateType::String, |builder, table| {
            let text = table.intern("hello");
            let text_node = builder.push_text_node(
                text,
                "hello".len() as u32,
                TemplateSegmentOrigin::Body,
                empty_location(),
            );

            builder.push_sequence_node(vec![text_node], empty_location())
        });

    assert_eq!(const_kind, TemplateConstValueKind::RenderableString);
}

#[test]
fn tir_view_classification_rejects_reactive_text() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth/#reactive", &mut string_table);
    let subscription = ReactiveSubscription {
        source: ReactiveSource {
            path: source_path,
            kind: ReactiveSourceKind::Declaration,
        },
        type_id: builtin_type_ids::STRING,
        location: empty_location(),
    };

    let const_kind = classify_store_view_template(
        &mut string_table,
        TemplateType::String,
        move |builder, table| {
            let text = table.intern("reactive text");
            let text_node = builder.push_text_node_with_subscription(
                text,
                "reactive text".len() as u32,
                TemplateSegmentOrigin::Body,
                Some(subscription),
                empty_location(),
            );

            builder.push_sequence_node(vec![text_node], empty_location())
        },
    );

    assert_eq!(const_kind, TemplateConstValueKind::NonConst);
}

#[test]
fn tir_view_classification_preserves_slot_insert_helper_kind() {
    let mut string_table = StringTable::new();
    let slot_name = string_table.intern("title");

    let const_kind = classify_store_view_template(
        &mut string_table,
        TemplateType::SlotInsert(SlotKey::Named(slot_name)),
        |builder, table| {
            let text = table.intern("helper");
            let text_node = builder.push_text_node(
                text,
                "helper".len() as u32,
                TemplateSegmentOrigin::Body,
                empty_location(),
            );

            builder.push_sequence_node(vec![text_node], empty_location())
        },
    );

    assert_eq!(const_kind, TemplateConstValueKind::SlotInsertHelper);
}

#[test]
fn tir_view_classification_preserves_loop_control_signal() {
    let mut string_table = StringTable::new();

    let const_kind =
        classify_store_view_template(&mut string_table, TemplateType::String, |builder, _| {
            builder.push_loop_control_node(TemplateLoopControlKind::Break, empty_location())
        });

    assert_eq!(const_kind, TemplateConstValueKind::LoopControlSignal);
}

#[test]
fn tir_view_classification_returns_non_const_for_runtime_expression() {
    let mut string_table = StringTable::new();

    let const_kind =
        classify_store_view_template(&mut string_table, TemplateType::String, |builder, table| {
            let runtime_expression = string_function_call_expression(table, "runtime_text");
            let runtime_node = builder.push_dynamic_expression_node(
                runtime_expression,
                TemplateSegmentOrigin::Body,
                None,
                empty_location(),
            );

            builder.push_sequence_node(vec![runtime_node], empty_location())
        });

    assert_eq!(const_kind, TemplateConstValueKind::NonConst);
}

#[test]
fn expression_overlay_requires_finalized_view_and_drives_classification() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let empty_context = TemplateViewContext::default();

    let (template_id, site_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let runtime_expression = string_function_call_expression(&mut string_table, "runtime_text");
        let dynamic_node = builder.push_dynamic_expression_node(
            runtime_expression,
            TemplateSegmentOrigin::Body,
            None,
            empty_location(),
        );
        let root = builder.push_sequence_node(vec![dynamic_node], empty_location());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        );
        let site_id = match &store
            .get_node(dynamic_node)
            .expect("dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            _ => unreachable!("test node should be a dynamic expression"),
        };

        (template_id, site_id)
    };

    let expression_overlay_id = store.allocate_expression_overlay(TirExpressionOverlay {
        overrides: vec![(
            site_id,
            Box::new(string_expression(&mut string_table, "normalized")),
        )],
    });
    let expression_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    let context = empty_context.merge(expression_context);
    let composed_view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateTirPhase::Composed,
        context,
    )
    .expect("composed test view should resolve");

    let error = match classify_effective_tir_view_template(&composed_view) {
        Ok(_) => panic!("expression overlays must not classify before Finalized"),
        Err(error) => error,
    };
    let crate::compiler_frontend::ast::templates::error::TemplateError::Infrastructure(error) =
        error
    else {
        panic!("phase rejection should be an internal classification invariant");
    };
    assert!(
        error
            .msg
            .contains("expression-overlay classification requires Finalized"),
        "phase rejection should identify the expression-overlay boundary"
    );

    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("test view should resolve");

    let classification = classify_effective_tir_view_template(&view)
        .expect("finalized effective view classification should succeed");

    assert_eq!(
        classification.const_value_kind,
        TemplateConstValueKind::RenderableString,
        "classification must use the normalized expression overlay instead of the structural runtime expression"
    );
}

/// Finalized view classification with a slot-resolution overlay must succeed.
///
/// WHAT: a template whose view context carries a slot-resolution dimension should
///       not be rejected merely because that dimension is present. When the
///       structural tree has no unresolved slots (the overlay is empty here),
///       classification proceeds normally.
/// WHY: slot-resolution overlays are now a supported overlay dimension for
///      classification. Wrapper-context overlays are also supported.
#[test]
fn finalized_tir_view_classification_accepts_slot_resolution_overlay() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let slot_overlay_id = store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
        resolutions: Vec::new(),
    });
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay_id),
        wrapper_context: None,
    };

    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("overlay");
        let text_node = builder.push_text_node(
            text,
            "overlay".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let root = builder.push_sequence_node(vec![text_node], empty_location());

        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        )
    };
    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("test view should resolve");

    let classification = classify_effective_tir_view_template(&view)
        .expect("classification with a slot-resolution overlay should succeed");

    assert_eq!(
        classification.const_value_kind,
        TemplateConstValueKind::RenderableString,
        "a text-only template with an empty slot-resolution overlay should classify as RenderableString"
    );
}

/// Composed view classification with a slot-resolution overlay must succeed.
///
/// WHAT: slot-resolution overlays are attached during composition, before
///       expression-overlay normalization requires `Finalized`.
/// WHY: Phase 4 finalization folds Composed slot-overlay views directly. The
///      effective classifier must accept that phase when no expression overlay
///      is present.
#[test]
fn composed_tir_view_classification_accepts_slot_resolution_overlay() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let slot_overlay_id = store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
        resolutions: Vec::new(),
    });
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay_id),
        wrapper_context: None,
    };

    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("overlay");
        let text_node = builder.push_text_node(
            text,
            "overlay".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let root = builder.push_sequence_node(vec![text_node], empty_location());

        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        )
    };
    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateTirPhase::Composed,
        context,
    )
    .expect("test view should resolve");

    let classification = classify_effective_tir_view_template(&view)
        .expect("classification with a Composed slot-resolution overlay should succeed");

    assert_eq!(
        classification.const_value_kind,
        TemplateConstValueKind::RenderableString,
        "a Composed text-only template with a slot-resolution overlay should classify as RenderableString"
    );
}

/// Finalized view classification with a resolved slot-resolution overlay returns
/// `WrapperTemplate` for a `String` template that still has structural slots.
///
/// WHAT: a `String` template with a `Slot` node and a slot-resolution overlay
///       should classify successfully. The structural tree has slots, so the
///       const-value kind is `WrapperTemplate` — the overlay will resolve the
///       slot at fold time, but classification reports the structural shape.
/// WHY: proves classification no longer rejects slot-resolution overlays and
///      still reports the correct const-value kind for slot-bearing templates.
#[test]
fn finalized_tir_view_classification_with_resolved_slot_returns_wrapper_template() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let (wrapper_template_id, fill_template_id) = {
        // Fill template: "filled".
        let fill_text_id = string_table.intern("filled");
        let mut fill_builder = TemplateIrBuilder::new(&mut store);
        let fill_text_node = fill_builder.push_text_node(
            fill_text_id,
            "filled".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let fill_root = fill_builder.push_sequence_node(vec![fill_text_node], empty_location());
        let fill_template_id = fill_builder.finish_template(
            fill_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        );

        // Wrapper template: "before" + $slot(default) + "after".
        let before_id = string_table.intern("before");
        let after_id = string_table.intern("after");
        let mut wrapper_builder = TemplateIrBuilder::new(&mut store);
        let before_node = wrapper_builder.push_text_node(
            before_id,
            "before".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let slot_node_id = wrapper_builder.push_slot_node(SlotKey::Default, empty_location());
        let after_node = wrapper_builder.push_text_node(
            after_id,
            "after".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let wrapper_root = wrapper_builder.push_sequence_node(
            vec![before_node, slot_node_id, after_node],
            empty_location(),
        );
        let wrapper_template_id = wrapper_builder.finish_template(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        );

        (wrapper_template_id, fill_template_id)
    };

    // Build a slot-resolution overlay that resolves the default slot to the
    // fill template. The slot occurrence ID is 0 (first slot in the store).
    let fill_ref = fill_template_id;
    let slot_overlay_id = store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
        resolutions: vec![(
            SlotOccurrenceId::new(0),
            TirSlotResolution::resolved(SlotKey::Default, vec![fill_ref]),
        )],
    });
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay_id),
        wrapper_context: None,
    };
    let view = TirView::with_minimum_phase(
        &store,
        wrapper_template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("test view should resolve");

    let classification = classify_effective_tir_view_template(&view)
        .expect("classification with resolved slot overlay should succeed");

    assert_eq!(
        classification.const_value_kind,
        TemplateConstValueKind::WrapperTemplate,
        "a String template with structural slots should classify as WrapperTemplate"
    );
    assert!(
        !classification.has_slot_insertions,
        "no escaped slot-insert children should be present"
    );
}

/// Finalized view classification with a wrapper-context overlay succeeds.
///
/// WHAT: wrapper-context overlays are now a supported overlay dimension. Because
///       inherited wrappers wrap child-template emissions, they do not change
///       the parent template's own const-value shape.
/// WHY: classification should not force a fallback merely because a
///      wrapper-context overlay is present.
#[test]
fn finalized_tir_view_classification_accepts_wrapper_context_overlay() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let wrapper_overlay_id =
        store.allocate_wrapper_context_overlay(TirWrapperContextOverlay::default());
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(wrapper_overlay_id),
    };

    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text = string_table.intern("overlay");
        let text_node = builder.push_text_node(
            text,
            "overlay".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let root = builder.push_sequence_node(vec![text_node], empty_location());

        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        )
    };
    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("test view should resolve");

    let classification = classify_effective_tir_view_template(&view)
        .expect("wrapper-context overlays should not prevent classification");

    assert_eq!(
        classification.const_value_kind,
        TemplateConstValueKind::RenderableString,
        "wrapper-context overlay should not change the parent const-value kind"
    );
}

/// Effective view classification of a structural slot returns `RenderableString`
/// whether the slot-resolution overlay is absent or allocated-but-empty.
///
/// WHAT: a template with one `Slot` node and no resolved entry folds the slot to
///       no output in both contexts. `has_unresolved_slots` stays `true` because
///       the structural slot remains present regardless of overlay allocation.
/// WHY: proves the presence of an allocated empty slot-resolution dimension alone
///      does not force a `WrapperTemplate` classification and does not erase the
///      structural slot flag. The no-overlay and empty-overlay rows are parallel
///      because the fold path treats an absent overlay and an uncovered overlay
///      identically.
#[test]
fn unresolved_slot_is_renderable_with_absent_or_empty_overlay() {
    let mut store = TemplateIrStore::new();

    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot_node = builder.push_slot_node(SlotKey::Default, empty_location());
        let root = builder.push_sequence_node(vec![slot_node], empty_location());

        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        )
    };

    // Row 1: no slot-resolution overlay (default context).
    let no_overlay_view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )
    .expect("no-overlay test view should resolve");

    let no_overlay_classification = classify_effective_tir_view_template(&no_overlay_view)
        .expect("no-overlay effective view classification should succeed");

    assert_eq!(
        no_overlay_classification.const_value_kind,
        TemplateConstValueKind::RenderableString,
        "an uncovered slot with no overlay should classify as RenderableString"
    );
    assert!(
        no_overlay_classification.has_unresolved_slots,
        "structural slot flag must remain true without an overlay"
    );

    // Row 2: an allocated empty slot-resolution overlay (no entries for the slot).
    let empty_overlay_id = store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
        resolutions: Vec::new(),
    });
    let empty_overlay_context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(empty_overlay_id),
        wrapper_context: None,
    };
    let empty_overlay_view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        empty_overlay_context,
    )
    .expect("empty-overlay test view should resolve");

    let empty_overlay_classification = classify_effective_tir_view_template(&empty_overlay_view)
        .expect("empty-overlay effective view classification should succeed");

    assert_eq!(
        empty_overlay_classification.const_value_kind,
        TemplateConstValueKind::RenderableString,
        "a slot not covered by an empty overlay should classify as RenderableString"
    );
    assert!(
        empty_overlay_classification.has_unresolved_slots,
        "structural slot flag must remain true with an empty overlay"
    );
}

/// Effective view classification with a resolved slot returns `WrapperTemplate`.
///
/// WHAT: the slot-resolution overlay covers the structural slot with a
///       `Resolved` entry, so the template wraps the resolved source's content.
/// WHY: this is the existing resolved-slot expectation; the new
///      unresolved-slot logic must not regress it.
#[test]
fn effective_view_classification_resolved_slot_returns_wrapper_template() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let (wrapper_template_id, fill_template_id) = {
        let fill_text_id = string_table.intern("filled");
        let mut fill_builder = TemplateIrBuilder::new(&mut store);
        let fill_text_node = fill_builder.push_text_node(
            fill_text_id,
            "filled".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let fill_root = fill_builder.push_sequence_node(vec![fill_text_node], empty_location());
        let fill_template_id = fill_builder.finish_template(
            fill_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        );

        let before_id = string_table.intern("before");
        let after_id = string_table.intern("after");
        let mut wrapper_builder = TemplateIrBuilder::new(&mut store);
        let before_node = wrapper_builder.push_text_node(
            before_id,
            "before".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let slot_node_id = wrapper_builder.push_slot_node(SlotKey::Default, empty_location());
        let after_node = wrapper_builder.push_text_node(
            after_id,
            "after".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let wrapper_root = wrapper_builder.push_sequence_node(
            vec![before_node, slot_node_id, after_node],
            empty_location(),
        );
        let wrapper_template_id = wrapper_builder.finish_template(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        );

        (wrapper_template_id, fill_template_id)
    };

    let fill_ref = fill_template_id;
    let slot_overlay_id = store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
        resolutions: vec![(
            SlotOccurrenceId::new(0),
            TirSlotResolution::resolved(SlotKey::Default, vec![fill_ref]),
        )],
    });
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay_id),
        wrapper_context: None,
    };
    let view = TirView::with_minimum_phase(
        &store,
        wrapper_template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("test view should resolve");

    let classification = classify_effective_tir_view_template(&view)
        .expect("classification with resolved slot overlay should succeed");

    assert_eq!(
        classification.const_value_kind,
        TemplateConstValueKind::WrapperTemplate,
        "a resolved slot should classify as WrapperTemplate"
    );
    assert!(
        classification.has_unresolved_slots,
        "structural slot flag must remain true"
    );
}

/// Effective view classification with two slots resolves only one returns
/// `WrapperTemplate`.
///
/// WHAT: at least one slot occurrence maps to `Resolved`, so the template wraps
///       content even though the other slot is uncovered.
/// WHY: the decision is "any resolved slot", not "all resolved".
#[test]
fn effective_view_classification_partially_resolved_slots_returns_wrapper_template() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let (wrapper_template_id, fill_template_id) = {
        let fill_text_id = string_table.intern("filled");
        let mut fill_builder = TemplateIrBuilder::new(&mut store);
        let fill_text_node = fill_builder.push_text_node(
            fill_text_id,
            "filled".len() as u32,
            TemplateSegmentOrigin::Body,
            empty_location(),
        );
        let fill_root = fill_builder.push_sequence_node(vec![fill_text_node], empty_location());
        let fill_template_id = fill_builder.finish_template(
            fill_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        );

        let mut wrapper_builder = TemplateIrBuilder::new(&mut store);
        let first_slot = wrapper_builder.push_slot_node(SlotKey::Default, empty_location());
        let second_slot = wrapper_builder.push_slot_node(SlotKey::Default, empty_location());
        let wrapper_root =
            wrapper_builder.push_sequence_node(vec![first_slot, second_slot], empty_location());
        let wrapper_template_id = wrapper_builder.finish_template(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        );

        (wrapper_template_id, fill_template_id)
    };

    let fill_ref = fill_template_id;
    let slot_overlay_id = store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
        resolutions: vec![(
            SlotOccurrenceId::new(0),
            TirSlotResolution::resolved(SlotKey::Default, vec![fill_ref]),
        )],
    });
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay_id),
        wrapper_context: None,
    };
    let view = TirView::with_minimum_phase(
        &store,
        wrapper_template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("test view should resolve");

    let classification = classify_effective_tir_view_template(&view)
        .expect("classification with partially resolved slots should succeed");

    assert_eq!(
        classification.const_value_kind,
        TemplateConstValueKind::WrapperTemplate,
        "partially resolved slots should still classify as WrapperTemplate"
    );
}

/// Effective view classification with two slots and no resolutions returns
/// `RenderableString`.
///
/// WHAT: every structural slot is uncovered by the slot-resolution overlay, so
///       all of them fold to no output.
/// WHY: confirms the "all unresolved" case for multiple slots.
#[test]
fn effective_view_classification_two_unresolved_slots_returns_renderable_string() {
    let mut store = TemplateIrStore::new();

    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let first_slot = builder.push_slot_node(SlotKey::Default, empty_location());
        let second_slot = builder.push_slot_node(SlotKey::Default, empty_location());
        let root = builder.push_sequence_node(vec![first_slot, second_slot], empty_location());

        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        )
    };

    let slot_overlay_id = store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
        resolutions: Vec::new(),
    });
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay_id),
        wrapper_context: None,
    };
    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("test view should resolve");

    let classification = classify_effective_tir_view_template(&view)
        .expect("effective view classification should succeed");

    assert_eq!(
        classification.const_value_kind,
        TemplateConstValueKind::RenderableString,
        "two uncovered slots should classify as RenderableString"
    );
}
