//! TIR wrapper-context overlay fold and handoff tests.
//!
//! WHAT: verifies that inherited child wrappers are read from the shared
//! module store during view folding and HIR handoff, and that the wrapper-
//! context overlay dimensions (expression overrides, slot resolution,
//! `$fresh` suppression, `IfChildEmits` mode) interact correctly with the
//! fold and handoff paths.
//! WHY: wrapper context is a TIR view dimension and must not require a second
//! store or a structural fallback.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateBranchSelector;
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult, TirFoldContext,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrBuilder;
use crate::compiler_frontend::ast::templates::tir::TemplateSlotPlan;
use crate::compiler_frontend::ast::templates::tir::fold::fold_prepared_template;
use crate::compiler_frontend::ast::templates::tir::fold_cache::TirFoldCache;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId, TemplateIrId,
    TemplateWrapperSetId,
};
use crate::compiler_frontend::ast::templates::tir::node::{TemplateIrBranch, TemplateIrNodeKind};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirExpressionOverlayId, TirSlotResolution,
    TirSlotResolutionOverlay, TirWrapperApplicationMode, TirWrapperContext,
    TirWrapperContextOverlay,
};
use crate::compiler_frontend::ast::templates::tir::preparation::{
    RuntimeTemplateReason, TemplatePreparation, TemplatePreparationFacts, TemplatePreparationMode,
    TemplatePreparationOutcome,
};
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::store::{
    MalformedTirStore, TemplateIrStore, TemplateWrapperSet,
};
use crate::compiler_frontend::ast::templates::tir::summary::TemplateIrSummary;
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, TirView};
use crate::compiler_frontend::ast::templates::tir::{
    owned_runtime_template_handoff_for_prepared_view, prepare_tir_view,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;

fn fold_context<'a>(string_table: &'a mut StringTable) -> TirFoldContext<'a> {
    TirFoldContext {
        string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
        fold_cache: TirFoldCache::new(),
    }
}

fn bool_expression(value: bool) -> Expression {
    Expression::bool(value, SourceLocation::default(), ValueMode::ImmutableOwned)
}

/// Builds a runtime (non-const) string reference expression.
fn runtime_string_expression() -> Expression {
    Expression::new(
        ExpressionKind::Reference(InternedPath::new()),
        SourceLocation::default(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    )
}

fn build_text_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrId {
    let text_id = string_table.intern(text);
    let mut builder = TemplateIrBuilder::new(store);
    let text_node = builder.push_text_node(
        text_id,
        text.len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let root = builder.push_sequence_node(vec![text_node], SourceLocation::default());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    )
}

/// Builds a `before $slot after` wrapper template with one default slot.
fn build_slot_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    before: &str,
    after: &str,
) -> TemplateIrId {
    let before_id = string_table.intern(before);
    let after_id = string_table.intern(after);
    let mut builder = TemplateIrBuilder::new(store);
    let before_node = builder.push_text_node(
        before_id,
        before.len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let slot_node = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let after_node = builder.push_text_node(
        after_id,
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

/// Builds a wrapper template with a default slot and a named slot, returning
/// the template ID, the named slot occurrence ID, and the named slot key.
fn build_two_slot_wrapper_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> (TemplateIrId, SlotOccurrenceId, SlotKey) {
    let before_id = string_table.intern("before");
    let named_id = string_table.intern("named");
    let after_id = string_table.intern("after");
    let named_key = SlotKey::Named(named_id);
    let mut builder = TemplateIrBuilder::new(store);
    let before_node = builder.push_text_node(
        before_id,
        "before".len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let default_slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let named_slot = builder.push_slot_node(named_key.clone(), SourceLocation::default());
    let after_node = builder.push_text_node(
        after_id,
        "after".len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let root = builder.push_sequence_node(
        vec![before_node, default_slot, named_slot, after_node],
        SourceLocation::default(),
    );
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    );
    let named_occurrence_id = match &store
        .get_node(named_slot)
        .expect("named slot node should exist")
        .kind
    {
        TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
        other => panic!("expected named slot node, got {other:?}"),
    };
    (template_id, named_occurrence_id, named_key)
}

/// Builds a wrapper template whose root is a single dynamic-expression node,
/// returning the template ID and the expression site ID.
fn build_expression_wrapper_template_with_expression(
    store: &mut TemplateIrStore,
    expression: Expression,
) -> (TemplateIrId, ExpressionSiteId) {
    let mut builder = TemplateIrBuilder::new(store);
    let dynamic_node = builder.push_dynamic_expression_node(
        expression,
        TemplateSegmentOrigin::Body,
        None,
        SourceLocation::default(),
    );
    let root = builder.push_sequence_node(vec![dynamic_node], SourceLocation::default());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    );
    let site_id = match &store
        .get_node(dynamic_node)
        .expect("expression wrapper node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected dynamic expression wrapper node, got {other:?}"),
    };
    (template_id, site_id)
}

/// Builds a branch template whose single branch has a false selector and no
/// fallback, so the template structurally emits no output.
fn build_false_no_else_branch_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
) -> TemplateIrId {
    let body_text = string_table.intern("hidden");
    let mut builder = TemplateIrBuilder::new(store);
    let body_node = builder.push_text_node(
        body_text,
        "hidden".len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(bool_expression(false)),
        body_node,
        SourceLocation::default(),
        builder.store.next_expression_site_id(),
    );
    let root = builder.push_branch_chain_node(vec![branch], None, SourceLocation::default());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            has_control_flow: true,
            ..TemplateIrSummary::empty()
        },
        SourceLocation::default(),
    )
}

/// Shared fixture for wrapper-context fold and handoff tests.
struct WrapperContextFixture {
    store: Rc<RefCell<TemplateIrStore>>,
    parent: TemplateIrId,
    /// The inherited wrapper template id, when the fixture built one.
    wrapper_template_id: Option<TemplateIrId>,
    wrapper_set_id: Option<TemplateWrapperSetId>,
    context: TemplateViewContext,
}

