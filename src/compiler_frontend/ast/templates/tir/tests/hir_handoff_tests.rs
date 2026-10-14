//! Owned HIR-handoff materialization tests.
//!
//! WHAT: checks that finalized TIR becomes owned runtime handoff data.
//! WHY: the AST/HIR boundary must consume one shared module store without
//! exposing TIR identity or store-internal traversal.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_rewrite::{
    promote_replacement, replacement_input,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CompletedOperation, ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
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
    TemplateIr, TemplateIrNode, TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
    TirSlotPlaceholder,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionOverlay,
    TirSlotResolutionOverlayId, TirWrapperApplicationMode, TirWrapperContext,
    TirWrapperContextOverlay,
};
use crate::compiler_frontend::ast::templates::tir::preparation::TemplatePreparationFacts;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateTirReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::store::{MalformedTirStore, TemplateIrStore};
use crate::compiler_frontend::ast::templates::tir::summary::{
    TemplateIrSummary, summarize_existing_root,
};
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, TirView};
use crate::compiler_frontend::ast::templates::tir::{
    RuntimeTemplateReason, TemplatePreparation, TemplatePreparationMode,
    TemplatePreparationOutcome, owned_runtime_slot_handoff_for_prepared_view,
    owned_runtime_template_handoff_for_prepared_view, prepare_tir_view,
};
use crate::compiler_frontend::ast::templates::tir::{TemplateSlotPlan, TemplateSlotSitePlan};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
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
            has_escaped_insert_helpers: false,
            final_value_kind: TemplateConstValueKind::NonConst,
        },
        outcome: TemplatePreparationOutcome::Runtime(RuntimeTemplateReason::RuntimeExpression),
    }
}

fn handoff_for_view(
    view: TirView<'_>,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    string_table: &StringTable,
) -> Result<crate::compiler_frontend::ast::templates::OwnedRuntimeTemplateHandoff, CompilerError> {
    let prepared = prepared_runtime(&view);
    owned_runtime_template_handoff_for_prepared_view(
        &prepared,
        view,
        expressions,
        const_values,
        string_table,
        None,
    )
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
        None,
    ))
}

/// Finishes a simple text-function template from its root node.
fn finish_text_template(store: &mut TemplateIrStore, root: TemplateIrNodeId) -> TemplateIrId {
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::StringFunction,
        summarize_existing_root(store, root).expect("text template root is acyclic"),
        None,
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
fn bool_reference_expression(
    expressions: &mut AstExpressionBuilder,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
    name: &str,
) -> ExprId {
    let path = path_fork
        .try_intern_portable_path(name, string_table)
        .expect("test path fits");
    expressions
        .reference_with_type_id(
            path,
            builtin_type_ids::BOOL,
            None,
            ValueMode::ImmutableReference,
            ConstRecordState::RuntimeValue,
        )
        .expect("bool reference expression should construct")
        .id()
}

/// Builds a structural string constant from the given pieces, mirroring the
/// production structural-string constant path.
fn structural_string_expression(
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    pieces: Vec<ConstStringPiece>,
    value_mode: ValueMode,
) -> ExprId {
    let value = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(pieces)),
            None,
        )
        .expect("structural string constant should resolve");
    expressions
        .constant(value, builtin_type_ids::STRING, None, value_mode)
        .expect("structural string constant expression should construct")
        .id()
}

/// Reads a bool literal's value through the typed store; other shapes yield `None`.
fn bool_literal_value(
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    id: ExprId,
) -> Option<bool> {
    let ExpressionOperationView::Constant(value) = expressions.view(id).ok()?.operation() else {
        return None;
    };
    match const_values.payload(value)? {
        ConstValuePayload::Bool(value) => Some(*value),
        _ => None,
    }
}

/// Builds a view context that overrides the given expression sites.
fn expression_overlay_context(
    store: &mut TemplateIrStore,
    overrides: Vec<(ExpressionSiteId, ExprId)>,
) -> TemplateViewContext {
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
        None,
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
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
    view_context: TemplateViewContext,
) -> Result<OwnedRuntimeTemplateBody, CompilerError> {
    let store_ref = store.borrow();
    let view = view_for(&store_ref, parent_template_id, view_context);
    handoff_for_view(view, expressions, const_values, string_table).map(|handoff| handoff.body)
}

