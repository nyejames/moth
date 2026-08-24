//! Tests for AST template normalization at the HIR boundary.

use super::super::public_const_templates::{
    const_template_value_from_projection, project_const_template_value,
};
use super::super::template_helpers::{
    FinalizedTemplateValue, TemplateValueFinalizationInputs, finalize_template_value,
};
use super::*;
use crate::compiler_frontend::ast::const_values::store::{
    ConstTemplateValue, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::SlotOccurrenceId;
use crate::compiler_frontend::ast::templates::tir::TirExpressionOverlayId;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::{
    MalformedTirStore, TemplateIr, TemplateIrBranch, TemplateIrBuilder, TemplateIrNode,
    TemplateIrNodeKind, TemplateIrStore, TemplateIrSummary, TemplateLoopHeaderExpressionSites,
    TemplateSlotPlan, TemplateTirPhase, TemplateTirReference, TemplateWrapperReference,
    TemplateWrapperSet, TirView,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparationMode, TemplatePreparationOutcome, prepare_tir_view,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionOverlay,
    TirWrapperContext, TirWrapperContextOverlay,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::ast_counters::{
    reset_ast_counters, test_read_ast_counter,
};
use std::cell::RefCell;
use std::rc::Rc;

fn finalized_folded(value: FinalizedTemplateValue) -> StringId {
    // This helper owns text-only assertions; provenance is covered by the focused fold tests.
    let FinalizedTemplateValue::Folded(value, _) = value else {
        panic!("test expected a folded finalization value");
    };
    value
}

/// Constructs a `Template` directly from a real module-local TIR reference.
fn template_with_reference(reference: TemplateTirReference, location: SourceLocation) -> Template {
    Template {
        tir_reference: reference,
        location,
    }
}

/// Builds a `Template` carrying a registered TIR root with a single text node,
/// matching the production shape parser-created const text templates carry
/// before finalization normalizes their enclosing payload.
fn registered_text_template(
    text: crate::compiler_frontend::symbols::string_interning::StringId,
    context: TemplateViewContext,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &StringTable,
) -> Template {
    let byte_len = string_table.resolve(text).len();
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(
            text,
            byte_len,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![text_node], SourceLocation::default());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };
    template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    )
}

/// Builds a nested wrapper-context graph whose wrapper references carry their
/// own exact overlay views. The unsafe variant places a runtime slot plan only
/// on the nested wrapper reached through the outer wrapper's overlay.
fn nested_wrapper_finalization_fixture(
    string_table: &mut StringTable,
    unsafe_nested_wrapper: bool,
) -> (Template, Rc<RefCell<TemplateIrStore>>) {
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();

    let (
        parent_template_id,
        parent_occurrence_id,
        outer_wrapper_set_id,
        nested_occurrence_id,
        outer_expression_site_id,
        inner_wrapper_set_id,
    ) = {
        let mut store = template_ir_store.borrow_mut();

        let child_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let text = string_table.intern("parent");
            let text_node = builder.push_text_node(
                text,
                "parent".len(),
                TemplateSegmentOrigin::Body,
                SourceLocation::default(),
            );
            let root = builder.push_sequence_node(vec![text_node], SourceLocation::default());
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            )
        };

        let nested_child_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let text = string_table.intern("nested");
            let text_node = builder.push_text_node(
                text,
                "nested".len(),
                TemplateSegmentOrigin::Body,
                SourceLocation::default(),
            );
            let root = builder.push_sequence_node(vec![text_node], SourceLocation::default());
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            )
        };

        let inner_wrapper_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let before = string_table.intern("inner-before");
            let after = string_table.intern("inner-after");
            let before_node = builder.push_text_node(
                before,
                "inner-before".len(),
                TemplateSegmentOrigin::Body,
                SourceLocation::default(),
            );
            let slot_node = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
            let after_node = builder.push_text_node(
                after,
                "inner-after".len(),
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
                TemplateIrSummary::default(),
                SourceLocation::default(),
            )
        };

        if unsafe_nested_wrapper {
            let runtime_slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
                location: SourceLocation::default(),
                contribution_sources: Vec::new(),
                slot_sites: Vec::new(),
            });
            store
                .attach_runtime_slot_plan(inner_wrapper_template_id, runtime_slot_plan_id)
                .expect("inner wrapper should accept the committed slot plan");
        }

        let inner_wrapper_reference = TemplateWrapperReference::new(
            inner_wrapper_template_id,
            TemplateTirPhase::Finalized,
            empty_context,
        );
        let inner_wrapper_set_id = store.push_wrapper_set(TemplateWrapperSet {
            wrappers: vec![inner_wrapper_reference],
        });

        let (outer_wrapper_template_id, nested_child_node, outer_dynamic_node) = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let outer_dynamic_text = string_table.intern("outer-structural");
            let outer_dynamic_node = builder.push_dynamic_expression_node(
                Expression::string_slice(
                    outer_dynamic_text,
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                ),
                TemplateSegmentOrigin::Body,
                None,
                SourceLocation::default(),
            );
            let nested_child_node = builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    nested_child_template_id,
                    TemplateTirPhase::Composed,
                    empty_context,
                ),
                SourceLocation::default(),
            );
            let slot_node = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
            let after = string_table.intern("outer-after");
            let after_node = builder.push_text_node(
                after,
                "outer-after".len(),
                TemplateSegmentOrigin::Body,
                SourceLocation::default(),
            );
            let root = builder.push_sequence_node(
                vec![outer_dynamic_node, nested_child_node, slot_node, after_node],
                SourceLocation::default(),
            );
            let template_id = builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            );
            (template_id, nested_child_node, outer_dynamic_node)
        };
        let nested_occurrence_id = match &store
            .get_node(nested_child_node)
            .expect("nested child node should exist")
            .kind
        {
            TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
            _ => panic!("expected nested child-template node"),
        };
        let outer_expression_site_id = match &store
            .get_node(outer_dynamic_node)
            .expect("outer dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            _ => panic!("expected outer dynamic-expression node"),
        };
        let outer_wrapper_reference = TemplateWrapperReference::new(
            outer_wrapper_template_id,
            TemplateTirPhase::Finalized,
            empty_context,
        );
        let outer_wrapper_set_id = store.push_wrapper_set(TemplateWrapperSet {
            wrappers: vec![outer_wrapper_reference],
        });

        let parent_child_node = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    child_template_id,
                    TemplateTirPhase::Composed,
                    empty_context,
                ),
                SourceLocation::default(),
            )
        };
        let parent_occurrence_id = match &store
            .get_node(parent_child_node)
            .expect("parent child node should exist")
            .kind
        {
            TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
            _ => panic!("expected parent child-template node"),
        };
        let parent_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let root =
                builder.push_sequence_node(vec![parent_child_node], SourceLocation::default());
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            )
        };

        (
            parent_template_id,
            parent_occurrence_id,
            outer_wrapper_set_id,
            nested_occurrence_id,
            outer_expression_site_id,
            inner_wrapper_set_id,
        )
    };

    let nested_context_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                nested_occurrence_id,
                TirWrapperContext {
                    inherited_wrapper_set: Some(inner_wrapper_set_id),
                    ..TirWrapperContext::default()
                },
            )],
        })
        .expect("test overlay allocation");
    let outer_expression_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                outer_expression_site_id,
                Box::new(Expression::string_slice(
                    string_table.intern("outer-overlay"),
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                )),
            )],
        })
        .expect("test overlay allocation");
    let outer_context = TemplateViewContext {
        expression_overlay: Some(outer_expression_overlay_id),
        slot_resolution: None,
        wrapper_context: Some(nested_context_overlay_id),
    };
    MalformedTirStore::new(&mut template_ir_store.borrow_mut()).set_wrapper_reference_context(
        outer_wrapper_set_id,
        0,
        outer_context,
    );

    let parent_context_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                parent_occurrence_id,
                TirWrapperContext {
                    inherited_wrapper_set: Some(outer_wrapper_set_id),
                    ..TirWrapperContext::default()
                },
            )],
        })
        .expect("test overlay allocation");
    let parent_context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(parent_context_overlay_id),
    };
    let template = template_with_reference(
        TemplateTirReference {
            root: parent_template_id,
            phase: TemplateTirPhase::Finalized,
            context: parent_context,
        },
        SourceLocation::default(),
    );

    (template, template_ir_store)
}

fn location_at(line: i32, column: i32) -> SourceLocation {
    use crate::compiler_frontend::compiler_messages::source_location::CharPosition;

    SourceLocation::new(
        InternedPath::default(),
        CharPosition {
            line_number: line,
            char_column: column,
        },
        CharPosition {
            line_number: line,
            char_column: column,
        },
    )
}

fn assert_expression_site_location(
    view: &TirView<'_>,
    site_id: ExpressionSiteId,
    line: i32,
    column: i32,
) {
    let location = view
        .source_location_for_expression_site(site_id)
        .expect("source-location lookup should succeed")
        .expect("source location should be present");

    assert_eq!(location.start_pos.line_number, line);
    assert_eq!(location.start_pos.char_column, column);
}

#[test]
fn finalization_fold_composed_tir_root_folds_view_text() {
    let mut string_table = StringTable::new();
    let view_text = string_table.intern("store-backed view");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template = registered_text_template(view_text, context, &template_ir_store, &string_table);

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("composed TIR root fold should succeed"),
    );

    assert_eq!(
        folded, view_text,
        "finalization should fold the composed TIR view text"
    );
}

