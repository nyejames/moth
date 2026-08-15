//! Owned HIR-handoff materialization tests.
//!
//! WHAT: checks that finalized TIR becomes owned runtime handoff data.
//! WHY: the AST/HIR boundary must consume one shared module store without
//! exposing TIR identity or store-internal traversal.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::builder::TemplateIrBuilder;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionOverlay,
    TirSlotResolutionOverlayId, TirWrapperApplicationMode, TirWrapperContext,
    TirWrapperContextOverlay,
};
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::summary::{
    TemplateIrSummary, summarize_existing_root,
};
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, TirView};
use crate::compiler_frontend::ast::templates::tir::{
    PreparedRuntime, RuntimeTemplateReason, owned_runtime_template_handoff_for_prepared_view,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::datatype::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn prepared_runtime(view: &TirView<'_>) -> PreparedRuntime {
    PreparedRuntime {
        identity: view.identity(),
        reason: RuntimeTemplateReason::RuntimeExpression,
    }
}

fn handoff_for_view(
    view: TirView<'_>,
) -> Result<crate::compiler_frontend::ast::templates::OwnedRuntimeTemplateHandoff, CompilerError> {
    let prepared = prepared_runtime(&view);
    owned_runtime_template_handoff_for_prepared_view(&prepared, view)
}

/// Pushes a literal text node into the store and returns its ID.
fn text_node_id(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrNodeId {
    let text_id = string_table.intern(text);
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: text_id,
            byte_len: text.len() as u32,
            origin: TemplateSegmentOrigin::Body,
        },
        empty_location(),
    ))
}

/// Finishes a simple text-function template from its root node.
fn finish_text_template(store: &mut TemplateIrStore, root: TemplateIrNodeId) -> TemplateIrId {
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::StringFunction,
        summarize_existing_root(store, root),
        empty_location(),
    ))
}

/// Builds and finishes a one-node text template, returning its root ID.
fn text_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrId {
    let text_node = text_node_id(store, string_table, text);
    finish_text_template(store, text_node)
}

/// Builds a bool-typed reference expression for selector/header overrides.
fn bool_reference_expression(string_table: &mut StringTable, name: &str) -> Expression {
    Expression::reference_with_type_id(
        InternedPath::from_single_str(name, string_table),
        DataType::Bool,
        builtin_type_ids::BOOL,
        empty_location(),
        ValueMode::ImmutableReference,
        ConstRecordState::RuntimeValue,
    )
}

/// Builds a view context that overrides the given expression sites.
fn expression_overlay_context(
    store: &mut TemplateIrStore,
    overrides: Vec<(ExpressionSiteId, Expression)>,
) -> TemplateViewContext {
    let overrides = overrides
        .into_iter()
        .map(|(site_id, expression)| (site_id, Box::new(expression)))
        .collect();
    let expression_overlay_id =
        store.allocate_expression_overlay(TirExpressionOverlay { overrides });
    TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    }
}

/// Builds a view context that resolves the given slot occurrences.
fn slot_resolution_context(
    store: &mut TemplateIrStore,
    resolutions: Vec<(SlotOccurrenceId, TirSlotResolution)>,
) -> TemplateViewContext {
    let slot_resolution_overlay_id =
        store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay { resolutions });
    TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_resolution_overlay_id),
        wrapper_context: None,
    }
}

/// Pushes a child-template reference node and returns its node ID.
fn child_template_node_id(
    store: &mut TemplateIrStore,
    reference: TemplateTirChildReference,
) -> TemplateIrNodeId {
    let occurrence_id = store.next_child_template_occurrence_id();
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        },
        empty_location(),
    ))
}

/// Builds a finalized module-local child reference for a template root.
fn child_reference(
    template_id: TemplateIrId,
    context: TemplateViewContext,
) -> TemplateTirChildReference {
    TemplateTirChildReference::new(template_id, TemplateTirPhase::Finalized, context)
}

fn view_for(
    store: &TemplateIrStore,
    root: TemplateIrId,
    context: TemplateViewContext,
) -> TirView<'_> {
    TirView::with_minimum_phase(
        store,
        root,
        TemplateTirPhase::Finalized,
        TemplateTirPhase::Finalized,
        context,
    )
    .expect("finalized test view should construct")
}

/// Materializes the parent template through the fold-context entry point,
/// returning the full `Result` so success tests can unwrap and error tests
/// can assert on the `CompilerError`.
fn materialize_parent_handoff_result(
    store: Rc<RefCell<TemplateIrStore>>,
    parent_template_id: TemplateIrId,
    _string_table: &mut StringTable,
    view_context: TemplateViewContext,
) -> Result<OwnedRuntimeTemplateBody, CompilerError> {
    let store_ref = store.borrow();
    let view = view_for(&store_ref, parent_template_id, view_context);
    handoff_for_view(view).map(|handoff| handoff.body)
}