/// Convenience wrapper for success-path tests that expect materialization to
/// succeed.
fn materialize_parent_handoff(
    store: Rc<RefCell<TemplateIrStore>>,
    parent_template_id: TemplateIrId,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
    context: TemplateViewContext,
) -> OwnedRuntimeTemplateBody {
    materialize_parent_handoff_result(
        store,
        parent_template_id,
        expressions,
        const_values,
        string_table,
        context,
    )
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

fn build_conditional_wrapper_template(
    store: &mut TemplateIrStore,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> TemplateIrId {
    let selector = expressions
        .bool(const_values, true, None, ValueMode::ImmutableOwned)
        .expect("conditional wrapper selector should construct")
        .id();
    let mut builder = TemplateIrBuilder::new(store);
    let default_slot = builder.push_slot_node(SlotKey::Default, None);
    let positional_slot = builder.push_slot_node(SlotKey::Positional(2), None);
    let body = builder.push_sequence_node(vec![default_slot, positional_slot], None);
    let root = builder.push_conditional_node(TemplateBranchSelector::Bool(selector), body, None);
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    )
}

fn build_loop_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let body_default_slot = builder.push_slot_node(SlotKey::Default, None);
    let aggregate_before = builder.push_text_node(
        string_table.intern("aggregate-before"),
        "aggregate-before".len(),
        TemplateSegmentOrigin::Body,
        None,
    );
    let aggregate_positional_slot = builder.push_slot_node(SlotKey::Positional(1), None);
    let aggregate_after = builder.push_text_node(
        string_table.intern("aggregate-after"),
        "aggregate-after".len(),
        TemplateSegmentOrigin::Body,
        None,
    );
    let aggregate_wrapper = builder.push_sequence_node(
        vec![aggregate_before, aggregate_positional_slot, aggregate_after],
        None,
    );
    let root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: expressions
                .bool(const_values, true, None, ValueMode::ImmutableOwned)
                .expect("loop wrapper condition should construct")
                .id(),
        },
        body_default_slot,
        Some(aggregate_wrapper),
        None,
    );
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
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
        None,
    );
    let nested_positional_slot = builder.push_slot_node(SlotKey::Positional(0), None);
    let nested_after = builder.push_text_node(
        string_table.intern("nested-after"),
        "nested-after".len(),
        TemplateSegmentOrigin::Body,
        None,
    );
    let nested_root = builder.push_sequence_node(
        vec![nested_before, nested_positional_slot, nested_after],
        None,
    );
    let nested_template_id = builder.finish_template(
        nested_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    );
    let nested_child = builder.push_child_template_node(nested_template_id, None);
    builder.finish_template(
        nested_child,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    )
}

fn build_expression_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    expressions: &mut AstExpressionBuilder,
) -> (TemplateIrId, ExpressionSiteId) {
    let expression_site_id = store.next_expression_site_id();
    let expression_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: bool_reference_expression(expressions, path_fork, string_table, "original"),
            origin: TemplateSegmentOrigin::Body,
            site_id: expression_site_id,
        },
        None,
    ));
    let mut builder = TemplateIrBuilder::new(store);
    let slot_node = builder.push_slot_node(SlotKey::Default, None);
    let root = builder.push_sequence_node(vec![expression_node, slot_node], None);
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
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
    let named_slot = builder.push_slot_node(SlotKey::Named(named_slot_name), None);
    builder.finish_template(
        named_slot,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
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
        None,
    );
    let slot_node = builder.push_slot_node(SlotKey::Default, None);
    let after_node = builder.push_text_node(
        string_table.intern(after),
        after.len(),
        TemplateSegmentOrigin::Body,
        None,
    );
    let root = builder.push_sequence_node(vec![before_node, slot_node, after_node], None);
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
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
            None,
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
        None,
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
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let template_id = text_template(&mut store.borrow_mut(), &mut strings, "hello");
    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, template_id, TemplateViewContext::default());
        handoff_for_view(view, &expressions, &const_values, &strings)
            .expect("text handoff should succeed")
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
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let source = "hello owned handoff text";
    let template_id = {
        let mut store_ref = store.borrow_mut();
        text_template(&mut store_ref, &mut strings, source)
    };
    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, template_id, TemplateViewContext::default());
        handoff_for_view(view, &expressions, &const_values, &strings)
            .expect("text handoff should succeed")
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
    let resource_id = resources.intern_origin(resource_origin.clone(), None);
    let before = strings.intern("before");
    let after = strings.intern("after");
    let structural_expression = structural_string_expression(
        &mut expressions,
        &mut const_values,
        vec![
            ConstStringPiece::Text(before),
            ConstStringPiece::Resource(resource_id),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(after),
        ],
        ValueMode::ImmutableOwned,
    );
    let site_id = store.borrow_mut().next_expression_site_id();
    let dynamic_node = store.borrow_mut().push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: structural_expression,
            origin: TemplateSegmentOrigin::Body,
            site_id,
        },
        None,
    ));
    let template_id = finish_text_template(&mut store.borrow_mut(), dynamic_node);

    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, template_id, TemplateViewContext::default());
        let prepared = prepared_runtime(&view);
        owned_runtime_template_handoff_for_prepared_view(
            &prepared,
            view,
            &expressions,
            &const_values,
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
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let (parent_id, view_context) = {
        let mut store_ref = store.borrow_mut();
        let source_id = text_template(&mut store_ref, &mut strings, "filled");
        let occurrence_id = store_ref.next_slot_occurrence_id();
        let slot = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Slot {
                placeholder: TirSlotPlaceholder::with_wrapper_sets(
                    SlotKey::Default,
                    occurrence_id,
                    None,
                    None,
                    false,
                ),
            },
            None,
        ));
        let summary = summarize_existing_root(&store_ref, slot).expect("slot root is acyclic");
        let parent_id = store_ref.push_template(TemplateIr::new(
            slot,
            Style::default(),
            TemplateType::StringFunction,
            summary,
            None,
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
        handoff_for_view(view, &expressions, &const_values, &strings)
            .expect("slot handoff should succeed")
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
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let (parent_id, _occurrence_id, view_context) = {
        let mut store_ref = store.borrow_mut();
        let occurrence_id = store_ref.next_slot_occurrence_id();
        let slot = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Slot {
                placeholder: TirSlotPlaceholder::with_wrapper_sets(
                    SlotKey::Default,
                    occurrence_id,
                    None,
                    None,
                    false,
                ),
            },
            None,
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
        handoff_for_view(view, &expressions, &const_values, &StringTable::new())
            .expect("handoff materialization should succeed")
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
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
            None,
        ));
        let summary =
            summarize_existing_root(&store_ref, child_node).expect("child root is acyclic");
        store_ref.push_template(TemplateIr::new(
            child_node,
            Style::default(),
            TemplateType::StringFunction,
            summary,
            None,
        ))
    };
    let handoff = {
        let store_ref = store.borrow();
        let view = view_for(&store_ref, parent_id, TemplateViewContext::default());
        handoff_for_view(view, &expressions, &const_values, &strings)
            .expect("child handoff should succeed")
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (parent_id, _child_site_id, context) = {
        let mut store_ref = store.borrow_mut();
        let child_context = TemplateViewContext::default();
        let child_site_id = store_ref.next_expression_site_id();
        let child_expression =
            bool_reference_expression(&mut expressions, &mut path_fork, &mut strings, "original");
        let child_root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: child_expression,
                origin: TemplateSegmentOrigin::Body,
                site_id: child_site_id,
            },
            None,
        ));
        let child_template_id = finish_text_template(&mut store_ref, child_root);
        let child_node = child_template_node_id(
            &mut store_ref,
            child_reference(child_template_id, child_context),
        );
        let parent_id = finish_text_template(&mut store_ref, child_node);
        let override_expression = expressions
            .bool(&mut const_values, true, None, ValueMode::ImmutableOwned)
            .expect("child overlay literal should construct")
            .id();
        let context =
            expression_overlay_context(&mut store_ref, vec![(child_site_id, override_expression)]);
        (parent_id, child_site_id, context)
    };

    let body = materialize_parent_handoff(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    );
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
        bool_literal_value(&expressions, &const_values, *expression) == Some(true),
        "parent root override should win over the child's empty overlay"
    );
}

