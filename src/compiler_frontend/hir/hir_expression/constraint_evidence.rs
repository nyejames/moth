//! Selected evidence from AST code that HIR lowering intentionally omits.
//!
//! WHAT: walks the retained AST of skipped catch handlers and statements after a terminator,
//!       recording every selected call site and `$fast_math` range guard in the owner's evidence.
//! WHY: constraint verification treats fast selection as a property of selected source, not of
//!      liveness. Lowering omits statically infallible handlers and dead statements after a
//!      terminator, so the ordinary HIR scan cannot see the calls and range guards they still
//!      select. Recording at the skip point keeps the lowering optimisations without losing the
//!      evidence.
//!
//! The expression walk owns calls and nested value blocks; the statement walk owns `-> !` calls
//! and range guards. Each nested value block is visited through its expression edge, so the two
//! traversals do not duplicate one another.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ExpressionStore, ResolvedCallableTarget,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::never_call::{NeverCall, NeverCallTarget};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
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
    ///      owns their call sites and fast range-guard witnesses.
    pub(in crate::compiler_frontend::hir) fn record_skipped_statement_evidence(
        &mut self,
        store: &ExpressionStore,
        nodes: &[AstNode],
    ) -> Result<(), HirConstructionFailure> {
        let generation = self.skipped_source_walk.begin_walk();
        let skipped_function = self.current_function_id_or_error()?;
        for node in nodes {
            node.visit_expression_ids(&mut |id| {
                self.skipped_source_walk.push_root(id);
                // Release folding may already have replaced a call below this root with a
                // constant, so the skipped walk can no longer record the call itself. Keeping the
                // root's authored span lets projection recognise an erased call that stood in
                // skipped source and withhold its liveness-conditioned trap/failure evidence.
                if let Ok(view) = store.view(id)
                    && let Some(span) = view.span()
                {
                    self.skipped_source_evidence
                        .entry(skipped_function)
                        .or_default()
                        .record_skipped_span(span);
                }
            });
        }
        self.record_skipped_statement_nodes(nodes)?;
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
                    self.record_skipped_value_block_nodes(block)?;
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
        nodes: &[AstNode],
    ) -> Result<(), HirConstructionFailure> {
        for node in nodes {
            match &node.kind {
                NodeKind::NeverCall(call) => self.record_skipped_never_call(call)?,
                NodeKind::RangeLoop { range, body, .. } => {
                    if range.fast_math {
                        let function = self.current_function_id_or_error()?;
                        self.record_skipped_fast_use(
                            function,
                            HirFastUseProvenance::RangeGuard,
                            node.span,
                        );
                    }
                    self.record_skipped_statement_nodes(body)?;
                }
                NodeKind::If(_, then_body, else_body, _) => {
                    self.record_skipped_statement_nodes(then_body)?;
                    if let Some(else_body) = else_body {
                        self.record_skipped_statement_nodes(else_body)?;
                    }
                }
                NodeKind::Match { arms, default, .. } => {
                    for arm in arms {
                        self.record_skipped_statement_nodes(&arm.body)?;
                    }
                    if let Some(default) = default {
                        self.record_skipped_statement_nodes(default)?;
                    }
                }
                NodeKind::LexicalScope { body }
                | NodeKind::Function(_, _, body)
                | NodeKind::EntryStart(_, body) => {
                    self.record_skipped_statement_nodes(body)?;
                }
                NodeKind::CollectionLoop { body, .. } | NodeKind::WhileLoop(_, body) => {
                    self.record_skipped_statement_nodes(body)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn record_skipped_value_block_nodes(
        &mut self,
        block: &ValueBlock,
    ) -> Result<(), HirConstructionFailure> {
        match block {
            ValueBlock::If(block) => {
                self.record_skipped_statement_nodes(&block.then_body)?;
                self.record_skipped_statement_nodes(&block.else_body)
            }
            ValueBlock::LexicalScope(block) => self.record_skipped_statement_nodes(&block.body),
            ValueBlock::Match(block) => {
                for arm in &block.arms {
                    self.record_skipped_statement_nodes(&arm.body)?;
                }
                if let Some(default) = &block.default {
                    self.record_skipped_statement_nodes(default)?;
                }
                Ok(())
            }
            ValueBlock::Catch(block) => match &block.handler {
                FallibleHandling::Handler { body, .. } => self.record_skipped_statement_nodes(body),
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