#[test]
fn finalization_normalizes_dynamic_expression_payloads_into_expression_overlay() {
    let mut string_table = StringTable::new();
    let normalized_text = string_table.intern("normalized dynamic payload");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let dynamic_expression = Expression::template(
        registered_text_template(normalized_text, context, &template_ir_store, &string_table),
        ValueMode::ImmutableOwned,
    );
    let expression_location = location_at(31, 7);
    let (template_id, dynamic_node_id, site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let (template_id, dynamic_node_id) = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let dynamic_node_id = builder.push_dynamic_expression_node(
                dynamic_expression,
                TemplateSegmentOrigin::Body,
                None,
                expression_location.clone(),
            );
            let template_id = builder.finish_template(
                dynamic_node_id,
                Style::default(),
                TemplateType::StringFunction,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            );
            (template_id, dynamic_node_id)
        };

        let site_id = match &store
            .get_node(dynamic_node_id)
            .expect("dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            other => panic!("expected dynamic expression node, got {other:?}"),
        };

        (template_id, dynamic_node_id, site_id)
    };

    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("template normalization should install the dynamic expression overlay");

    let reference = &template.tir_reference;
    assert_ne!(
        reference.context,
        TemplateViewContext::default(),
        "normalization should update the template reference with an expression overlay"
    );
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Finalized,
        "normalization should advance the effective reference to the finalized phase"
    );

    let store = template_ir_store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Finalized,
        reference.context,
    )
    .expect("updated template reference should build a finalized TirView");

    let expression_by_site = view
        .effective_expression_for_site(site_id)
        .expect("site lookup should be valid")
        .expect("normalized dynamic expression should be visible by site");
    assert!(
        matches!(expression_by_site.kind, ExpressionKind::StringSlice(text) if text == normalized_text)
    );
    assert_expression_site_location(&view, site_id, 31, 7);

    let expression_by_node = view
        .effective_expression_for_node(dynamic_node_id)
        .expect("node lookup should be valid")
        .expect("normalized dynamic expression should be visible by node");
    assert!(
        matches!(expression_by_node.kind, ExpressionKind::StringSlice(text) if text == normalized_text)
    );

    let structural_expression_is_unchanged = {
        let store = template_ir_store.borrow();
        let node = store
            .get_node(dynamic_node_id)
            .expect("dynamic node should remain in the structural store");
        matches!(
            &node.kind,
            TemplateIrNodeKind::DynamicExpression { expression, .. }
                if matches!(expression.kind, ExpressionKind::Template(_))
        )
    };
    assert!(
        structural_expression_is_unchanged,
        "Phase 10 dynamic-expression normalization should layer the normalized payload through an overlay"
    );
}

#[test]
fn finalization_merges_expression_overrides_without_duplicate_sites() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let dynamic_node = builder.push_dynamic_expression_node(
            Expression::int(1, SourceLocation::default(), ValueMode::ImmutableOwned),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        builder.finish_template(
            dynamic_node,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };
    let site_id = {
        let store = template_ir_store.borrow();
        match &store
            .get_node(store.get_template(template_id).expect("template").root)
            .expect("dynamic node")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            other => panic!("expected dynamic expression node, got {other:?}"),
        }
    };
    let existing_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                site_id,
                Box::new(Expression::int(
                    2,
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                )),
            )],
        })
        .expect("test overlay allocation");
    let initial_context = TemplateViewContext {
        expression_overlay: Some(existing_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: initial_context,
        },
        SourceLocation::default(),
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("normalization should canonicalize the root expression overlay");

    let store = template_ir_store.borrow();
    let overlay_id = template
        .tir_reference
        .context
        .expression_overlay
        .expect("normalization should retain an expression overlay");
    let overlay = store
        .expression_overlay(overlay_id)
        .expect("normalized expression overlay should exist");
    assert_eq!(overlay.overrides.len(), 1);
    assert_eq!(overlay.overrides[0].0, site_id);
    assert!(matches!(
        overlay.overrides[0].1.kind,
        ExpressionKind::Int(2)
    ));
}

#[test]
fn finalization_does_not_mark_parsed_expression_overlay_reference_finalized() {
    let mut string_table = StringTable::new();
    let normalized_text = string_table.intern("normalized parsed dynamic payload");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let dynamic_expression = Expression::template(
        registered_text_template(normalized_text, context, &template_ir_store, &string_table),
        ValueMode::ImmutableOwned,
    );
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let dynamic_node_id = builder.push_dynamic_expression_node(
            dynamic_expression,
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        builder.finish_template(
            dynamic_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Parsed,
            context,
        },
        SourceLocation::default(),
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("template normalization should preserve parsed reference identity");

    let reference = &template.tir_reference;
    assert_ne!(
        reference.context,
        TemplateViewContext::default(),
        "parsed references may receive expression overlays without becoming finalized views"
    );
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Parsed,
        "parsed references are not stable finalization views and must keep their parsed phase"
    );
}

#[test]
fn finalization_uses_durable_phase_for_pre_finalized_descendant_overlay_collection() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let (root_template_id, root_context, child_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let child_dynamic_node = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_dynamic_expression_node(
                Expression::int(1, SourceLocation::default(), ValueMode::ImmutableOwned),
                TemplateSegmentOrigin::Body,
                None,
                SourceLocation::default(),
            )
        };
        let child_site_id = match &store
            .get_node(child_dynamic_node)
            .expect("child dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            other => panic!("expected child dynamic expression, got {other:?}"),
        };
        let child_template_id = store.push_template(TemplateIr::new(
            child_dynamic_node,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        ));
        let child_expression_overlay = store
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(
                    child_site_id,
                    Box::new(Expression::int(
                        2,
                        SourceLocation::default(),
                        ValueMode::ImmutableOwned,
                    )),
                )],
            })
            .expect("child expression overlay should allocate");

        let child_node = {
            let occurrence_id = store.next_child_template_occurrence_id();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference: TemplateTirChildReference::new(
                        child_template_id,
                        TemplateTirPhase::Composed,
                        TemplateViewContext {
                            expression_overlay: Some(child_expression_overlay),
                            slot_resolution: None,
                            wrapper_context: None,
                        },
                    ),
                    occurrence_id,
                },
                SourceLocation::default(),
            ))
        };
        let root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![child_node],
            },
            SourceLocation::default(),
        ));
        let root_template_id = store.push_template(TemplateIr::new(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        ));

        (
            root_template_id,
            TemplateViewContext::default(),
            child_site_id,
        )
    };
    let mut template = template_with_reference(
        TemplateTirReference {
            root: root_template_id,
            phase: TemplateTirPhase::Parsed,
            context: root_context,
        },
        SourceLocation::default(),
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("pre-finalized descendant overlays should normalize");

    assert_eq!(template.tir_reference.phase, TemplateTirPhase::Parsed);
    let store = template_ir_store.borrow();
    let view = TirView::new(
        &store,
        template.tir_reference.root,
        template.tir_reference.phase,
        template.tir_reference.context,
    )
    .expect("the parsed reference should retain its exact durable identity");
    let effective_expression = view
        .effective_expression_for_site(child_site_id)
        .expect("child expression site should remain valid")
        .expect("normalization should publish the descendant overlay");
    assert!(matches!(effective_expression.kind, ExpressionKind::Int(2)));
}

#[test]
fn finalization_normalizes_branch_selector_payloads_into_expression_overlay() {
    let mut string_table = StringTable::new();
    let normalized_text = string_table.intern("normalized branch selector payload");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let selector_expression = Expression::template(
        registered_text_template(normalized_text, context, &template_ir_store, &string_table),
        ValueMode::ImmutableOwned,
    );
    let selector_location = location_at(41, 9);
    let (template_id, branch_chain_node_id, selector_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let branch_body = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children: vec![] },
            SourceLocation::default(),
        ));
        let selector_site_id = store.next_expression_site_id();
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(selector_expression),
            branch_body,
            selector_location.clone(),
            selector_site_id,
        );
        let branch_chain_node_id = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::BranchChain {
                branches: vec![branch],
                fallback: None,
            },
            SourceLocation::default(),
        ));
        let template_id = store.push_template(TemplateIr::new(
            branch_chain_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        ));

        (template_id, branch_chain_node_id, selector_site_id)
    };

    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("template normalization should install the branch selector overlay");

    let reference = &template.tir_reference;
    assert_ne!(
        reference.context,
        TemplateViewContext::default(),
        "normalization should update the template reference with an expression overlay"
    );
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Finalized,
        "normalization should advance the effective reference to the finalized phase"
    );

    let store = template_ir_store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Finalized,
        reference.context,
    )
    .expect("updated template reference should build a finalized TirView");

    let expression_by_site = view
        .effective_expression_for_site(selector_site_id)
        .expect("site lookup should be valid")
        .expect("normalized branch selector should be visible by site");
    assert!(
        matches!(expression_by_site.kind, ExpressionKind::StringSlice(text) if text == normalized_text)
    );
    assert_expression_site_location(&view, selector_site_id, 41, 9);

    let structural_selector_is_unchanged = {
        let store = template_ir_store.borrow();
        let node = store
            .get_node(branch_chain_node_id)
            .expect("branch chain node should remain in the structural store");
        matches!(
            &node.kind,
            TemplateIrNodeKind::BranchChain { branches, .. }
                if matches!(branches[0].condition_expression().kind, ExpressionKind::Template(_))
        )
    };
    assert!(
        structural_selector_is_unchanged,
        "Phase 10 branch-selector normalization should layer the normalized payload through an overlay"
    );
}