#[test]
fn prepared_handoff_preserves_structural_and_nested_value_contexts_after_tir_release() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let weak_store = Rc::downgrade(&store);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = path_fork
        .try_intern_portable_path("main.moth", &mut strings)
        .expect("fixture scope fits");

    let completed_expression = {
        let mut store_ref = store.borrow_mut();
        let [stale_text, parent_text, nested_text] =
            ["stale-structural", "effective-parent", "effective-nested"].map(|text| {
                let text = strings.intern(text);
                expressions
                    .string_slice(&mut const_values, text, None, ValueMode::ImmutableOwned)
                    .expect("fixture string fits")
                    .id()
            });
        let false_selector = expressions
            .bool(&mut const_values, false, None, ValueMode::ImmutableOwned)
            .expect("structural selector fits")
            .id();
        let true_selector = expressions
            .bool(&mut const_values, true, None, ValueMode::ImmutableOwned)
            .expect("nested selector fits")
            .id();
        let parent_selector = bool_reference_expression(
            &mut expressions,
            &mut path_fork,
            &mut strings,
            "parent_ready",
        );
        let text_site = store_ref.next_expression_site_id();
        let selector_site = store_ref.next_expression_site_id();
        let slot_occurrence = store_ref.next_slot_occurrence_id();
        let child_occurrence = store_ref.next_child_template_occurrence_id();
        let child_id = text_template(&mut store_ref, &mut strings, "child");
        let dynamic_text = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: stale_text,
                origin: TemplateSegmentOrigin::Body,
                site_id: text_site,
            },
            None,
        ));
        let selected_body = text_node_id(&mut store_ref, &mut strings, "selected-body");
        let conditional = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Conditional {
                selector: Box::new(TemplateBranchSelector::Bool(false_selector)),
                selector_site_id: selector_site,
                body: selected_body,
            },
            None,
        ));
        let slot = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Slot {
                placeholder: TirSlotPlaceholder::with_wrapper_sets(
                    SlotKey::Default,
                    slot_occurrence,
                    None,
                    None,
                    false,
                ),
            },
            None,
        ));
        let child = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: child_reference(child_id, TemplateViewContext::default()),
                occurrence_id: child_occurrence,
            },
            None,
        ));
        // Both accepted contexts remain runtime values even though their visible constant
        // text and selector differ, so both use the real runtime handoff contract.
        let runtime_expression = bool_reference_expression(
            &mut expressions,
            &mut path_fork,
            &mut strings,
            "content_ready",
        );
        let runtime_site = store_ref.next_expression_site_id();
        let runtime = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: runtime_expression,
                origin: TemplateSegmentOrigin::Body,
                site_id: runtime_site,
            },
            None,
        ));
        let shared_root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![dynamic_text, conditional, slot, child, runtime],
            },
            None,
        ));
        let shared_template = finish_text_template(&mut store_ref, shared_root);

        // The same structural root has two complete value contexts. Losing any nested
        // dimension changes text, the branch, the slot source or the child wrapper.
        let mut contexts = Vec::new();
        for (text, selector, slot_text, before, after) in [
            (
                parent_text,
                parent_selector,
                "parent-slot",
                "<parent>",
                "</parent>",
            ),
            (
                nested_text,
                true_selector,
                "nested-slot",
                "<nested>",
                "</nested>",
            ),
        ] {
            let mut context = expression_overlay_context(
                &mut store_ref,
                vec![(text_site, text), (selector_site, selector)],
            );
            let source = text_template(&mut store_ref, &mut strings, slot_text);
            context.slot_resolution = slot_resolution_context(
                &mut store_ref,
                vec![(
                    slot_occurrence,
                    TirSlotResolution::resolved(SlotKey::Default, vec![source]),
                )],
            )
            .slot_resolution;
            let wrapper = build_slot_wrapper_template(&mut store_ref, &mut strings, before, after);
            let wrapper_set =
                store_ref.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
                    wrapper,
                    TemplateTirPhase::Finalized,
                    TemplateViewContext::default(),
                )]);
            context.wrapper_context = Some(
                store_ref
                    .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
                        contexts: vec![(
                            child_occurrence,
                            TirWrapperContext::inherited(wrapper_set),
                        )],
                    })
                    .expect("wrapper context fits"),
            );
            contexts.push(context);
        }
        let parent_context = contexts[0];
        let nested_context = contexts[1];
        let nested_template = Template {
            tir_reference: TemplateTirReference {
                root: shared_template,
                phase: TemplateTirPhase::Finalized,
                context: nested_context,
            },
            span: None,
        };
        let nested_expression = expressions
            .template(nested_template, ValueMode::ImmutableOwned)
            .expect("nested template expression fits")
            .id();
        let structural_child = child_template_node_id(
            &mut store_ref,
            child_reference(
                shared_template,
                TemplateViewContext {
                    expression_overlay: nested_context.expression_overlay,
                    ..parent_context
                },
            ),
        );
        let nested_site = store_ref.next_expression_site_id();
        let nested_value = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: nested_expression,
                origin: TemplateSegmentOrigin::Body,
                site_id: nested_site,
            },
            None,
        ));
        let root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![structural_child, nested_value],
            },
            None,
        ));
        let root_id = finish_text_template(&mut store_ref, root);
        let root_template = Template {
            tir_reference: TemplateTirReference {
                root: root_id,
                phase: TemplateTirPhase::Finalized,
                context: parent_context,
            },
            span: None,
        };
        let root_expression = expressions
            .template(root_template, ValueMode::ImmutableOwned)
            .expect("root template expression fits")
            .id();
        let view = view_for(&store_ref, root_id, parent_context);
        let prepared = prepare_tir_view(
            &view,
            &expressions,
            &const_values,
            TemplatePreparationMode::Value,
        )
        .expect("both structural and nested contexts prepare");
        assert_eq!(prepared.identity, view.identity());
        assert!(matches!(
            prepared.outcome,
            TemplatePreparationOutcome::Runtime(_)
        ));

        let changed_view = view_for(&store_ref, root_id, nested_context);
        let error = owned_runtime_template_handoff_for_prepared_view(
            &prepared,
            changed_view,
            &expressions,
            &const_values,
            &strings,
            None,
        )
        .expect_err("accepted preparation cannot authorise a changed context");
        assert_eq!(error.error_type, ErrorType::Compiler);
        assert!(error.msg.contains("identity does not match"));

        let mut handoff = owned_runtime_template_handoff_for_prepared_view(
            &prepared,
            view.clone(),
            &expressions,
            &const_values,
            &strings,
            None,
        )
        .expect("handoff consumes the same accepted view without rebuilding its overlay");
        let ExpressionOperationView::Template(nested_template) = expressions
            .view(nested_expression)
            .expect("nested value exists")
            .operation()
        else {
            panic!("nested value must still have its own TIR context before projection");
        };
        let nested_view = view
            .nested_template_value(nested_template.tir_reference)
            .expect("nested value enters its complete context");
        let nested_prepared = prepare_tir_view(
            &nested_view,
            &expressions,
            &const_values,
            TemplatePreparationMode::Value,
        )
        .expect("nested exact view prepares");
        assert!(matches!(
            nested_prepared.outcome,
            TemplatePreparationOutcome::Runtime(_)
        ));
        let nested_handoff = owned_runtime_template_handoff_for_prepared_view(
            &nested_prepared,
            nested_view,
            &expressions,
            &const_values,
            &strings,
            None,
        )
        .expect("nested accepted view materialises its own context");

        // Model the completed-expression projection used after accepted-view materialisation,
        // not a second template interpretation or a private module-finaliser entry point.
        let nested_input = replacement_input(
            expressions
                .view(nested_expression)
                .expect("nested occurrence exists"),
            CompletedOperation::RuntimeTemplate(nested_handoff),
        );
        let completed_nested = promote_replacement(&mut expressions, nested_input)
            .expect("nested neutral payload commits")
            .id();
        handoff.visit_expression_ids_mut(&mut |expression| {
            if *expression == nested_expression {
                *expression = completed_nested;
            }
        });
        let root_input = replacement_input(
            expressions
                .view(root_expression)
                .expect("root occurrence exists"),
            CompletedOperation::RuntimeTemplate(handoff),
        );
        promote_replacement(&mut expressions, root_input)
            .expect("root neutral payload commits")
            .id()
    };

    // End every TIR reader before freezing. A sole Rc release proves that no hidden
    // preparation or handoff owner retains the module's construction graph.
    let released_store = Rc::try_unwrap(store)
        .expect("last TIR reader has ended")
        .into_inner();
    assert!(weak_store.upgrade().is_none());
    drop(released_store);
    let mut roots = vec![AstNode {
        kind: NodeKind::ExpressionStatement(completed_expression),
        span: None,
        scope,
    }];
    let frozen = expressions
        .freeze(&mut roots)
        .expect("only neutral completed expressions survive");
    let NodeKind::ExpressionStatement(root) = roots[0].kind else {
        panic!("completed root remains an expression statement");
    };
    let ExpressionOperationView::RuntimeTemplate(handoff) = frozen
        .view(root)
        .expect("root is paired with frozen owner")
        .operation()
    else {
        panic!("completed root must contain a neutral runtime payload");
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children, .. }) =
        &handoff.body
    else {
        panic!("root keeps its structural and nested value positions");
    };
    let OwnedRuntimeTemplateNode::ChildTemplate { template, .. } = &children[0] else {
        panic!("first position remains a structural child");
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
        children: structural,
        ..
    }) = &template.body
    else {
        panic!("structural child keeps its accepted body");
    };
    let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = &structural[0] else {
        panic!("parent text is an expression leaf");
    };
    let ExpressionOperationView::Constant(value) = frozen.view(*expression).unwrap().operation()
    else {
        panic!("parent text remains a constant expression");
    };
    assert_eq!(
        strings.resolve(const_values.string_value(value).unwrap()),
        "effective-parent"
    );
    let OwnedRuntimeTemplateNode::Conditional { selector, body, .. } = &structural[1] else {
        panic!("parent branch remains lazy");
    };
    let ExpressionOperationView::Reference(path) = frozen
        .view(selector.condition_expression())
        .unwrap()
        .operation()
    else {
        panic!("parent branch must use the overlaid runtime selector");
    };
    assert_eq!(
        path_fork.render_native(path, &strings, &mut Vec::new()),
        PathBuf::from("parent_ready"),
    );
    assert_owned_text_node(body, "selected-body");
    assert_owned_text_node(&structural[2], "parent-slot");
    let OwnedRuntimeTemplateNode::Sequence {
        children: wrapper, ..
    } = &structural[3]
    else {
        panic!("parent wrapper remains materialised");
    };
    assert_owned_text_node(&wrapper[0], "<parent>");
    assert_owned_text_node(&wrapper[1], "child");
    assert_owned_text_node(&wrapper[2], "</parent>");

    let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = &children[1] else {
        panic!("second position remains an independently owned nested expression");
    };
    let ExpressionOperationView::RuntimeTemplate(nested) = frozen
        .view(*expression)
        .expect("nested ID is paired with frozen owner")
        .operation()
    else {
        panic!("nested value must not retain a build-only Template after TIR release");
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
        children: nested,
        ..
    }) = &nested.body
    else {
        panic!("nested value keeps its own accepted body");
    };
    let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = &nested[0] else {
        panic!("nested text is an expression leaf");
    };
    let ExpressionOperationView::Constant(value) = frozen.view(*expression).unwrap().operation()
    else {
        panic!("nested text remains a constant expression");
    };
    assert_eq!(
        strings.resolve(const_values.string_value(value).unwrap()),
        "effective-nested"
    );
    let OwnedRuntimeTemplateNode::Conditional { selector, body, .. } = &nested[1] else {
        panic!("nested branch keeps its own selector");
    };
    let ExpressionOperationView::Constant(value) = frozen
        .view(selector.condition_expression())
        .unwrap()
        .operation()
    else {
        panic!("nested branch must not inherit the parent runtime selector");
    };
    assert!(matches!(
        const_values.payload(value),
        Some(ConstValuePayload::Bool(true))
    ));
    assert_owned_text_node(body, "selected-body");
    assert_owned_text_node(&nested[2], "nested-slot");
    let OwnedRuntimeTemplateNode::Sequence {
        children: wrapper, ..
    } = &nested[3]
    else {
        panic!("nested wrapper remains materialised");
    };
    assert_owned_text_node(&wrapper[0], "<nested>");
    assert_owned_text_node(&wrapper[1], "child");
    assert_owned_text_node(&wrapper[2], "</nested>");
    let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = &nested[4] else {
        panic!("nested runtime leaf survives alongside its own constant facts");
    };
    let ExpressionOperationView::Reference(path) = frozen
        .view(*expression)
        .expect("nested runtime leaf belongs to frozen owner")
        .operation()
    else {
        panic!("nested runtime dependence remains a reference");
    };
    assert_eq!(
        path_fork.render_native(path, &strings, &mut Vec::new()),
        PathBuf::from("content_ready"),
    );
}

