//! HIR CFG reachability adapter for JS function lowering.
//!
//! WHAT: owns which successor edges can execute — pruning the failure edge of a numeric Invoke
//!       whose shared classification yields a scalar — and collects block closures through the
//!       emitter's block lookup.
//! WHY: emission, cycle detection, structured-branch inspection and every helper-demand scan must
//!      agree which edges execute. A block still reached through another live edge remains a
//!      target of that edge, so it keeps its emitted code and helper demand. Local declarations
//!      are the exception: they reserve storage through the unpruned CFG, because a pruned block's
//!      locals can still be written on a live path.
//!
//! This module must not own symbol lookup, source text emission, or identifier
//! generation. Those responsibilities belong to their focused owners.

use crate::backends::js::JsEmitter;
use crate::backends::js::js_statement::NumericDelivery;
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::utils::{
    for_each_terminator_target, try_for_each_terminator_target,
};

impl<'hir> JsEmitter<'hir> {
    /// Whether this Invoke can take its failure edge.
    ///
    /// WHAT: false for a numeric Invoke whose shared classification yields a scalar, and for a
    ///       proven integer-narrowing cast Invoke. Every other operation keeps its failure edge.
    /// WHY: reachability, branch inspection and emission must agree on edge liveness, so this
    ///      predicate asks the same proof query lowering does rather than re-deriving the cast
    ///      predicate here.
    pub(crate) fn invoke_failure_edge_is_live(
        &self,
        id: HirNodeId,
        operation: &HirFallibleOperation,
    ) -> Result<bool, InfrastructureFailure> {
        match operation {
            HirFallibleOperation::NumericOp { op, .. } => Ok(!self
                .classify_numeric_operation(*op, id, NumericDelivery::Invoke)?
                .is_scalar()),
            HirFallibleOperation::CastOp { policy, .. } => {
                Ok(!self.integer_narrowing_is_proven(id, *policy))
            }
            _ => Ok(true),
        }
    }

    /// Visit each successor block whose edge can actually execute, stopping at the first error.
    ///
    /// WHAT: every HIR successor except the failure edge of a numeric Invoke whose shared
    ///       classification yields a scalar and of a proven integer-narrowing cast Invoke.
    ///       Allocation-free: the visitor runs directly over the terminator.
    /// WHY: reachability, cycle detection, structured-branch inspection and helper-demand scans
    ///      must not disagree about a dead edge.
    pub(crate) fn for_each_live_terminator_target(
        &self,
        terminator: &HirTerminator,
        mut visit: impl FnMut(BlockId) -> Result<(), InfrastructureFailure>,
    ) -> Result<(), InfrastructureFailure> {
        if let HirTerminator::Invoke {
            id,
            operation,
            success,
            ..
        } = terminator
            && !self.invoke_failure_edge_is_live(*id, operation)?
        {
            return visit(*success);
        }

        try_for_each_terminator_target(terminator, visit)
    }

    /// Collect every block reachable through the unpruned CFG.
    ///
    /// WHAT: the plain transitive closure over all HIR successors, including the edges that
    ///       [`Self::for_each_live_terminator_target`] prunes for emission.
    /// WHY: local declarations are storage reservations, not executed code. HIR validation only
    ///      requires a written local to belong to the same function, so a slot can be declared in
    ///      a block that only a pruned failure edge reaches. Declaring from this set keeps every
    ///      name live emission can write reserved at the cost of a few unused `let` lines.
    pub(crate) fn collect_declaration_blocks(
        &self,
        entry_block: BlockId,
    ) -> Result<Vec<BlockId>, InfrastructureFailure> {
        self.collect_sorted_blocks(entry_block, |block_id, visit| {
            let block = self.block_by_id(block_id)?;
            for_each_terminator_target(&block.terminator, visit);
            Ok(())
        })
    }

    /// Collect every block reachable through an edge that can execute.
    ///
    /// WHAT: the transitive closure over [`Self::for_each_live_terminator_target`], which is the
    ///       exact block set function emission, cycle detection and helper-demand scans visit.
    /// WHY: a pruned failure block must not emit code or demand helpers, while a handler that
    ///      another live edge reaches stays in the set.
    pub(crate) fn collect_reachable_blocks(
        &self,
        entry_block: BlockId,
    ) -> Result<Vec<BlockId>, InfrastructureFailure> {
        self.collect_sorted_blocks(entry_block, |block_id, visit| {
            let block = self.block_by_id(block_id)?;
            self.for_each_live_terminator_target(&block.terminator, |target| {
                visit(target);
                Ok(())
            })
        })
    }

    /// Run the shared breadth-first closure and return ascending block order.
    ///
    /// WHY: declaration and emission order must not depend on the successor visit order, so both
    ///      callers share one stable sort.
    fn collect_sorted_blocks(
        &self,
        entry_block: BlockId,
        for_each_successor: impl FnMut(
            BlockId,
            &mut dyn FnMut(BlockId),
        ) -> Result<(), InfrastructureFailure>,
    ) -> Result<Vec<BlockId>, InfrastructureFailure> {
        let mut order = crate::compiler_frontend::hir::utils::collect_reachable_blocks(
            entry_block,
            for_each_successor,
        )?;
        order.sort_by_key(|block_id| block_id.0);
        Ok(order)
    }
}
