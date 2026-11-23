//! Selected evidence from AST code that HIR lowering intentionally omits.
//!
//! WHAT: walks the retained AST of skipped catch handlers and statements after a terminator,
//!       recording every selected call site and `$fast_math` range guard in the owner's evidence.
//! WHY: constraint verification treats fast selection as a property of selected source, not of
//!      liveness. Lowering omits statically infallible handlers and dead statements after a
//!      terminator, so the ordinary HIR scan cannot see the calls and range guards they still
//!      select. Recording at the skip point keeps the lowering optimisations without losing the
//!      evidence. Release speculation never folds a catch handler body or a handled subject, so
//!      this walk sees the same selected source in both profiles.
//!
//! The expression walk owns calls, nested value blocks and owned runtime template handoffs; the
//! statement walk owns `-> !` calls and statement range guards. Each nested value block is visited
//! through its expression edge, so the two traversals do not duplicate one another.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ExpressionStore, ResolvedCallableTarget,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::control_flow_exits::{
    ConditionLookup, range_ordinal_update_is_emitted,
};
use crate::compiler_frontend::ast::statements::never_call::{NeverCall, NeverCallTarget};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeTemplateNode, visit_node_expression_ids,
    walk_owned_runtime_slot_application_handoff, walk_owned_runtime_template_handoff,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::failure_facts::HirFastUseProvenance;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;

impl<'a> HirBuilder<'a> {
    /// Records evidence from a catch handler that static infallibility removes.
    ///
    /// WHAT: walks the handler's retained statements and expressions as selected source.
    /// WHY: the handler body has no HIR blocks, so the ordinary CFG scan cannot see its selected
    ///      callees or its fast range guards. The walk visits nested value blocks through the
    ///      expression store's edges, including nested catch handlers, without lowering dead work.
    pub(super) fn record_skipped_catch_handler_evidence(
        &mut self,
        store: &ExpressionStore,
        handler: &FallibleHandling,
    ) -> Result<(), HirConstructionFailure> {
        let FallibleHandling::Handler { body, .. } = handler else {
            return Ok(());
        };
        self.record_skipped_statement_evidence(store, body)
    }

    /// Records evidence from selected statements that the CFG never receives.
    ///
    /// WHAT: walks a skipped handler body or the statements remaining after a terminator.
    /// WHY: both skip paths omit selected source for the same optimisation reason, so one walker
    ///      owns their call sites and fast range-guard witnesses. The release producer never folds
    ///      an authored-dead statement, a catch handler or a handled subject, so both walked bodies
    ///      still contain the calls and operations the selected-source evidence describes.
    pub(in crate::compiler_frontend::hir) fn record_skipped_statement_evidence(
        &mut self,
        store: &ExpressionStore,
        nodes: &[AstNode],
    ) -> Result<(), HirConstructionFailure> {
        let generation = self.skipped_source_walk.begin_walk();
        for node in nodes {
            node.visit_expression_ids(&mut |id| self.skipped_source_walk.push_root(id));
        }
        self.record_skipped_statement_nodes(store, nodes)?;
        while let Some(id) = self.skipped_source_walk.pop_root() {
            if !self.skipped_source_walk.visit_once(id, generation) {
                continue;
            }
            let view = store.view(id)?;
            match view.operation() {
                ExpressionOperationView::Call(call)
                | ExpressionOperationView::MethodCall { call, .. } => {
                    let metadata = call.metadata();
                    let target = match metadata.target {
                        ResolvedCallableTarget::Function(path) => {
                            self.resolve_call_target_or_error(&path)?
                        }
                        ResolvedCallableTarget::Host(id) => CallTarget::External(id),
                    };
                    let function = self.current_function_id_or_error()?;
                    self.record_skipped_call_site(function, target, metadata.span);
                }
                ExpressionOperationView::ValueBlock(block) => {
                    self.record_skipped_value_block_nodes(store, block)?;
                }
                ExpressionOperationView::RuntimeTemplate(handoff) => {
                    walk_owned_runtime_template_handoff(handoff, &mut |node| {
                        self.record_skipped_template_node(store, node)
                    })?;
                }
                ExpressionOperationView::RuntimeSlotApplication(handoff) => {
                    walk_owned_runtime_slot_application_handoff(handoff, &mut |node| {
                        self.record_skipped_template_node(store, node)
                    })?;
                }
                _ => {}
            }
            view.visit_child_ids(&mut |child| self.skipped_source_walk.push_root(child))?;
        }
        Ok(())
    }

