//! TIR wrapper-context overlay construction tests.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrBuilder;
use crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId;
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrBranch;
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirExpressionOverlayId, TirWrapperApplicationMode,
};
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateTirReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::summary::TemplateIrSummary;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;

use super::super::attach_wrapper_context_overlay;

fn text_template(
    store: &mut TemplateIrStore,
    strings: &mut StringTable,
    text: &str,
    style: Style,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let text_node = builder.push_text_node(
        strings.intern(text),
        text.len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let root = builder.push_sequence_node(vec![text_node], SourceLocation::default());
    builder.finish_template(
        root,
        style,
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    )
}

fn control_flow_template(store: &mut TemplateIrStore, strings: &mut StringTable) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let body = builder.push_text_node(
        strings.intern("body"),
        4,
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(Expression::bool(
            false,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        body,
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

fn wrapper_template(
    store: &mut TemplateIrStore,
    strings: &mut StringTable,
) -> TemplateWrapperReference {
    let mut builder = TemplateIrBuilder::new(store);
    let before = builder.push_text_node(
        strings.intern("before"),
        6,
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let slot = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let after = builder.push_text_node(
        strings.intern("after"),
        5,
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let root = builder.push_sequence_node(vec![before, slot, after], SourceLocation::default());
    let wrapper_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    );
    TemplateWrapperReference::new(
        wrapper_id,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )
}

fn parent_with_branch_body_child(
    store: &mut TemplateIrStore,
    child: TemplateIrId,
    context: TemplateViewContext,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let child_node = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(child, TemplateTirPhase::Composed, context),
        SourceLocation::default(),
    );
    let body = builder.push_sequence_node(vec![child_node], SourceLocation::default());
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(Expression::bool(
            true,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        body,
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

fn parent_with_loop_body_child(
    store: &mut TemplateIrStore,
    child: TemplateIrId,
    context: TemplateViewContext,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let child_node = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(child, TemplateTirPhase::Composed, context),
        SourceLocation::default(),
    );
    let body = builder.push_sequence_node(vec![child_node], SourceLocation::default());
    let root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(Expression::bool(
                true,
                SourceLocation::default(),
                ValueMode::ImmutableOwned,
            )),
        },
        body,
        None,
        SourceLocation::default(),
    );
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

fn parent_with_child(
    store: &mut TemplateIrStore,
    child: TemplateIrId,
    context: TemplateViewContext,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let child_node = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(child, TemplateTirPhase::Composed, context),
        SourceLocation::default(),
    );
    let root = builder.push_sequence_node(vec![child_node], SourceLocation::default());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    )
}

fn reference(root: TemplateIrId, context: TemplateViewContext) -> TemplateTirReference {
    TemplateTirReference {
        root,
        phase: TemplateTirPhase::Composed,
        context,
    }
}

#[test]
fn fresh_child_suppresses_inherited_wrappers() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let empty = TemplateViewContext::default();
    let (parent, wrapper) = {
        let mut store = store.borrow_mut();
        let child = text_template(
            &mut store,
            &mut strings,
            "child",
            Style {
                skip_parent_child_wrappers: true,
                ..Style::default()
            },
        );
        let parent = parent_with_child(&mut store, child, empty);
        (parent, wrapper_template(&mut store, &mut strings))
    };
    let mut parent_reference = reference(parent, empty);

    attach_wrapper_context_overlay(&mut parent_reference, &[wrapper], &store)
        .expect("fresh child should be represented by an overlay context");

    let store = store.borrow();
    let overlay = store
        .wrapper_context_overlay(
            parent_reference
                .context
                .wrapper_context
                .expect("wrapper context should exist"),
        )
        .expect("wrapper context payload should exist");
    assert_eq!(overlay.contexts.len(), 1);
    let (_, context) = &overlay.contexts[0];
    assert!(context.skip_parent_child_wrappers);
    assert!(context.inherited_wrapper_set.is_none());
}

#[test]
fn control_flow_child_uses_if_child_emits_wrapper_mode() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let empty = TemplateViewContext::default();
    let (parent, wrapper) = {
        let mut store = store.borrow_mut();
        let child = control_flow_template(&mut store, &mut strings);
        let parent = parent_with_child(&mut store, child, empty);
        (parent, wrapper_template(&mut store, &mut strings))
    };
    let mut parent_reference = reference(parent, empty);

    attach_wrapper_context_overlay(&mut parent_reference, &[wrapper], &store)
        .expect("control-flow child should receive an inherited context");

    let store = store.borrow();
    let overlay = store
        .wrapper_context_overlay(
            parent_reference
                .context
                .wrapper_context
                .expect("wrapper context should exist"),
        )
        .expect("wrapper context payload should exist");
    let (_, context) = &overlay.contexts[0];
    assert!(!context.skip_parent_child_wrappers);
    assert!(context.inherited_wrapper_set.is_some());
    assert_eq!(
        context.application_mode,
        TirWrapperApplicationMode::IfChildEmits
    );
}

#[test]
fn overlay_records_direct_child_inside_branch_body() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let empty = TemplateViewContext::default();
    let (parent, wrapper) = {
        let mut store = store.borrow_mut();
        let child = text_template(&mut store, &mut strings, "child", Style::default());
        let parent = parent_with_branch_body_child(&mut store, child, empty);
        (parent, wrapper_template(&mut store, &mut strings))
    };
    let mut parent_reference = reference(parent, empty);

    attach_wrapper_context_overlay(&mut parent_reference, &[wrapper], &store)
        .expect("branch-body child should receive an inherited context");

    let store = store.borrow();
    let overlay = store
        .wrapper_context_overlay(
            parent_reference
                .context
                .wrapper_context
                .expect("wrapper context should exist"),
        )
        .expect("wrapper context payload should exist");
    assert_eq!(overlay.contexts.len(), 1);
    let (_, context) = &overlay.contexts[0];
    assert!(!context.skip_parent_child_wrappers);
    assert!(context.inherited_wrapper_set.is_some());
    assert_eq!(context.application_mode, TirWrapperApplicationMode::Always);
}