/// Convenience wrapper for success-path tests that expect materialization to
/// succeed.
fn materialize_parent_handoff(
    store: Rc<RefCell<TemplateIrStore>>,
    parent_template_id: TemplateIrId,
    string_table: &mut StringTable,
    context: TemplateViewContext,
) -> OwnedRuntimeTemplateBody {
    materialize_parent_handoff_result(store, parent_template_id, string_table, context)
        .expect("handoff materialization should succeed")
}

fn assert_owned_text_node(
    node: &OwnedRuntimeTemplateNode,
    expected: &str,
    string_table: &StringTable,
) {
    match node {
        OwnedRuntimeTemplateNode::Text { text, .. } => {
            assert_eq!(string_table.resolve(*text), expected);
        }
        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
            let OwnedRuntimeTemplateBody::Render(child) = &template.body else {
                panic!("expected rendered child handoff, got {:?}", template.body);
            };
            assert_owned_text_node(child, expected, string_table);
        }
        _ => panic!("expected owned text or child node, got {:?}", node),
    }
}

// ---------------------------------------------------------------------------
//  Wrapper template builders
// ---------------------------------------------------------------------------

fn build_branch_wrapper_template(store: &mut TemplateIrStore) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let default_slot = builder.push_slot_node(SlotKey::Default, empty_location());
    let positional_slot = builder.push_slot_node(SlotKey::Positional(2), empty_location());
    let branches = vec![
        TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                true,
                empty_location(),
                ValueMode::ImmutableOwned,
            )),
            default_slot,
            empty_location(),
        ),
        TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                false,
                empty_location(),
                ValueMode::ImmutableOwned,
            )),
            positional_slot,
            empty_location(),
        ),
    ];
    let root = builder.push_branch_chain_node(branches, None, empty_location());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    )
}

fn build_loop_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let body_default_slot = builder.push_slot_node(SlotKey::Default, empty_location());
    let aggregate_before = builder.push_text_node(
        string_table.intern("aggregate-before"),
        "aggregate-before".len() as u32,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let aggregate_positional_slot =
        builder.push_slot_node(SlotKey::Positional(1), empty_location());
    let aggregate_after = builder.push_text_node(
        string_table.intern("aggregate-after"),
        "aggregate-after".len() as u32,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let aggregate_wrapper = builder.push_sequence_node(
        vec![aggregate_before, aggregate_positional_slot, aggregate_after],
        empty_location(),
    );
    let root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(Expression::bool(
                true,
                empty_location(),
                ValueMode::ImmutableOwned,
            )),
        },
        body_default_slot,
        Some(aggregate_wrapper),
        empty_location(),
    );
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    )
}

fn build_child_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let nested_before = builder.push_text_node(
        string_table.intern("nested-before"),
        "nested-before".len() as u32,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let nested_positional_slot = builder.push_slot_node(SlotKey::Positional(0), empty_location());
    let nested_after = builder.push_text_node(
        string_table.intern("nested-after"),
        "nested-after".len() as u32,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let nested_root = builder.push_sequence_node(
        vec![nested_before, nested_positional_slot, nested_after],
        empty_location(),
    );
    let nested_template_id = builder.finish_template(
        nested_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    );
    let nested_child = builder.push_child_template_node(nested_template_id, empty_location());
    builder.finish_template(
        nested_child,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    )
}

fn build_expression_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> (TemplateIrId, ExpressionSiteId) {
    let expression_site_id = store.next_expression_site_id();
    let expression_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(bool_reference_expression(string_table, "original")),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: expression_site_id,
        },
        empty_location(),
    ));
    let mut builder = TemplateIrBuilder::new(store);
    let slot_node = builder.push_slot_node(SlotKey::Default, empty_location());
    let root = builder.push_sequence_node(vec![expression_node, slot_node], empty_location());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    );
    (template_id, expression_site_id)
}

fn build_slotless_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> TemplateIrId {
    text_template(store, string_table, "slotless-wrapper")
}

fn build_named_only_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> TemplateIrId {
    let named_slot_name = string_table.intern("named");
    let mut builder = TemplateIrBuilder::new(store);
    let named_slot = builder.push_slot_node(SlotKey::Named(named_slot_name), empty_location());
    builder.finish_template(
        named_slot,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    )
}

