//! Focused tests for the `TirView` read API.
//!
//! WHAT: exercises phase ordering, constructor validation (root existence,
//! view context existence, minimum-phase checks), root template/node lookup,
//! effective node lookup, child view construction, overlay-dimension entry
//! accessors, and invariant errors for invalid refs.
//!
//! WHY: `TirView` is the central read API for all future template consumers.
//! These tests guard the invariants later phases depend on: invalid store
//! IDs produce `CompilerError` instead of panics, minimum-phase checks reject
//! unready roots, and overlay dimension accessors resolve entries through the
//! value-carried view context.

use super::super::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId,
};
use super::super::node::TemplateIrNodeKind;
use super::super::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirExpressionOverlayId, TirSlotResolution,
    TirSlotResolutionOverlay, TirSlotResolutionOverlayId, TirWrapperApplicationMode,
    TirWrapperContext, TirWrapperContextOverlay,
};
use super::super::refs::{
    TemplateTirChildReference, TemplateTirReference, TemplateWrapperReference,
};
use super::super::store::TemplateIrStore;
use super::super::summary::TemplateIrSummary;
use super::super::view::{TemplateTirPhase, TirView};
use super::builder::TemplateIrBuilder;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::template::{
    Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

// -------------------------
//  Test helpers
// -------------------------

fn bool_expression() -> Expression {
    Expression {
        kind: ExpressionKind::Bool(true),
        type_id: builtin_type_ids::BOOL,
        diagnostic_type: DataType::Bool,
        function_receiver: None,
        value_mode: ValueMode::ImmutableOwned,
        location: SourceLocation::default(),
        reactive_source: None,
        reactive_template: None,
        const_record_state: ConstRecordState::RuntimeValue,
        contains_regular_division: false,
        synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
    }
}

/// Builds a template whose root is a single `DynamicExpression` node.
///
/// WHAT: returns the template ID and the root node ID so tests can construct a
///       `TemplateIrNodeId` and query the view for effective expressions.
fn build_template_with_dynamic_expression(
    store: &mut super::super::store::TemplateIrStore,
) -> (TemplateIrId, TemplateIrNodeId) {
    let mut builder = TemplateIrBuilder::new(store);
    let root = builder.push_dynamic_expression_node(
        bool_expression(),
        TemplateSegmentOrigin::Body,
        None,
        SourceLocation::default(),
    );
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );
    (template_id, root)
}

/// Builds a single empty template inside `store` and returns its `TemplateIrId`.
fn build_empty_template(store: &mut super::super::store::TemplateIrStore) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let root = builder.push_sequence_node(vec![], SourceLocation::default());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    )
}