#[test]
fn finalization_normalizes_loop_header_payloads_into_expression_overlay() {
    let mut string_table = StringTable::new();
    let normalized_text = string_table.intern("normalized loop header payload");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let header_expression = Expression::template(
        registered_text_template(normalized_text, context, &template_ir_store, &string_table),
        ValueMode::ImmutableOwned,
    );
    let loop_location = location_at(51, 11);
    let (template_id, loop_node_id, condition_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let loop_body = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children: vec![] },
            SourceLocation::default(),
        ));
        let header = TemplateLoopHeader::Conditional {
            condition: Box::new(header_expression),
        };
        let header_sites = store.allocate_loop_header_expression_sites(&header);
        let condition_site_id = match header_sites {
            TemplateLoopHeaderExpressionSites::Conditional { condition } => condition,
            _ => panic!("expected conditional loop header sites"),
        };
        let loop_node_id = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Loop {
                header,
                header_sites,
                body: loop_body,
                aggregate_wrapper: None,
            },
            loop_location.clone(),
        ));
        let template_id = store.push_template(TemplateIr::new(
            loop_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        ));

        (template_id, loop_node_id, condition_site_id)
    };

    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("template normalization should install the loop header overlay");

    let reference = &template.tir_reference;
    assert_ne!(
        reference.context,
        TemplateViewContext::default(),
        "normalization should update the template reference with an expression overlay"
    );
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Finalized,
        "normalization should advance the effective reference to the finalized phase"
    );

    let store = template_ir_store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Finalized,
        reference.context,
    )
    .expect("updated template reference should build a finalized TirView");

    let expression_by_site = view
        .effective_expression_for_site(condition_site_id)
        .expect("site lookup should be valid")
        .expect("normalized loop header expression should be visible by site");
    assert!(
        matches!(expression_by_site.kind, ExpressionKind::StringSlice(text) if text == normalized_text)
    );
    assert_expression_site_location(&view, condition_site_id, 51, 11);

    let structural_header_is_unchanged = {
        let store = template_ir_store.borrow();
        let node = store
            .get_node(loop_node_id)
            .expect("loop node should remain in the structural store");
        matches!(
            &node.kind,
            TemplateIrNodeKind::Loop {
                header: TemplateLoopHeader::Conditional { condition },
                ..
            } if matches!(condition.kind, ExpressionKind::Template(_))
        )
    };
    assert!(
        structural_header_is_unchanged,
        "Phase 10 loop-header normalization should layer the normalized payload through an overlay"
    );
}

#[test]
fn finalization_fold_uses_finalized_expression_overlay_view() {
    #[cfg(feature = "benchmark_counters")]
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    let mut string_table = StringTable::new();
    let structural_text = string_table.intern("structural dynamic payload");
    let overlay_text = string_table.intern("finalized expression overlay");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();

    let (template_id, dynamic_node) = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let dynamic_node = builder.push_dynamic_expression_node(
            Expression::string_slice(
                structural_text,
                SourceLocation::default(),
                ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![dynamic_node], SourceLocation::default());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );

        (template_id, dynamic_node)
    };

    let site_id = {
        let store = template_ir_store.borrow();
        match &store
            .get_node(dynamic_node)
            .expect("dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            _ => panic!("expected dynamic expression node"),
        }
    };

    let expression_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                site_id,
                Box::new(Expression::string_slice(
                    overlay_text,
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                )),
            )],
        })
        .expect("test overlay allocation");
    let expression_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    let context = empty_context.merge(expression_context);

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Finalized,
            context,
        },
        SourceLocation::default(),
    );

    #[cfg(feature = "benchmark_counters")]
    reset_ast_counters();

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("expression-overlay view fold should succeed"),
    );

    assert_eq!(
        folded, overlay_text,
        "finalized expression overlays must fold from the same effective TirView instead of the structural payload"
    );
}

#[test]
fn finalization_classifies_root_expression_overlay_through_nested_children() {
    let mut string_table = StringTable::new();
    let dynamic_text = string_table.intern("root-overlay-dynamic");
    let branch_text = string_table.intern("root-overlay-branch");
    let loop_text = string_table.intern("root-overlay-loop");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();

    let (root_template_id, dynamic_site_id, selector_site_id, loop_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let (leaf_template_id, dynamic_node, branch_node, loop_node) = {
            let mut builder = TemplateIrBuilder::new(&mut store);

            let dynamic_node = builder.push_dynamic_expression_node(
                Expression::reference_with_type_id(
                    InternedPath::from_single_str("nested_dynamic", &mut string_table),
                    DataType::StringSlice,
                    builtin_type_ids::STRING,
                    SourceLocation::default(),
                    ValueMode::ImmutableReference,
                    ConstRecordState::RuntimeValue,
                ),
                TemplateSegmentOrigin::Body,
                None,
                SourceLocation::default(),
            );
            let branch_text_node = builder.push_text_node(
                branch_text,
                "root-overlay-branch".len(),
                TemplateSegmentOrigin::Body,
                SourceLocation::default(),
            );
            let nested_selector_site = builder.store.next_expression_site_id();
            let branch_node = builder.push_branch_chain_node(
                vec![TemplateIrBranch::new(
                    TemplateBranchSelector::Bool(Expression::reference_with_type_id(
                        InternedPath::from_single_str("nested_selector", &mut string_table),
                        DataType::Bool,
                        builtin_type_ids::BOOL,
                        SourceLocation::default(),
                        ValueMode::ImmutableReference,
                        ConstRecordState::RuntimeValue,
                    )),
                    branch_text_node,
                    SourceLocation::default(),
                    nested_selector_site,
                )],
                None,
                SourceLocation::default(),
            );
            let loop_text_node = builder.push_text_node(
                loop_text,
                "root-overlay-loop".len(),
                TemplateSegmentOrigin::Body,
                SourceLocation::default(),
            );
            let loop_node = builder.push_loop_node(
                TemplateLoopHeader::Conditional {
                    condition: Box::new(Expression::reference_with_type_id(
                        InternedPath::from_single_str("nested_loop", &mut string_table),
                        DataType::Bool,
                        builtin_type_ids::BOOL,
                        SourceLocation::default(),
                        ValueMode::ImmutableReference,
                        ConstRecordState::RuntimeValue,
                    )),
                },
                loop_text_node,
                None,
                SourceLocation::default(),
            );
            let leaf_root = builder.push_sequence_node(
                vec![dynamic_node, branch_node, loop_node],
                SourceLocation::default(),
            );
            let leaf_template_id = builder.finish_template(
                leaf_root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            );
            (leaf_template_id, dynamic_node, branch_node, loop_node)
        };

        let dynamic_site_id = match &store
            .get_node(dynamic_node)
            .expect("dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            _ => panic!("expected a dynamic-expression node"),
        };
        let (selector_site_id, loop_site_id) = match &store
            .get_node(branch_node)
            .expect("branch node should exist")
            .kind
        {
            TemplateIrNodeKind::BranchChain { branches, .. } => {
                let selector_site_id = branches[0].selector_site_id;
                let loop_site_id = match &store
                    .get_node(loop_node)
                    .expect("loop node should exist")
                    .kind
                {
                    TemplateIrNodeKind::Loop {
                        header_sites: TemplateLoopHeaderExpressionSites::Conditional { condition },
                        ..
                    } => *condition,
                    _ => panic!("expected a conditional loop node"),
                };
                (selector_site_id, loop_site_id)
            }
            _ => panic!("expected a branch-chain node"),
        };

        let mut builder = TemplateIrBuilder::new(&mut store);
        let mut descendant_template_id = leaf_template_id;
        for _ in 0..3 {
            let child_reference = TemplateTirChildReference::new(
                descendant_template_id,
                TemplateTirPhase::Composed,
                empty_context,
            );
            let child_node = builder.push_child_template_node_with_reference(
                child_reference,
                SourceLocation::default(),
            );
            let root = builder.push_sequence_node(vec![child_node], SourceLocation::default());
            descendant_template_id = builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            );
        }

        (
            descendant_template_id,
            dynamic_site_id,
            selector_site_id,
            loop_site_id,
        )
    };

    let expression_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (
                    dynamic_site_id,
                    Box::new(Expression::string_slice(
                        dynamic_text,
                        SourceLocation::default(),
                        ValueMode::ImmutableOwned,
                    )),
                ),
                (
                    selector_site_id,
                    Box::new(Expression::bool(
                        true,
                        SourceLocation::default(),
                        ValueMode::ImmutableOwned,
                    )),
                ),
                (
                    loop_site_id,
                    Box::new(Expression::bool(
                        false,
                        SourceLocation::default(),
                        ValueMode::ImmutableOwned,
                    )),
                ),
            ],
        })
        .expect("test overlay allocation");
    let root_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: root_template_id,
            phase: TemplateTirPhase::Finalized,
            context: root_context,
        },
        SourceLocation::default(),
    );

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("root overlay should classify and fold through nested descendants"),
    );

    assert_eq!(
        string_table.resolve(folded),
        "root-overlay-dynamicroot-overlay-branch",
        "dynamic, branch-selector, and loop-header overlays must all reach the nested leaf"
    );
}

