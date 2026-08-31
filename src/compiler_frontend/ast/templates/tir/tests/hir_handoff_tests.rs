//! Owned HIR-handoff materialization tests.
//!
//! WHAT: checks that finalized TIR becomes owned runtime handoff data.
//! WHY: the AST/HIR boundary must consume one shared module store without
//! exposing TIR identity or store-internal traversal.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotSiteId;
use crate::compiler_frontend::ast::templates::tir::TemplateIrBuilder;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind,
    TemplateLoopHeaderExpressionSites, TirSlotPlaceholder,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionOverlay,
    TirSlotResolutionOverlayId, TirWrapperApplicationMode, TirWrapperContext,
    TirWrapperContextOverlay,
};
use crate::compiler_frontend::ast::templates::tir::preparation::TemplatePreparationFacts;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::store::{MalformedTirStore, TemplateIrStore};
use crate::compiler_frontend::ast::templates::tir::summary::{
    TemplateIrSummary, summarize_existing_root,
};
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, TirView};
use crate::compiler_frontend::ast::templates::tir::{
    RuntimeTemplateReason, TemplatePreparation, TemplatePreparationOutcome,
    owned_runtime_template_handoff_for_prepared_view,
};
use crate::compiler_frontend::ast::templates::tir::{TemplateSlotPlan, TemplateSlotSitePlan};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::datatype::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn prepared_runtime(view: &TirView<'_>) -> TemplatePreparation {
    TemplatePreparation {
        identity: view.identity(),
        facts: TemplatePreparationFacts {
            is_const_evaluable_shape: false,
            has_unresolved_slot_occurrences: false,
            has_resolved_slot_sources: false,
            has_escaped_insert_helpers: false,
            wrapper_foldable: false,
            has_runtime_slot_plan: false,
            has_runtime_slot_sites: false,
            has_reactive_dependence: false,
            final_value_kind: TemplateConstValueKind::NonConst,
        },
        outcome: TemplatePreparationOutcome::Runtime(RuntimeTemplateReason::RuntimeExpression),
    }
}

fn handoff_for_view(
    view: TirView<'_>,
    string_table: &StringTable,
) -> Result<crate::compiler_frontend::ast::templates::OwnedRuntimeTemplateHandoff, CompilerError> {
    let prepared = prepared_runtime(&view);
    owned_runtime_template_handoff_for_prepared_view(&prepared, view, string_table, None)
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
            byte_len: text.len(),
            origin: TemplateSegmentOrigin::Body,
        },
        SourceLocation::default(),
    ))
}

/// Finishes a simple text-function template from its root node.
fn finish_text_template(store: &mut TemplateIrStore, root: TemplateIrNodeId) -> TemplateIrId {
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::StringFunction,
        summarize_existing_root(store, root).expect("text template root is acyclic"),
        SourceLocation::default(),
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
        SourceLocation::default(),
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
    let expression_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay { overrides })
        .expect("test overlay allocation");
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
    let slot_resolution_overlay_id = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay { resolutions })
        .expect("test overlay allocation");
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
        SourceLocation::default(),
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
    string_table: &mut StringTable,
    view_context: TemplateViewContext,
) -> Result<OwnedRuntimeTemplateBody, CompilerError> {
    let store_ref = store.borrow();
    let view = view_for(&store_ref, parent_template_id, view_context);
    handoff_for_view(view, string_table).map(|handoff| handoff.body)
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

/// Asserts an owned handoff node carries exactly the expected plain text.
///
/// WHAT: reads the owned string payload directly rather than resolving a handle.
/// WHY: the handoff now owns its text, so no string table is needed here; a piece list
///      containing a resource or site root returns `None` and fails this assertion.
fn assert_owned_text_node(node: &OwnedRuntimeTemplateNode, expected: &str) {
    match node {
        OwnedRuntimeTemplateNode::Text { text, .. } => {
            assert_eq!(text.clone().into_text().as_deref(), Some(expected));
        }
        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
            let OwnedRuntimeTemplateBody::Render(child) = &template.body else {
                panic!("expected rendered child handoff, got {:?}", template.body);
            };
            assert_owned_text_node(child, expected);
        }
        _ => panic!("expected owned text or child node, got {:?}", node),
    }
}