/// The parent view phase to use for fold/handoff. Expression overlays require
/// `Finalized` so the normalized payload is stable; otherwise `Composed`.
fn fixture_parent_view_phase(context: TemplateViewContext) -> TemplateTirPhase {
    let has_expression_overlay = context.expression_overlay.is_some();
    if has_expression_overlay {
        TemplateTirPhase::Finalized
    } else {
        TemplateTirPhase::Composed
    }
}

/// Builds a wrapper-context view context that inherits `wrapper_set_id` for
/// `child_occurrence_id`, layered with the supplied `wrapper_context` fields.
fn allocate_wrapper_context_overlay(
    store: &mut TemplateIrStore,
    wrapper_set_id: TemplateWrapperSetId,
    child_occurrence_id: ChildTemplateOccurrenceId,
    wrapper_context: TirWrapperContext,
) -> TemplateViewContext {
    let wrapper_context_overlay_id = store
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
    TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(wrapper_context_overlay_id),
    }
}

/// Builds a parent with one child occurrence wrapped by a `before $slot after`
/// wrapper through a wrapper-context overlay. The child template is built by
/// `build_child`, and the wrapper context fields come from `wrapper_context`.
fn build_wrapper_context_fixture(
    string_table: &mut StringTable,
    wrapper_context: TirWrapperContext,
    build_child: impl FnOnce(&mut TemplateIrStore, &mut StringTable) -> TemplateIrId,
) -> WrapperContextFixture {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let (parent, wrapper_template_id, wrapper_set_id, context) = {
        let mut tir = store.borrow_mut();
        let empty_overlay = TemplateViewContext::default();
        let child_template_id = build_child(&mut tir, string_table);
        let wrapper_template_id =
            build_slot_wrapper_template(&mut tir, string_table, "before", "after");

        let mut builder = TemplateIrBuilder::new(&mut tir);
        let child_node = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                child_template_id,
                TemplateTirPhase::Composed,
                empty_overlay,
            ),
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![child_node], SourceLocation::default());
        let parent = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        );

        let wrapper_ref = TemplateWrapperReference::new(
            wrapper_template_id,
            TemplateTirPhase::Finalized,
            empty_overlay,
        );
        let wrapper_set_id = tir.push_or_reuse_wrapper_set(vec![wrapper_ref]);
        let context = allocate_wrapper_context_overlay(
            &mut tir,
            wrapper_set_id,
            ChildTemplateOccurrenceId::new(0),
            wrapper_context,
        );
        (parent, wrapper_template_id, wrapper_set_id, context)
    };
    WrapperContextFixture {
        store,
        parent,
        wrapper_template_id: Some(wrapper_template_id),
        wrapper_set_id: Some(wrapper_set_id),
        context,
    }
}

/// Builds a parent with one child occurrence wrapped by an expression wrapper.
/// The wrapper's own view context carries an expression override on its site,
/// and the parent view context carries a wrapper-context overlay plus an
/// optional outer expression override on the same site.
fn build_expression_wrapper_fixture(
    string_table: &mut StringTable,
    wrapper_expression: Expression,
    outer_expression: Option<Expression>,
) -> (WrapperContextFixture, ExpressionSiteId) {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let (parent, wrapper_template_id, wrapper_set_id, site_id, context) = {
        let mut tir = store.borrow_mut();
        let empty_overlay = TemplateViewContext::default();
        let child_template_id = build_text_template(&mut tir, string_table, "child");
        let (wrapper_template_id, site_id) =
            build_expression_wrapper_template_with_expression(&mut tir, wrapper_expression.clone());

        let mut builder = TemplateIrBuilder::new(&mut tir);
        let child_node = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                child_template_id,
                TemplateTirPhase::Composed,
                empty_overlay,
            ),
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![child_node], SourceLocation::default());
        let parent = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        );

        let wrapper_expression_overlay_id = tir
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(site_id, Box::new(wrapper_expression))],
            })
            .expect("test overlay allocation");
        let wrapper_context = TemplateViewContext {
            expression_overlay: Some(wrapper_expression_overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        };

        let wrapper_ref = TemplateWrapperReference::new(
            wrapper_template_id,
            TemplateTirPhase::Finalized,
            wrapper_context,
        );
        let wrapper_set_id = tir.push_or_reuse_wrapper_set(vec![wrapper_ref]);

        let parent_context = if let Some(outer_expression) = outer_expression {
            let wrapper_context_overlay_id = tir
                .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
                    contexts: vec![(
                        ChildTemplateOccurrenceId::new(0),
                        TirWrapperContext::inherited(wrapper_set_id),
                    )],
                })
                .expect("test overlay allocation");
            let outer_expression_overlay_id = tir
                .allocate_expression_overlay(TirExpressionOverlay {
                    overrides: vec![(site_id, Box::new(outer_expression))],
                })
                .expect("test overlay allocation");
            TemplateViewContext {
                expression_overlay: Some(outer_expression_overlay_id),
                slot_resolution: None,
                wrapper_context: Some(wrapper_context_overlay_id),
            }
        } else {
            allocate_wrapper_context_overlay(
                &mut tir,
                wrapper_set_id,
                ChildTemplateOccurrenceId::new(0),
                TirWrapperContext::default(),
            )
        };
        (
            parent,
            wrapper_template_id,
            wrapper_set_id,
            site_id,
            parent_context,
        )
    };
    let _ = wrapper_template_id;
    (
        WrapperContextFixture {
            store,
            parent,
            wrapper_template_id: Some(wrapper_template_id),
            wrapper_set_id: Some(wrapper_set_id),
            context,
        },
        site_id,
    )
}