#[test]
fn finalization_ignores_parsed_child_overlay_before_later_composed_descendant() {
    let mut string_table = StringTable::new();
    let structural_text = string_table.intern("structural");
    let override_text = string_table.intern("root-override");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();
    let missing_context = TemplateViewContext {
        expression_overlay: Some(TirExpressionOverlayId::new(999)),
        ..TemplateViewContext::default()
    };

    let (root_template_id, descendant_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let (descendant_template_id, descendant_site_id) = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let dynamic_node = builder.push_dynamic_expression_node(
                Expression::string_slice(
                    structural_text,
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                ),
                TemplateSegmentOrigin::Body,
                None,
                SourceLocation::default(),
            );
            let root = builder.push_sequence_node(vec![dynamic_node], SourceLocation::default());
            let template_id = builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            );
            let site_id = match &store
                .get_node(dynamic_node)
                .expect("descendant dynamic node should exist")
                .kind
            {
                TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
                _ => panic!("expected descendant dynamic-expression node"),
            };
            (template_id, site_id)
        };

        let parsed_child_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let child_node = builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    descendant_template_id,
                    TemplateTirPhase::Composed,
                    empty_context,
                ),
                SourceLocation::default(),
            );
            let root = builder.push_sequence_node(vec![child_node], SourceLocation::default());
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            )
        };

        let mut builder = TemplateIrBuilder::new(&mut store);
        let child_node = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                parsed_child_template_id,
                TemplateTirPhase::Parsed,
                missing_context,
            ),
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![child_node], SourceLocation::default());
        let root_template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );

        (root_template_id, descendant_site_id)
    };

    let expression_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                descendant_site_id,
                Box::new(Expression::string_slice(
                    override_text,
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                )),
            )],
        })
        .expect("test overlay allocation");
    let root_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    let template = template_with_reference(
        TemplateTirReference {
            root: root_template_id,
            phase: TemplateTirPhase::Finalized,
            context: root_context,
        },
        SourceLocation::default(),
    );

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("a Parsed child must not consume its missing overlay during finalization"),
    );

    assert_eq!(
        folded, override_text,
        "the finalized root expression overlay must reach the later composed descendant"
    );
}

#[test]
fn finalization_rejects_nested_runtime_wrapper_in_exact_wrapper_overlay() {
    let mut string_table = StringTable::new();
    let (template, template_ir_store) =
        nested_wrapper_finalization_fixture(&mut string_table, true);

    let result = finalize_template_value(
        &template,
        TemplateValueFinalizationInputs {
            string_table: &mut string_table,
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            template_ir_store: &template_ir_store,
        },
        TemplatePreparationMode::Value,
    )
    .expect("runtime nested wrapper should be a valid non-foldable shape");

    assert!(
        matches!(result, FinalizedTemplateValue::Runtime(_)),
        "the production safety gate must not fold through a runtime nested wrapper hidden in the exact wrapper overlay"
    );
}

#[test]
fn finalization_keeps_valid_runtime_slot_plan_out_of_folded_string() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("runtime root");
    let template = registered_text_template(text, context, &template_ir_store, &string_table);
    let template_id = template.tir_reference.root;

    {
        let mut store = template_ir_store.borrow_mut();
        let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
            location: SourceLocation::default(),
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
        });
        store
            .attach_runtime_slot_plan(template_id, slot_plan_id)
            .expect("template should accept the committed slot plan");
    }

    let result = finalize_template_value(
        &template,
        TemplateValueFinalizationInputs {
            string_table: &mut string_table,
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            template_ir_store: &template_ir_store,
        },
        TemplatePreparationMode::Value,
    )
    .expect("valid runtime slot plan should use the handoff path");

    assert!(
        matches!(result, FinalizedTemplateValue::Runtime(_)),
        "a valid runtime slot plan must not become a folded empty string"
    );
    assert!(
        template_ir_store
            .borrow()
            .get_template(template_id)
            .expect("template should remain in the store")
            .runtime_slot_plan
            .is_some(),
        "the runtime slot plan must remain available for owned handoff"
    );
}

#[test]
fn finalization_replaces_renderable_runtime_slot_plan_with_owned_handoff() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("runtime handoff");
    let template = registered_text_template(text, context, &template_ir_store, &string_table);
    let template_id = template.tir_reference.root;

    {
        let mut store = template_ir_store.borrow_mut();
        let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
            location: SourceLocation::default(),
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
        });
        store
            .attach_runtime_slot_plan(template_id, slot_plan_id)
            .expect("template should accept the committed slot plan");
    }

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("renderable runtime slot plans should use the owned handoff path");

    let ExpressionKind::RuntimeSlotApplicationHandoff(handoff) = expression.kind else {
        panic!("expected renderable runtime slot plan to become an owned slot handoff");
    };
    assert!(
        handoff.slot_sites.is_empty(),
        "the owned handoff must retain the valid empty slot plan"
    );
    assert!(
        template_ir_store
            .borrow()
            .get_template(template_id)
            .expect("template should remain in the store")
            .runtime_slot_plan
            .is_some(),
        "normalization must retain the source runtime slot plan"
    );
}

#[test]
fn runtime_handoff_shape_uses_root_slot_plan_not_preparation_reason() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("runtime slot root");
    let mut template = registered_text_template(text, context, &template_ir_store, &string_table);
    template.tir_reference.phase = TemplateTirPhase::Finalized;
    let template_id = template.tir_reference.root;

    {
        let mut store = template_ir_store.borrow_mut();
        let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
            location: SourceLocation::default(),
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
        });
        store
            .attach_runtime_slot_plan(template_id, slot_plan_id)
            .expect("template should accept the committed slot plan");
    }

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };
    let store = template_ir_store.borrow();
    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )
    .expect("finalized runtime-slot view should construct");
    let prepared = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("runtime-slot preparation should succeed");
    assert!(matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Runtime(_)
    ));

    let normalized = super::materialize_runtime_template_handoff_for_hir(
        &template,
        &mut context,
        &prepared,
        None,
    )
    .expect("prepared runtime handoff should materialize")
    .expect("runtime template should produce a normalized handoff");

    assert!(
        matches!(
            normalized,
            NormalizedTemplateExpression::RuntimeSlotApplication(..)
        ),
        "the actual root slot plan must select the specialized runtime-slot handoff shape"
    );
}

#[test]
fn module_constant_normalization_rejects_runtime_slot_plan_with_structured_diagnostic() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("module constant runtime plan");
    let template = registered_text_template(text, context, &template_ir_store, &string_table);
    let template_id = template.tir_reference.root;

    {
        let mut store = template_ir_store.borrow_mut();
        let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
            location: SourceLocation::default(),
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
        });
        store
            .attach_runtime_slot_plan(template_id, slot_plan_id)
            .expect("template should accept the committed slot plan");
    }

    let expression = Expression::template(template, ValueMode::ImmutableOwned);
    let ExpressionKind::Template(template) = &expression.kind else {
        panic!("module constant regression must start from a template expression");
    };
    let projected = project_const_template_value(
        template,
        &template_ir_store.borrow(),
        &mut string_table,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
    )
    .expect("a runtime slot plan must classify rather than fail preparation");
    let Err(result) = const_template_value_from_projection(projected, template) else {
        panic!("runtime-plan module constants must be rejected structurally");
    };

    let ConstValueStoreError::Diagnostic(diagnostic) = result else {
        panic!(
            "runtime-plan module constants must not report the old internal fold transformation error"
        );
    };
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::NonFoldableConstTemplate,
        }
    ));
    assert_eq!(
        diagnostic.primary_location, expression.location,
        "the established const diagnostic must retain the template source location"
    );
}

#[test]
fn finalization_accepts_supported_nested_wrapper_exact_view() {
    let mut string_table = StringTable::new();
    let (template, template_ir_store) =
        nested_wrapper_finalization_fixture(&mut string_table, false);

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("supported nested wrapper should fold through the exact views"),
    );

    assert_eq!(
        string_table.resolve(folded),
        "outer-structuralinner-beforenestedinner-afterparentouter-after",
        "supported exact-view wrapper traversal must preserve structural expression authority and wrapper order"
    );
}

#[test]
fn finalization_fold_uses_resolved_slot_view_context() {
    let mut string_table = StringTable::new();
    let before_text = string_table.intern("before");
    let after_text = string_table.intern("after");
    let fill_text = string_table.intern("filled");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let reference = {
        let mut store = template_ir_store.borrow_mut();
        let mut fill_builder = TemplateIrBuilder::new(&mut store);
        let fill_node = fill_builder.push_text_node(
            fill_text,
            "filled".len(),
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let fill_root = fill_builder.push_sequence_node(vec![fill_node], SourceLocation::default());
        let fill_template_id = fill_builder.finish_template(
            fill_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );

        let mut wrapper_builder = TemplateIrBuilder::new(&mut store);
        let before_node = wrapper_builder.push_text_node(
            before_text,
            "before".len(),
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let slot_node = wrapper_builder.push_slot_node(SlotKey::Default, SourceLocation::default());
        let after_node = wrapper_builder.push_text_node(
            after_text,
            "after".len(),
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let wrapper_root = wrapper_builder.push_sequence_node(
            vec![before_node, slot_node, after_node],
            SourceLocation::default(),
        );
        let wrapper_template_id = wrapper_builder.finish_template(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );

        let slot_occurrence_id = match &store
            .get_node(slot_node)
            .expect("slot node should exist")
            .kind
        {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            _ => panic!("expected slot node"),
        };

        let slot_overlay_id = store
            .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
                resolutions: vec![(
                    slot_occurrence_id,
                    TirSlotResolution::resolved(SlotKey::Default, vec![fill_template_id]),
                )],
            })
            .expect("test overlay allocation");
        let context = TemplateViewContext {
            expression_overlay: None,
            slot_resolution: Some(slot_overlay_id),
            wrapper_context: None,
        };
        assert!(
            context.slot_resolution.is_some(),
            "test must exercise a real slot-resolution overlay"
        );

        TemplateTirReference {
            root: wrapper_template_id,
            phase: TemplateTirPhase::Composed,
            context,
        }
    };

    let template = template_with_reference(reference, SourceLocation::default());

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("resolved slot-overlay fold should succeed"),
    );

    let expected = string_table.intern("beforefilledafter");
    assert_eq!(
        folded, expected,
        "composed slot overlays must fold from the effective TirView"
    );
}