// ---------------------------------------------------------------------------
//  Wrapper template builders
// ---------------------------------------------------------------------------

fn build_branch_wrapper_template(store: &mut TemplateIrStore) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let default_slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let positional_slot = builder.push_slot_node(SlotKey::Positional(2), SourceLocation::default());
    let branches = vec![
        TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                true,
                SourceLocation::default(),
                ValueMode::ImmutableOwned,
            )),
            default_slot,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        ),
        TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                false,
                SourceLocation::default(),
                ValueMode::ImmutableOwned,
            )),
            positional_slot,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        ),
    ];
    let root = builder.push_branch_chain_node(branches, None, SourceLocation::default());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    )
}

fn build_loop_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let body_default_slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let aggregate_before = builder.push_text_node(
        string_table.intern("aggregate-before"),
        "aggregate-before".len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let aggregate_positional_slot =
        builder.push_slot_node(SlotKey::Positional(1), SourceLocation::default());
    let aggregate_after = builder.push_text_node(
        string_table.intern("aggregate-after"),
        "aggregate-after".len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let aggregate_wrapper = builder.push_sequence_node(
        vec![aggregate_before, aggregate_positional_slot, aggregate_after],
        SourceLocation::default(),
    );
    let root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(Expression::bool(
                true,
                SourceLocation::default(),
                ValueMode::ImmutableOwned,
            )),
        },
        body_default_slot,
        Some(aggregate_wrapper),
        SourceLocation::default(),
    );
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    )
}

fn build_child_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let nested_before = builder.push_text_node(
        string_table.intern("nested-before"),
        "nested-before".len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let nested_positional_slot =
        builder.push_slot_node(SlotKey::Positional(0), SourceLocation::default());
    let nested_after = builder.push_text_node(
        string_table.intern("nested-after"),
        "nested-after".len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let nested_root = builder.push_sequence_node(
        vec![nested_before, nested_positional_slot, nested_after],
        SourceLocation::default(),
    );
    let nested_template_id = builder.finish_template(
        nested_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    );
    let nested_child =
        builder.push_child_template_node(nested_template_id, SourceLocation::default());
    builder.finish_template(
        nested_child,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
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
        SourceLocation::default(),
    ));
    let mut builder = TemplateIrBuilder::new(store);
    let slot_node = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let root =
        builder.push_sequence_node(vec![expression_node, slot_node], SourceLocation::default());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
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
    let named_slot =
        builder.push_slot_node(SlotKey::Named(named_slot_name), SourceLocation::default());
    builder.finish_template(
        named_slot,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
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
        before.len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let slot_node = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let after_node = builder.push_text_node(
        string_table.intern(after),
        after.len(),
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
        TemplateIrSummary::empty(),
        SourceLocation::default(),
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
            SourceLocation::default(),
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

    let wrapper_overlay_id = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                child_occurrence_id,
                TirWrapperContext::inherited(wrapper_set_id),
            )],
        })
        .expect("test overlay allocation");
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
        SourceLocation::default(),
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

    let wrapper_overlay_id = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                child_occurrence_id,
                TirWrapperContext {
                    inherited_wrapper_set: Some(wrapper_set_id),
                    ..wrapper_context
                },
            )],
        })
        .expect("test overlay allocation");
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
        handoff_for_view(view, &strings).expect("text handoff should succeed")
    };

    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Text { text, .. }) =
        handoff.body
    else {
        panic!("text template should materialize as an owned text node");
    };
    assert_eq!(text.clone().into_text().as_deref(), Some("hello"));
}