/// Builds a template whose root node is a sequence with one text child.
///
/// WHAT: the child node is a `Text` node so tests can verify `effective_node`
///       resolves a non-root node through the view.
fn build_template_with_text_child(
    store: &mut super::super::store::TemplateIrStore,
    text_string_id: crate::compiler_frontend::symbols::string_interning::StringId,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let text_node = builder.push_text_node(
        text_string_id,
        5,
        crate::compiler_frontend::ast::templates::template::TemplateSegmentOrigin::Body,
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
}

/// Creates one module-local store containing an empty template and an empty view context.
struct TestStore {
    store: TemplateIrStore,
    root_ref: TemplateIrId,
    context: TemplateViewContext,
}

fn build_test_store() -> TestStore {
    let mut store = TemplateIrStore::new();
    let template_id = build_empty_template(&mut store);
    let context = TemplateViewContext::default();

    TestStore {
        store,
        root_ref: template_id,
        context,
    }
}

// -------------------------
//  Phase ordering tests
// -------------------------

#[test]
fn phase_ordering_and_is_at_least_are_monotonic() {
    // The phase sequence Parsed -> Composed -> Formatted -> Finalized is the
    // single owner for both the derived ordering and the is_at_least helper.
    assert!(TemplateTirPhase::Parsed < TemplateTirPhase::Composed);
    assert!(TemplateTirPhase::Composed < TemplateTirPhase::Formatted);
    assert!(TemplateTirPhase::Formatted < TemplateTirPhase::Finalized);

    assert!(TemplateTirPhase::Parsed.is_at_least(TemplateTirPhase::Parsed));
    assert!(TemplateTirPhase::Composed.is_at_least(TemplateTirPhase::Parsed));
    assert!(TemplateTirPhase::Formatted.is_at_least(TemplateTirPhase::Composed));
    assert!(TemplateTirPhase::Finalized.is_at_least(TemplateTirPhase::Formatted));

    assert!(!TemplateTirPhase::Parsed.is_at_least(TemplateTirPhase::Composed));
    assert!(!TemplateTirPhase::Composed.is_at_least(TemplateTirPhase::Formatted));
    assert!(!TemplateTirPhase::Formatted.is_at_least(TemplateTirPhase::Finalized));
}

#[test]
fn phase_display_matches_variant_names() {
    assert_eq!(TemplateTirPhase::Parsed.to_string(), "Parsed");
    assert_eq!(TemplateTirPhase::Composed.to_string(), "Composed");
    assert_eq!(TemplateTirPhase::Formatted.to_string(), "Formatted");
    assert_eq!(TemplateTirPhase::Finalized.to_string(), "Finalized");
}

// -------------------------
//  Constructor validation tests
// -------------------------

#[test]
fn new_succeeds_for_valid_root_and_view_context() {
    let TestStore {
        store,
        root_ref,
        context,
        ..
    } = build_test_store();

    let view = TirView::new(&store, root_ref, TemplateTirPhase::Parsed, context);

    assert!(view.is_ok());
}

#[test]
fn new_fails_for_missing_root_template() {
    let TestStore { store, context, .. } = build_test_store();

    let missing_root = TemplateIrId::new(99);
    let error = TirView::new(&store, missing_root, TemplateTirPhase::Parsed, context)
        .expect_err("missing root should be rejected");

    assert!(error.msg.contains("does not exist"));
}

// -------------------------
//  Occurrence-keyed overlay lookup tests
// -------------------------

/// Extracts the `ExpressionSiteId` from a `DynamicExpression` root node.
fn dynamic_expression_site_id(
    store: &super::super::store::TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> ExpressionSiteId {
    let node = store
        .get_node(node_id)
        .expect("dynamic expression node should exist");
    match &node.kind {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        _ => panic!("expected DynamicExpression node"),
    }
}

#[test]
fn effective_expression_for_site_resolves_override_and_none_cases() {
    let mut store = TemplateIrStore::new();

    let (template_id, root_node) = build_template_with_dynamic_expression(&mut store);
    let site_id = dynamic_expression_site_id(&store, root_node);

    // Override present: the site covered by the overlay resolves.
    let present_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(site_id, Box::new(bool_expression()))],
        })
        .expect("test overlay allocation");
    let present_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: Some(present_overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        },
    )
    .expect("view should construct");
    assert!(
        present_view
            .effective_expression_for_site(site_id)
            .expect("expression lookup should succeed")
            .is_some(),
        "override should be present for the covered site"
    );

    // No overlay: a site queried with an empty context returns Ok(None).
    let none_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )
    .expect("view should construct");
    assert!(
        none_view
            .effective_expression_for_site(site_id)
            .expect("expression lookup should succeed")
            .is_none(),
        "no override should exist without an expression overlay"
    );

    // Uncovered site: the overlay covers a different site, so this site is None.
    let other_site = store.next_expression_site_id();
    assert_ne!(other_site, site_id);
    let uncovered_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(other_site, Box::new(bool_expression()))],
        })
        .expect("test overlay allocation");
    let uncovered_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: Some(uncovered_overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        },
    )
    .expect("view should construct");
    assert!(
        uncovered_view
            .effective_expression_for_site(site_id)
            .expect("expression lookup should succeed")
            .is_none(),
        "no override should exist for an uncovered site"
    );
}

#[test]
fn effective_expression_for_node_resolves_override_and_none_cases() {
    let mut store = TemplateIrStore::new();

    // Override present: a DynamicExpression node with a matching site override.
    let (expression_template_id, expression_root) =
        build_template_with_dynamic_expression(&mut store);
    let expression_site = dynamic_expression_site_id(&store, expression_root);
    let expression_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(expression_site, Box::new(bool_expression()))],
        })
        .expect("test overlay allocation");
    let expression_view = TirView::new(
        &store,
        expression_template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: Some(expression_overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        },
    )
    .expect("view should construct");
    assert!(
        expression_view
            .effective_expression_for_node(expression_root)
            .expect("expression lookup should succeed")
            .is_some(),
        "override should be present for the dynamic expression node"
    );

    // None: the root node is a Sequence, not a DynamicExpression, so no override
    // is returned even though the overlay has an entry for a different site.
    let mut string_table = crate::compiler_frontend::symbols::string_interning::StringTable::new();
    let text_template_id = build_template_with_text_child(&mut store, string_table.intern("text"));
    let text_root = store
        .get_template(text_template_id)
        .expect("template should exist")
        .root;
    let unused_site = store.next_expression_site_id();
    let unused_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(unused_site, Box::new(bool_expression()))],
        })
        .expect("test overlay allocation");
    let text_view = TirView::new(
        &store,
        text_template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: Some(unused_overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        },
    )
    .expect("view should construct");
    assert!(
        text_view
            .effective_expression_for_node(text_root)
            .expect("expression lookup should succeed")
            .is_none(),
        "no override for a non-DynamicExpression node"
    );
}