/// Builds a before/slot/after wrapper template with a default slot and
/// caller-supplied marker text so distinct wrappers can be told apart.
///
/// WHY: identical wrappers cannot prove the innermost-to-outermost handoff
///      nesting order; distinct before/after markers expose which layer is
///      innermost.
fn build_slot_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    before: &str,
    after: &str,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let before_node = builder.push_text_node(
        string_table.intern(before),
        before.len() as u32,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let slot_node = builder.push_slot_node(SlotKey::Default, empty_location());
    let after_node = builder.push_text_node(
        string_table.intern(after),
        after.len() as u32,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let root =
        builder.push_sequence_node(vec![before_node, slot_node, after_node], empty_location());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    )
}

/// Builds one parent child occurrence with an inherited wrapper and returns the
/// parent plus the wrapper-context overlay that activates it. The wrapper's own
/// view context is `empty_context` unless a separate wrapper overlay is
/// supplied through `build_parent_with_inherited_wrapper_and_overlay`.
fn build_parent_with_inherited_wrapper(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    empty_context: TemplateViewContext,
    string_table: &mut StringTable,
) -> (TemplateIrId, TemplateViewContext) {
    build_parent_with_inherited_wrapper_and_overlay(
        store,
        wrapper_template_id,
        empty_context,
        empty_context,
        string_table,
    )
}

fn build_parent_with_inherited_wrapper_and_overlay(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    empty_context: TemplateViewContext,
    wrapper_context: TemplateViewContext,
    string_table: &mut StringTable,
) -> (TemplateIrId, TemplateViewContext) {
    let (parent_template_id, wrapper_set_id, child_occurrence_id) = {
        let child_template_id = text_template(store, string_table, "child");
        let child_occurrence_id = store.next_child_template_occurrence_id();
        let child_reference = child_reference(child_template_id, empty_context);
        let child_node = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: child_reference,
                occurrence_id: child_occurrence_id,
            },
            empty_location(),
        ));
        let parent_template_id = finish_text_template(store, child_node);
        let wrapper_reference = TemplateWrapperReference::new(
            wrapper_template_id,
            TemplateTirPhase::Finalized,
            wrapper_context,
        );
        let wrapper_set_id = store.push_or_reuse_wrapper_set(vec![wrapper_reference]);
        (parent_template_id, wrapper_set_id, child_occurrence_id)
    };

    let wrapper_overlay_id = store.allocate_wrapper_context_overlay(TirWrapperContextOverlay {
        contexts: vec![(
            child_occurrence_id,
            TirWrapperContext::inherited(wrapper_set_id),
        )],
    });
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(wrapper_overlay_id),
    };

    (parent_template_id, context)
}

/// Builds a parent whose single child occurrence inherits one wrapper set built
/// from `wrapper_template_ids` (stored innermost-to-outermost), activated with
/// the supplied wrapper-context fields. The wrapper application mode comes
/// from `wrapper_context`.
///
/// WHY: focused multi-wrapper handoff tests need a single inherited wrapper
///      set holding distinct wrappers, which the single-wrapper builder above
///      cannot express.
fn build_parent_with_inherited_wrapper_set(
    store: &mut TemplateIrStore,
    wrapper_template_ids: &[TemplateIrId],
    wrapper_context: TirWrapperContext,
    string_table: &mut StringTable,
) -> (TemplateIrId, TemplateViewContext) {
    let empty_context = TemplateViewContext::default();
    let child_template_id = text_template(store, string_table, "child");
    let child_occurrence_id = store.next_child_template_occurrence_id();
    let child_reference = child_reference(child_template_id, empty_context);
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: child_reference,
            occurrence_id: child_occurrence_id,
        },
        empty_location(),
    ));
    let parent_template_id = finish_text_template(store, child_node);

    let wrapper_refs: Vec<TemplateWrapperReference> = wrapper_template_ids
        .iter()
        .map(|wrapper_template_id| {
            TemplateWrapperReference::new(
                *wrapper_template_id,
                TemplateTirPhase::Finalized,
                empty_context,
            )
        })
        .collect();
    let wrapper_set_id = store.push_or_reuse_wrapper_set(wrapper_refs);

    let wrapper_overlay_id = store.allocate_wrapper_context_overlay(TirWrapperContextOverlay {
        contexts: vec![(
            child_occurrence_id,
            TirWrapperContext {
                inherited_wrapper_set: Some(wrapper_set_id),
                ..wrapper_context
            },
        )],
    });
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(wrapper_overlay_id),
    };

    (parent_template_id, context)
}

// ---------------------------------------------------------------------------
//  Text and slot handoff
// ---------------------------------------------------------------------------