#[test]
fn owned_handoff_text_uses_interned_text_without_a_narrowed_byte_len() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let source = "hello owned handoff text";
    let template_id = {
        let mut store_ref = store.borrow_mut();
        text_template(&mut store_ref, &mut strings, source)
    };
    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, template_id, TemplateViewContext::default());
        handoff_for_view(view, &strings).expect("text handoff should succeed")
    };

    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Text { text, .. }) =
        handoff.body
    else {
        panic!("text template should materialize as an owned text node");
    };
    assert_eq!(text.clone().into_text().as_deref(), Some(source));
    assert_eq!(
        text.clone().into_text().map(|value| value.len()),
        Some(source.len())
    );
}

#[test]
fn owned_handoff_preserves_structural_string_pieces() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("site"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let resource_path =
        PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
            .expect("test resource path should be portable");
    let resource_origin = StableResourceOriginId::module_owned(module_origin, resource_path);
    let mut resources = ModuleResourceTable::new();
    let resource_id = resources.intern_origin(resource_origin.clone(), SourceLocation::default());
    let before = strings.intern("before");
    let after = strings.intern("after");
    let structural_expression = Expression::new(
        ExpressionKind::StructuralString {
            pieces: vec![
                crate::compiler_frontend::ast::const_values::store::ConstStringPiece::Text(before),
                crate::compiler_frontend::ast::const_values::store::ConstStringPiece::Resource(
                    resource_id,
                ),
                crate::compiler_frontend::ast::const_values::store::ConstStringPiece::SiteRoot,
                crate::compiler_frontend::ast::const_values::store::ConstStringPiece::Text(after),
            ],
        },
        SourceLocation::default(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );
    let site_id = store.borrow_mut().next_expression_site_id();
    let dynamic_node = store.borrow_mut().push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(structural_expression),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        SourceLocation::default(),
    ));
    let template_id = finish_text_template(&mut store.borrow_mut(), dynamic_node);

    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, template_id, TemplateViewContext::default());
        let prepared = prepared_runtime(&view);
        owned_runtime_template_handoff_for_prepared_view(
            &prepared,
            view,
            &strings,
            Some(&resources),
        )
        .expect("structural string should materialize through the owned handoff")
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Text { text, .. }) =
        handoff.body
    else {
        panic!("structural dynamic expression should become an owned string node");
    };
    assert_eq!(
        text,
        OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Text("before".to_owned()),
            OwnedFoldedStringPiece::Resource(resource_origin),
            OwnedFoldedStringPiece::SiteRoot,
            OwnedFoldedStringPiece::Text("after".to_owned()),
        ])
    );
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
                    SourceLocation::default(),
                ),
            },
            SourceLocation::default(),
        ));
        let summary = summarize_existing_root(&store_ref, slot).expect("slot root is acyclic");
        let parent_id = store_ref.push_template(TemplateIr::new(
            slot,
            Style::default(),
            TemplateType::StringFunction,
            summary,
            SourceLocation::default(),
        ));
        let slot_overlay_id = store_ref
            .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
                resolutions: vec![(
                    occurrence_id,
                    TirSlotResolution::resolved(SlotKey::Default, vec![source_id]),
                )],
            })
            .expect("test overlay allocation");
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
        handoff_for_view(view, &strings).expect("slot handoff should succeed")
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
    assert_eq!(text.clone().into_text().as_deref(), Some("filled"));
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
                    SourceLocation::default(),
                ),
            },
            SourceLocation::default(),
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
        handoff_for_view(view, &StringTable::new()).expect("handoff materialization should succeed")
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
            SourceLocation::default(),
        ));
        let summary =
            summarize_existing_root(&store_ref, child_node).expect("child root is acyclic");
        store_ref.push_template(TemplateIr::new(
            child_node,
            Style::default(),
            TemplateType::StringFunction,
            summary,
            SourceLocation::default(),
        ))
    };
    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, parent_id, TemplateViewContext::default());
        handoff_for_view(view, &strings).expect("child handoff should succeed")
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
    assert_owned_text_node(child_node, "child");
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
            SourceLocation::default(),
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
                Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
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
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                )),
                origin: TemplateSegmentOrigin::Body,
                reactive_subscription: None,
                site_id: leaf_site_id,
            },
            SourceLocation::default(),
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
                    SourceLocation::default(),
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
            SourceLocation::default(),
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
            SourceLocation::default(),
        ));
        let child_node = child_template_node_id(
            &mut store_ref,
            child_reference(child_template_id, empty_context),
        );
        let parent_id = store_ref.push_template(TemplateIr::new(
            child_node,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        ));
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
    assert_owned_text_node(&branches[1].body, "child");
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
    assert_owned_text_node(&children[0], "aggregate-before");
    assert_owned_text_node(&children[1], "child");
    assert_owned_text_node(&children[2], "aggregate-after");
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
    assert_owned_text_node(&children[0], "nested-before");
    assert_owned_text_node(&children[1], "child");
    assert_owned_text_node(&children[2], "nested-after");
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
                Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
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
    assert_owned_text_node(child_body, "child");
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
    assert_owned_text_node(&children[0], "outer-before");
    assert_owned_text_node(&children[2], "outer-after");

    let OwnedRuntimeTemplateNode::Sequence {
        children: inner_children,
    } = &children[1]
    else {
        panic!("expected inner wrapper sequence, got {:?}", children[1]);
    };
    assert_eq!(inner_children.len(), 3);
    assert_owned_text_node(&inner_children[0], "inner-before");
    assert_owned_text_node(&inner_children[1], "child");
    assert_owned_text_node(&inner_children[2], "inner-after");
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
    assert_owned_text_node(&child, "child");

    let OwnedRuntimeTemplateNode::Sequence { children } = wrapper.as_ref() else {
        panic!("expected outer wrapper sequence, got {:?}", wrapper);
    };
    assert_eq!(children.len(), 3);
    assert_owned_text_node(&children[0], "outer-before");
    assert_owned_text_node(&children[2], "outer-after");

    let OwnedRuntimeTemplateNode::Sequence {
        children: inner_children,
    } = &children[1]
    else {
        panic!("expected inner wrapper sequence, got {:?}", children[1]);
    };
    assert_eq!(inner_children.len(), 3);
    assert_owned_text_node(&inner_children[0], "inner-before");
    assert!(
        matches!(inner_children[1], OwnedRuntimeTemplateNode::AggregateOutput),
        "innermost slot should be the AggregateOutput splice marker"
    );
    assert_owned_text_node(&inner_children[2], "inner-after");
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
    assert_owned_text_node(&children[0], "slotless-wrapper");
    assert_owned_text_node(&children[1], "child");
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
    assert_owned_text_node(&children[1], "child");
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
fn missing_wrapper_tree_node_propagates_layout_error() {
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
                    SourceLocation::default(),
                ),
            },
            SourceLocation::default(),
        ));
        let wrapper_root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![slot, TemplateIrNodeId::new(9999)],
            },
            SourceLocation::default(),
        ));
        let wrapper_template_id = store_ref.push_template(TemplateIr::new(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
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
        error.msg.contains("TIR slot layout requested missing node"),
        "expected layout-owned node error, got: {}",
        error.msg
    );
}