#[test]
fn runtime_child_reference_uses_structural_handoff() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let child_site_id = store_ref.next_expression_site_id();
        let child_expression =
            bool_reference_expression(&mut expressions, &mut path_fork, &mut strings, "runtime");
        let child_root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: child_expression,
                origin: TemplateSegmentOrigin::Body,
                site_id: child_site_id,
            },
            None,
        ));
        let child_template_id = finish_text_template(&mut store_ref, child_root);
        let child_node = child_template_node_id(
            &mut store_ref,
            child_reference(child_template_id, empty_context),
        );
        let parent_id = finish_text_template(&mut store_ref, child_node);
        (parent_id, empty_context)
    };

    let body = materialize_parent_handoff_result(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    )
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
    let ExpressionOperationView::Reference(path) = expressions
        .view(*expression)
        .expect("runtime reference expression should exist")
        .operation()
    else {
        panic!("expected an owned reference expression, got {expression:?}");
    };
    let mut scratch = Vec::new();
    assert_eq!(
        path_fork.render_native(path, &strings, &mut scratch),
        PathBuf::from("runtime"),
        "runtime reference value should survive structural handoff"
    );
}

#[test]
fn missing_child_node_authority_propagates_as_compiler_error_through_hir_handoff() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let missing_child_root = TemplateIrNodeId::new(999);
        let child_template_id = store_ref.push_template(TemplateIr::new(
            missing_child_root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::empty(),
            None,
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
            None,
        ));
        (parent_id, empty_context)
    };

    let error = materialize_parent_handoff_result(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    )
    .expect_err("malformed child authority must reach the HIR handoff caller");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("TirView::effective_node")
            && error.msg.contains("TemplateIrNodeId(999)"),
        "expected child node authority context, got: {}",
        error.msg
    );
}