#[test]
fn owned_handoff_materializes_text_from_the_shared_store() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let template_id = text_template(&mut store.borrow_mut(), &mut strings, "hello");
    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, template_id, TemplateViewContext::default());
        handoff_for_view(view).expect("text handoff should succeed")
    };

    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Text { text, .. }) =
        handoff.body
    else {
        panic!("text template should materialize as an owned text node");
    };
    assert_eq!(strings.resolve(text), "hello");
}

#[test]
fn owned_handoff_resolves_slot_overlay_to_a_child_template() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, view_context) = {
        let mut store_ref = store.borrow_mut();
        let source_id = text_template(&mut store_ref, &mut strings, "filled");
        let occurrence_id = store_ref.next_slot_occurrence_id();
        let slot = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Slot {
                placeholder: TirSlotPlaceholder::new(
                    SlotKey::Default,
                    occurrence_id,
                    empty_location(),
                ),
            },
            empty_location(),
        ));
        let summary = summarize_existing_root(&store_ref, slot);
        let parent_id = store_ref.push_template(TemplateIr::new(
            slot,
            Style::default(),
            TemplateType::StringFunction,
            summary,
            empty_location(),
        ));
        let slot_overlay_id =
            store_ref.allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
                resolutions: vec![(
                    occurrence_id,
                    TirSlotResolution::resolved(SlotKey::Default, vec![source_id]),
                )],
            });
        let context = TemplateViewContext {
            expression_overlay: None,
            slot_resolution: Some(slot_overlay_id),
            wrapper_context: None,
        };
        (parent_id, context)
    };
    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, parent_id, view_context);
        handoff_for_view(view).expect("slot handoff should succeed")
    };

    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ChildTemplate {
        template, ..
    }) = handoff.body
    else {
        panic!("resolved slot should materialize as a child-template handoff");
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Text { text, .. }) =
        template.body
    else {
        panic!("slot source should materialize as text");
    };
    assert_eq!(strings.resolve(text), "filled");
}

#[test]
fn owned_handoff_missing_slot_resolution_renders_slot_placeholder() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let (parent_id, _occurrence_id, view_context) = {
        let mut store_ref = store.borrow_mut();
        let occurrence_id = store_ref.next_slot_occurrence_id();
        let slot = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Slot {
                placeholder: TirSlotPlaceholder::new(
                    SlotKey::Default,
                    occurrence_id,
                    empty_location(),
                ),
            },
            empty_location(),
        ));
        let parent_id = finish_text_template(&mut store_ref, slot);
        let context = slot_resolution_context(
            &mut store_ref,
            vec![(occurrence_id, TirSlotResolution::missing(SlotKey::Default))],
        );
        (parent_id, occurrence_id, context)
    };
    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, parent_id, view_context);
        handoff_for_view(view).expect("handoff materialization should succeed")
    };

    assert!(
        matches!(
            &handoff.body,
            OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Slot { .. })
        ),
        "missing slot resolution should materialize as a structural no-output slot placeholder, got {:?}",
        handoff.body
    );
}

#[test]
fn owned_handoff_preserves_child_boundary() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let parent_id = {
        let mut store_ref = store.borrow_mut();
        let child_id = text_template(&mut store_ref, &mut strings, "child");
        let occurrence_id = store_ref.next_child_template_occurrence_id();
        let child_node = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: TemplateTirChildReference::new(
                    child_id,
                    TemplateTirPhase::Parsed,
                    TemplateViewContext::default(),
                ),
                occurrence_id,
            },
            empty_location(),
        ));
        let summary = summarize_existing_root(&store_ref, child_node);
        store_ref.push_template(TemplateIr::new(
            child_node,
            Style::default(),
            TemplateType::StringFunction,
            summary,
            empty_location(),
        ))
    };
    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, parent_id, TemplateViewContext::default());
        handoff_for_view(view).expect("child handoff should succeed")
    };

    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ChildTemplate {
        template, ..
    }) = handoff.body
    else {
        panic!("child boundary should remain an owned child handoff");
    };
    let OwnedRuntimeTemplateBody::Render(child_node) = &template.body else {
        panic!(
            "child boundary should render an owned node, got {:?}",
            template.body
        );
    };
    assert_owned_text_node(child_node, "child", &strings);
}

// ---------------------------------------------------------------------------
//  Expression-overlay and child handoff
// ---------------------------------------------------------------------------