#[test]
fn missing_child_in_wrapper_propagates_layout_error() {
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
            SourceLocation::default(),
        ));
        let wrapper_template_id = store_ref.push_template(TemplateIr::new(
            missing_child_node,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        ));
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
            .contains("TIR slot layout referenced missing child template"),
        "expected layout-owned child-template error, got: {}",
        error.msg
    );
}

fn runtime_site_template(
    store: &mut TemplateIrStore,
    plan: TemplateSlotPlanId,
    site: RuntimeSlotSiteId,
) -> TemplateIrId {
    let runtime_site = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::RuntimeSlotSite { plan, site },
        SourceLocation::default(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        runtime_site,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    ));
    store
        .attach_runtime_slot_plan(template_id, plan)
        .expect("runtime slot plan should attach");
    template_id
}

#[test]
fn handoff_rejects_runtime_slot_site_from_a_different_plan() {
    let mut store = TemplateIrStore::new();
    let mut strings = StringTable::new();
    let render_root = text_node_id(&mut store, &mut strings, "site");
    let owner_plan = store.push_slot_plan(TemplateSlotPlan {
        location: SourceLocation::default(),
        contribution_sources: Vec::new(),
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root,
            location: SourceLocation::default(),
        }],
    });
    let other_plan = store.push_slot_plan(TemplateSlotPlan {
        location: SourceLocation::default(),
        contribution_sources: Vec::new(),
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root,
            location: SourceLocation::default(),
        }],
    });
    let template_id = runtime_site_template(&mut store, other_plan, RuntimeSlotSiteId(0));
    store
        .attach_runtime_slot_plan(template_id, owner_plan)
        .expect("owner plan should replace the forged plan");

    let view = view_for(&store, template_id, TemplateViewContext::default());
    let error = handoff_for_view(view, &strings).expect_err("wrong-plan site must fail at handoff");
    assert!(error.msg.contains("outside its owning slot application"));
}