/// Builds a parent whose inherited wrapper has a default slot and a named
/// slot. The child is injected into the default slot; the named slot is
/// resolved through a slot-resolution overlay on the wrapper's own overlay
/// set, sourcing from a `resolved` text template.
fn build_slot_resolution_wrapper_fixture(
    string_table: &mut StringTable,
) -> (WrapperContextFixture, TemplateIrId) {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let (parent, wrapper_template_id, wrapper_set_id, source_template_id, context) = {
        let mut tir = store.borrow_mut();
        let empty_overlay = TemplateViewContext::default();
        let child_template_id = build_text_template(&mut tir, string_table, "injected");
        let source_template_id = build_text_template(&mut tir, string_table, "resolved");
        let (wrapper_template_id, named_slot_id, named_key) =
            build_two_slot_wrapper_template(&mut tir, string_table);

        let mut builder = TemplateIrBuilder::new(&mut tir);
        let child_node = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                child_template_id,
                TemplateTirPhase::Composed,
                empty_overlay,
            ),
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![child_node], SourceLocation::default());
        let parent = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        );

        let slot_overlay_id = tir
            .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
                resolutions: vec![(
                    named_slot_id,
                    TirSlotResolution::resolved(named_key, vec![source_template_id]),
                )],
            })
            .expect("test overlay allocation");
        let wrapper_context = TemplateViewContext {
            expression_overlay: None,
            slot_resolution: Some(slot_overlay_id),
            wrapper_context: None,
        };

        let wrapper_ref = TemplateWrapperReference::new(
            wrapper_template_id,
            TemplateTirPhase::Finalized,
            wrapper_context,
        );
        let wrapper_set_id = tir.push_or_reuse_wrapper_set(vec![wrapper_ref]);
        let context = allocate_wrapper_context_overlay(
            &mut tir,
            wrapper_set_id,
            ChildTemplateOccurrenceId::new(0),
            TirWrapperContext::default(),
        );
        (
            parent,
            wrapper_template_id,
            wrapper_set_id,
            source_template_id,
            context,
        )
    };
    let _ = wrapper_template_id;
    (
        WrapperContextFixture {
            store,
            parent,
            wrapper_template_id: Some(wrapper_template_id),
            wrapper_set_id: Some(wrapper_set_id),
            context,
        },
        source_template_id,
    )
}

/// Builds nested wrapper contexts whose outer wrapper also carries an
/// expression override. The nested occurrence must enter the outer wrapper's
/// exact view before applying its own inherited wrapper.
fn build_nested_virtual_wrapper_fixture(string_table: &mut StringTable) -> WrapperContextFixture {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();

    let (
        parent_template_id,
        outer_wrapper_template_id,
        outer_wrapper_set_id,
        inner_wrapper_set_id,
        parent_occurrence_id,
        nested_occurrence_id,
        outer_expression_site_id,
    ) = {
        let mut tir = store.borrow_mut();
        let parent_child_template_id = build_text_template(&mut tir, string_table, "parent");
        let nested_child_template_id = build_text_template(&mut tir, string_table, "nested");
        let inner_wrapper_template_id =
            build_slot_wrapper_template(&mut tir, string_table, "inner-before", "inner-after");
        let inner_wrapper_set_id =
            tir.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
                inner_wrapper_template_id,
                TemplateTirPhase::Finalized,
                empty_context,
            )]);

        let outer_expression = string_table.intern("outer-structural");
        let outer_dynamic_node = {
            let mut builder = TemplateIrBuilder::new(&mut tir);
            builder.push_dynamic_expression_node(
                Expression::string_slice(
                    outer_expression,
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                ),
                TemplateSegmentOrigin::Body,
                None,
                SourceLocation::default(),
            )
        };
        let (outer_wrapper_template_id, nested_child_node) = {
            let mut builder = TemplateIrBuilder::new(&mut tir);
            let nested_child_node = builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    nested_child_template_id,
                    TemplateTirPhase::Composed,
                    empty_context,
                ),
                SourceLocation::default(),
            );
            let slot_node = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
            let after_text = string_table.intern("outer-after");
            let after_node = builder.push_text_node(
                after_text,
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
                TemplateIrSummary::empty(),
                SourceLocation::default(),
            );
            (template_id, nested_child_node)
        };

        let parent_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut tir);
            let parent_child_node = builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    parent_child_template_id,
                    TemplateTirPhase::Composed,
                    empty_context,
                ),
                SourceLocation::default(),
            );
            let root =
                builder.push_sequence_node(vec![parent_child_node], SourceLocation::default());
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                SourceLocation::default(),
            )
        };

        let parent_occurrence_id = match &tir
            .get_node(
                tir.get_template(parent_template_id)
                    .expect("parent template should exist")
                    .root,
            )
            .expect("parent root should exist")
            .kind
        {
            TemplateIrNodeKind::Sequence { children } => match &tir
                .get_node(children[0])
                .expect("parent child node should exist")
                .kind
            {
                TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
                other => panic!("expected parent child-template node, got {other:?}"),
            },
            other => panic!("expected parent sequence root, got {other:?}"),
        };
        let nested_occurrence_id = match &tir
            .get_node(nested_child_node)
            .expect("nested child node should exist")
            .kind
        {
            TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
            other => panic!("expected nested child-template node, got {other:?}"),
        };
        let outer_expression_site_id = match &tir
            .get_node(outer_dynamic_node)
            .expect("outer dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            other => panic!("expected outer dynamic-expression node, got {other:?}"),
        };
        let outer_wrapper_set_id =
            tir.push_or_reuse_wrapper_set(vec![TemplateWrapperReference::new(
                outer_wrapper_template_id,
                TemplateTirPhase::Finalized,
                empty_context,
            )]);

        (
            parent_template_id,
            outer_wrapper_template_id,
            outer_wrapper_set_id,
            inner_wrapper_set_id,
            parent_occurrence_id,
            nested_occurrence_id,
            outer_expression_site_id,
        )
    };

    let nested_context_overlay_id = {
        let mut tir = store.borrow_mut();
        tir.allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                nested_occurrence_id,
                TirWrapperContext::inherited(inner_wrapper_set_id),
            )],
        })
        .expect("test overlay allocation")
    };
    let outer_expression_overlay_id = {
        let mut tir = store.borrow_mut();
        tir.allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                outer_expression_site_id,
                Box::new(Expression::string_slice(
                    string_table.intern("outer-overlay"),
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                )),
            )],
        })
        .expect("test overlay allocation")
    };
    let outer_context = {
        TemplateViewContext {
            expression_overlay: Some(outer_expression_overlay_id),
            slot_resolution: None,
            wrapper_context: Some(nested_context_overlay_id),
        }
    };

    {
        let mut tir = store.borrow_mut();
        MalformedTirStore::new(&mut tir).set_wrapper_reference_context(
            outer_wrapper_set_id,
            0,
            outer_context,
        );
    }

    let parent_context_overlay_id = {
        let mut tir = store.borrow_mut();
        tir.allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                parent_occurrence_id,
                TirWrapperContext::inherited(outer_wrapper_set_id),
            )],
        })
        .expect("test overlay allocation")
    };
    let parent_context = {
        TemplateViewContext {
            expression_overlay: None,
            slot_resolution: None,
            wrapper_context: Some(parent_context_overlay_id),
        }
    };

    WrapperContextFixture {
        store,
        parent: parent_template_id,
        wrapper_template_id: Some(outer_wrapper_template_id),
        wrapper_set_id: Some(outer_wrapper_set_id),
        context: parent_context,
    }
}