#[test]
fn finalization_fold_composed_root_with_unfilled_slot_emits_no_slot_output() {
    let mut string_table = StringTable::new();
    let text_id = string_table.intern("text before unfilled slot");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // An unfilled slot contributes no output. Finalization folds that rule
    // directly from the composed TIR root.
    let reference = {
        let location = SourceLocation::default();
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(
            text_id,
            "text before unfilled slot".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let slot_node = builder.push_slot_node(SlotKey::Default, location.clone());
        let root = builder.push_sequence_node(vec![text_node, slot_node], location.clone());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location,
        );

        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        }
    };

    let template = template_with_reference(reference, SourceLocation::default());

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("composed slot-root fold should succeed"),
    );

    assert_eq!(
        folded, text_id,
        "the unfilled slot must contribute no output to the composed TIR root"
    );
}

#[test]
fn finalization_fold_formatted_root_with_unfilled_slot_emits_no_slot_output() {
    #[cfg(feature = "benchmark_counters")]
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    let mut string_table = StringTable::new();
    let text_id = string_table.intern("formatted text before unfilled slot");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let reference = {
        let location = SourceLocation::default();
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(
            text_id,
            "formatted text before unfilled slot".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let slot_node = builder.push_slot_node(SlotKey::Default, location.clone());
        let root = builder.push_sequence_node(vec![text_node, slot_node], location.clone());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary {
                slot_count: 1,
                ..TemplateIrSummary::default()
            },
            location,
        );

        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Formatted,
            context,
        }
    };

    let template = template_with_reference(reference, SourceLocation::default());

    #[cfg(feature = "benchmark_counters")]
    reset_ast_counters();

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("formatted slot-root fold should succeed"),
    );

    assert_eq!(
        folded, text_id,
        "the unfilled slot must contribute no output to the formatted TIR root"
    );

    #[cfg(feature = "benchmark_counters")]
    {
        assert_eq!(
            test_read_ast_counter(AstCounter::TirFinalizationFoldAttempts),
            1,
            "slot-bearing formatted roots are now real store fold attempts"
        );
        assert_eq!(
            test_read_ast_counter(AstCounter::TirFinalizationFoldSuccesses),
            1,
            "the store-backed fold completes directly"
        );
    }
}

fn runtime_template_handoff_from_expression(expression: Expression) -> OwnedRuntimeTemplateHandoff {
    let ExpressionKind::RuntimeTemplateHandoff(handoff) = expression.kind else {
        panic!("expected expression normalization to return an owned runtime-template handoff");
    };

    *handoff
}

#[test]
fn branch_tir_root_normalizes_into_owned_runtime_handoff() {
    let mut string_table = StringTable::new();
    let location = SourceLocation::default();
    let branch_text = string_table.intern("branch body");
    let fallback_text = string_table.intern("fallback body");
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let branch_body = builder.push_text_node(
            branch_text,
            "branch body".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let fallback_body = builder.push_text_node(
            fallback_text,
            "fallback body".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::reference_with_type_id(
                InternedPath::from_single_str("show_branch", &mut string_table),
                DataType::Bool,
                builtin_type_ids::BOOL,
                location.clone(),
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )),
            branch_body,
            location.clone(),
            builder.store.next_expression_site_id(),
        );
        let root =
            builder.push_branch_chain_node(vec![branch], Some(fallback_body), location.clone());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            location,
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    );

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("branch TIR root should normalize through the finalized effective view");

    let handoff = runtime_template_handoff_from_expression(expression);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::BranchChain {
        branches,
        fallback,
        ..
    }) = handoff.body
    else {
        panic!("expected a branch-chain runtime handoff");
    };
    assert_eq!(branches.len(), 1);
    assert!(
        fallback.is_some(),
        "the fallback must remain owned by the handoff"
    );
}

#[test]
fn loop_tir_root_normalizes_into_owned_runtime_handoff() {
    let mut string_table = StringTable::new();
    let location = SourceLocation::default();
    let loop_text = string_table.intern("loop body");
    let open_text = string_table.intern("[");
    let close_text = string_table.intern("]");
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let aggregate_output = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::AggregateOutput,
            location.clone(),
        ));
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_text_node(
            loop_text,
            "loop body".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let open =
            builder.push_text_node(open_text, 1, TemplateSegmentOrigin::Body, location.clone());
        let close =
            builder.push_text_node(close_text, 1, TemplateSegmentOrigin::Body, location.clone());
        let aggregate_wrapper =
            builder.push_sequence_node(vec![open, aggregate_output, close], location.clone());
        let header = TemplateLoopHeader::Conditional {
            condition: Box::new(Expression::reference_with_type_id(
                InternedPath::from_single_str("keep_looping", &mut string_table),
                DataType::Bool,
                builtin_type_ids::BOOL,
                location.clone(),
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )),
        };
        let root = builder.push_loop_node(header, body, Some(aggregate_wrapper), location.clone());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            location,
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    );

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("loop TIR root should normalize through the finalized effective view");

    let handoff = runtime_template_handoff_from_expression(expression);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Loop { .. }) = handoff.body
    else {
        panic!("expected a loop runtime handoff");
    };
}

fn collect_owned_handoff_string_slice_expressions(
    handoff: &OwnedRuntimeTemplateHandoff,
    string_slices: &mut Vec<crate::compiler_frontend::symbols::string_interning::StringId>,
) {
    match &handoff.body {
        OwnedRuntimeTemplateBody::Render(root) => {
            collect_owned_node_string_slice_expressions(root, string_slices);
        }

        OwnedRuntimeTemplateBody::RuntimeSlotApplication(slot_handoff) => {
            collect_owned_node_string_slice_expressions(&slot_handoff.wrapper, string_slices);
            for source in &slot_handoff.contribution_sources {
                collect_owned_node_string_slice_expressions(&source.render_root, string_slices);
            }
            for site in &slot_handoff.slot_sites {
                collect_owned_node_string_slice_expressions(&site.render_root, string_slices);
            }
        }
    }
}

fn collect_owned_node_string_slice_expressions(
    node: &OwnedRuntimeTemplateNode,
    string_slices: &mut Vec<crate::compiler_frontend::symbols::string_interning::StringId>,
) {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            if let ExpressionKind::StringSlice(text) = &expression.kind {
                string_slices.push(*text);
            }
        }

        OwnedRuntimeTemplateNode::Sequence { children, .. } => {
            for child in children {
                collect_owned_node_string_slice_expressions(child, string_slices);
            }
        }

        OwnedRuntimeTemplateNode::BranchChain {
            branches, fallback, ..
        } => {
            for branch in branches {
                collect_owned_node_string_slice_expressions(&branch.body, string_slices);
            }
            if let Some(fallback) = fallback {
                collect_owned_node_string_slice_expressions(fallback, string_slices);
            }
        }

        OwnedRuntimeTemplateNode::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            collect_owned_node_string_slice_expressions(body, string_slices);
            if let Some(wrapper) = aggregate_wrapper {
                collect_owned_node_string_slice_expressions(wrapper, string_slices);
            }
        }

        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
            collect_owned_handoff_string_slice_expressions(template, string_slices);
        }

        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            collect_owned_node_string_slice_expressions(child, string_slices);
            collect_owned_node_string_slice_expressions(wrapper, string_slices);
        }

        OwnedRuntimeTemplateNode::Text { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::LoopControl { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => {}
    }
}