#[test]
fn overlay_records_direct_child_inside_loop_body() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let empty = TemplateViewContext::default();
    let (parent, wrapper) = {
        let mut store = store.borrow_mut();
        let child = text_template(&mut store, &mut strings, "child", Style::default());
        let parent = parent_with_loop_body_child(&mut store, child, empty);
        (parent, wrapper_template(&mut store, &mut strings))
    };
    let mut parent_reference = reference(parent, empty);

    attach_wrapper_context_overlay(&mut parent_reference, &[wrapper], &store)
        .expect("loop-body child should receive an inherited context");

    let store = store.borrow();
    let overlay = store
        .wrapper_context_overlay(
            parent_reference
                .context
                .wrapper_context
                .expect("wrapper context should exist"),
        )
        .expect("wrapper context payload should exist");
    assert_eq!(overlay.contexts.len(), 1);
    let (_, context) = &overlay.contexts[0];
    assert!(!context.skip_parent_child_wrappers);
    assert!(context.inherited_wrapper_set.is_some());
    assert_eq!(context.application_mode, TirWrapperApplicationMode::Always);
}

#[test]
fn missing_parent_template_is_an_internal_error() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty = TemplateViewContext::default();
    let mut reference = reference(TemplateIrId::new(99), empty);

    let error = attach_wrapper_context_overlay(&mut reference, &[], &store)
        .expect_err("missing parent should be rejected");
    assert!(error.msg.contains("owning template"));
}

#[test]
fn missing_child_template_is_an_internal_error() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let empty = TemplateViewContext::default();
    let (parent, wrapper) = {
        let mut store = store.borrow_mut();
        let parent = parent_with_child(&mut store, TemplateIrId::new(99), empty);
        (parent, wrapper_template(&mut store, &mut strings))
    };
    let mut reference = reference(parent, empty);

    let error = attach_wrapper_context_overlay(&mut reference, &[wrapper], &store)
        .expect_err("missing child should be rejected");
    assert!(error.msg.contains("child template"));
}

#[test]
fn template_without_child_contexts_keeps_its_view_context() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let empty = TemplateViewContext::default();
    let (parent, wrapper) = {
        let mut store = store.borrow_mut();
        let parent = text_template(&mut store, &mut strings, "plain", Style::default());
        (parent, wrapper_template(&mut store, &mut strings))
    };
    let mut reference = reference(parent, empty);

    attach_wrapper_context_overlay(&mut reference, &[wrapper], &store)
        .expect("templates without child occurrences should be valid");
    assert_eq!(reference.context, empty);
}

#[test]
fn missing_current_overlay_is_rejected_before_allocation() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let (parent, wrapper) = {
        let mut store = store.borrow_mut();
        let child = text_template(&mut store, &mut strings, "child", Style::default());
        let parent = parent_with_child(&mut store, child, TemplateViewContext::default());
        (parent, wrapper_template(&mut store, &mut strings))
    };
    let missing = TemplateViewContext {
        expression_overlay: Some(TirExpressionOverlayId::new(999)),
        ..TemplateViewContext::default()
    };
    let mut reference = reference(parent, missing);
    let wrapper_count = store.borrow().wrapper_set_count();

    let error = attach_wrapper_context_overlay(&mut reference, &[wrapper], &store)
        .expect_err("missing current overlay should be rejected");
    assert!(error.msg.contains("expression overlay"));
    assert_eq!(store.borrow().wrapper_set_count(), wrapper_count);
}

#[test]
fn missing_child_overlay_is_rejected_before_allocation() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut strings = StringTable::new();
    let empty = TemplateViewContext::default();
    let missing = TemplateViewContext {
        expression_overlay: Some(TirExpressionOverlayId::new(999)),
        ..TemplateViewContext::default()
    };
    let (parent, wrapper) = {
        let mut store = store.borrow_mut();
        let child = text_template(&mut store, &mut strings, "child", Style::default());
        // The child reference carries a missing expression overlay so child
        // validation must fail before any wrapper set is allocated.
        let parent = parent_with_child(&mut store, child, missing);
        (parent, wrapper_template(&mut store, &mut strings))
    };
    let wrapper_count = store.borrow().wrapper_set_count();
    let mut parent_reference = reference(parent, empty);

    let error = attach_wrapper_context_overlay(&mut parent_reference, &[wrapper], &store)
        .expect_err("missing child overlay should fail before allocation");

    assert!(
        error.msg.contains("child reference") && error.msg.contains("expression overlay"),
        "error must report the missing child overlay: {error:?}"
    );
    assert_eq!(
        store.borrow().wrapper_set_count(),
        wrapper_count,
        "failed child resolution must not allocate a wrapper set"
    );
    assert_eq!(parent_reference.context, empty);
}