#[test]
fn effective_slot_resolution_resolves_present_and_none_cases() {
    let mut store = TemplateIrStore::new();
    let template_id = build_empty_template(&mut store);

    // Present: the typed resolution carries its exact source template.
    let occurrence_id = store.next_slot_occurrence_id();
    let source = template_id;
    let slot_overlay_id = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
            resolutions: vec![(
                occurrence_id,
                TirSlotResolution::resolved(SlotKey::Default, vec![source]),
            )],
        })
        .expect("test overlay allocation");
    let present_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: None,
            slot_resolution: Some(slot_overlay_id),
            wrapper_context: None,
        },
    )
    .expect("view should construct");
    let resolution = present_view
        .effective_slot_resolution(occurrence_id)
        .expect("slot resolution lookup should succeed")
        .expect("resolution should be present");
    assert_eq!(resolution.sources(), &[source]);

    // None: an empty context returns Ok(None) for any occurrence.
    let none_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )
    .expect("view should construct");
    assert!(
        none_view
            .effective_slot_resolution(occurrence_id)
            .expect("slot resolution lookup should succeed")
            .is_none(),
        "no resolution without a slot-resolution overlay"
    );
}

#[test]
fn effective_wrapper_context_resolves_present_and_none_cases() {
    let mut store = TemplateIrStore::new();
    let template_id = build_empty_template(&mut store);

    // Present: the typed wrapper context carries its exact field values.
    let occurrence_id = store.next_child_template_occurrence_id();
    let wrapper_context = TirWrapperContext {
        inherited_wrapper_set: None,
        skip_parent_child_wrappers: true,
        application_mode: TirWrapperApplicationMode::IfChildEmits,
    };
    let wrapper_overlay_id = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(occurrence_id, wrapper_context.clone())],
        })
        .expect("test overlay allocation");
    let present_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: None,
            slot_resolution: None,
            wrapper_context: Some(wrapper_overlay_id),
        },
    )
    .expect("view should construct");
    let found = present_view
        .effective_wrapper_context(occurrence_id)
        .expect("wrapper context lookup should succeed")
        .expect("wrapper context should be present");
    assert_eq!(found, &wrapper_context);

    // None without overlay: an empty context returns Ok(None).
    let none_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )
    .expect("view should construct");
    assert!(
        none_view
            .effective_wrapper_context(occurrence_id)
            .expect("wrapper context lookup should succeed")
            .is_none(),
        "no context without a wrapper-context overlay"
    );

    // None for uncovered occurrence: the overlay covers a different occurrence.
    let uncovered_occurrence = store.next_child_template_occurrence_id();
    let uncovered_overlay_id = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(uncovered_occurrence, TirWrapperContext::empty())],
        })
        .expect("test overlay allocation");
    let uncovered_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: None,
            slot_resolution: None,
            wrapper_context: Some(uncovered_overlay_id),
        },
    )
    .expect("view should construct");
    assert!(
        uncovered_view
            .effective_wrapper_context(ChildTemplateOccurrenceId::new(99))
            .expect("wrapper context lookup should succeed")
            .is_none(),
        "no context for an occurrence not covered by the overlay"
    );
}

#[test]
fn new_fails_for_missing_view_context() {
    let TestStore {
        store, root_ref, ..
    } = build_test_store();

    let missing_context = TemplateViewContext {
        expression_overlay: Some(TirExpressionOverlayId::new(99)),
        ..TemplateViewContext::default()
    };
    let error = TirView::new(&store, root_ref, TemplateTirPhase::Parsed, missing_context)
        .expect_err("missing view context should be rejected");

    assert!(error.msg.contains("expression overlay"));
    assert!(error.msg.contains("does not exist"));
}

#[test]
fn with_minimum_phase_succeeds_when_phase_satisfies_minimum() {
    let TestStore {
        store,
        root_ref,
        context,
        ..
    } = build_test_store();

    let view = TirView::with_minimum_phase(
        &store,
        root_ref,
        TemplateTirPhase::Formatted,
        TemplateTirPhase::Composed,
        context,
    );

    assert!(view.is_ok());
    assert_eq!(view.unwrap().phase(), TemplateTirPhase::Formatted);
}

#[test]
fn with_minimum_phase_fails_when_phase_is_below_minimum() {
    let TestStore {
        store,
        root_ref,
        context,
        ..
    } = build_test_store();

    let error = TirView::with_minimum_phase(
        &store,
        root_ref,
        TemplateTirPhase::Parsed,
        TemplateTirPhase::Composed,
        context,
    )
    .expect_err("phase below minimum should be rejected");

    assert!(error.msg.contains("does not satisfy minimum phase"));
}

// -------------------------
//  Read accessor tests
// -------------------------

#[test]
fn constructor_accessors_return_exact_identity() {
    let TestStore {
        store,
        root_ref,
        context,
        ..
    } = build_test_store();

    // One owner for the three constructor identity accessors: root_ref, phase
    // and context each return the exact value supplied to the constructor.
    let view = TirView::new(&store, root_ref, TemplateTirPhase::Composed, context)
        .expect("view should construct");

    assert_eq!(view.root_ref(), root_ref);
    assert_eq!(view.phase(), TemplateTirPhase::Composed);
    assert_eq!(view.context(), context);
}

#[test]
fn root_template_resolves_the_root_template_entry() {
    let TestStore {
        store,
        root_ref,
        context,
        ..
    } = build_test_store();

    let view = TirView::new(&store, root_ref, TemplateTirPhase::Parsed, context)
        .expect("view should construct");

    let template = view.root_template().expect("root template should resolve");
    assert_eq!(template.kind, TemplateType::String);
}