#[test]
fn parent_root_expression_overlay_applies_inside_child() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, _child_site_id, context) = {
        let mut store_ref = store.borrow_mut();
        let child_context = TemplateViewContext::default();
        let child_site_id = store_ref.next_expression_site_id();
        let child_expression = bool_reference_expression(&mut strings, "original");
        let child_root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: Box::new(child_expression),
                origin: TemplateSegmentOrigin::Body,
                reactive_subscription: None,
                site_id: child_site_id,
            },
            empty_location(),
        ));
        let child_template_id = finish_text_template(&mut store_ref, child_root);
        let child_node = child_template_node_id(
            &mut store_ref,
            child_reference(child_template_id, child_context),
        );
        let parent_id = finish_text_template(&mut store_ref, child_node);
        let context = expression_overlay_context(
            &mut store_ref,
            vec![(
                child_site_id,
                Expression::bool(true, empty_location(), ValueMode::ImmutableOwned),
            )],
        );
        (parent_id, child_site_id, context)
    };

    let body = materialize_parent_handoff(store, parent_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ChildTemplate {
        template, ..
    }) = body
    else {
        panic!("expected child template");
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::DynamicExpression {
        expression,
        ..
    }) = &template.body
    else {
        panic!("expected child dynamic expression, got {:?}", template.body);
    };

    assert!(
        matches!(expression.kind, ExpressionKind::Bool(true)),
        "parent root override should win over the child's empty overlay"
    );
}

#[test]
fn prepared_handoff_preserves_root_overlay_through_nested_children() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (root_id, _leaf_site_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let leaf_site_id = store_ref.next_expression_site_id();
        let stale_structural_text = strings.intern("stale-structural");
        let leaf_root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: Box::new(Expression::string_slice(
                    stale_structural_text,
                    empty_location(),
                    ValueMode::ImmutableOwned,
                )),
                origin: TemplateSegmentOrigin::Body,
                reactive_subscription: None,
                site_id: leaf_site_id,
            },
            empty_location(),
        ));
        let leaf_template_id = finish_text_template(&mut store_ref, leaf_root);
        let middle_child = child_template_node_id(
            &mut store_ref,
            child_reference(leaf_template_id, empty_context),
        );
        let middle_template_id = finish_text_template(&mut store_ref, middle_child);
        let root_child = child_template_node_id(
            &mut store_ref,
            child_reference(middle_template_id, empty_context),
        );
        let root_id = finish_text_template(&mut store_ref, root_child);
        let effective_root_text = strings.intern("effective-root");
        let context = expression_overlay_context(
            &mut store_ref,
            vec![(
                leaf_site_id,
                Expression::string_slice(
                    effective_root_text,
                    empty_location(),
                    ValueMode::ImmutableOwned,
                ),
            )],
        );
        (root_id, leaf_site_id, context)
    };

    let body = materialize_parent_handoff(store, root_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ChildTemplate {
        template: middle_template,
    }) = &body
    else {
        panic!("expected root child template handoff, got {body:?}");
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ChildTemplate {
        template: leaf_template,
    }) = &middle_template.body
    else {
        panic!(
            "expected nested leaf child template handoff, got {:?}",
            middle_template.body
        );
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::DynamicExpression {
        expression,
        ..
    }) = &leaf_template.body
    else {
        panic!(
            "stale structural leaf must not be folded into text, got {:?}",
            leaf_template.body
        );
    };

    let ExpressionKind::StringSlice(text) = expression.kind else {
        panic!(
            "expected the root expression overlay to survive structurally, got {:?}",
            expression.kind
        );
    };
    assert_eq!(strings.resolve(text), "effective-root");
}

#[test]
fn runtime_child_reference_uses_structural_handoff() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let child_site_id = store_ref.next_expression_site_id();
        let child_expression = bool_reference_expression(&mut strings, "runtime");
        let child_root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: Box::new(child_expression),
                origin: TemplateSegmentOrigin::Body,
                reactive_subscription: None,
                site_id: child_site_id,
            },
            empty_location(),
        ));
        let child_template_id = finish_text_template(&mut store_ref, child_root);
        let child_node = child_template_node_id(
            &mut store_ref,
            child_reference(child_template_id, empty_context),
        );
        let parent_id = finish_text_template(&mut store_ref, child_node);
        (parent_id, empty_context)
    };

    let body = materialize_parent_handoff_result(store, parent_id, &mut strings, context)
        .expect("runtime reference should use structural handoff");

    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ChildTemplate {
        template, ..
    }) = body
    else {
        panic!("expected child template handoff, got {body:?}");
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::DynamicExpression {
        expression,
        ..
    }) = &template.body
    else {
        panic!(
            "runtime-reference child should remain an owned dynamic expression, got {:?}",
            template.body
        );
    };
    let ExpressionKind::Reference(path) = &expression.kind else {
        panic!(
            "expected an owned reference expression, got {:?}",
            expression.kind
        );
    };
    assert_eq!(
        path.to_path_buf(&strings),
        PathBuf::from("runtime"),
        "runtime reference value should survive structural handoff"
    );
}

