//! HIR control-flow traversal helpers.
//!
//! WHAT: small utilities for extracting successor block IDs from terminators and
//!       walking the HIR block graph.
//! WHY: reachability, validation, and backend passes all need terminator-target
//!      iteration; centralising it here avoids duplicated match logic.

use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use rustc_hash::FxHashSet;
use std::collections::VecDeque;

/// Extract the successor block IDs from a HIR terminator.
pub fn terminator_targets(terminator: &HirTerminator) -> Vec<BlockId> {
    let mut targets = Vec::new();
    for_each_terminator_target(terminator, |target| targets.push(target));
    targets
}

/// Visit each successor block ID from a HIR terminator without allocating a temporary vector.
pub fn for_each_terminator_target(terminator: &HirTerminator, mut visit: impl FnMut(BlockId)) {
    let _ = try_for_each_terminator_target(terminator, |target| {
        visit(target);
        Ok::<(), std::convert::Infallible>(())
    });
}

/// Rewrite each successor block ID in place.
pub fn for_each_terminator_target_mut(
    terminator: &mut HirTerminator,
    mut visit: impl FnMut(&mut BlockId),
) {
    match terminator {
        HirTerminator::Jump { target, .. }
        | HirTerminator::Break { target }
        | HirTerminator::Continue { target } => visit(target),
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => {
            visit(then_block);
            visit(else_block);
        }
        HirTerminator::Invoke {
            success, failure, ..
        } => {
            visit(success);
            visit(failure);
        }
        HirTerminator::Match { arms, .. } => {
            for arm in arms {
                visit(&mut arm.body);
            }
        }
        HirTerminator::Return(_)
        | HirTerminator::ReturnError(_)
        | HirTerminator::NeverCall { .. }
        | HirTerminator::RuntimeFailure { .. }
        | HirTerminator::Uninitialized
        | HirTerminator::AssertFailure { .. } => {}
    }
}

/// Fallible successor visitation for analysis passes that need error propagation while scanning.
pub fn try_for_each_terminator_target<E>(
    terminator: &HirTerminator,
    mut visit: impl FnMut(BlockId) -> Result<(), E>,
) -> Result<(), E> {
    match terminator {
        HirTerminator::Jump { target, .. } => visit(*target)?,

        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => {
            visit(*then_block)?;
            visit(*else_block)?;
        }

        HirTerminator::Invoke {
            success, failure, ..
        } => {
            visit(*success)?;
            visit(*failure)?;
        }

        HirTerminator::Match { arms, .. } => {
            for arm in arms {
                visit(arm.body)?;
            }
        }

        HirTerminator::Break { target } | HirTerminator::Continue { target } => visit(*target)?,

        HirTerminator::Return(_)
        | HirTerminator::ReturnError(_)
        | HirTerminator::NeverCall { .. }
        | HirTerminator::RuntimeFailure { .. }
        | HirTerminator::Uninitialized
        | HirTerminator::AssertFailure { .. } => {}
    }

    Ok(())
}

/// Breadth-first traversal of the blocks reachable from `entry`, in deterministic BFS order.
///
/// WHAT: visits each reachable block once and returns the visit order. The callback reports a
///       block's successors through the sink it receives, so no per-block successor list is built.
/// WHY: every reachability question shares one walk with one visited set and one queue. A failure
///      such as a missing block stops the walk through the callback's error type.
pub fn collect_reachable_blocks<E>(
    entry: BlockId,
    mut for_each_successor: impl FnMut(BlockId, &mut dyn FnMut(BlockId)) -> Result<(), E>,
) -> Result<Vec<BlockId>, E> {
    let mut visited = FxHashSet::default();
    let mut queue = VecDeque::from([entry]);
    let mut order = Vec::new();

    while let Some(block_id) = queue.pop_front() {
        if !visited.insert(block_id) {
            continue;
        }
        order.push(block_id);
        for_each_successor(block_id, &mut |successor| queue.push_back(successor))?;
    }

    Ok(order)
}

/// Breadth-first traversal of one function's structural block subgraph.
///
/// WHAT: every block reachable from `entry` through any terminator target, in deterministic BFS
///       order. A missing block stays in the order, but its successors are not followed.
/// WHY: post-lane analyses must scan pruned catch-handler subgraphs that the live CFG no longer
///      links, and they run after `compact_block_ids`, so one sparse walk over the module's dense
///      block table is the shared evidence order for fast use, trap use and call sites.
pub fn collect_structural_blocks(hir: &HirModule, entry: BlockId) -> Vec<BlockId> {
    let Ok(order) =
        collect_reachable_blocks::<std::convert::Infallible>(entry, |block_id, visit| {
            if let Some(block) = hir.block(block_id) {
                for_each_terminator_target(&block.terminator, visit);
            }
            Ok(())
        });
    order
}