fn fixture_parent_view(
    fixture: &WrapperContextFixture,
) -> (TemplateTirPhase, std::cell::Ref<'_, TemplateIrStore>) {
    let store = fixture.store.borrow();
    let phase = fixture_parent_view_phase(fixture.context);
    (phase, store)
}

fn fold_fixture_result(
    fixture: &WrapperContextFixture,
    string_table: &mut StringTable,
) -> Result<TemplateEmission, TemplateError> {
    prepared_fold_fixture_result(fixture, string_table)
}

fn fold_fixture(
    fixture: &WrapperContextFixture,
    string_table: &mut StringTable,
) -> TemplateEmission {
    fold_fixture_result(fixture, string_table).expect("fold should succeed")
}

fn prepared_fold_fixture_result(
    fixture: &WrapperContextFixture,
    string_table: &mut StringTable,
) -> Result<TemplateEmission, TemplateError> {
    let (phase, store) = fixture_parent_view(fixture);
    let view = TirView::new(&store, fixture.parent, phase, fixture.context)
        .expect("test view should construct");
    let preparation = prepare_tir_view(&view, TemplatePreparationMode::Value)?;
    if let TemplatePreparationOutcome::Runtime(reason) = preparation.outcome {
        return Err(CompilerError::compiler_error(format!(
            "supported wrapper fixture unexpectedly requires runtime: {:?}",
            reason
        ))
        .into());
    }
    if let TemplatePreparationOutcome::Helper(_) = preparation.outcome {
        return Err(CompilerError::compiler_error(
            "supported wrapper fixture unexpectedly produced a helper.",
        )
        .into());
    }
    let mut context = fold_context(string_table);
    // Wrapper-context tests here assert rendered text; provenance is not owned by this helper.
    let TemplateFoldResult { emission, .. } =
        fold_prepared_template(&preparation, view, &mut context)?;
    Ok(emission)
}

fn handoff_fixture_result(
    fixture: &WrapperContextFixture,
    _string_table: &mut StringTable,
) -> Result<OwnedRuntimeTemplateHandoff, TemplateError> {
    let (phase, store) = fixture_parent_view(fixture);
    let view = TirView::new(&store, fixture.parent, phase, fixture.context)
        .expect("test view should construct");
    let prepared = TemplatePreparation {
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
    };
    owned_runtime_template_handoff_for_prepared_view(&prepared, view).map_err(Into::into)
}

fn handoff_fixture(
    fixture: &WrapperContextFixture,
    string_table: &mut StringTable,
) -> OwnedRuntimeTemplateHandoff {
    handoff_fixture_result(fixture, string_table).expect("handoff should succeed")
}

fn assert_text_node(node: &OwnedRuntimeTemplateNode, expected: &str, string_table: &StringTable) {
    let OwnedRuntimeTemplateNode::Text { text, .. } = node else {
        panic!("expected Text node, got {:?}", node);
    };
    assert_eq!(string_table.resolve(*text), expected);
}

fn assert_text_body(body: &OwnedRuntimeTemplateBody, expected: &str, string_table: &StringTable) {
    let OwnedRuntimeTemplateBody::Render(node) = body else {
        panic!("expected Render body, got {:?}", body);
    };
    match node {
        OwnedRuntimeTemplateNode::Text { text, .. } => {
            assert_eq!(string_table.resolve(*text), expected);
        }
        OwnedRuntimeTemplateNode::Sequence { children } if children.len() == 1 => {
            assert_text_node(&children[0], expected, string_table);
        }
        other => panic!("expected Text or single-child sequence, got {:?}", other),
    }
}

fn assert_child_or_text_node(
    node: &OwnedRuntimeTemplateNode,
    expected: &str,
    string_table: &StringTable,
) {
    match node {
        OwnedRuntimeTemplateNode::Text { text, .. } => {
            assert_eq!(string_table.resolve(*text), expected);
        }
        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
            assert_text_body(&template.body, expected, string_table);
        }
        other => panic!("expected Text or ChildTemplate node, got {:?}", other),
    }
}

fn expect_single_render_child(body: &OwnedRuntimeTemplateBody) -> &OwnedRuntimeTemplateNode {
    match body {
        OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
            children, ..
        }) => {
            assert_eq!(
                children.len(),
                1,
                "expected parent root to be a single-child sequence, got {:?}",
                children
            );
            &children[0]
        }
        other => panic!("expected Render(Sequence) body, got {:?}", other),
    }
}

fn text_child_builder(
    text: &str,
) -> impl FnOnce(&mut TemplateIrStore, &mut StringTable) -> TemplateIrId {
    let text = text.to_owned();
    move |store, strings| build_text_template(store, strings, &text)
}

// ---------------------------------------------------------------------------
//  Fold: inherited wrapper context, $fresh, and IfChildEmits
// ---------------------------------------------------------------------------

#[test]
fn wrapper_context_overlay_folds_inherited_wrapper() {
    let mut string_table = StringTable::new();
    let fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext::default(),
        text_child_builder("child"),
    );
    let emission = fold_fixture(&fixture, &mut string_table);
    let TemplateEmission::Output(output_id) = emission else {
        panic!("expected Output emission, got {:?}", emission);
    };
    assert_eq!(
        string_table.resolve(output_id),
        "beforechildafter",
        "inherited wrapper should wrap child output"
    );
}