#[test]
fn child_infrastructure_error_propagates_through_hir_handoff() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let missing_child_root = TemplateIrNodeId::new(999);
        let child_template_id = store_ref.push_template(TemplateIr::new(
            missing_child_root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::empty(),
            empty_location(),
        ));
        let child_node = child_template_node_id(
            &mut store_ref,
            child_reference(child_template_id, empty_context),
        );
        let parent_id = finish_text_template(&mut store_ref, child_node);
        (parent_id, empty_context)
    };

    let error = materialize_parent_handoff_result(store, parent_id, &mut strings, context)
        .expect_err("malformed child authority must reach the HIR handoff caller");

    assert!(
        error.msg.contains("missing node"),
        "expected a stable infrastructure lane, got: {}",
        error.msg
    );
}

// ---------------------------------------------------------------------------
//  Inherited wrapper handoff
// ---------------------------------------------------------------------------

#[test]
fn inherited_wrapper_handoff_injects_through_branch_boundaries() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let wrapper_template_id = build_branch_wrapper_template(&mut store_ref);
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let body = materialize_parent_handoff(store, parent_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::BranchChain {
        branches,
        fallback,
        ..
    }) = body
    else {
        panic!("expected branch-chain wrapper handoff, got {:?}", body);
    };

    assert!(fallback.is_none());
    assert_eq!(branches.len(), 2);
    assert!(matches!(
        branches[0].body,
        OwnedRuntimeTemplateNode::Slot { .. }
    ));
    assert_owned_text_node(&branches[1].body, "child", &strings);
}

#[test]
fn inherited_wrapper_handoff_injects_through_loop_body_and_aggregate() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let wrapper_template_id = build_loop_wrapper_template(&mut store_ref, &mut strings);
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let body = materialize_parent_handoff(store, parent_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Loop {
        body,
        aggregate_wrapper,
        ..
    }) = body
    else {
        panic!("expected loop wrapper handoff, got {:?}", body);
    };

    assert!(matches!(*body, OwnedRuntimeTemplateNode::Slot { .. }));
    let Some(aggregate_wrapper) = aggregate_wrapper else {
        panic!("expected aggregate wrapper to remain present");
    };
    let OwnedRuntimeTemplateNode::Sequence { children } = aggregate_wrapper.as_ref() else {
        panic!(
            "expected aggregate wrapper sequence, got {:?}",
            aggregate_wrapper
        );
    };
    assert_eq!(children.len(), 3);
    assert_owned_text_node(&children[0], "aggregate-before", &strings);
    assert_owned_text_node(&children[1], "child", &strings);
    assert_owned_text_node(&children[2], "aggregate-after", &strings);
}

#[test]
fn inherited_wrapper_handoff_injects_through_child_template() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let wrapper_template_id = build_child_wrapper_template(&mut store_ref, &mut strings);
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let body = materialize_parent_handoff(store, parent_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ChildTemplate { template }) =
        body
    else {
        panic!("expected child wrapper handoff, got {:?}", body);
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children }) =
        template.body
    else {
        panic!("expected nested child sequence, got {:?}", template.body);
    };

    assert_eq!(children.len(), 3);
    assert_owned_text_node(&children[0], "nested-before", &strings);
    assert_owned_text_node(&children[1], "child", &strings);
    assert_owned_text_node(&children[2], "nested-after", &strings);
}

#[test]
fn inherited_wrapper_handoff_applies_wrapper_overlay() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let (wrapper_template_id, expression_site_id) =
            build_expression_wrapper_template(&mut store_ref, &mut strings);
        let wrapper_context = expression_overlay_context(
            &mut store_ref,
            vec![(
                expression_site_id,
                Expression::bool(true, empty_location(), ValueMode::ImmutableOwned),
            )],
        );
        let (parent_id, context) = build_parent_with_inherited_wrapper_and_overlay(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            wrapper_context,
            &mut strings,
        );
        // Wrapper references are structural transitions and therefore use the
        // active parent's complete expression overlay. Keep the override on
        // that parent view rather than relying on the referenced wrapper
        // context to import it.
        (parent_id, context.merge(wrapper_context))
    };

    let body = materialize_parent_handoff(store, parent_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children }) = body
    else {
        panic!("expected wrapper sequence, got {:?}", body);
    };

    let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = &children[0] else {
        panic!("expected wrapper expression, got {:?}", children[0]);
    };
    assert!(
        matches!(expression.kind, ExpressionKind::Bool(true)),
        "wrapper overlay should override the wrapper expression"
    );
    let OwnedRuntimeTemplateNode::ChildTemplate { template } = &children[1] else {
        panic!("expected child handoff, got {:?}", children[1]);
    };
    let OwnedRuntimeTemplateBody::Render(child_body) = &template.body else {
        panic!("expected rendered child handoff, got {:?}", template.body);
    };
    assert_owned_text_node(child_body, "child", &strings);
}

