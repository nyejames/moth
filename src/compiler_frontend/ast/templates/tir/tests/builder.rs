//! Test-only TIR fixture builder.
//!
//! Production parser emission uses `TemplateConstructionContext` and the store.
//! Tests keep this facade so fixtures can push nodes without repeating store
//! allocation details.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateElseMarker, TemplateLoopControlKind, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::ids::{TemplateIrId, TemplateIrNodeId};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder,
};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::summary::TemplateIrSummary;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

pub(crate) struct TemplateIrBuilder<'store> {
    pub(crate) store: &'store mut TemplateIrStore,
}

impl<'store> TemplateIrBuilder<'store> {
    pub(crate) fn new(store: &'store mut TemplateIrStore) -> Self {
        Self { store }
    }

    pub(crate) fn push_text_node_with_subscription(
        &mut self,
        text: StringId,
        byte_len: usize,
        origin: TemplateSegmentOrigin,
        reactive_subscription: Option<ReactiveSubscription>,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        let node_id = self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Text {
                text,
                byte_len,
                origin,
            },
            span,
        ));

        if let Some(subscription) = reactive_subscription {
            self.store
                .set_node_reactive_subscription(node_id, subscription)
                .expect("a just-pushed node must accept a reactive subscription");
        }

        node_id
    }

    pub(crate) fn push_text_node(
        &mut self,
        text: StringId,
        byte_len: usize,
        origin: TemplateSegmentOrigin,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        self.push_text_node_with_subscription(text, byte_len, origin, None, span)
    }

    pub(crate) fn push_sequence_node(
        &mut self,
        children: Vec<TemplateIrNodeId>,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children },
            span,
        ))
    }

    pub(crate) fn push_child_template_node_with_reference(
        &mut self,
        reference: TemplateTirChildReference,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        let occurrence_id = self.store.next_child_template_occurrence_id();
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference,
                occurrence_id,
            },
            span,
        ))
    }

    pub(crate) fn push_child_template_node(
        &mut self,
        template: TemplateIrId,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        let reference = TemplateTirChildReference::new(
            template,
            TemplateTirPhase::Parsed,
            TemplateViewContext::default(),
        );
        self.push_child_template_node_with_reference(reference, span)
    }

    pub(crate) fn push_dynamic_expression_node(
        &mut self,
        expression: Expression,
        origin: TemplateSegmentOrigin,
        reactive_subscription: Option<ReactiveSubscription>,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        let site_id = self.store.next_expression_site_id();
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: Box::new(expression),
                origin,
                reactive_subscription,
                site_id,
            },
            span,
        ))
    }

    pub(crate) fn push_tir_slot_placeholder_node(
        &mut self,
        placeholder: TirSlotPlaceholder,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Slot { placeholder },
            span,
        ))
    }

    pub(crate) fn push_slot_node(
        &mut self,
        key: SlotKey,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        let occurrence_id = self.store.next_slot_occurrence_id();
        let placeholder =
            TirSlotPlaceholder::with_wrapper_sets(key, occurrence_id, None, None, false);
        self.push_tir_slot_placeholder_node(placeholder, span)
    }

    pub(crate) fn push_insert_contribution_node(
        &mut self,
        template: TemplateIrId,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::InsertContribution { template },
            span,
        ))
    }

    pub(crate) fn push_branch_chain_node(
        &mut self,
        branches: Vec<TemplateIrBranch>,
        fallback: Option<TemplateIrNodeId>,
        else_marker: Option<TemplateElseMarker>,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::BranchChain {
                branches,
                fallback,
                else_marker,
            },
            span,
        ))
    }

    pub(crate) fn push_loop_node(
        &mut self,
        header: TemplateLoopHeader,
        body: TemplateIrNodeId,
        aggregate_wrapper: Option<TemplateIrNodeId>,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        let header_sites = self.store.allocate_loop_header_expression_sites(&header);
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Loop {
                header,
                header_sites,
                body,
                aggregate_wrapper,
            },
            span,
        ))
    }

    pub(crate) fn push_loop_control_node(
        &mut self,
        kind: TemplateLoopControlKind,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::LoopControl { kind },
            span,
        ))
    }

    pub(crate) fn finish_template(
        &mut self,
        root: TemplateIrNodeId,
        style: Style,
        kind: TemplateType,
        summary: TemplateIrSummary,
        span: Option<SourceSpan>,
    ) -> TemplateIrId {
        self.store
            .push_template(TemplateIr::new(root, style, kind, summary, span))
    }
}