#[test]
fn wrapper_context_fold_applies_inherited_wrapper_set_innermost_to_outermost() {
    // A single inherited wrapper set holding two distinct wrappers must fold to
    // `outer(inner(child))`. `TemplateWrapperSet::wrappers` is stored
    // innermost-to-outermost, so forward fold consumption applies the innermost
    // wrapper directly around the child and the outermost wrapper last.
    let mut string_table = StringTable::new();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let (parent, wrapper_set_id, context) = {
        let mut tir = store.borrow_mut();
        let child_template_id = build_text_template(&mut tir, &mut string_table, "child");
        let inner_wrapper =
            build_slot_wrapper_template(&mut tir, &mut string_table, "inner-before", "inner-after");
        let outer_wrapper =
            build_slot_wrapper_template(&mut tir, &mut string_table, "outer-before", "outer-after");

        let mut builder = TemplateIrBuilder::new(&mut tir);
        let child_node = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                child_template_id,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![child_node], SourceLocation::default());
        let parent = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        );

        let inner_ref = TemplateWrapperReference::new(
            inner_wrapper,
            TemplateTirPhase::Finalized,
            TemplateViewContext::default(),
        );
        let outer_ref = TemplateWrapperReference::new(
            outer_wrapper,
            TemplateTirPhase::Finalized,
            TemplateViewContext::default(),
        );
        let wrapper_set_id = tir.push_or_reuse_wrapper_set(vec![inner_ref, outer_ref]);
        let context = allocate_wrapper_context_overlay(
            &mut tir,
            wrapper_set_id,
            ChildTemplateOccurrenceId::new(0),
            TirWrapperContext::default(),
        );
        (parent, wrapper_set_id, context)
    };

    let fixture = WrapperContextFixture {
        store,
        parent,
        wrapper_template_id: None,
        wrapper_set_id: Some(wrapper_set_id),
        context,
    };

    let emission = fold_fixture(&fixture, &mut string_table);
    let TemplateEmission::Output(output_id) = emission else {
        panic!("expected Output emission, got {emission:?}");
    };
    assert_eq!(
        string_table.resolve(output_id),
        "outer-beforeinner-beforechildinner-afterouter-after",
        "a single innermost-to-outermost wrapper set must fold to outer(inner(child))"
    );
}

#[test]
fn prepared_fold_keeps_parent_expression_authority_through_nested_wrappers() {
    let mut string_table = StringTable::new();
    let fixture = build_nested_virtual_wrapper_fixture(&mut string_table);

    let emission = prepared_fold_fixture_result(&fixture, &mut string_table)
        .expect("supported nested wrapper should pass the production fold gate");
    let TemplateEmission::Output(output_id) = emission else {
        panic!("expected Output emission, got {emission:?}");
    };
    assert_eq!(
        string_table.resolve(output_id),
        "outer-structuralinner-beforenestedinner-afterparentouter-after",
        "nested structural wrappers must retain the parent expression authority"
    );

    let handoff = handoff_fixture(&fixture, &mut string_table);
    let outer = expect_single_render_child(&handoff.body);
    let OwnedRuntimeTemplateNode::Sequence { children } = outer else {
        panic!("expected the outer wrapper sequence, got {outer:?}");
    };
    assert_eq!(children.len(), 4);

    let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = &children[0] else {
        panic!("expected the structural outer expression in the handoff");
    };
    assert!(matches!(
        expression.kind,
        ExpressionKind::StringSlice(text) if string_table.resolve(text) == "outer-structural"
    ));

    let OwnedRuntimeTemplateNode::Sequence {
        children: inner_children,
    } = &children[1]
    else {
        panic!("expected the nested occurrence wrapper in the handoff");
    };
    assert_eq!(inner_children.len(), 3);
    assert_text_node(&inner_children[0], "inner-before", &string_table);
    assert_child_or_text_node(&inner_children[1], "nested", &string_table);
    assert_text_node(&inner_children[2], "inner-after", &string_table);
    assert_child_or_text_node(&children[2], "parent", &string_table);
    assert_text_node(&children[3], "outer-after", &string_table);
}

#[test]
fn wrapper_context_overlay_honors_fresh_suppression() {
    let mut string_table = StringTable::new();
    let fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext {
            inherited_wrapper_set: None,
            skip_parent_child_wrappers: true,
            application_mode: TirWrapperApplicationMode::Always,
        },
        text_child_builder("child"),
    );
    let emission = fold_fixture(&fixture, &mut string_table);
    let TemplateEmission::Output(output_id) = emission else {
        panic!("expected Output emission, got {:?}", emission);
    };
    assert_eq!(
        string_table.resolve(output_id),
        "child",
        "$fresh suppression should prevent wrapper application"
    );
}

#[test]
fn prepared_fold_applies_if_child_emits_only_when_the_child_emits_output() {
    // A child that structurally outputs receives the inherited wrapper.
    let mut string_table = StringTable::new();
    let fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext {
            inherited_wrapper_set: None,
            skip_parent_child_wrappers: false,
            application_mode: TirWrapperApplicationMode::IfChildEmits,
        },
        text_child_builder("child"),
    );
    let emission = fold_fixture(&fixture, &mut string_table);
    let TemplateEmission::Output(output_id) = emission else {
        panic!(
            "expected Output emission for an emitting child, got {:?}",
            emission
        );
    };
    assert_eq!(
        string_table.resolve(output_id),
        "beforechildafter",
        "IfChildEmits should wrap a child that structurally outputs"
    );

    // A child that structurally emits nothing must not render the wrapper.
    let silent_fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext {
            inherited_wrapper_set: None,
            skip_parent_child_wrappers: false,
            application_mode: TirWrapperApplicationMode::IfChildEmits,
        },
        build_false_no_else_branch_template,
    );
    let silent_emission = fold_fixture(&silent_fixture, &mut string_table);
    assert_eq!(
        silent_emission,
        TemplateEmission::NoOutput,
        "false no-else child should not render inherited wrappers"
    );
}

// ---------------------------------------------------------------------------
//  Fold: expression overlays through inherited wrappers
// ---------------------------------------------------------------------------

