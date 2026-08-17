//! Parser-facing TIR construction owner.
//!
//! One `TemplateConstructionContext` holds the module store handle, recorded
//! root children, control-flow node, head-node count and source location
//! while a template is being parsed. Parser callers record through this type
//! and call store allocation here. After `finish`, the durable value is a
//! `TemplateTirReference`; this context is consumed.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotPlaceholder, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateLoopControlKind, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::ids::{
    ExpressionSiteId, TemplateIrId, TemplateIrNodeId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind,
};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateTirReference,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::summary::summarize_existing_root;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Parser-local owner for in-progress TIR emission.
pub(crate) struct TemplateConstructionContext {
    store: Rc<RefCell<TemplateIrStore>>,
    children: Vec<TemplateIrNodeId>,
    head_node_count: u32,
    control_flow_node_id: Option<TemplateIrNodeId>,
    location: SourceLocation,
}

impl TemplateConstructionContext {
    pub(crate) fn new(store: Rc<RefCell<TemplateIrStore>>, location: SourceLocation) -> Self {
        Self {
            store,
            children: Vec::new(),
            head_node_count: 0,
            control_flow_node_id: None,
            location,
        }
    }

    pub(crate) fn location(&self) -> &SourceLocation {
        &self.location
    }

    pub(crate) fn store(&self) -> std::cell::Ref<'_, TemplateIrStore> {
        self.store.borrow()
    }

    pub(crate) fn store_handle(&self) -> Rc<RefCell<TemplateIrStore>> {
        Rc::clone(&self.store)
    }

    pub(crate) fn root_children(&self) -> &[TemplateIrNodeId] {
        &self.children
    }

    /// Parser-recorded control-flow owner, if this template recorded one.
    pub(crate) fn control_flow_node_id(&self) -> Option<TemplateIrNodeId> {
        self.control_flow_node_id
    }

    pub(crate) fn next_expression_site_id(&self) -> ExpressionSiteId {
        self.store.borrow_mut().next_expression_site_id()
    }

    // -------------------------
    //  Recording — text
    // -------------------------

    pub(crate) fn record_text(
        &mut self,
        text: StringId,
        byte_len: usize,
        location: SourceLocation,
    ) {
        self.record_text_segment(text, byte_len, TemplateSegmentOrigin::Body, None, location);
    }

    pub(crate) fn record_head_text(
        &mut self,
        text: StringId,
        byte_len: usize,
        location: SourceLocation,
    ) {
        self.record_text_segment(text, byte_len, TemplateSegmentOrigin::Head, None, location);
    }

    pub(crate) fn record_reactive_head_text(
        &mut self,
        text: StringId,
        byte_len: usize,
        reactive_subscription: Option<ReactiveSubscription>,
        location: SourceLocation,
    ) {
        self.record_text_segment(
            text,
            byte_len,
            TemplateSegmentOrigin::Head,
            reactive_subscription,
            location,
        );
    }

    fn record_text_segment(
        &mut self,
        text: StringId,
        byte_len: usize,
        origin: TemplateSegmentOrigin,
        reactive_subscription: Option<ReactiveSubscription>,
        location: SourceLocation,
    ) {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Text {
                    text,
                    byte_len,
                    origin,
                },
                location,
            ));

            if let Some(subscription) = reactive_subscription {
                store
                    .set_node_reactive_subscription(node_id, subscription)
                    .expect("a just-pushed text node must accept a reactive subscription");
            }

            node_id
        };

        self.children.push(node_id);
        self.note_head_origin(origin);
    }

    // -------------------------
    //  Recording — expressions
    // -------------------------

    pub(crate) fn record_head_dynamic_expression(
        &mut self,
        expression: Expression,
        reactive_subscription: Option<ReactiveSubscription>,
        location: SourceLocation,
    ) {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let site_id = store.next_expression_site_id();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::DynamicExpression {
                    expression: Box::new(expression),
                    origin: TemplateSegmentOrigin::Head,
                    reactive_subscription,
                    site_id,
                },
                location,
            ))
        };

        self.children.push(node_id);
        self.note_head_origin(TemplateSegmentOrigin::Head);
    }

    // -------------------------
    //  Recording — structure
    // -------------------------

    pub(crate) fn record_child_template(
        &mut self,
        child_reference: &TemplateTirReference,
        origin: TemplateSegmentOrigin,
        location: SourceLocation,
    ) {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let occurrence_id = store.next_child_template_occurrence_id();
            let reference = TemplateTirChildReference::new(
                child_reference.root,
                child_reference.phase,
                child_reference.context,
            );
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference,
                    occurrence_id,
                },
                location,
            ))
        };

        self.children.push(node_id);
        self.note_head_origin(origin);
    }

    pub(crate) fn record_slot(
        &mut self,
        slot: SlotPlaceholder,
        location: SourceLocation,
    ) -> Result<(), TemplateError> {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let placeholder = store.tir_slot_placeholder_from_ast(&slot, location.clone())?;
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Slot { placeholder },
                location,
            ))
        };

        self.children.push(node_id);
        Ok(())
    }

    pub(crate) fn record_insert_contribution(
        &mut self,
        contribution_template_id: TemplateIrId,
        location: SourceLocation,
    ) {
        let node_id = {
            let mut store = self.store.borrow_mut();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::InsertContribution {
                    template: contribution_template_id,
                },
                location,
            ))
        };

        self.children.push(node_id);
    }

    // -------------------------
    //  Recording — control flow
    // -------------------------

    pub(crate) fn record_branch_chain(
        &mut self,
        branches: Vec<TemplateIrBranch>,
        fallback: Option<TemplateIrNodeId>,
        location: SourceLocation,
    ) {
        let node_id = {
            let mut store = self.store.borrow_mut();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::BranchChain { branches, fallback },
                location,
            ))
        };

        self.record_control_flow_node(node_id);
    }

    pub(crate) fn record_loop(
        &mut self,
        header: TemplateLoopHeader,
        body: TemplateIrNodeId,
        location: SourceLocation,
    ) {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let header_sites = store.allocate_loop_header_expression_sites(&header);
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Loop {
                    header,
                    header_sites,
                    body,
                    aggregate_wrapper: None,
                },
                location,
            ))
        };

        self.record_control_flow_node(node_id);
    }

    pub(crate) fn record_loop_control(
        &mut self,
        kind: TemplateLoopControlKind,
        location: SourceLocation,
    ) {
        let node_id = {
            let mut store = self.store.borrow_mut();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::LoopControl { kind },
                location,
            ))
        };

        self.children.push(node_id);
    }

    fn record_control_flow_node(&mut self, node_id: TemplateIrNodeId) {
        self.children.push(node_id);
        if self.control_flow_node_id.is_none() {
            self.control_flow_node_id = Some(node_id);
        }
    }

    fn note_head_origin(&mut self, origin: TemplateSegmentOrigin) {
        if origin == TemplateSegmentOrigin::Head {
            self.head_node_count += 1;
        }
    }

    // -------------------------
    //  Whitespace trimming
    // -------------------------

    pub(crate) fn trim_leading_whitespace(&mut self, string_table: &StringTable) {
        let store = self.store.borrow();
        let first_meaningful_index = self
            .children
            .iter()
            .position(|child_id| !node_is_whitespace_only_text(*child_id, &store, string_table))
            .unwrap_or(self.children.len());

        if first_meaningful_index == 0 {
            return;
        }

        drop(store);
        self.children.drain(0..first_meaningful_index);
    }

    pub(crate) fn trim_trailing_whitespace(&mut self, string_table: &StringTable) {
        let store = self.store.borrow();

        while self
            .children
            .last()
            .is_some_and(|child_id| node_is_whitespace_only_text(*child_id, &store, string_table))
        {
            if self.children.pop().is_none() {
                break;
            }
        }
    }

    // -------------------------
    //  Finalization
    // -------------------------

    /// Seals recorded children into a store template and returns the durable
    /// reference. The construction context cannot be reused after this call.
    pub(crate) fn finish(
        self,
        style: Style,
        kind: TemplateType,
        phase: TemplateTirPhase,
    ) -> Result<TemplateTirReference, CompilerError> {
        let TemplateConstructionContext {
            store,
            children,
            mut head_node_count,
            control_flow_node_id,
            location,
        } = self;

        let mut store = store.borrow_mut();

        // Render-unit preparation moves a control-flow template's shared head
        // prefix into branch bodies or the loop aggregate wrapper. The owner
        // root must not retain those prefix nodes as ordinary siblings, or
        // skipped branches and zero-iteration loops still render the wrapper
        // shell.
        let root_children: Vec<TemplateIrNodeId> = if control_flow_node_id.is_some() {
            let first_control_flow_index = children.iter().position(|&child_id| {
                store.get_node(child_id).is_some_and(|node| {
                    matches!(
                        node.kind,
                        TemplateIrNodeKind::BranchChain { .. } | TemplateIrNodeKind::Loop { .. }
                    )
                })
            });

            match first_control_flow_index {
                Some(index) if index > 0 => {
                    head_node_count = 0;
                    children[index..].to_vec()
                }
                _ => children,
            }
        } else {
            children
        };

        // Use a single control-flow child directly as the root. Linear
        // templates and multi-child owner roots stay Sequence-shaped.
        let root = match root_children.as_slice() {
            [child_id]
                if store.get_node(*child_id).is_some_and(|node| {
                    matches!(
                        node.kind,
                        TemplateIrNodeKind::BranchChain { .. } | TemplateIrNodeKind::Loop { .. }
                    )
                }) =>
            {
                *child_id
            }
            _ => store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Sequence {
                    children: root_children,
                },
                location.clone(),
            )),
        };

        let mut summary = summarize_existing_root(&store, root)?;
        summary.set_head_node_count(head_node_count);

        let template_id =
            store.push_template(TemplateIr::new(root, style, kind, summary, location));

        Ok(TemplateTirReference {
            root: template_id,
            phase,
            context: TemplateViewContext::default(),
        })
    }
}

fn node_is_whitespace_only_text(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> bool {
    let Some(node) = store.get_node(node_id) else {
        return false;
    };
    let TemplateIrNodeKind::Text { text, .. } = &node.kind else {
        return false;
    };

    string_table.resolve(*text).trim().is_empty()
}