#[test]
fn root_node_resolves_the_root_body_node() {
    let TestStore {
        store,
        root_ref,
        context,
        ..
    } = build_test_store();

    let view = TirView::new(&store, root_ref, TemplateTirPhase::Parsed, context)
        .expect("view should construct");

    let node = view.root_node().expect("root node should resolve");
    assert!(matches!(node.kind, TemplateIrNodeKind::Sequence { .. }));
}

#[test]
fn effective_node_resolves_a_non_root_node() {
    use crate::compiler_frontend::symbols::string_interning::StringTable;

    let mut store = TemplateIrStore::new();

    let mut string_table = StringTable::new();
    let text_id = string_table.intern("hello");

    let (template_id, child_node_id) = {
        let template_id = build_template_with_text_child(&mut store, text_id);

        // Recover the text child node ID from the root sequence.
        let root = store
            .get_template(template_id)
            .expect("template should exist")
            .root;
        let root_node = store.get_node(root).expect("root node should exist");
        let child_node_id = match &root_node.kind {
            TemplateIrNodeKind::Sequence { children } => children[0],
            other => panic!("root should be a sequence, got {other:?}"),
        };
        (template_id, child_node_id)
    };

    let context = TemplateViewContext::default();
    let root_ref = template_id;

    let view = TirView::new(&store, root_ref, TemplateTirPhase::Parsed, context)
        .expect("view should construct");

    let node_ref = child_node_id;
    let node = view
        .effective_node(node_ref)
        .expect("effective node should resolve");

    assert!(matches!(node.kind, TemplateIrNodeKind::Text { .. }));
}

#[test]
fn effective_node_errors_for_invalid_node_ref() {
    let TestStore {
        store,
        root_ref,
        context,
        ..
    } = build_test_store();

    let view = TirView::new(&store, root_ref, TemplateTirPhase::Parsed, context)
        .expect("view should construct");

    let invalid_node_ref = TemplateIrNodeId::new(99);
    let error = view
        .effective_node(invalid_node_ref)
        .expect_err("invalid node ref should be rejected");

    assert!(error.msg.contains("does not exist"));
}

// -------------------------
//  Child view construction tests
// -------------------------

#[test]
fn structural_child_constructs_a_valid_view_for_a_child_template() {
    let mut store = TemplateIrStore::new();

    let parent_id = { build_empty_template(&mut store) };
    let child_id = { build_empty_template(&mut store) };

    let context = TemplateViewContext::default();
    let parent_ref = parent_id;
    let child_ref = child_id;

    let parent_view = TirView::new(&store, parent_ref, TemplateTirPhase::Parsed, context)
        .expect("parent view should construct");

    let child_view = parent_view
        .structural_child(TemplateTirChildReference::new(
            child_ref,
            TemplateTirPhase::Parsed,
            context,
        ))
        .expect("child view should construct");

    assert_eq!(child_view.root_ref(), child_ref);
    assert_eq!(child_view.phase(), TemplateTirPhase::Parsed);
}

#[test]
fn structural_child_rejects_a_missing_view_context() {
    let TestStore {
        store,
        root_ref,
        context,
        ..
    } = build_test_store();

    let view = TirView::new(&store, root_ref, TemplateTirPhase::Parsed, context)
        .expect("view should construct");

    let missing_context = TemplateViewContext {
        slot_resolution: Some(TirSlotResolutionOverlayId::new(999)),
        ..TemplateViewContext::default()
    };
    let error = view
        .structural_child(TemplateTirChildReference::new(
            TemplateIrId::new(0),
            TemplateTirPhase::Composed,
            missing_context,
        ))
        .expect_err("missing view context should be rejected");

    assert!(error.msg.contains("does not exist"));
}

#[test]
fn named_view_transitions_preserve_their_documented_overlay_authority() {
    let mut store = TemplateIrStore::new();
    let parent_id = build_empty_template(&mut store);
    let child_id = build_empty_template(&mut store);

    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay::default())
        .expect("test overlay allocation");
    let slot_overlay = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("test overlay allocation");
    let wrapper_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay::default())
        .expect("test overlay allocation");
    let parent_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: None,
        wrapper_context: None,
    };
    let referenced_context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay),
        wrapper_context: Some(wrapper_overlay),
    };
    let parent_view = TirView::new(
        &store,
        parent_id,
        TemplateTirPhase::Composed,
        parent_context,
    )
    .expect("parent view should construct");

    let parsed_child = parent_view
        .structural_child(TemplateTirChildReference::new(
            child_id,
            TemplateTirPhase::Parsed,
            referenced_context,
        ))
        .expect("parsed child transition should construct");
    assert_eq!(
        parsed_child.context(),
        TemplateViewContext {
            expression_overlay: Some(expression_overlay),
            slot_resolution: None,
            wrapper_context: None,
        }
    );

    let composed_child = parent_view
        .structural_child(TemplateTirChildReference::new(
            child_id,
            TemplateTirPhase::Composed,
            referenced_context,
        ))
        .expect("composed child transition should construct");
    assert_eq!(
        composed_child.context().expression_overlay,
        Some(expression_overlay)
    );
    assert_eq!(composed_child.context().slot_resolution, Some(slot_overlay));
    assert_eq!(
        composed_child.context().wrapper_context,
        Some(wrapper_overlay)
    );

    let nested_context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay),
        wrapper_context: Some(wrapper_overlay),
    };
    let nested = parent_view
        .nested_template_value(TemplateTirReference {
            root: child_id,
            phase: TemplateTirPhase::Composed,
            context: nested_context,
        })
        .expect("nested template transition should construct");
    assert_eq!(nested.context(), nested_context);

    let wrapper = parent_view
        .wrapper(TemplateWrapperReference::new(
            child_id,
            TemplateTirPhase::Composed,
            referenced_context,
        ))
        .expect("wrapper transition should construct");
    assert_eq!(
        wrapper.context().expression_overlay,
        Some(expression_overlay)
    );
    assert_eq!(wrapper.context().slot_resolution, Some(slot_overlay));
    assert_eq!(wrapper.context().wrapper_context, Some(wrapper_overlay));
}