#[test]
fn prepared_fold_applies_wrapper_expression_overlay() {
    let mut string_table = StringTable::new();
    let wrapper_text = string_table.intern("wrapper-overlay");
    let (fixture, _) = build_expression_wrapper_fixture(
        &mut string_table,
        Expression::string_slice(
            wrapper_text,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        ),
        None,
    );
    let emission = fold_fixture(&fixture, &mut string_table);
    let TemplateEmission::Output(output_id) = emission else {
        panic!("expected Output emission, got {:?}", emission);
    };
    assert_eq!(
        string_table.resolve(output_id),
        "wrapper-overlaychild",
        "inherited wrappers must fold through their exact expression overlay"
    );
}

#[test]
fn preparation_classifies_outer_override_by_const_vs_runtime_expression() {
    // A const outer override replaces a runtime wrapper-local expression so the
    // whole wrapper folds to a constant result.
    let mut string_table = StringTable::new();
    let outer_text = string_table.intern("outer-const");
    let (const_outer_fixture, _) = build_expression_wrapper_fixture(
        &mut string_table,
        runtime_string_expression(),
        Some(Expression::string_slice(
            outer_text,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
    );
    let emission = prepared_fold_fixture_result(&const_outer_fixture, &mut string_table)
        .unwrap_or_else(|error| {
            panic!("const outer override should make the wrapper foldable: {error:?}")
        });
    let TemplateEmission::Output(output_id) = emission else {
        panic!("expected Output emission, got {:?}", emission);
    };
    assert_eq!(
        string_table.resolve(output_id),
        "outer-constchild",
        "the const outer override must replace the runtime wrapper-local expression"
    );

    // A runtime outer override keeps the wrapper on the runtime handoff path and
    // survives into the owned handoff as the effective expression.
    let wrapper_text = string_table.intern("wrapper-local");
    let (runtime_outer_fixture, _) = build_expression_wrapper_fixture(
        &mut string_table,
        Expression::string_slice(
            wrapper_text,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        ),
        Some(runtime_string_expression()),
    );

    let preparation = {
        let (phase, store) = fixture_parent_view(&runtime_outer_fixture);
        let view = TirView::new(
            &store,
            runtime_outer_fixture.parent,
            phase,
            runtime_outer_fixture.context,
        )
        .expect("parent view should construct");
        prepare_tir_view(&view, TemplatePreparationMode::Value)
            .expect("outer runtime wrapper override should be a valid runtime result")
    };
    assert!(
        matches!(preparation.outcome, TemplatePreparationOutcome::Runtime(_)),
        "a runtime outer override must not be classified as a const wrapper"
    );

    let handoff = handoff_fixture(&runtime_outer_fixture, &mut string_table);
    let wrapped = expect_single_render_child(&handoff.body);
    let OwnedRuntimeTemplateNode::Sequence { children } = wrapped else {
        panic!(
            "expected wrapper sequence in the owned handoff, got {:?}",
            wrapped
        );
    };
    let expression_node = match children.first() {
        Some(OwnedRuntimeTemplateNode::DynamicExpression { .. }) => &children[0],
        Some(OwnedRuntimeTemplateNode::Sequence { children }) if children.len() == 1 => {
            &children[0]
        }
        Some(other) => {
            panic!("expected the wrapper expression to survive in the handoff, got {other:?}")
        }
        None => panic!("expected a wrapper expression in the owned handoff"),
    };
    let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = expression_node else {
        panic!(
            "expected the wrapper expression node, got {:?}",
            expression_node
        );
    };
    assert!(
        matches!(expression.kind, ExpressionKind::Reference(_)),
        "the outer runtime expression must override the const wrapper-local expression"
    );
}

#[test]
fn preparation_ignores_runtime_referenced_wrapper_expression_overlay() {
    let mut string_table = StringTable::new();
    let wrapper_text = string_table.intern("wrapper-overlay");
    let (fixture, site_id) = build_expression_wrapper_fixture(
        &mut string_table,
        Expression::string_slice(
            wrapper_text,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        ),
        None,
    );
    let runtime_context = {
        let mut tir = fixture.store.borrow_mut();
        let expression_overlay_id = tir
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(site_id, Box::new(runtime_string_expression()))],
            })
            .expect("test overlay allocation");
        TemplateViewContext {
            expression_overlay: Some(expression_overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        }
    };
    {
        let mut tir = fixture.store.borrow_mut();
        let wrapper_template_id = fixture
            .wrapper_template_id
            .expect("expression wrapper should be present");
        let wrapper_set_id = fixture
            .wrapper_set_id
            .expect("expression wrapper set should be present");
        let _ = wrapper_template_id;
        MalformedTirStore::new(&mut tir).set_wrapper_reference_context(
            wrapper_set_id,
            0,
            runtime_context,
        );
    }

    let preparation = {
        let (phase, store) = fixture_parent_view(&fixture);
        let view = TirView::new(&store, fixture.parent, phase, fixture.context)
            .expect("parent view should construct");
        prepare_tir_view(&view, TemplatePreparationMode::Value)
            .expect("referenced wrapper expression should be ignored by the structural transition")
    };
    assert!(
        matches!(preparation.outcome, TemplatePreparationOutcome::Foldable),
        "a referenced wrapper expression must not change the parent fold decision"
    );

    let handoff = handoff_fixture(&fixture, &mut string_table);
    let wrapped = expect_single_render_child(&handoff.body);
    let OwnedRuntimeTemplateNode::Sequence { children } = wrapped else {
        panic!("expected wrapper handoff sequence, got {wrapped:?}");
    };
    let expression_node = match children.first() {
        Some(OwnedRuntimeTemplateNode::DynamicExpression { .. }) => &children[0],
        Some(OwnedRuntimeTemplateNode::Sequence { children }) if children.len() == 1 => {
            &children[0]
        }
        Some(other) => panic!("expected structural wrapper expression, got {other:?}"),
        None => panic!("expected structural wrapper expression"),
    };
    let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = expression_node else {
        panic!("expected wrapper expression node, got {expression_node:?}");
    };
    assert!(
        matches!(expression.kind, ExpressionKind::StringSlice(text) if text == wrapper_text),
        "owned handoff must retain the structural wrapper expression"
    );
}

// ---------------------------------------------------------------------------
//  Fold: child injection and slot resolution through wrapper context
// ---------------------------------------------------------------------------