// ---------------------------------------------------------------------------
//  Inherited wrapper handoff
// ---------------------------------------------------------------------------

#[test]
fn inherited_wrapper_handoff_preserves_conditional_body() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let wrapper_template_id =
            build_conditional_wrapper_template(&mut store_ref, &mut expressions, &mut const_values);
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let body = materialize_parent_handoff(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    );
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Conditional {
        selector,
        body,
        ..
    }) = body
    else {
        panic!("expected conditional handoff, got {:?}", body);
    };

    assert!(matches!(
        selector.as_ref(),
        TemplateBranchSelector::Bool(expression)
            if bool_literal_value(&expressions, &const_values, *expression) == Some(true)
    ));
    let OwnedRuntimeTemplateNode::Sequence { children, .. } = body.as_ref() else {
        panic!("expected conditional body sequence, got {body:?}");
    };
    assert_eq!(children.len(), 2);
    assert!(
        matches!(&children[0], OwnedRuntimeTemplateNode::Slot { .. }),
        "the leading default slot should remain unfilled"
    );
    assert_owned_text_node(&children[1], "child");
}

#[test]
fn conditional_handoff_materializes_the_effective_selector_overlay() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut store = TemplateIrStore::new();
    let mut strings = StringTable::new();
    let (template_id, selector_site_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_text_node(
            strings.intern("conditional body"),
            "conditional body".len(),
            TemplateSegmentOrigin::Body,
            None,
        );
        let conditional = builder.push_conditional_node(
            TemplateBranchSelector::Bool(
                expressions
                    .bool(&mut const_values, true, None, ValueMode::ImmutableOwned)
                    .expect("conditional selector literal should construct")
                    .id(),
            ),
            body,
            None,
        );
        let selector_site_id = match &builder
            .store
            .get_node(conditional)
            .expect("conditional node should exist")
            .kind
        {
            TemplateIrNodeKind::Conditional {
                selector_site_id, ..
            } => *selector_site_id,
            other => panic!("expected conditional TIR node, got {other:?}"),
        };
        let template_id = builder.finish_template(
            conditional,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::empty(),
            None,
        );
        (template_id, selector_site_id)
    };
    let override_expression = expressions
        .bool(&mut const_values, false, None, ValueMode::ImmutableOwned)
        .expect("selector overlay literal should construct")
        .id();
    let context =
        expression_overlay_context(&mut store, vec![(selector_site_id, override_expression)]);

    let body = materialize_parent_handoff_result(
        Rc::new(RefCell::new(store)),
        template_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    )
    .expect("conditional selector overlay should materialize");

    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Conditional {
        selector,
        body,
        ..
    }) = body
    else {
        panic!("expected one conditional runtime node, got {body:?}");
    };
    assert!(matches!(
        selector.as_ref(),
        TemplateBranchSelector::Bool(expression)
            if bool_literal_value(&expressions, &const_values, *expression) == Some(false)
    ));
    assert_owned_text_node(body.as_ref(), "conditional body");
}