    /// Records `Never` calls and fast range guards in one omitted statement body.
    ///
    /// WHAT: walks statement bodies that the ordinary HIR call scan never sees.
    /// WHY: a `-> !` call and a `$fast_math` range guard are both selected computations even when
    ///      lowering removes the surrounding statement sequence. Nested value blocks are reached
    ///      separately through the expression walk, so their expression edges remain the single
    ///      traversal owner.
    fn record_skipped_statement_nodes(
        &mut self,
        store: &ExpressionStore,
        nodes: &[AstNode],
    ) -> Result<(), HirConstructionFailure> {
        for node in nodes {
            match &node.kind {
                NodeKind::NeverCall(call) => self.record_skipped_never_call(call)?,
                NodeKind::RangeLoop {
                    bindings,
                    range,
                    body,
                } => {
                    // Mirror lowering exactly: a `$fast_math` loop selects range evidence only for
                    // the checks lowering emits. Lowering drops the step block, and with it the
                    // generated ordinal update, when the body can neither fall through nor
                    // `continue`; a skipped copy must not record a trap a live copy never runs.
                    // Conditions use the pre-fold selection table so a release-only fold cannot
                    // turn an authored fallthrough into a terminator here while debug keeps it.
                    if range.fast_math {
                        let ordinal_update_is_emitted = range_ordinal_update_is_emitted(
                            bindings,
                            body,
                            store,
                            ConditionLookup::PreFold(&self.pre_fold_bool_conditions),
                        );
                        if range.has_generated_range_check(ordinal_update_is_emitted) {
                            let function = self.current_function_id_or_error()?;
                            self.record_skipped_fast_use(
                                function,
                                HirFastUseProvenance::RangeGuard,
                                node.span,
                            );
                        }
                    }
                    self.record_skipped_statement_nodes(store, body)?;
                }
                NodeKind::If(_, then_body, else_body, _) => {
                    self.record_skipped_statement_nodes(store, then_body)?;
                    if let Some(else_body) = else_body {
                        self.record_skipped_statement_nodes(store, else_body)?;
                    }
                }
                NodeKind::Match { arms, default, .. } => {
                    for arm in arms {
                        self.record_skipped_statement_nodes(store, &arm.body)?;
                    }
                    if let Some(default) = default {
                        self.record_skipped_statement_nodes(store, default)?;
                    }
                }
                NodeKind::LexicalScope { body }
                | NodeKind::Function(_, _, body)
                | NodeKind::EntryStart(_, body) => {
                    self.record_skipped_statement_nodes(store, body)?;
                }
                NodeKind::CollectionLoop { body, .. } | NodeKind::WhileLoop(_, body) => {
                    self.record_skipped_statement_nodes(store, body)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Records one owned runtime handoff node's expression roots and range-guard evidence.
    ///
    /// WHAT: queues every expression the node owns for the evidence walk, then records the fast
    ///       range guards a loop node selects.
    /// WHY: an owned handoff carries its expressions and loops outside the statement tree, so the
    ///      ordinary expression and statement walks never reach them. Both handoff shapes share
    ///      this per-node step.
    fn record_skipped_template_node(
        &mut self,
        store: &ExpressionStore,
        node: &OwnedRuntimeTemplateNode,
    ) -> Result<(), HirConstructionFailure> {
        visit_node_expression_ids(node, &mut |child| self.skipped_source_walk.push_root(child));
        self.record_skipped_template_loop_evidence(store, node)
    }

    /// Records one owned runtime loop's fast range-guard rows.
    ///
    /// WHAT: mirrors live template lowering for a template loop inside a skipped region: a bound
    ///       ordinal owns the generated checked update, and an authored step owns the generated
    ///       zero-step guard.
    /// WHY: HIR's skipped statement walk sees statement loops, but an owned template handoff
    ///      carries its loops inside the handoff tree rather than the statement tree. Without this
    ///      arm a template loop after a terminator would lose its selected `$safe_math` evidence.
    fn record_skipped_template_loop_evidence(
        &mut self,
        store: &ExpressionStore,
        node: &OwnedRuntimeTemplateNode,
    ) -> Result<(), HirConstructionFailure> {
        let OwnedRuntimeTemplateNode::Loop { header, span, .. } = node else {
            return Ok(());
        };
        let TemplateLoopHeader::Range { bindings, range } = header else {
            return Ok(());
        };
        if !range.fast_math {
            return Ok(());
        }
        let function = self.current_function_id_or_error()?;
        if bindings.index.is_some() {
            self.record_skipped_fast_use(function, HirFastUseProvenance::RangeGuard, *span);
        }
        if let Some(step) = range.step {
            // HIR labels the generated zero-step guard with the authored step's own span.
            let step_span = store.view(step)?.span();
            self.record_skipped_fast_use(function, HirFastUseProvenance::RangeGuard, step_span);
        }
        Ok(())
    }

    fn record_skipped_value_block_nodes(
        &mut self,
        store: &ExpressionStore,
        block: &ValueBlock,
    ) -> Result<(), HirConstructionFailure> {
        match block {
            ValueBlock::If(block) => {
                self.record_skipped_statement_nodes(store, &block.then_body)?;
                self.record_skipped_statement_nodes(store, &block.else_body)
            }
            ValueBlock::LexicalScope(block) => {
                self.record_skipped_statement_nodes(store, &block.body)
            }
            ValueBlock::Match(block) => {
                for arm in &block.arms {
                    self.record_skipped_statement_nodes(store, &arm.body)?;
                }
                if let Some(default) = &block.default {
                    self.record_skipped_statement_nodes(store, default)?;
                }
                Ok(())
            }
            ValueBlock::Catch(block) => match &block.handler {
                FallibleHandling::Handler { body, .. } => {
                    self.record_skipped_statement_nodes(store, body)
                }
                FallibleHandling::Propagate => Ok(()),
            },
        }
    }

    fn record_skipped_never_call(
        &mut self,
        call: &NeverCall,
    ) -> Result<(), HirConstructionFailure> {
        let function = self.current_function_id_or_error()?;
        let target = match &call.target {
            NeverCallTarget::Source(path) => self.resolve_call_target_or_error(path)?,
            NeverCallTarget::External(id) => CallTarget::External(*id),
        };
        self.record_skipped_call_site(function, target, call.span);
        Ok(())
    }
}