#[test]
fn prepared_fold_injects_child_before_resolving_other_wrapper_slots() {
    let mut string_table = StringTable::new();
    let (fixture, _) = build_slot_resolution_wrapper_fixture(&mut string_table);
    let emission = fold_fixture(&fixture, &mut string_table);
    let TemplateEmission::Output(output_id) = emission else {
        panic!("expected Output emission, got {:?}", emission);
    };
    assert_eq!(
        string_table.resolve(output_id),
        "beforeinjectedresolvedafter",
        "the injected target must win while other slots preserve overlay-resolved sources"
    );
}

#[test]
fn preparation_falls_back_for_runtime_non_injected_slot_source() {
    let mut string_table = StringTable::new();
    let resolved_text = string_table.intern("resolved");
    let (fixture, source_template_id) = build_slot_resolution_wrapper_fixture(&mut string_table);

    {
        let mut tir = fixture.store.borrow_mut();
        let slot_plan_id = tir.push_slot_plan(TemplateSlotPlan {
            location: SourceLocation::default(),
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
        });
        tir.attach_runtime_slot_plan(source_template_id, slot_plan_id)
            .expect("source template should accept the committed slot plan");
    }
    let _ = resolved_text;

    let preparation = {
        let store = fixture.store.borrow();
        let view = TirView::new(
            &store,
            fixture.parent,
            TemplateTirPhase::Composed,
            fixture.context,
        )
        .expect("parent view should construct");
        prepare_tir_view(&view, TemplatePreparationMode::Value)
            .expect("runtime slot source should be an eligible runtime result")
    };
    assert_eq!(
        match preparation.outcome {
            TemplatePreparationOutcome::Runtime(reason) => Some(reason),
            TemplatePreparationOutcome::Foldable | TemplatePreparationOutcome::Helper(_) => None,
        },
        Some(RuntimeTemplateReason::InheritedWrapperApplication),
        "a runtime source in a non-injected wrapper slot must stay on the handoff path"
    );
    let handoff = handoff_fixture(&fixture, &mut string_table);
    assert!(
        format!("{:?}", handoff.body).contains("RuntimeSlotApplication"),
        "owned handoff must retain the runtime slot source instead of losing it during folding"
    );
}

// ---------------------------------------------------------------------------
//  Fold: below-Composed structural fallback
// ---------------------------------------------------------------------------

#[test]
fn below_composed_wrapper_reference_uses_structural_root_without_overlay_lookup() {
    let mut string_table = StringTable::new();
    let fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext::default(),
        text_child_builder("child"),
    );
    let wrapper_template_id = fixture
        .wrapper_template_id
        .expect("fixture should have a wrapper template");

    {
        let mut tir = fixture.store.borrow_mut();
        let wrapper_set_id = fixture
            .wrapper_set_id
            .expect("fixture should have a wrapper set");
        let _ = wrapper_template_id;
        MalformedTirStore::new(&mut tir).mutate_wrapper(wrapper_set_id, 0, |wrapper| {
            wrapper.phase = TemplateTirPhase::Parsed;
            wrapper.context = TemplateViewContext {
                expression_overlay: Some(TirExpressionOverlayId::new(999)),
                ..TemplateViewContext::default()
            };
        });
    }

    let emission = fold_fixture(&fixture, &mut string_table);
    let TemplateEmission::Output(output_id) = emission else {
        panic!("expected Output emission, got {:?}", emission);
    };
    assert_eq!(string_table.resolve(output_id), "beforechildafter");

    let handoff = handoff_fixture(&fixture, &mut string_table);
    let wrapped = expect_single_render_child(&handoff.body);
    let OwnedRuntimeTemplateNode::Sequence { children, .. } = wrapped else {
        panic!("expected structural wrapper sequence, got {:?}", wrapped);
    };
    assert_eq!(children.len(), 3);
    assert_text_node(&children[0], "before", &string_table);
    assert_child_or_text_node(&children[1], "child", &string_table);
    assert_text_node(&children[2], "after", &string_table);
}

// ---------------------------------------------------------------------------
//  Fold: slot-insert rejection
// ---------------------------------------------------------------------------

#[test]
fn prepared_fold_rejects_slot_insert_from_wrapper_context_set() {
    let mut string_table = StringTable::new();
    let fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext::default(),
        text_child_builder("child"),
    );
    {
        let mut tir = fixture.store.borrow_mut();
        tir.set_template_kind(
            fixture
                .wrapper_template_id
                .expect("fixture should include its wrapper template"),
            TemplateType::SlotInsert(SlotKey::Default),
        )
        .expect("wrapper kind should be writable");
    }
    let (phase, store) = fixture_parent_view(&fixture);
    let view = TirView::new(&store, fixture.parent, phase, fixture.context)
        .expect("test view should construct");
    let preparation = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("slot insert helper should prepare without folding");
    assert!(!matches!(
        preparation.outcome,
        TemplatePreparationOutcome::Foldable
    ));
}

#[test]
fn prepared_fold_rejects_slot_insert_from_effective_slot_source() {
    let mut string_table = StringTable::new();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let (wrapper_template_id, source_template_id, context) = {
        let mut tir = store.borrow_mut();
        let wrapper = build_slot_wrapper_template(&mut tir, &mut string_table, "", "");
        let source = build_text_template(&mut tir, &mut string_table, "escaped");
        tir.set_template_kind(source, TemplateType::SlotInsert(SlotKey::Default))
            .expect("source kind should be writable");
        let slot_overlay_id = tir
            .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
                resolutions: vec![(
                    SlotOccurrenceId::new(0),
                    TirSlotResolution::resolved(SlotKey::Default, vec![source]),
                )],
            })
            .expect("test overlay allocation");
        let context = TemplateViewContext {
            expression_overlay: None,
            slot_resolution: Some(slot_overlay_id),
            wrapper_context: None,
        };
        (wrapper, source, context)
    };
    let _ = source_template_id;

    let store_ref = store.borrow();
    let view = TirView::new(
        &store_ref,
        wrapper_template_id,
        TemplateTirPhase::Composed,
        context,
    )
    .expect("slot-overlay view should construct");
    let preparation = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("slot insert source should prepare without folding");
    assert!(!matches!(
        preparation.outcome,
        TemplatePreparationOutcome::Foldable
    ));
}