#[test]
fn structural_transition_does_not_import_referenced_expression_overlay() {
    let mut store = TemplateIrStore::new();
    let parent_id = build_empty_template(&mut store);
    let child_id = build_empty_template(&mut store);
    let child_expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay::default())
        .expect("test overlay allocation");

    let parent_view = TirView::new(
        &store,
        parent_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("parent view should construct");
    let referenced_context = TemplateViewContext {
        expression_overlay: Some(child_expression_overlay),
        ..TemplateViewContext::default()
    };
    let child_view = parent_view
        .structural_child(TemplateTirChildReference::new(
            child_id,
            TemplateTirPhase::Composed,
            referenced_context,
        ))
        .expect("child view should construct");
    let wrapper_view = parent_view
        .wrapper(TemplateWrapperReference::new(
            child_id,
            TemplateTirPhase::Composed,
            referenced_context,
        ))
        .expect("wrapper view should construct");

    for structural_view in [child_view, wrapper_view] {
        assert_eq!(structural_view.context().expression_overlay, None);
    }
}

#[test]
fn resolved_slot_source_and_structural_helper_preserve_exact_parent_view() {
    let mut store = TemplateIrStore::new();
    let parent_id = build_empty_template(&mut store);
    let source_id = build_empty_template(&mut store);
    let expression_overlay = store
        .allocate_expression_overlay(TirExpressionOverlay::default())
        .expect("test overlay allocation");
    let slot_overlay = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("test overlay allocation");
    let wrapper_overlay = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay::default())
        .expect("test overlay allocation");
    let parent_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay),
        slot_resolution: Some(slot_overlay),
        wrapper_context: Some(wrapper_overlay),
    };
    let parent_view = TirView::new(
        &store,
        parent_id,
        TemplateTirPhase::Formatted,
        parent_context,
    )
    .expect("parent view should construct");

    let resolved_source = parent_view
        .resolved_slot_source(source_id)
        .expect("resolved source transition should construct");
    let helper = parent_view
        .structural_helper(source_id)
        .expect("structural helper transition should construct");

    for view in [resolved_source, helper] {
        assert_eq!(view.root_ref(), source_id);
        assert_eq!(view.phase(), TemplateTirPhase::Formatted);
        assert_eq!(view.context(), parent_context);
    }
}

// -------------------------
//  Overlay-dimension entry accessor tests
// -------------------------