#[test]
fn inherited_wrapper_handoff_injects_through_loop_body_and_aggregate() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let wrapper_template_id = build_loop_wrapper_template(
            &mut store_ref,
            &mut strings,
            &mut expressions,
            &mut const_values,
        );
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let body = materialize_parent_handoff(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    );
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
    let OwnedRuntimeTemplateNode::Sequence { children, .. } = aggregate_wrapper.as_ref() else {
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
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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

    let body = materialize_parent_handoff(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    );
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ChildTemplate {
        template, ..
    }) = body
    else {
        panic!("expected child wrapper handoff, got {:?}", body);
    };
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children, .. }) =
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let (wrapper_template_id, expression_site_id) = build_expression_wrapper_template(
            &mut store_ref,
            &mut strings,
            &mut path_fork,
            &mut expressions,
        );
        let override_expression = expressions
            .bool(&mut const_values, true, None, ValueMode::ImmutableOwned)
            .expect("wrapper overlay literal should construct")
            .id();
        let wrapper_context = expression_overlay_context(
            &mut store_ref,
            vec![(expression_site_id, override_expression)],
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

    let body = materialize_parent_handoff(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    );
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children, .. }) =
        body
    else {
        panic!("expected wrapper sequence, got {:?}", body);
    };

    let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = &children[0] else {
        panic!("expected wrapper expression, got {:?}", children[0]);
    };
    assert!(
        bool_literal_value(&expressions, &const_values, *expression) == Some(true),
        "wrapper overlay should override the wrapper expression"
    );
    let OwnedRuntimeTemplateNode::ChildTemplate { template, .. } = &children[1] else {
        panic!("expected child handoff, got {:?}", children[1]);
    };
    let OwnedRuntimeTemplateBody::Render(child_body) = &template.body else {
        panic!("expected rendered child handoff, got {:?}", template.body);
    };
    assert_owned_text_node(child_body, "child");
}