#[test]
fn inherited_wrapper_handoff_applies_wrapper_set_innermost_to_outermost() {
    // A single inherited wrapper set holding two distinct wrappers must hand off
    // as `outer(inner(child))`. `TemplateWrapperSet::wrappers` is stored
    // innermost-to-outermost, so forward handoff consumption applies the innermost
    // wrapper directly around the child and the outermost wrapper last.
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let inner = build_slot_wrapper_template(
            &mut store_ref,
            &mut strings,
            "inner-before",
            "inner-after",
        );
        let outer = build_slot_wrapper_template(
            &mut store_ref,
            &mut strings,
            "outer-before",
            "outer-after",
        );
        build_parent_with_inherited_wrapper_set(
            &mut store_ref,
            &[inner, outer],
            TirWrapperContext::default(),
            &mut strings,
        )
    };

    let body = materialize_parent_handoff(store, parent_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children }) = body
    else {
        panic!("expected outer wrapper sequence, got {body:?}");
    };
    assert_eq!(children.len(), 3);
    assert_owned_text_node(&children[0], "outer-before", &strings);
    assert_owned_text_node(&children[2], "outer-after", &strings);

    let OwnedRuntimeTemplateNode::Sequence {
        children: inner_children,
    } = &children[1]
    else {
        panic!("expected inner wrapper sequence, got {:?}", children[1]);
    };
    assert_eq!(inner_children.len(), 3);
    assert_owned_text_node(&inner_children[0], "inner-before", &strings);
    assert_owned_text_node(&inner_children[1], "child", &strings);
    assert_owned_text_node(&inner_children[2], "inner-after", &strings);
}

#[test]
fn inherited_wrapper_handoff_applies_conditional_wrapper_set_innermost_to_outermost() {
    // The IfChildEmits aggregate-wrapper path must also consume the
    // innermost-to-outermost store order forward, producing a
    // `ConditionalWrapper` whose wrapper tree is `outer(inner(AggregateOutput))`.
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let inner = build_slot_wrapper_template(
            &mut store_ref,
            &mut strings,
            "inner-before",
            "inner-after",
        );
        let outer = build_slot_wrapper_template(
            &mut store_ref,
            &mut strings,
            "outer-before",
            "outer-after",
        );
        build_parent_with_inherited_wrapper_set(
            &mut store_ref,
            &[inner, outer],
            TirWrapperContext {
                inherited_wrapper_set: None,
                skip_parent_child_wrappers: false,
                application_mode: TirWrapperApplicationMode::IfChildEmits,
            },
            &mut strings,
        )
    };

    let body = materialize_parent_handoff(store, parent_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ConditionalWrapper {
        child,
        wrapper,
        ..
    }) = body
    else {
        panic!("expected ConditionalWrapper, got {body:?}");
    };

    // The original child is carried unwrapped beside the aggregate wrapper tree.
    assert_owned_text_node(&child, "child", &strings);

    let OwnedRuntimeTemplateNode::Sequence { children } = wrapper.as_ref() else {
        panic!("expected outer wrapper sequence, got {:?}", wrapper);
    };
    assert_eq!(children.len(), 3);
    assert_owned_text_node(&children[0], "outer-before", &strings);
    assert_owned_text_node(&children[2], "outer-after", &strings);

    let OwnedRuntimeTemplateNode::Sequence {
        children: inner_children,
    } = &children[1]
    else {
        panic!("expected inner wrapper sequence, got {:?}", children[1]);
    };
    assert_eq!(inner_children.len(), 3);
    assert_owned_text_node(&inner_children[0], "inner-before", &strings);
    assert!(
        matches!(inner_children[1], OwnedRuntimeTemplateNode::AggregateOutput),
        "innermost slot should be the AggregateOutput splice marker"
    );
    assert_owned_text_node(&inner_children[2], "inner-after", &strings);
}