// ---------------------------------------------------------------------------
//  Preparation: cyclic nested wrapper termination
// ---------------------------------------------------------------------------

#[test]
fn preparation_terminates_for_cyclic_nested_wrapper_contexts() {
    let mut string_table = StringTable::new();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let (parent_template_id, parent_context) = {
        let mut tir = store.borrow_mut();
        let empty_overlay = TemplateViewContext::default();
        let child_template_id = build_text_template(&mut tir, &mut string_table, "child");

        // The wrapper template references a forward parent template id, creating
        // a structural cycle once the wrapper set inherits itself.
        let wrapper_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut tir);
            let child = builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    TemplateIrId::new(2),
                    TemplateTirPhase::Composed,
                    empty_overlay,
                ),
                SourceLocation::default(),
            );
            let root = builder.push_sequence_node(vec![child], SourceLocation::default());
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                SourceLocation::default(),
            )
        };

        let parent_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut tir);
            let child = builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    child_template_id,
                    TemplateTirPhase::Composed,
                    empty_overlay,
                ),
                SourceLocation::default(),
            );
            let root = builder.push_sequence_node(vec![child], SourceLocation::default());
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                SourceLocation::default(),
            )
        };

        // Self-referential wrapper context: the wrapper inherits wrapper set 0,
        // which is allocated below as the set that contains this wrapper.
        let nested_context_overlay_id = tir
            .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
                contexts: vec![(
                    ChildTemplateOccurrenceId::new(0),
                    TirWrapperContext::inherited(TemplateWrapperSetId::new(0)),
                )],
            })
            .expect("test overlay allocation");
        let nested_wrapper_context = TemplateViewContext {
            expression_overlay: None,
            slot_resolution: None,
            wrapper_context: Some(nested_context_overlay_id),
        };
        let parent_context_overlay_id = tir
            .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
                contexts: vec![(
                    ChildTemplateOccurrenceId::new(1),
                    TirWrapperContext::inherited(TemplateWrapperSetId::new(0)),
                )],
            })
            .expect("test overlay allocation");
        let parent_context = TemplateViewContext {
            expression_overlay: None,
            slot_resolution: None,
            wrapper_context: Some(parent_context_overlay_id),
        };

        tir.push_wrapper_set(TemplateWrapperSet {
            wrappers: vec![TemplateWrapperReference::new(
                wrapper_template_id,
                TemplateTirPhase::Finalized,
                nested_wrapper_context,
            )],
        });

        (parent_template_id, parent_context)
    };

    let store_ref = store.borrow();
    let view = TirView::new(
        &store_ref,
        parent_template_id,
        TemplateTirPhase::Composed,
        parent_context,
    )
    .expect("cyclic wrapper view should construct");
    let error = match prepare_tir_view(&view, TemplatePreparationMode::Value) {
        Err(error) => error,
        Ok(prepared) => panic!("cyclic wrapper contexts must be CompilerError, got {prepared:?}"),
    };
    let TemplateError::Infrastructure(error) = error else {
        panic!("cyclic wrapper contexts must stay on the infrastructure lane, got {error:?}");
    };
    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::Compiler
    );
}

// ---------------------------------------------------------------------------
//  Handoff: inherited wrapper, $fresh, and conditional wrapper
// ---------------------------------------------------------------------------

#[test]
fn handoff_tir_view_applies_inherited_wrapper_context_overlay() {
    let mut string_table = StringTable::new();
    let fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext::default(),
        text_child_builder("child"),
    );
    let handoff = handoff_fixture(&fixture, &mut string_table);

    let wrapped = expect_single_render_child(&handoff.body);
    let OwnedRuntimeTemplateNode::Sequence { children, .. } = wrapped else {
        panic!("expected Sequence wrapper root, got {:?}", wrapped);
    };
    assert_eq!(
        children.len(),
        3,
        "wrapper should produce before + child + after"
    );
    assert_text_node(&children[0], "before", &string_table);
    assert_child_or_text_node(&children[1], "child", &string_table);
    assert_text_node(&children[2], "after", &string_table);
}

#[test]
fn handoff_tir_view_honors_fresh_suppression_in_wrapper_context_overlay() {
    let mut string_table = StringTable::new();
    let fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext {
            inherited_wrapper_set: None,
            skip_parent_child_wrappers: true,
            application_mode: TirWrapperApplicationMode::Always,
        },
        text_child_builder("child"),
    );
    let handoff = handoff_fixture(&fixture, &mut string_table);
    let child_node = expect_single_render_child(&handoff.body);
    assert_child_or_text_node(child_node, "child", &string_table);
}

#[test]
fn handoff_tir_view_materializes_if_child_emits_as_conditional_wrapper() {
    let mut string_table = StringTable::new();
    let fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext {
            inherited_wrapper_set: None,
            skip_parent_child_wrappers: false,
            application_mode: TirWrapperApplicationMode::IfChildEmits,
        },
        text_child_builder("child"),
    );
    let handoff = handoff_fixture(&fixture, &mut string_table);

    let wrapped = expect_single_render_child(&handoff.body);
    let OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } = wrapped else {
        panic!("expected ConditionalWrapper, got {:?}", wrapped);
    };
    assert_child_or_text_node(child, "child", &string_table);
    let OwnedRuntimeTemplateNode::Sequence { children, .. } = wrapper.as_ref() else {
        panic!("expected wrapper sequence, got {:?}", wrapper);
    };
    assert_eq!(children.len(), 3);
    assert_text_node(&children[0], "before", &string_table);
    assert!(matches!(
        children[1],
        OwnedRuntimeTemplateNode::AggregateOutput
    ));
    assert_text_node(&children[2], "after", &string_table);
}

#[test]
fn wrapper_fixture_keeps_child_in_the_shared_store() {
    let mut string_table = StringTable::new();
    let fixture = build_wrapper_context_fixture(
        &mut string_table,
        TirWrapperContext::default(),
        text_child_builder("child"),
    );
    let store = fixture.store.borrow();
    assert!(store.get_template(fixture.parent).is_some());
}