#[test]
fn overlay_dimension_accessors_resolve_each_dimension() {
    let mut store = TemplateIrStore::new();
    let template_id = build_empty_template(&mut store);

    // Seed index zero, then select a nonzero typed overlay in each dimension so
    // the accessors must resolve the exact ID rather than merely return an entry.
    store
        .allocate_expression_overlay(TirExpressionOverlay::default())
        .expect("test overlay allocation");
    store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("test overlay allocation");
    store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay::default())
        .expect("test overlay allocation");

    let expression_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay::default())
        .expect("test overlay allocation");
    let slot_overlay_id = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("test overlay allocation");
    let wrapper_overlay_id = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay::default())
        .expect("test overlay allocation");

    // Empty context: every dimension accessor returns None.
    let empty_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )
    .expect("view should construct");
    assert!(
        empty_view
            .expression_overlay()
            .expect("expression overlay lookup should succeed")
            .is_none()
    );
    assert!(
        empty_view
            .slot_resolution_overlay()
            .expect("slot overlay lookup should succeed")
            .is_none()
    );
    assert!(
        empty_view
            .wrapper_context_overlay()
            .expect("wrapper overlay lookup should succeed")
            .is_none()
    );

    // Expression dimension set: expression overlay resolves, the other two None.
    let expression_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: Some(expression_overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        },
    )
    .expect("view should construct");
    let expression_overlay = expression_view
        .expression_overlay()
        .expect("expression overlay lookup should succeed")
        .expect("expression overlay should be present");
    assert!(std::ptr::eq(
        expression_overlay,
        store
            .expression_overlay(expression_overlay_id)
            .expect("selected expression overlay should exist")
    ));
    assert!(
        expression_view
            .slot_resolution_overlay()
            .expect("slot overlay lookup should succeed")
            .is_none()
    );
    assert!(
        expression_view
            .wrapper_context_overlay()
            .expect("wrapper overlay lookup should succeed")
            .is_none()
    );

    // Slot-resolution dimension set: slot overlay resolves, the other two None.
    let slot_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: None,
            slot_resolution: Some(slot_overlay_id),
            wrapper_context: None,
        },
    )
    .expect("view should construct");
    assert!(
        slot_view
            .expression_overlay()
            .expect("expression overlay lookup should succeed")
            .is_none()
    );
    let slot_overlay = slot_view
        .slot_resolution_overlay()
        .expect("slot overlay lookup should succeed")
        .expect("slot overlay should be present");
    assert!(std::ptr::eq(
        slot_overlay,
        store
            .slot_resolution_overlay(slot_overlay_id)
            .expect("selected slot overlay should exist")
    ));
    assert!(
        slot_view
            .wrapper_context_overlay()
            .expect("wrapper overlay lookup should succeed")
            .is_none()
    );

    // Wrapper-context dimension set: wrapper overlay resolves, the other two None.
    let wrapper_view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext {
            expression_overlay: None,
            slot_resolution: None,
            wrapper_context: Some(wrapper_overlay_id),
        },
    )
    .expect("view should construct");
    assert!(
        wrapper_view
            .expression_overlay()
            .expect("expression overlay lookup should succeed")
            .is_none()
    );
    assert!(
        wrapper_view
            .slot_resolution_overlay()
            .expect("slot overlay lookup should succeed")
            .is_none()
    );
    let wrapper_overlay = wrapper_view
        .wrapper_context_overlay()
        .expect("wrapper overlay lookup should succeed")
        .expect("wrapper overlay should be present");
    assert!(std::ptr::eq(
        wrapper_overlay,
        store
            .wrapper_context_overlay(wrapper_overlay_id)
            .expect("selected wrapper overlay should exist")
    ));
}

// -------------------------
//  Source-location recovery tests
// -------------------------

/// Creates a `SourceLocation` with a specific line and column so tests can
/// distinguish locations by their position data.
///
/// WHAT: builds a `SourceLocation` using the default interned scope and the
///       given start/end line and column. Using non-default positions lets
///       assertions prove the correct location was returned rather than a
///       coincidental `Default`.
fn location_at(line: i32, column: i32) -> SourceLocation {
    use crate::compiler_frontend::compiler_messages::source_location::CharPosition;
    use crate::compiler_frontend::symbols::interned_path::InternedPath;

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

/// Asserts that an optional location result matches the expected line and column.
fn assert_location(
    result: Result<
        Option<SourceLocation>,
        crate::compiler_frontend::compiler_errors::CompilerError,
    >,
    line: i32,
    column: i32,
) {
    let location = result
        .expect("location lookup should succeed")
        .expect("location should be found");
    assert_eq!(location.start_pos.line_number, line);
    assert_eq!(location.start_pos.char_column, column);
}

/// Builds a template whose root is a `Sequence` containing one `Slot` node.
///
/// WHAT: returns the template ID, the root node ID, and the slot occurrence ID
///       so tests can query the view for the slot's source location.
fn build_template_with_slot(
    store: &mut super::super::store::TemplateIrStore,
    slot_location: SourceLocation,
) -> (TemplateIrId, SlotOccurrenceId) {
    let mut builder = TemplateIrBuilder::new(store);
    let slot_node = builder.push_slot_node(
        crate::compiler_frontend::ast::templates::template::SlotKey::Default,
        slot_location,
    );
    let root = builder.push_sequence_node(vec![slot_node], SourceLocation::default());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );
    let occurrence_id = {
        let node = store.get_node(slot_node).expect("slot node should exist");
        match &node.kind {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            _ => panic!("expected Slot node"),
        }
    };
    (template_id, occurrence_id)
}

/// Builds a template whose root is a `Sequence` containing one `ChildTemplate`
/// node referencing a second empty template in the module-local store.
///
/// WHAT: returns the parent template ID, the child template ID, and the
///       child-template occurrence ID so tests can verify the occurrence location
///       is recovered and that traversal does not cross into the child root.
fn build_template_with_child_template(
    store: &mut super::super::store::TemplateIrStore,
    child_template_location: SourceLocation,
    child_occurrence_location: SourceLocation,
) -> (
    TemplateIrId,
    TemplateIrId,
    super::super::ids::ChildTemplateOccurrenceId,
) {
    let mut builder = TemplateIrBuilder::new(store);

    // Build the child template first so the parent can reference it.
    let child_root = builder.push_sequence_node(vec![], SourceLocation::default());
    let child_template_id = builder.finish_template(
        child_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        child_template_location,
    );

    let child_node = builder.push_child_template_node(child_template_id, child_occurrence_location);
    let root = builder.push_sequence_node(vec![child_node], SourceLocation::default());
    let parent_template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );

    let occurrence_id = {
        let node = store.get_node(child_node).expect("child node should exist");
        match &node.kind {
            TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
            _ => panic!("expected ChildTemplate node"),
        }
    };

    (parent_template_id, child_template_id, occurrence_id)
}