/// Builds a `Template` with a registered TIR root containing a text segment and
/// a runtime reference expression, matching the production shape for ordinary
/// runtime templates that are not const-foldable.
///
/// WHAT: the resulting template is not const-foldable because the reference is
///       a runtime value, so it must go through the runtime-template handoff path.
/// WHY: gives the new store-focused test a simple, representative input shape.
fn registered_runtime_template(
    text: crate::compiler_frontend::symbols::string_interning::StringId,
    reference_name: &str,
    context: TemplateViewContext,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Template {
    let byte_len = string_table.resolve(text).len();
    let reference_path = InternedPath::from_single_str(reference_name, string_table);
    let reference_expression = Expression::reference_with_type_id(
        reference_path,
        DataType::StringSlice,
        builtin_type_ids::STRING,
        SourceLocation::default(),
        ValueMode::ImmutableReference,
        ConstRecordState::RuntimeValue,
    );
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(
            text,
            byte_len,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let dynamic_node = builder.push_dynamic_expression_node(
            reference_expression,
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let root =
            builder.push_sequence_node(vec![text_node, dynamic_node], SourceLocation::default());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };
    template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    )
}

#[test]
fn ordinary_runtime_template_handoff_uses_module_tir_store() {
    let mut string_table = StringTable::new();
    let text = string_table.intern("hello ");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template =
        registered_runtime_template(text, "name", context, &template_ir_store, &mut string_table);

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("ordinary runtime template expression normalization should succeed");

    let handoff = runtime_template_handoff_from_expression(expression);
    assert!(
        matches!(handoff.body, OwnedRuntimeTemplateBody::Render(_)),
        "ordinary runtime templates must materialize a render body handoff"
    );
}

#[test]
fn folded_template_preserves_selected_effective_dynamic_provenance() {
    let mut string_table = StringTable::new();
    let unselected_text = string_table.intern("unselected");
    let selected_structural_text = string_table.intern("selected structural");
    let selected_effective_text = string_table.intern("selected effective");
    let location = SourceLocation::default();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let unselected_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "unselected",
    );
    let selected_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "selected",
    );

    let (template_id, selected_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let unselected_node = builder.push_dynamic_expression_node(
            Expression::string_slice(unselected_text, location.clone(), ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
                    unselected_member,
                )),
            TemplateSegmentOrigin::Body,
            None,
            location.clone(),
        );
        let selected_node = builder.push_dynamic_expression_node(
            Expression::string_slice(
                selected_structural_text,
                location.clone(),
                ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            location.clone(),
        );
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                false,
                location.clone(),
                ValueMode::ImmutableOwned,
            )),
            unselected_node,
            location.clone(),
            builder.store.next_expression_site_id(),
        );
        let root = builder.push_branch_chain_node(vec![branch], Some(selected_node), location);
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );
        let selected_site_id = match store
            .get_node(selected_node)
            .expect("selected dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => site_id,
            _ => panic!("expected selected dynamic expression node"),
        };
        (template_id, selected_site_id)
    };

    let overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                selected_site_id,
                Box::new(
                    Expression::string_slice(
                        selected_effective_text,
                        SourceLocation::default(),
                        ValueMode::ImmutableOwned,
                    )
                    .with_synthetic_interface_provenance(
                        SyntheticInterfaceProvenance::single(selected_member.clone()),
                    ),
                ),
            )],
        })
        .expect("test overlay allocation");
    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext {
                expression_overlay: Some(overlay_id),
                ..TemplateViewContext::default()
            },
        },
        SourceLocation::default(),
    );
    let constant_expression = Expression::template(template.clone(), ValueMode::ImmutableOwned);
    assert!(
        constant_expression
            .synthetic_interface_provenance
            .is_empty(),
        "the outer template must start without injected provenance"
    );

    let ExpressionKind::Template(constant_template) = &constant_expression.kind else {
        panic!("module constant regression must start from a template expression");
    };
    let projected_constant = project_const_template_value(
        constant_template,
        &template_ir_store.borrow(),
        &mut string_table,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
    )
    .expect("selected exact TIR fold should project module constants");
    assert_eq!(
        projected_constant.provenance.members(),
        std::slice::from_ref(&selected_member),
        "module constant projection must retain selected folded provenance"
    );

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("selected exact TIR fold should normalize");

    assert!(matches!(
        expression.kind,
        ExpressionKind::StringSlice(value) if value == selected_effective_text
    ));
    assert_eq!(
        expression.synthetic_interface_provenance.members(),
        &[selected_member],
        "only the selected effective dynamic payload may reach the folded value"
    );
}

#[test]
fn runtime_template_expression_normalization_replaces_template_with_owned_handoff() {
    let mut string_table = StringTable::new();
    let text = string_table.intern("hello ");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template =
        registered_runtime_template(text, "name", context, &template_ir_store, &mut string_table);

    // This test covers preservation of metadata already carried by the outer runtime template
    // expression. Nested effective-fold provenance is covered by the folded-template test above.
    let provenance_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "html",
    );
    let mut expression = Expression::template(template, ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
            provenance_member.clone(),
        ));

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("runtime template expression normalization should succeed");

    let ExpressionKind::RuntimeTemplateHandoff(handoff) = &expression.kind else {
        panic!("runtime template expression should be replaced with an owned handoff");
    };
    assert!(
        matches!(handoff.body, OwnedRuntimeTemplateBody::Render(_)),
        "ordinary runtime templates must keep using the render handoff body"
    );
    assert_eq!(expression.diagnostic_type, DataType::Template);
    assert_eq!(expression.value_mode, ValueMode::ImmutableOwned);
    assert_eq!(
        expression.synthetic_interface_provenance.members(),
        &[provenance_member]
    );
    assert!(
        expression
            .reactive_template
            .as_ref()
            .is_some_and(|metadata| metadata.template_backed),
        "runtime handoff expressions must preserve template-backed metadata"
    );
}

#[test]
fn runtime_template_expression_handoff_uses_finalized_expression_overlay_view() {
    let mut string_table = StringTable::new();
    let overlay_text = string_table.intern("normalized overlay text");
    let runtime_path = InternedPath::from_single_str("runtime_name", &mut string_table);

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();
    let nested_template_expression = Expression::template(
        registered_text_template(
            overlay_text,
            empty_context,
            &template_ir_store,
            &string_table,
        ),
        ValueMode::ImmutableOwned,
    );

    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let normalized_dynamic_node = builder.push_dynamic_expression_node(
            nested_template_expression,
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let runtime_dynamic_node = builder.push_dynamic_expression_node(
            Expression::reference_with_type_id(
                runtime_path,
                DataType::StringSlice,
                builtin_type_ids::STRING,
                SourceLocation::default(),
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(
            vec![normalized_dynamic_node, runtime_dynamic_node],
            SourceLocation::default(),
        );
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: empty_context,
        },
        SourceLocation::default(),
    );
    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("runtime template normalization should use the finalized view handoff");

    let ExpressionKind::RuntimeTemplateHandoff(handoff) = &expression.kind else {
        panic!("runtime template expression should be replaced with an owned handoff");
    };

    let mut string_slices = Vec::new();
    collect_owned_handoff_string_slice_expressions(handoff, &mut string_slices);
    assert!(
        string_slices.contains(&overlay_text),
        "runtime handoff must materialize normalized dynamic expressions from the final effective TirView"
    );
    assert!(
        expression.reactive_template.is_some(),
        "runtime handoff replacement should preserve template metadata"
    );
}

/// Proves that a nested runtime template inside a TIR dynamic expression node
/// is normalized through the final effective view.
#[test]
fn nested_runtime_template_normalizes_through_final_view() {
    let mut string_table = StringTable::new();
    let nested_text = string_table.intern("nested runtime text");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // Build a TIR whose sole dynamic expression holds a nested runtime
    // template (text plus a runtime reference, so it is not const-foldable).
    let nested_template = registered_runtime_template(
        nested_text,
        "runtime_ref",
        context,
        &template_ir_store,
        &mut string_table,
    );

    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);

        let dynamic_node = builder.push_dynamic_expression_node(
            Expression::template(nested_template, ValueMode::ImmutableOwned),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![dynamic_node], SourceLocation::default());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    );
    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("nested runtime template normalization should succeed through the final TIR view");

    let ExpressionKind::RuntimeTemplateHandoff(handoff) = &expression.kind else {
        panic!("outer template expression should be replaced with an owned handoff");
    };

    // The handoff must contain the nested runtime template handoff inside a
    // DynamicExpression node, proving the overlay path normalized it.
    let mut found_nested_handoff = false;
    if let OwnedRuntimeTemplateBody::Render(root) = &handoff.body {
        find_runtime_handoff_in_node(root, &mut found_nested_handoff);
    }
    assert!(
        found_nested_handoff,
        "handoff must contain the nested runtime template handoff materialized from the final TIR view"
    );
}

/// Recursively checks whether any DynamicExpression node in the owned handoff
/// tree carries a RuntimeTemplateHandoff expression kind.
fn find_runtime_handoff_in_node(node: &OwnedRuntimeTemplateNode, found: &mut bool) {
    if *found {
        return;
    }
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            if matches!(expression.kind, ExpressionKind::RuntimeTemplateHandoff(_)) {
                *found = true;
            }
        }
        OwnedRuntimeTemplateNode::Sequence { children, .. } => {
            for child in children {
                find_runtime_handoff_in_node(child, found);
            }
        }
        OwnedRuntimeTemplateNode::BranchChain {
            branches, fallback, ..
        } => {
            for branch in branches {
                find_runtime_handoff_in_node(&branch.body, found);
            }
            if let Some(fallback) = fallback {
                find_runtime_handoff_in_node(fallback, found);
            }
        }
        OwnedRuntimeTemplateNode::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            find_runtime_handoff_in_node(body, found);
            if let Some(wrapper) = aggregate_wrapper {
                find_runtime_handoff_in_node(wrapper, found);
            }
        }
        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            find_runtime_handoff_in_node(child, found);
            find_runtime_handoff_in_node(wrapper, found);
        }
        _ => {}
    }
}

/// Proves that a const child template referenced from the outer TIR view folds
/// correctly through the final view.
#[test]
fn nested_const_template_folds_through_final_view() {
    let mut string_table = StringTable::new();
    let child_text_str = "child folded text";
    let child_text = string_table.intern(child_text_str);
    let child_byte_len = child_text_str.len();

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // Build a child template (const text) and an outer template whose TIR
    // root is a sequence containing a child-template ref to it.
    let outer_template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);

        let child_root = builder.push_text_node(
            child_text,
            child_byte_len,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let child_id = builder.finish_template(
            child_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );

        let child_ref_node = builder.push_child_template_node(child_id, SourceLocation::default());
        let outer_root =
            builder.push_sequence_node(vec![child_ref_node], SourceLocation::default());
        builder.finish_template(
            outer_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: outer_template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    );

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("fold through final view should succeed"),
    );

    assert_eq!(
        folded, child_text,
        "fold must produce the child template's text from the final TIR view"
    );
}