#[test]
fn handoff_rejects_out_of_range_runtime_slot_site() {
    let mut store = TemplateIrStore::new();
    let plan = store.push_slot_plan(TemplateSlotPlan {
        location: SourceLocation::default(),
        contribution_sources: Vec::new(),
        slot_sites: Vec::new(),
    });
    let template_id = runtime_site_template(&mut store, plan, RuntimeSlotSiteId(0));

    let view = view_for(&store, template_id, TemplateViewContext::default());
    let error = handoff_for_view(view, &StringTable::new())
        .expect_err("out-of-range site must fail at handoff");
    assert!(error.msg.contains("out-of-range runtime slot site"));
}

#[test]
fn handoff_rejects_mismatched_runtime_slot_site_identity() {
    let mut store = TemplateIrStore::new();
    let mut strings = StringTable::new();
    let render_root = text_node_id(&mut store, &mut strings, "site");
    let plan = store.push_slot_plan(TemplateSlotPlan {
        location: SourceLocation::default(),
        contribution_sources: Vec::new(),
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root,
            location: SourceLocation::default(),
        }],
    });
    MalformedTirStore::new(&mut store).replace_slot_sites(
        plan,
        vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(7),
            key: SlotKey::Default,
            render_root,
            location: SourceLocation::default(),
        }],
    );
    let template_id = runtime_site_template(&mut store, plan, RuntimeSlotSiteId(0));

    let view = view_for(&store, template_id, TemplateViewContext::default());
    let error = handoff_for_view(view, &strings).expect_err("mismatched site identity must fail");
    assert!(
        error
            .msg
            .contains("slot site whose stored identity does not match")
    );
}

#[test]
fn handoff_rejects_mismatched_loop_header_shape() {
    let mut store = TemplateIrStore::new();
    let mut strings = StringTable::new();
    let body = text_node_id(&mut store, &mut strings, "body");
    let loop_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Loop {
            header: TemplateLoopHeader::Conditional {
                condition: Box::new(Expression::bool(
                    false,
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                )),
            },
            header_sites: TemplateLoopHeaderExpressionSites::Collection {
                iterable: ExpressionSiteId::new(0),
            },
            body,
            aggregate_wrapper: None,
        },
        SourceLocation::default(),
    ));
    let template_id = finish_text_template(&mut store, loop_node);
    let view = view_for(&store, template_id, TemplateViewContext::default());

    let error =
        handoff_for_view(view, &strings).expect_err("loop header shape mismatch must fail closed");
    assert!(error.msg.contains("loop header shape mismatch"));
}

/// Exact-view child cycles must fail before owned-handoff recursion.
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
    let actual_id = store.push_template(TemplateIr::new(
        child_node,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    ));
    assert_eq!(actual_id, template_id);

    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("cyclic view should construct");

    let error = handoff_for_view(view, &StringTable::new())
        .expect_err("exact-view child cycles must fail before handoff recursion");
    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::Compiler
    );
}