/// Builds a template whose root is a `Sequence` containing one
/// `DynamicExpression` node, using a caller-provided source location.
fn build_template_with_dynamic_expression_at(
    store: &mut super::super::store::TemplateIrStore,
    expression_location: SourceLocation,
) -> (TemplateIrId, ExpressionSiteId) {
    let mut builder = TemplateIrBuilder::new(store);
    let expr_node = builder.push_dynamic_expression_node(
        bool_expression_with_location(&expression_location),
        TemplateSegmentOrigin::Body,
        None,
        expression_location,
    );
    let root = builder.push_sequence_node(vec![expr_node], SourceLocation::default());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );
    let site_id = {
        let node = store
            .get_node(expr_node)
            .expect("expression node should exist");
        match &node.kind {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            _ => panic!("expected DynamicExpression node"),
        }
    };
    (template_id, site_id)
}

/// A bool expression that carries a specific source location.
fn bool_expression_with_location(location: &SourceLocation) -> Expression {
    Expression {
        kind: ExpressionKind::Bool(true),
        type_id: builtin_type_ids::BOOL,
        diagnostic_type: DataType::Bool,
        function_receiver: None,
        value_mode: ValueMode::ImmutableOwned,
        location: location.clone(),
        reactive_source: None,
        reactive_template: None,
        const_record_state: ConstRecordState::RuntimeValue,
        contains_regular_division: false,
        synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
    }
}

/// Builds a template whose root is a `BranchChain` with one branch whose
/// selector is a `Bool` expression, plus a fallback body.
///
/// WHAT: returns the template ID and the branch selector's `ExpressionSiteId`
///       so tests can verify the selector site location is recovered from
///       `TemplateIrBranch::location`.
fn build_template_with_branch_chain(
    store: &mut super::super::store::TemplateIrStore,
    branch_location: SourceLocation,
) -> (TemplateIrId, ExpressionSiteId) {
    use crate::compiler_frontend::ast::templates::template_control_flow::TemplateBranchSelector;

    let mut builder = TemplateIrBuilder::new(store);

    let branch_body = builder.push_sequence_node(vec![], SourceLocation::default());
    let fallback_body = builder.push_sequence_node(vec![], SourceLocation::default());

    let branch = super::super::node::TemplateIrBranch::new(
        TemplateBranchSelector::Bool(bool_expression_with_location(&branch_location)),
        branch_body,
        branch_location,
        builder.store.next_expression_site_id(),
    );

    let root = builder.push_branch_chain_node(
        vec![branch],
        Some(fallback_body),
        SourceLocation::default(),
    );
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );

    let site_id = {
        let node = store
            .get_node(root)
            .expect("branch chain node should exist");
        match &node.kind {
            TemplateIrNodeKind::BranchChain { branches, .. } => branches[0].selector_site_id,
            _ => panic!("expected BranchChain node"),
        }
    };

    (template_id, site_id)
}

/// Builds a template whose root is a `Loop` with a `Conditional` (while) header,
/// so tests can verify the loop-header expression-site location is recovered from
/// the `Loop` node location.
fn build_template_with_conditional_loop(
    store: &mut super::super::store::TemplateIrStore,
    loop_location: SourceLocation,
) -> (TemplateIrId, ExpressionSiteId) {
    use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;

    let mut builder = TemplateIrBuilder::new(store);
    let body = builder.push_sequence_node(vec![], SourceLocation::default());
    let root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(bool_expression_with_location(&loop_location)),
        },
        body,
        None,
        loop_location,
    );
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );

    let site_id = {
        let node = store.get_node(root).expect("loop node should exist");
        match &node.kind {
            TemplateIrNodeKind::Loop { header_sites, .. } => match header_sites {
                super::super::node::TemplateLoopHeaderExpressionSites::Conditional {
                    condition,
                } => *condition,
                _ => panic!("expected Conditional loop header sites"),
            },
            _ => panic!("expected Loop node"),
        }
    };

    (template_id, site_id)
}

#[test]
fn source_location_for_slot_occurrence_resolves_present_and_missing() {
    let mut store = TemplateIrStore::new();
    let (template_id, occurrence_id) = build_template_with_slot(&mut store, location_at(7, 12));

    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )
    .expect("view should construct");

    // Present: the slot node location is recovered.
    assert_location(
        view.source_location_for_slot_occurrence(occurrence_id),
        7,
        12,
    );

    // Missing: an unknown occurrence id returns Ok(None).
    assert!(
        view.source_location_for_slot_occurrence(super::super::ids::SlotOccurrenceId::new(99))
            .expect("lookup should succeed")
            .is_none(),
        "missing slot occurrence should return Ok(None)"
    );
}