/// Proves that reactive subscriptions stored on TIR dynamic expression nodes
/// are collected into the expression's reactive metadata through the finalized
/// effective view.
#[test]
fn reactive_metadata_derived_from_nested_final_view() {
    let mut string_table = StringTable::new();
    let reactive_path = InternedPath::from_single_str("reactive_source", &mut string_table);

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // Build a TIR with a dynamic expression carrying a reactive subscription.
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);

        let subscription = ReactiveSubscription {
            source: ReactiveSource {
                path: reactive_path.clone(),
                kind: ReactiveSourceKind::Declaration,
            },
            type_id: builtin_type_ids::STRING,
            location: SourceLocation::default(),
        };

        let dynamic_node = builder.push_dynamic_expression_node(
            Expression::reference_with_type_id(
                reactive_path.clone(),
                DataType::StringSlice,
                builtin_type_ids::STRING,
                SourceLocation::default(),
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            ),
            TemplateSegmentOrigin::Body,
            Some(subscription),
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![dynamic_node], SourceLocation::default());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    );
    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("reactive template normalization should succeed");

    let metadata = expression
        .reactive_template
        .as_ref()
        .expect("runtime handoff replacement should preserve reactive template metadata");

    assert!(
        metadata.template_backed,
        "reactive metadata should be template-backed"
    );
    assert!(
        metadata.subscriptions.iter().any(|sub| {
            sub.source.path == reactive_path
                && matches!(sub.source.kind, ReactiveSourceKind::Declaration)
        }),
        "reactive metadata must contain the subscription from the final TIR view"
    );
}

/// Proves that a slot-insert helper artifact surviving composition is rejected
/// after final view traversal, not silently passed to HIR.
#[test]
fn helper_artifact_rejected_after_final_view_traversal() {
    let mut string_table = StringTable::new();
    let text = string_table.intern("slot insert content");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // Build a TIR root with simple text. The template kind is SlotInsert,
    // which finalization must reject as a helper artifact.
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(
            text,
            "slot insert content".len(),
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![text_node], SourceLocation::default());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        SourceLocation::default(),
    );

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
    };

    let result = normalize_expression_templates(&mut expression, &mut context);
    assert!(
        result.is_err(),
        "slot-insert helper artifact must be rejected after final view traversal"
    );

    let TemplateNormalizationError::Diagnostic(diagnostic) =
        result.expect_err("error was asserted above")
    else {
        panic!(
            "helper artifact rejection should produce a diagnostic, not an infrastructure error"
        );
    };
    assert!(
        matches!(
            diagnostic.as_ref().payload,
            DiagnosticPayload::InvalidTemplateStructure {
                reason: InvalidTemplateStructureReason::HelperOutsideWrapperSlot
            }
        ),
        "diagnostic must be HelperOutsideWrapperSlot"
    );
}

#[test]
fn retained_signature_default_normalizes_template_to_string_slice() {
    // Proves the retained-only generic default normalization path exercised by generic
    // declarations without an emitted node: a `FunctionSignature` parameter whose default is a
    // live TIR template normalizes to a TIR-free `StringSlice` through the real
    // `normalize_retained_signature_defaults` helper used by
    // `synchronize_normalized_public_defaults`, not through a direct
    // `normalize_expression_templates` call labelled generic.
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("generic default text");
    let template = registered_text_template(text, context, &template_ir_store, &string_table);

    let parameter_default = Expression::template(template, ValueMode::ImmutableOwned);
    let mut signature = FunctionSignature {
        parameters: vec![Declaration {
            id: InternedPath::new(),
            value: parameter_default,
        }],
        returns: Vec::new(),
    };

    normalize_retained_signature_defaults(
        &mut signature,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        &template_ir_store,
        &mut string_table,
    )
    .expect("a const text template default should normalize to a folded string");

    assert!(
        matches!(
            &signature.parameters[0].value.kind,
            ExpressionKind::StringSlice(normalized) if *normalized == text,
        ),
        "a retained template default must normalize to a TIR-free StringSlice, got {:?}",
        signature.parameters[0].value.kind
    );
}

#[test]
fn static_true_assertion_discards_normalized_runtime_template_message_after_validation() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let template = registered_runtime_template(
        string_table.intern("inactive: "),
        "name",
        TemplateViewContext::default(),
        &template_ir_store,
        &mut string_table,
    );
    let template_expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut type_environment = TypeEnvironment::new();
    let option_string_type_id = type_environment.intern_option(builtin_type_ids::STRING);
    let message = Expression::new(
        ExpressionKind::Coerced {
            value: Box::new(template_expression),
            to_type: option_string_type_id,
        },
        SourceLocation::default(),
        option_string_type_id,
        DataType::Option(Box::new(DataType::StringSlice)),
        ValueMode::ImmutableOwned,
    );
    let mut node = AstNode {
        kind: NodeKind::Assert {
            condition: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
            message,
        },
        location: SourceLocation::default(),
        scope: InternedPath::new(),
    };
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store,
    };

    normalize_ast_node_templates(&mut node, &mut context)
        .expect("static-true assertion messages should normalize before discard");
    // The production finalizer calls this cleanup immediately after its authoritative type/TIR
    // validation pass. This unit test exercises the same post-validation boundary directly.
    discard_inactive_assertion_messages(std::slice::from_mut(&mut node));

    let NodeKind::Assert { message, .. } = node.kind else {
        panic!("expected the test node to remain an assertion");
    };
    assert!(
        matches!(message.kind, ExpressionKind::OptionNone),
        "inactive assertion messages must be replaced with typed none, got {:?}",
        message.kind
    );
    assert_eq!(message.type_id, option_string_type_id);
    assert!(message.reactive_template.is_none());
    assert!(message.synthetic_interface_provenance.is_empty());
}

// ---------------------------------------------------------------------------
//  Emitted-default collection and receiver secondary-index synchronization
// ---------------------------------------------------------------------------
//
// Focused tests for the extracted `collect_emitted_declaration_defaults` and
// `synchronize_receiver_secondary_indexes` helpers. These are internal
// side-table invariants integration output cannot inspect, so they own a
// focused test beside the synchronization owner.

fn function_node(path: InternedPath) -> AstNode {
    AstNode {
        kind: NodeKind::Function(
            path,
            FunctionSignature {
                parameters: Vec::new(),
                returns: Vec::new(),
            },
            Vec::new(),
        ),
        location: SourceLocation::default(),
        scope: InternedPath::new(),
    }
}

fn struct_node(path: InternedPath) -> AstNode {
    AstNode {
        kind: NodeKind::StructDefinition(path, Vec::new()),
        location: SourceLocation::default(),
        scope: InternedPath::new(),
    }
}

fn marker_signature(parameter_count: usize) -> FunctionSignature {
    FunctionSignature {
        parameters: (0..parameter_count)
            .map(|_| Declaration {
                id: InternedPath::new(),
                value: Expression::no_value(
                    SourceLocation::default(),
                    DataType::Inferred,
                    ValueMode::default(),
                ),
            })
            .collect(),
        returns: Vec::new(),
    }
}

fn receiver_entry(
    function_path: InternedPath,
    receiver: ReceiverKey,
    source_file: InternedPath,
    signature: FunctionSignature,
) -> ReceiverMethodEntry {
    ReceiverMethodEntry {
        function_path,
        receiver,
        source_file,
        receiver_mutable: false,
        signature,
    }
}

#[test]
fn collect_emitted_declaration_defaults_rejects_duplicate_function_paths() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("dup_func", &mut string_table);
    let emitted = vec![function_node(path.clone()), function_node(path.clone())];

    let result = collect_emitted_declaration_defaults(&emitted);

    assert!(
        result.is_err(),
        "duplicate emitted function declaration paths must be rejected"
    );
}