#[test]
fn inherited_wrapper_handoff_applies_wrapper_set_innermost_to_outermost() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    // A single inherited wrapper set holding two distinct wrappers must hand off
    // as `outer(inner(child))`. `TemplateWrapperSet::wrappers` is stored
    // innermost-to-outermost, so forward handoff consumption applies the innermost
    // wrapper directly around the child and the outermost wrapper last.
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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

    let body = materialize_parent_handoff(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    );
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children, .. }) =
        body
    else {
        panic!("expected outer wrapper sequence, got {body:?}");
    };
    assert_eq!(children.len(), 3);
    assert_owned_text_node(&children[0], "outer-before");
    assert_owned_text_node(&children[2], "outer-after");

    let OwnedRuntimeTemplateNode::Sequence {
        children: inner_children,
        ..
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
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    // The IfChildEmits aggregate-wrapper path must also consume the
    // innermost-to-outermost store order forward, producing a
    // `ConditionalWrapper` whose wrapper tree is `outer(inner(AggregateOutput))`.
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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

    let body = materialize_parent_handoff(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    );
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

    let OwnedRuntimeTemplateNode::Sequence { children, .. } = wrapper.as_ref() else {
        panic!("expected outer wrapper sequence, got {:?}", wrapper);
    };
    assert_eq!(children.len(), 3);
    assert_owned_text_node(&children[0], "outer-before");
    assert_owned_text_node(&children[2], "outer-after");

    let OwnedRuntimeTemplateNode::Sequence {
        children: inner_children,
        ..
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
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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

    let body = materialize_parent_handoff(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    );
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children, .. }) =
        body
    else {
        panic!("expected slotless wrapper sequence, got {:?}", body);
    };

    assert_eq!(children.len(), 2);
    assert_owned_text_node(&children[0], "slotless-wrapper");
    assert_owned_text_node(&children[1], "child");
}

#[test]
fn inherited_named_only_wrapper_handoff_preserves_named_slot_and_appends_child() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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

    let body = materialize_parent_handoff(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    );
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { children, .. }) =
        body
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
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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

    let error = materialize_parent_handoff_result(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        valid_context,
    )
    .expect_err("malformed child overlay should produce a CompilerError");

    assert!(
        error.msg.contains("slot resolution overlay"),
        "expected error about missing slot resolution overlay, got: {}",
        error.msg
    );
}

#[test]
fn missing_wrapper_tree_node_propagates_layout_error() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let (parent_id, context) = {
        let mut store_ref = store.borrow_mut();
        let empty_context = TemplateViewContext::default();
        let slot_occurrence_id = store_ref.next_slot_occurrence_id();
        let slot = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Slot {
                placeholder: TirSlotPlaceholder::with_wrapper_sets(
                    SlotKey::Default,
                    slot_occurrence_id,
                    None,
                    None,
                    false,
                ),
            },
            None,
        ));
        let wrapper_root = store_ref.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![slot, TemplateIrNodeId::new(9999)],
            },
            None,
        ));
        let wrapper_template_id = store_ref.push_template(TemplateIr::new(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        ));
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let error = materialize_parent_handoff_result(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    )
    .expect_err("missing wrapper tree node should produce a CompilerError");

    assert!(
        error.msg.contains("TIR slot layout requested missing node"),
        "expected layout-owned node error, got: {}",
        error.msg
    );
}

#[test]
fn missing_child_in_wrapper_propagates_layout_error() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
            None,
        ));
        let wrapper_template_id = store_ref.push_template(TemplateIr::new(
            missing_child_node,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::empty(),
            None,
        ));
        build_parent_with_inherited_wrapper(
            &mut store_ref,
            wrapper_template_id,
            empty_context,
            &mut strings,
        )
    };

    let error = materialize_parent_handoff_result(
        store,
        parent_id,
        &expressions,
        &const_values,
        &mut strings,
        context,
    )
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
        None,
    ));
    let template_id = store.push_template(TemplateIr::new(
        runtime_site,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::empty(),
        None,
    ));
    store
        .attach_runtime_slot_plan(template_id, plan)
        .expect("runtime slot plan should attach");
    template_id
}

#[test]
fn runtime_slot_handoff_preserves_root_conditional_wrapper() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut store = TemplateIrStore::new();
    let mut strings = StringTable::new();
    let slot_render_root = text_node_id(&mut store, &mut strings, "slot-render");
    let slot_plan = store.push_slot_plan(TemplateSlotPlan {
        contribution_sources: Vec::new(),
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root: slot_render_root,
            span: None,
        }],
    });
    let template_id = runtime_site_template(&mut store, slot_plan, RuntimeSlotSiteId(0));

    let wrapper_template_id = build_slot_wrapper_template(
        &mut store,
        &mut strings,
        "conditional-before",
        "conditional-after",
    );
    let wrapper_set = store.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
        wrapper_template_id,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )]);
    store
        .set_conditional_child_wrapper_set(template_id, wrapper_set)
        .expect("conditional wrapper set should attach");

    let view = view_for(&store, template_id, TemplateViewContext::default());
    let prepared = prepared_runtime(&view);

    assert!(
        owned_runtime_slot_handoff_for_prepared_view(
            &prepared,
            view,
            &expressions,
            &const_values,
            &strings,
            None
        )
        .expect("direct slot handoff should succeed")
        .is_none(),
        "direct slot handoff cannot carry the root's conditional wrapper"
    );

    let view = view_for(&store, template_id, TemplateViewContext::default());
    let handoff = owned_runtime_template_handoff_for_prepared_view(
        &prepared,
        view,
        &expressions,
        &const_values,
        &strings,
        None,
    )
    .expect("generic handoff should preserve the conditional wrapper");
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::ConditionalWrapper {
        child,
        wrapper,
        ..
    }) = handoff.body
    else {
        panic!("expected one conditional wrapper around the slot application");
    };

    let OwnedRuntimeTemplateNode::ChildTemplate { template, .. } = child.as_ref() else {
        panic!("conditional wrapper should retain the slot application as its child");
    };
    let OwnedRuntimeTemplateBody::RuntimeSlotApplication(application) = &template.body else {
        panic!("conditional wrapper child should preserve the runtime slot application");
    };
    assert_eq!(application.slot_sites.len(), 1);
    assert_owned_text_node(&application.slot_sites[0].render_root, "slot-render");

    let OwnedRuntimeTemplateNode::Sequence { children, .. } = wrapper.as_ref() else {
        panic!("expected the conditional wrapper template's sequence, got {wrapper:?}");
    };
    assert_eq!(children.len(), 3);
    assert_owned_text_node(&children[0], "conditional-before");
    assert!(
        matches!(children[1], OwnedRuntimeTemplateNode::AggregateOutput),
        "the wrapper's default slot should be the aggregate-output splice"
    );
    assert_owned_text_node(&children[2], "conditional-after");
}