#[test]
fn source_location_for_child_template_occurrence_resolves_present_and_missing() {
    let mut store = TemplateIrStore::new();
    let (parent_template_id, _child_template_id, occurrence_id) =
        build_template_with_child_template(&mut store, location_at(1, 1), location_at(9, 20));

    let view = TirView::new(
        &store,
        parent_template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )
    .expect("view should construct");

    // Present: the child-template occurrence location is recovered.
    assert_location(
        view.source_location_for_child_template_occurrence(occurrence_id),
        9,
        20,
    );

    // Missing: an unknown occurrence id returns Ok(None).
    assert!(
        view.source_location_for_child_template_occurrence(
            super::super::ids::ChildTemplateOccurrenceId::new(99)
        )
        .expect("lookup should succeed")
        .is_none(),
        "missing child-template occurrence should return Ok(None)"
    );
}

#[test]
fn source_location_for_expression_site_resolves_each_node_kind_and_missing() {
    let mut store = TemplateIrStore::new();

    // DynamicExpression site: location recovered from the expression node.
    let (dynamic_template_id, dynamic_site_id) =
        build_template_with_dynamic_expression_at(&mut store, location_at(11, 30));
    let dynamic_view = TirView::new(
        &store,
        dynamic_template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )
    .expect("view should construct");
    assert_location(
        dynamic_view.source_location_for_expression_site(dynamic_site_id),
        11,
        30,
    );

    // Missing: an unknown expression site returns Ok(None). Asserted here so the
    // immutable view borrow ends before the next mutable template build.
    assert!(
        dynamic_view
            .source_location_for_expression_site(ExpressionSiteId::new(99))
            .expect("lookup should succeed")
            .is_none(),
        "missing expression site should return Ok(None)"
    );

    // BranchChain selector site: location recovered from the branch node.
    let (branch_template_id, branch_site_id) =
        build_template_with_branch_chain(&mut store, location_at(15, 8));
    let branch_view = TirView::new(
        &store,
        branch_template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )
    .expect("view should construct");
    assert_location(
        branch_view.source_location_for_expression_site(branch_site_id),
        15,
        8,
    );

    // Loop header site: location recovered from the conditional loop node.
    let (loop_template_id, loop_site_id) =
        build_template_with_conditional_loop(&mut store, location_at(21, 4));
    let loop_view = TirView::new(
        &store,
        loop_template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    )
    .expect("view should construct");
    assert_location(
        loop_view.source_location_for_expression_site(loop_site_id),
        21,
        4,
    );
}

#[test]
fn source_location_lookup_does_not_cross_into_child_template() {
    let mut store = TemplateIrStore::new();

    // Build a parent template that references a child template. The child has
    // its own slot with occurrence ID 0, while the parent has no slot of its own.
    let (parent_template_id, child_template_id, child_slot_occurrence_id) = {
        let (parent_template_id, child_template_id, child_slot_node) = {
            let mut builder = TemplateIrBuilder::new(&mut store);

            let child_slot_node = builder.push_slot_node(
                crate::compiler_frontend::ast::templates::template::SlotKey::Default,
                location_at(31, 6),
            );
            let child_root =
                builder.push_sequence_node(vec![child_slot_node], SourceLocation::default());
            let child_template_id = builder.finish_template(
                child_root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                location_at(1, 1),
            );

            let parent_child_node =
                builder.push_child_template_node(child_template_id, location_at(9, 20));
            let parent_root =
                builder.push_sequence_node(vec![parent_child_node], SourceLocation::default());
            let parent_template_id = builder.finish_template(
                parent_root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                SourceLocation::default(),
            );

            (parent_template_id, child_template_id, child_slot_node)
        };

        let child_slot_occurrence_id = {
            let node = store
                .get_node(child_slot_node)
                .expect("child slot node should exist");
            match &node.kind {
                TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
                _ => panic!("expected child Slot node"),
            }
        };

        (
            parent_template_id,
            child_template_id,
            child_slot_occurrence_id,
        )
    };

    // Look up the child's slot occurrence ID from the parent view: it must
    // not cross into the child root, so it should return Ok(None).
    let context = TemplateViewContext::default();
    let parent_ref = parent_template_id;
    let parent_view = TirView::new(&store, parent_ref, TemplateTirPhase::Parsed, context)
        .expect("parent view should construct");

    // The child-owned slot exists, but the parent view must not traverse into it.
    assert!(
        parent_view
            .source_location_for_slot_occurrence(child_slot_occurrence_id)
            .expect("lookup should succeed")
            .is_none(),
        "parent view must not cross into child template for slot occurrence lookup"
    );

    // A child view over the child template root should find the child's own
    // slot occurrence, proving the lookup works when the correct root is used.
    let child_view = TirView::new(&store, child_template_id, TemplateTirPhase::Parsed, context)
        .expect("child view should construct");

    assert_location(
        child_view.source_location_for_slot_occurrence(child_slot_occurrence_id),
        31,
        6,
    );
}