#[test]
fn collect_emitted_declaration_defaults_rejects_duplicate_struct_paths() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("dup_struct", &mut string_table);
    let emitted = vec![struct_node(path.clone()), struct_node(path.clone())];

    let result = collect_emitted_declaration_defaults(&emitted);

    assert!(
        result.is_err(),
        "duplicate emitted struct declaration paths must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_copies_signatures_and_preserves_order() {
    let mut string_table = StringTable::new();
    let struct_a = InternedPath::from_single_str("StructA", &mut string_table);
    let struct_b = InternedPath::from_single_str("StructB", &mut string_table);
    let source_file = InternedPath::from_single_str("root.moth", &mut string_table);

    // Both methods share the bare method name "shared" but live on different receivers, so
    // by_method_name holds two entries under one name while by_receiver_and_name splits them
    // across two keys. The paths differ by their receiver parent, so the function paths are
    // distinct while the method names match. The insertion order [a, b] must survive
    // synchronization.
    let method_a = struct_a.join_str("shared", &mut string_table);
    let method_b = struct_b.join_str("shared", &mut string_table);
    let shared_name = method_a
        .name()
        .expect("multi-component path has a final name");

    let primary_a = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(2),
    );
    let primary_b = receiver_entry(
        method_b.clone(),
        ReceiverKey::Struct(struct_b.clone()),
        source_file.clone(),
        marker_signature(3),
    );

    // Secondary entries intentionally carry stale zero-parameter signatures so the copy from
    // the primary index is observable.
    let secondary_a = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(0),
    );
    let secondary_b = receiver_entry(
        method_b.clone(),
        ReceiverKey::Struct(struct_b.clone()),
        source_file.clone(),
        marker_signature(0),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary_a);
    catalog.by_function_path.insert(method_b.clone(), primary_b);
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_a.clone()), shared_name),
        vec![secondary_a],
    );
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_b.clone()), shared_name),
        vec![secondary_b],
    );
    catalog.by_method_name.insert(
        shared_name,
        vec![
            receiver_entry(
                method_a.clone(),
                ReceiverKey::Struct(struct_a.clone()),
                source_file.clone(),
                marker_signature(0),
            ),
            receiver_entry(
                method_b.clone(),
                ReceiverKey::Struct(struct_b.clone()),
                source_file.clone(),
                marker_signature(0),
            ),
        ],
    );

    synchronize_receiver_secondary_indexes(&mut catalog)
        .expect("a consistent catalog must synchronize without error");

    // by_receiver_and_name entries received the synchronized primary signatures.
    let synced_a =
        &catalog.by_receiver_and_name[&(ReceiverKey::Struct(struct_a.clone()), shared_name)][0];
    assert_eq!(
        synced_a.signature.parameters.len(),
        2,
        "by_receiver_and_name entry for method_a must copy the primary signature"
    );
    let synced_b =
        &catalog.by_receiver_and_name[&(ReceiverKey::Struct(struct_b.clone()), shared_name)][0];
    assert_eq!(
        synced_b.signature.parameters.len(),
        3,
        "by_receiver_and_name entry for method_b must copy the primary signature"
    );

    // by_method_name preserves insertion order [method_a, method_b] and copied signatures.
    let name_entries = &catalog.by_method_name[&shared_name];
    assert_eq!(
        name_entries.len(),
        2,
        "both shared-name methods are retained"
    );
    assert_eq!(
        name_entries[0].function_path, method_a,
        "vector order is preserved"
    );
    assert_eq!(
        name_entries[1].function_path, method_b,
        "vector order is preserved"
    );
    assert_eq!(
        name_entries[0].signature.parameters.len(),
        2,
        "by_method_name entry for method_a must copy the primary signature"
    );
    assert_eq!(
        name_entries[1].signature.parameters.len(),
        3,
        "by_method_name entry for method_b must copy the primary signature"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_missing_by_receiver_and_name_entry() {
    let mut string_table = StringTable::new();
    let struct_a = InternedPath::from_single_str("StructA", &mut string_table);
    let source_file = InternedPath::from_single_str("root.moth", &mut string_table);
    let method_a = InternedPath::from_single_str("method", &mut string_table);
    let method_name = method_a.name().expect("single-component path has a name");

    let primary = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(1),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary);
    // Omit by_receiver_and_name; by_method_name is present and consistent.
    catalog.by_method_name.insert(
        method_name,
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );

    let result = synchronize_receiver_secondary_indexes(&mut catalog);

    assert!(
        result.is_err(),
        "a primary with no matching by_receiver_and_name entry must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_duplicate_by_receiver_and_name_entry() {
    let mut string_table = StringTable::new();
    let struct_a = InternedPath::from_single_str("StructA", &mut string_table);
    let source_file = InternedPath::from_single_str("root.moth", &mut string_table);
    let method_a = InternedPath::from_single_str("method", &mut string_table);
    let method_name = method_a.name().expect("single-component path has a name");

    let primary = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(1),
    );
    let duplicate = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(0),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary);
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_a.clone()), method_name),
        vec![duplicate.clone(), duplicate],
    );
    catalog.by_method_name.insert(
        method_name,
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );

    let result = synchronize_receiver_secondary_indexes(&mut catalog);

    assert!(
        result.is_err(),
        "two by_receiver_and_name entries joining one primary must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_wrong_receiver_key() {
    let mut string_table = StringTable::new();
    let struct_a = InternedPath::from_single_str("StructA", &mut string_table);
    let struct_b = InternedPath::from_single_str("StructB", &mut string_table);
    let source_file = InternedPath::from_single_str("root.moth", &mut string_table);
    let method_a = InternedPath::from_single_str("method", &mut string_table);
    let method_name = method_a.name().expect("single-component path has a name");

    // The primary is filed under StructA, and by_receiver_and_name stores the entry under the
    // matching (StructA, name) key, but the entry itself claims receiver StructB. The primary
    // validation passes (it finds the entry by function path), and the secondary loop must
    // reject the wrong receiver key.
    let primary = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(1),
    );
    let wrong_key_entry = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_b.clone()),
        source_file.clone(),
        marker_signature(0),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary);
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_a.clone()), method_name),
        vec![wrong_key_entry],
    );
    catalog.by_method_name.insert(
        method_name,
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );

    let result = synchronize_receiver_secondary_indexes(&mut catalog);

    assert!(
        result.is_err(),
        "a by_receiver_and_name entry stored under the wrong receiver key must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_primary_path_key_mismatch() {
    let mut string_table = StringTable::new();
    let receiver_path = InternedPath::from_single_str("Counter", &mut string_table);
    let indexed_path = InternedPath::from_single_str("indexed", &mut string_table);
    let claimed_path = InternedPath::from_single_str("claimed", &mut string_table);
    let source_file = InternedPath::from_single_str("root.moth", &mut string_table);

    let primary = receiver_entry(
        claimed_path,
        ReceiverKey::Struct(receiver_path),
        source_file,
        marker_signature(1),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(indexed_path, primary);

    let result = synchronize_receiver_secondary_indexes(&mut catalog);

    assert!(
        result.is_err(),
        "a by_function_path map key that differs from its entry path must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_extra_secondary_entry() {
    let mut string_table = StringTable::new();
    let struct_a = InternedPath::from_single_str("StructA", &mut string_table);
    let struct_b = InternedPath::from_single_str("StructB", &mut string_table);
    let source_file = InternedPath::from_single_str("root.moth", &mut string_table);
    let method_a = InternedPath::from_single_str("method", &mut string_table);
    let orphan = InternedPath::from_single_str("orphan", &mut string_table);
    let method_name = method_a.name().expect("single-component path has a name");
    let orphan_name = orphan.name().expect("single-component path has a name");

    let primary = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(1),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary);
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_a.clone()), method_name),
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );
    catalog.by_method_name.insert(
        method_name,
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );
    // An extra by_receiver_and_name entry whose function path is not a primary.
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_b.clone()), orphan_name),
        vec![receiver_entry(
            orphan.clone(),
            ReceiverKey::Struct(struct_b.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );

    let result = synchronize_receiver_secondary_indexes(&mut catalog);

    assert!(
        result.is_err(),
        "a by_receiver_and_name entry with no matching by_function_path primary must be rejected"
    );
}

// ------------------------------
//  Slot-bearing template classification uses effective view
// ------------------------------

/// Builds a finalized slot template whose effective overlay resolves one fill.
///
/// WHAT: gives the resolver a module-local root plus the slot-resolution
///       overlay that makes the template an effective wrapper value.
/// WHY: const-fact classification must preserve the overlay-backed wrapper
///      category rather than reading only the structural root.
fn build_resolved_slot_template_store() -> (Template, Rc<RefCell<TemplateIrStore>>) {
    let location = SourceLocation::default();
    let store_handle = Rc::new(RefCell::new(TemplateIrStore::new()));

    let (template_id, fill_template_id) = {
        let mut store = store_handle.borrow_mut();

        let mut fill_builder = TemplateIrBuilder::new(&mut store);
        let fill_root = fill_builder.push_sequence_node(Vec::new(), location.clone());
        let fill_template_id = fill_builder.finish_template(
            fill_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location.clone(),
        );

        let mut wrapper_builder = TemplateIrBuilder::new(&mut store);
        let slot_node = wrapper_builder.push_slot_node(SlotKey::Default, location.clone());
        let template_id = wrapper_builder.finish_template(
            slot_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location.clone(),
        );

        (template_id, fill_template_id)
    };

    let slot_overlay_id = store_handle
        .borrow_mut()
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
            resolutions: vec![(
                SlotOccurrenceId::new(0),
                TirSlotResolution::resolved(SlotKey::Default, vec![fill_template_id]),
            )],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay_id),
        wrapper_context: None,
    };

    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Finalized,
            context,
        },
        location,
    };

    (template, store_handle)
}

#[test]
fn slot_bearing_module_constant_classifies_through_effective_tir_view() {
    let mut string_table = StringTable::new();

    let (template, registry) = build_resolved_slot_template_store();
    let projected = project_const_template_value(
        &template,
        &registry.borrow(),
        &mut string_table,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
    )
    .expect("slot template should project as a const template value");

    assert_eq!(
        projected.kind,
        TemplateConstValueKind::WrapperTemplate,
        "resolved-slot module constants must classify through the effective TIR view"
    );

    let value = const_template_value_from_projection(projected, &template)
        .expect("an effective wrapper is a supported module-constant store value");
    assert!(
        matches!(
            value,
            ConstTemplateValue::Public {
                kind: ConstValueKind::TemplateWrapper,
                hir_visible: true,
                folded: Some(_),
                ..
            }
        ),
        "a wrapper module constant keeps its public projection and its folded string"
    );
}