#[test]
fn handoff_rejects_runtime_slot_site_from_a_different_plan() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut store = TemplateIrStore::new();
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let render_root = text_node_id(&mut store, &mut strings, "site");
    let owner_plan = store.push_slot_plan(TemplateSlotPlan {
        contribution_sources: Vec::new(),
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root,
            span: None,
        }],
    });
    let other_plan = store.push_slot_plan(TemplateSlotPlan {
        contribution_sources: Vec::new(),
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root,
            span: None,
        }],
    });
    let template_id = runtime_site_template(&mut store, other_plan, RuntimeSlotSiteId(0));
    store
        .attach_runtime_slot_plan(template_id, owner_plan)
        .expect("owner plan should replace the forged plan");

    let view = view_for(&store, template_id, TemplateViewContext::default());
    let error = handoff_for_view(view, &expressions, &const_values, &strings)
        .expect_err("wrong-plan site must fail at handoff");
    assert!(error.msg.contains("outside its owning slot application"));
}

#[test]
fn handoff_rejects_out_of_range_runtime_slot_site() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut store = TemplateIrStore::new();
    let plan = store.push_slot_plan(TemplateSlotPlan {
        contribution_sources: Vec::new(),
        slot_sites: Vec::new(),
    });
    let template_id = runtime_site_template(&mut store, plan, RuntimeSlotSiteId(0));

    let view = view_for(&store, template_id, TemplateViewContext::default());
    let error = handoff_for_view(view, &expressions, &const_values, &StringTable::new())
        .expect_err("out-of-range site must fail at handoff");
    assert!(error.msg.contains("out-of-range runtime slot site"));
}

#[test]
fn handoff_rejects_mismatched_runtime_slot_site_identity() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut store = TemplateIrStore::new();
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let render_root = text_node_id(&mut store, &mut strings, "site");
    let plan = store.push_slot_plan(TemplateSlotPlan {
        contribution_sources: Vec::new(),
        slot_sites: vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(0),
            key: SlotKey::Default,
            render_root,
            span: None,
        }],
    });
    MalformedTirStore::new(&mut store).replace_slot_sites(
        plan,
        vec![TemplateSlotSitePlan {
            site: RuntimeSlotSiteId(7),
            key: SlotKey::Default,
            render_root,
            span: None,
        }],
    );
    let template_id = runtime_site_template(&mut store, plan, RuntimeSlotSiteId(0));

    let view = view_for(&store, template_id, TemplateViewContext::default());
    let error = handoff_for_view(view, &expressions, &const_values, &strings)
        .expect_err("mismatched site identity must fail");
    assert!(
        error
            .msg
            .contains("slot site whose stored identity does not match")
    );
}

#[test]
fn handoff_rejects_mismatched_loop_header_shape() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut store = TemplateIrStore::new();
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let body = text_node_id(&mut store, &mut strings, "body");
    let loop_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Loop {
            header: TemplateLoopHeader::Conditional {
                condition: expressions
                    .bool(&mut const_values, false, None, ValueMode::ImmutableOwned)
                    .expect("loop header condition literal should construct")
                    .id(),
            },
            header_sites: TemplateLoopHeaderExpressionSites::Collection {
                iterable: ExpressionSiteId::new(0),
            },
            body,
            aggregate_wrapper: None,
        },
        None,
    ));
    let template_id = finish_text_template(&mut store, loop_node);
    let view = view_for(&store, template_id, TemplateViewContext::default());

    let error = handoff_for_view(view, &expressions, &const_values, &strings)
        .expect_err("loop header shape mismatch must fail closed");
    assert!(
        error
            .msg
            .contains("loop header shape does not match its expression sites")
    );
}

/// Exact-view child cycles must fail before owned-handoff recursion.
#[test]
fn handoff_rejects_exact_view_child_cycle() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
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
        None,
    ));
    assert_eq!(actual_id, template_id);

    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("cyclic view should construct");

    let error = handoff_for_view(view, &expressions, &const_values, &StringTable::new())
        .expect_err("exact-view child cycles must fail before handoff recursion");
    assert_eq!(error.error_type, ErrorType::Compiler);
}