#[test]
fn inherited_slotless_wrapper_handoff_appends_child_after_wrapper_content() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let wrapper_template_id = build_slotless_wrapper_template(&mut store_ref, &mut strings);
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let body = materialize_parent_handoff(store, parent_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children }) = body
    else {
        panic!("expected slotless wrapper sequence, got {:?}", body);
    };

    assert_eq!(children.len(), 2);
    assert_owned_text_node(&children[0], "slotless-wrapper", &strings);
    assert_owned_text_node(&children[1], "child", &strings);
}

#[test]
fn inherited_named_only_wrapper_handoff_preserves_named_slot_and_appends_child() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let wrapper_template_id = build_named_only_wrapper_template(&mut store_ref, &mut strings);
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let body = materialize_parent_handoff(store, parent_id, &mut strings, context);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children }) = body
    else {
        panic!("expected named-only wrapper sequence, got {:?}", body);
    };

    assert_eq!(children.len(), 2);
    assert!(matches!(children[0], OwnedRuntimeTemplateNode::Slot { .. }));
    assert_owned_text_node(&children[1], "child", &strings);
}

// ---------------------------------------------------------------------------
//  Malformed-authority handoff failures
// ---------------------------------------------------------------------------

#[test]
fn malformed_child_view_context_propagates_view_failure() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, valid_context) = {
        let mut store_ref = store.borrow_mut();
        let valid_context = TemplateViewContext::default();
        let child_template_id = text_template(&mut store_ref, &mut strings, "child text");
        // Use an unallocated slot overlay so the Composed child transition fails.
        let invalid_context = TemplateViewContext {
            slot_resolution: Some(TirSlotResolutionOverlayId::new(99)),
            ..TemplateViewContext::default()
        };
        let child_node = child_template_node_id(
            &mut store_ref,
            child_reference(child_template_id, invalid_context),
        );
        let parent_id = finish_text_template(&mut store_ref, child_node);
        (parent_id, valid_context)
    };

    let error = materialize_parent_handoff_result(store, parent_id, &mut strings, valid_context)
        .expect_err("malformed child overlay should produce a CompilerError");

    assert!(
        error.msg.contains("slot resolution overlay"),
        "expected error about missing slot resolution overlay, got: {}",
        error.msg
    );
}

#[test]
fn missing_wrapper_tree_node_propagates_schema_extraction_error() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let slot_occurrence_id = store_ref.next_slot_occurrence_id();
        let slot = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Slot {
                placeholder: TirSlotPlaceholder::new(
                    SlotKey::Default,
                    slot_occurrence_id,
                    empty_location(),
                ),
            },
            empty_location(),
        ));
        let wrapper_root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![slot, TemplateIrNodeId::new(9999)],
            },
            empty_location(),
        ));
        let wrapper_template_id = store_ref.push_template(TemplateIr::new(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            empty_location(),
        ));
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let error = materialize_parent_handoff_result(store, parent_id, &mut strings, context)
        .expect_err("missing wrapper tree node should produce a CompilerError");

    assert!(
        error.msg.contains("TIR slot schema extraction: node ID"),
        "expected schema-owned node error, got: {}",
        error.msg
    );
}

#[test]
fn missing_child_in_wrapper_propagates_schema_extraction_error() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let missing_child_reference = child_reference(TemplateIrId::new(9999), empty_context);
        let missing_child_occurrence_id = store_ref.next_child_template_occurrence_id();
        let missing_child_node = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: missing_child_reference,
                occurrence_id: missing_child_occurrence_id,
            },
            empty_location(),
        ));
        let wrapper_template_id = finish_text_template(&mut store_ref, missing_child_node);
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let error = materialize_parent_handoff_result(store, parent_id, &mut strings, context)
        .expect_err("missing child in wrapper should produce a CompilerError");

    assert!(
        error
            .msg
            .contains("TIR slot schema extraction: child template ID"),
        "expected schema-owned child-template error, got: {}",
        error.msg
    );
}

/// Exact-view child cycles must fail before owned-handoff recursion.
///
/// This test is ignored until Phase 2C adds a cycle guard. Running it against
/// the current materializer recurses until the process overflows.
#[ignore = "Phase 0 reproduced: exact-view cycles stack-overflow during handoff; un-ignore in Phase 2C"]
#[test]
fn handoff_rejects_exact_view_child_cycle() {
    let mut store = TemplateIrStore::new();
    let template_id = TemplateIrId::new(store.template_count());
    let child = TemplateTirChildReference::new(
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    );
    let child_node = child_template_node_id(&mut store, child);
    let actual_id = finish_text_template(&mut store, child_node);
    assert_eq!(actual_id, template_id);

    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("cyclic view should construct");

    let error = handoff_for_view(view)
        .expect_err("exact-view child cycles must fail before handoff recursion");
    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::Compiler
    );
}
