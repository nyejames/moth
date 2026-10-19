//! Parser-facing TIR construction owner.
//!
//! One `TemplateConstructionContext` holds the module store handle, recorded
//! root children, control-flow node, and head-node count while a template is
//! being parsed. Parser callers record through this type and call store
//! allocation here. After `finish`, the durable value is a
//! `TemplateTirReference`; this context is consumed.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::LoopBindings;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::ExprId;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, DeferredExpressionSuffix, DeferredNumericExpression, DeferredRangeHeader,
    NumericBindingId, NumericConstructionDomainId, StatementExpression,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    SlotPlaceholder, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::ids::{TemplateIrId, TemplateIrNodeId};
use crate::compiler_frontend::ast::templates::tir::node::{
    RetainedNumericIntakeRole, TemplateBranchSelectorKind, TemplateIr, TemplateIrNode,
    TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::ast::templates::tir::numeric_intake::{
    NumericTemplateIntakePosition, RangeOperandSlot,
};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateTirReference,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::summary::summarize_existing_root;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// Parser-local owner for in-progress TIR emission.
pub(crate) struct TemplateConstructionContext {
    store: Rc<RefCell<TemplateIrStore>>,
    children: Vec<TemplateIrNodeId>,
    head_node_count: u32,
    control_flow_node_id: Option<TemplateIrNodeId>,
    span: Option<SourceSpan>,
}

/// True when this node is (or will complete into) a control-flow owner root.
///
/// WHAT: a built `Conditional`/`Loop` node, or a retained position whose role completes into a
///       conditional `loop` node.
/// WHY: `finish` uses a single control-flow child directly as the template root; a retained loop
///      condition is such a child before completion installs its `Loop` node.
fn is_control_flow_root_node(node: &TemplateIrNode) -> bool {
    match &node.kind {
        TemplateIrNodeKind::Conditional { .. } | TemplateIrNodeKind::Loop { .. } => true,
        TemplateIrNodeKind::RetainedNumericIntake(role) => role.completes_into_control_flow(),
        _ => false,
    }
}

impl TemplateConstructionContext {
    pub(crate) fn new(store: Rc<RefCell<TemplateIrStore>>, span: Option<SourceSpan>) -> Self {
        Self {
            store,
            children: Vec::new(),
            head_node_count: 0,
            control_flow_node_id: None,
            span,
        }
    }
    pub(crate) fn span(&self) -> Option<SourceSpan> {
        self.span
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

    // -------------------------
    //  Recording — text
    // -------------------------

    pub(crate) fn record_text(
        &mut self,
        text: StringId,
        byte_len: usize,
        span: Option<SourceSpan>,
    ) {
        self.record_text_segment(text, byte_len, TemplateSegmentOrigin::Body, span);
    }

    pub(crate) fn record_head_text(
        &mut self,
        text: StringId,
        byte_len: usize,
        span: Option<SourceSpan>,
    ) {
        self.record_text_segment(text, byte_len, TemplateSegmentOrigin::Head, span);
    }

    fn record_text_segment(
        &mut self,
        text: StringId,
        byte_len: usize,
        origin: TemplateSegmentOrigin,
        span: Option<SourceSpan>,
    ) {
        let node_id = {
            let mut store = self.store.borrow_mut();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Text {
                    text,
                    byte_len,
                    origin,
                },
                span,
            ))
        };

        self.children.push(node_id);
        self.note_head_origin(origin);
    }

    // -------------------------
    //  Recording — expressions
    // -------------------------

    pub(crate) fn record_head_dynamic_expression(
        &mut self,
        expression: ExprId,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let site_id = store.next_expression_site_id();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::DynamicExpression {
                    expression,
                    origin: TemplateSegmentOrigin::Head,
                    site_id,
                },
                span,
            ))
        };

        self.children.push(node_id);
        self.note_head_origin(TemplateSegmentOrigin::Head);

        node_id
    }

    /// Records one retained head numeric intake as a payload-free TIR position.
    ///
    /// WHAT: appends a `RetainedNumericIntake` node at the head's document position, allocates the
    ///       expression site reserved for the completed value, and registers the retained fragment
    ///       with the store under the owning numeric body domain.
    /// WHY: plan N23 forbids selecting a pending value's type from a non-constraining formatting
    ///      consumer, so the position must not hold a typed payload; body-root completion resumes
    ///      the ordinary typed expression intake and installs the completed value here.
    pub(crate) fn record_head_numeric_intake(
        &mut self,
        domain: NumericConstructionDomainId,
        items: Vec<ExpressionRpnItem>,
        expected: ExpectedType,
        value_mode: ValueMode,
        suffix: DeferredExpressionSuffix,
        span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let expression_site = store.next_expression_site_id();
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::RetainedNumericIntake(
                    RetainedNumericIntakeRole::HeadExpression { expression_site },
                ),
                span,
            ));
            store.register_pending_numeric_template_intake(
                domain,
                NumericTemplateIntakePosition::HeadExpression(node_id),
                items,
                expected,
                value_mode,
                suffix,
            )?;
            node_id
        };

        self.children.push(node_id);
        self.note_head_origin(TemplateSegmentOrigin::Head);
        Ok(())
    }

    // -------------------------
    //  Recording — structure
    // -------------------------

    pub(crate) fn record_child_template(
        &mut self,
        child_reference: &TemplateTirReference,
        origin: TemplateSegmentOrigin,
        span: Option<SourceSpan>,
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
                span,
            ))
        };

        self.children.push(node_id);
        self.note_head_origin(origin);
    }

    pub(crate) fn record_slot(
        &mut self,
        slot: SlotPlaceholder,
        span: Option<SourceSpan>,
    ) -> Result<(), TemplateError> {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let placeholder = store.tir_slot_placeholder_from_ast(&slot)?;
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Slot { placeholder },
                span,
            ))
        };

        self.children.push(node_id);
        Ok(())
    }

    pub(crate) fn record_insert_contribution(
        &mut self,
        contribution_template_id: TemplateIrId,
        span: Option<SourceSpan>,
    ) {
        let node_id = {
            let mut store = self.store.borrow_mut();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::InsertContribution {
                    template: contribution_template_id,
                },
                span,
            ))
        };

        self.children.push(node_id);
    }

    // -------------------------
    //  Recording — control flow
    // -------------------------

    pub(crate) fn record_conditional(
        &mut self,
        selector: TemplateBranchSelector,
        body: TemplateIrNodeId,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let selector_site_id = store.next_expression_site_id();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Conditional {
                    selector: Box::new(selector),
                    selector_site_id,
                    body,
                },
                span,
            ))
        };

        self.record_control_flow_node(node_id);

        node_id
    }

    pub(crate) fn record_loop(
        &mut self,
        header: TemplateLoopHeader,
        body: TemplateIrNodeId,
        span: Option<SourceSpan>,
    ) -> TemplateIrNodeId {
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
                span,
            ))
        };

        self.record_control_flow_node(node_id);

        node_id
    }

    fn record_control_flow_node(&mut self, node_id: TemplateIrNodeId) {
        self.children.push(node_id);
        if self.control_flow_node_id.is_none() {
            self.control_flow_node_id = Some(node_id);
        }
    }

    /// Records one retained loop condition as a payload-free TIR position.
    ///
    /// WHAT: appends a `RetainedNumericIntake` node where the `loop` node belongs, allocates the
    ///       loop's condition expression site (the same site an ordinary conditional loop
    ///       allocates), and registers the retained fragment under the enclosing body's domain.
    /// WHY: the suffix parser has already parsed the loop body, but the condition still
    ///      references a numeric origin with no selected type, so the loop node cannot be built
    ///      yet. Carrying the body node and the header site on the position lets body-root
    ///      completion install the whole `Loop` node in place, in authored document order.
    pub(crate) fn record_retained_loop_condition(
        &mut self,
        domain: NumericConstructionDomainId,
        deferred: DeferredNumericExpression,
        body: TemplateIrNodeId,
        span: Option<SourceSpan>,
    ) -> Result<TemplateIrNodeId, CompilerError> {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let header_sites = TemplateLoopHeaderExpressionSites::Conditional {
                condition: store.next_expression_site_id(),
            };
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::RetainedNumericIntake(
                    RetainedNumericIntakeRole::ConditionalLoopCondition {
                        header_sites,
                        body,
                        render_unit: None,
                    },
                ),
                span,
            ));
            store.register_pending_numeric_template_intake(
                domain,
                NumericTemplateIntakePosition::ConditionalLoopCondition(node_id),
                deferred.items,
                deferred.expected,
                deferred.value_mode,
                deferred.suffix,
            )?;
            node_id
        };

        self.record_control_flow_node(node_id);
        Ok(node_id)
    }

    /// Records one retained range loop header as a payload-free TIR position.
    ///
    /// WHAT: appends a `RetainedNumericIntake` node where the `loop` node belongs, allocates the
    ///       header's expression sites, and records how the header completes:
    ///       an unbound header registers one intake record per retained operand plus its store row
    ///       holding the authored end kind, omitted-start site and already built operands; a bound
    ///       header registers its counter row reference plus the concrete positional index.
    /// WHY: the header's exact domain is a forward result of its operands' selected types, so the
    ///      body parses against the authored bindings while the header stays payload-free until
    ///      the owning body root completes it — through the shared range policy for an unbound
    ///      header, or through the counter's own recipe for a bound one.
    pub(crate) fn record_retained_loop_range(
        &mut self,
        domain: NumericConstructionDomainId,
        header: DeferredRangeHeader,
        body: TemplateIrNodeId,
        span: Option<SourceSpan>,
    ) -> Result<TemplateIrNodeId, CompilerError> {
        match header {
            DeferredRangeHeader::Ready { bindings, range } => {
                // The header is already concrete: the loop was retained for its body only, and a
                // template body's own pending positions complete at this same body root, so the
                // loop node needs no retained position of its own.
                Ok(self.record_loop(TemplateLoopHeader::Range { bindings, range }, body, span))
            }

            DeferredRangeHeader::Unbound(spec) => {
                let node_id = {
                    let mut store = self.store.borrow_mut();

                    let has_step = spec.step.is_some();
                    let header_sites = TemplateLoopHeaderExpressionSites::Range {
                        start: store.next_expression_site_id(),
                        end: store.next_expression_site_id(),
                        step: has_step.then(|| store.next_expression_site_id()),
                    };

                    let node_id = store.push_node(TemplateIrNode::new(
                        TemplateIrNodeKind::RetainedNumericIntake(
                            RetainedNumericIntakeRole::LoopRange {
                                header_sites,
                                body,
                                render_unit: None,
                            },
                        ),
                        span,
                    ));

                    let mut built_start = None;
                    let mut built_end = None;
                    let mut built_step = None;

                    for (slot, operand) in [
                        (RangeOperandSlot::Start, spec.start),
                        (RangeOperandSlot::End, Some(spec.end)),
                        (RangeOperandSlot::Step, spec.step),
                    ] {
                        let Some(operand) = operand else {
                            continue;
                        };

                        match operand {
                            StatementExpression::Built(expression) => match slot {
                                RangeOperandSlot::Start => built_start = Some(expression),
                                RangeOperandSlot::End => built_end = Some(expression),
                                RangeOperandSlot::Step => built_step = Some(expression),
                            },

                            StatementExpression::Retained(deferred) => {
                                let position = match slot {
                                    RangeOperandSlot::Start => {
                                        NumericTemplateIntakePosition::LoopRangeStart(node_id)
                                    }
                                    RangeOperandSlot::End => {
                                        NumericTemplateIntakePosition::LoopRangeEnd(node_id)
                                    }
                                    RangeOperandSlot::Step => {
                                        NumericTemplateIntakePosition::LoopRangeStep(node_id)
                                    }
                                };

                                store.register_pending_numeric_template_intake(
                                    domain,
                                    position,
                                    deferred.items,
                                    deferred.expected,
                                    deferred.value_mode,
                                    deferred.suffix,
                                )?;
                            }
                        }
                    }

                    store.register_retained_range_header_operand_slots(
                        domain,
                        node_id,
                        Box::new(LoopBindings {
                            item: None,
                            index: None,
                        }),
                        spec.end_kind,
                        spec.span.or(span),
                        [built_start, built_end, built_step],
                    );

                    node_id
                };

                self.record_control_flow_node(node_id);
                Ok(node_id)
            }

            DeferredRangeHeader::Bound(retained) => {
                let node_id = {
                    let mut store = self.store.borrow_mut();

                    // The counter's recipe owns the operands and computes the forward domain, so
                    // this position needs no operand records. Start and end sites are always
                    // allocated; the step site is allocated with the finished spec if the recipe
                    // produced a step.
                    let header_sites = TemplateLoopHeaderExpressionSites::Range {
                        start: store.next_expression_site_id(),
                        end: store.next_expression_site_id(),
                        step: None,
                    };

                    let node_id = store.push_node(TemplateIrNode::new(
                        TemplateIrNodeKind::RetainedNumericIntake(
                            RetainedNumericIntakeRole::LoopRange {
                                header_sites,
                                body,
                                render_unit: None,
                            },
                        ),
                        span,
                    ));

                    store.register_retained_range_header_counter(
                        domain,
                        node_id,
                        retained.counter,
                        retained.index,
                    );

                    node_id
                };

                self.record_control_flow_node(node_id);
                Ok(node_id)
            }
        }
    }

    /// Records one retained collection loop header position.
    ///
    /// WHAT: keeps the parsed bindings in a store row, pushes the payload-free position that holds
    ///       the iterable's expression site and the body node, and registers the authored iterable
    ///       fragment against that position.
    /// WHY: the callee's returned collection is known, so the bindings are final at parse time, but
    ///      the iterable expression still reaches a pending numeric origin whose single static type
    ///      the owning body root selects; completion installs the concrete `Loop` node in place.
    ///      Registering this retained position as the template's control-flow child keeps it in
    ///      the same root and render-unit preparation path as a concrete loop.
    pub(crate) fn record_retained_loop_collection(
        &mut self,
        domain: NumericConstructionDomainId,
        bindings: Box<LoopBindings>,
        pending_item: Option<NumericBindingId>,
        deferred: DeferredNumericExpression,
        body: TemplateIrNodeId,
        span: Option<SourceSpan>,
    ) -> Result<TemplateIrNodeId, CompilerError> {
        let node_id = {
            let mut store = self.store.borrow_mut();

            let header_sites = TemplateLoopHeaderExpressionSites::Collection {
                iterable: store.next_expression_site_id(),
            };

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::RetainedNumericIntake(
                    RetainedNumericIntakeRole::LoopCollection {
                        header_sites,
                        body,
                        render_unit: None,
                    },
                ),
                span,
            ));

            store.register_retained_collection_header(node_id, bindings, pending_item);
            store.register_pending_numeric_template_intake(
                domain,
                NumericTemplateIntakePosition::LoopCollectionIterable(node_id),
                deferred.items,
                deferred.expected,
                deferred.value_mode,
                deferred.suffix,
            )?;
            node_id
        };

        self.record_control_flow_node(node_id);
        Ok(node_id)
    }

    /// Records one retained conditional selector as a payload-free TIR position.
    ///
    /// WHAT: appends a `RetainedNumericIntake` node where the `conditional` node belongs, allocates
    ///       the branch's selector site, keeps an option-present capture pattern in store scratch,
    ///       and registers the retained fragment under the enclosing body's domain.
    /// WHY: a branch's subject type is selected at the body root, so the conditional node cannot be
    ///      built at parse time even though the branch body still parses under the selector's
    ///      binding scope.
    pub(crate) fn record_retained_conditional_selector(
        &mut self,
        domain: NumericConstructionDomainId,
        deferred: DeferredNumericExpression,
        pattern: Option<Box<DeferredArmPattern>>,
        body: TemplateIrNodeId,
        span: Option<SourceSpan>,
    ) -> Result<TemplateIrNodeId, CompilerError> {
        let node_id = {
            let mut store = self.store.borrow_mut();
            let selector_site_id = store.next_expression_site_id();
            let kind = if pattern.is_some() {
                TemplateBranchSelectorKind::OptionPresent
            } else {
                TemplateBranchSelectorKind::Bool
            };

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::RetainedNumericIntake(
                    RetainedNumericIntakeRole::ConditionalSelector {
                        selector_site_id,
                        body,
                        kind,
                        render_unit: None,
                    },
                ),
                span,
            ));

            if let Some(pattern) = pattern {
                store.register_retained_selector_pattern(node_id, pattern);
            }

            store.register_pending_numeric_template_intake(
                domain,
                NumericTemplateIntakePosition::ConditionalSelector(node_id),
                deferred.items,
                deferred.expected,
                deferred.value_mode,
                deferred.suffix,
            )?;

            node_id
        };

        self.record_control_flow_node(node_id);
        Ok(node_id)
    }

    fn note_head_origin(&mut self, origin: TemplateSegmentOrigin) {
        if origin == TemplateSegmentOrigin::Head {
            self.head_node_count += 1;
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
            mut children,
            mut head_node_count,
            control_flow_node_id,
            span,
        } = self;

        let mut store = store.borrow_mut();

        // Render-unit preparation moves a control-flow template's shared head
        // prefix into its conditional body or loop aggregate wrapper. The
        // owner root must not retain those prefix nodes as ordinary siblings, or
        // unselected conditionals and zero-iteration loops still render the wrapper
        // shell.
        let root_children: Vec<TemplateIrNodeId> =
            if let Some(control_flow_node_id) = control_flow_node_id {
                // The recorded control-flow child is the first control-flow node this context
                // recorded. Matching it by identity also finds a retained control-flow position,
                // whose node kind is still the payload-free placeholder instead of a `Loop`.
                let first_control_flow_index = children
                    .iter()
                    .position(|&child_id| child_id == control_flow_node_id);

                match first_control_flow_index {
                    Some(index) if index > 0 => {
                        head_node_count = 0;
                        drop(children.drain(..index));
                        children
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
                if store
                    .get_node(*child_id)
                    .is_some_and(is_control_flow_root_node) =>
            {
                *child_id
            }
            _ => store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Sequence {
                    children: root_children,
                },
                span,
            )),
        };

        let mut summary = summarize_existing_root(&store, root)?;
        summary.set_head_node_count(head_node_count);

        let template_id = store.push_template(TemplateIr::new(root, style, kind, summary, span));

        Ok(TemplateTirReference {
            root: template_id,
            phase,
            context: TemplateViewContext::default(),
        })
    }
}
