//! HIR control-flow traversal helpers.
//!
//! WHAT: small utilities for extracting successor block IDs from terminators and
//!       walking the HIR block graph.
//! WHY: reachability, validation, and backend passes all need terminator-target
//!      iteration; centralising it here avoids duplicated match logic.

use crate::compiler_frontend::hir::ids::BlockId;
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
pub fn for_each_terminator_target_mut(terminator: &mut HirTerminator, mut visit: impl FnMut(&mut BlockId)) {
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
        HirTerminator::FallibleBranch {
            success_block,
            error_block,
            ..
        } => {
            visit(success_block);
            visit(error_block);
        }
        HirTerminator::Match { arms, .. } => {
            for arm in arms {
                visit(&mut arm.body);
            }
        }
        HirTerminator::Return(_)
        | HirTerminator::ReturnSuccess(_)
        | HirTerminator::ReturnError(_)
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

        HirTerminator::FallibleBranch {
            success_block,
            error_block,
            ..
        } => {
            visit(*success_block)?;
            visit(*error_block)?;
        }

        HirTerminator::Match { arms, .. } => {
            for arm in arms {
                visit(arm.body)?;
            }
        }

        HirTerminator::Break { target } | HirTerminator::Continue { target } => visit(*target)?,

        HirTerminator::Return(_)
        | HirTerminator::ReturnSuccess(_)
        | HirTerminator::ReturnError(_)
        | HirTerminator::RuntimeFailure { .. }
        | HirTerminator::Uninitialized
        | HirTerminator::AssertFailure { .. } => {}
    }

    Ok(())
}

/// Breadth-first traversal of reachable blocks starting from `entry`.
///
/// WHAT: collects every block ID reachable via terminator successors.
/// WHY: backends and analyses need a canonical reachability walk without reconstructing it
///      inline; this keeps successor logic in one place.
///
/// Callers supply a closure that resolves a block ID into its successor list. The closure can
/// fail (for example if the block is missing), so the error type is generic.
pub fn collect_reachable_blocks<E>(
    entry: BlockId,
    mut get_successors: impl FnMut(BlockId) -> Result<Vec<BlockId>, E>,
) -> Result<Vec<BlockId>, E> {
    let mut visited = FxHashSet::default();
    let mut queue = VecDeque::new();
    let mut order = Vec::new();

    queue.push_back(entry);

    while let Some(block_id) = queue.pop_front() {
        if !visited.insert(block_id) {
            continue;
        }
        order.push(block_id);
        for successor in get_successors(block_id)? {
            queue.push_back(successor);
        }
    }

    Ok(order)
}
