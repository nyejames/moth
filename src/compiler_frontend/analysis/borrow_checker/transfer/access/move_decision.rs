//! Move-versus-borrow decision helpers for access transfer.
//!
//! WHAT: decides when mutable-capable access may receive optional transfer responsibility instead
//! of remaining a borrow.
//! WHY: last-use-aware transfer refinement belongs in one focused helper instead of being duplicated across transfer code.

use crate::compiler_frontend::analysis::borrow_checker::state::{
    FunctionLayout, FutureUseKind, RootSet,
};
use crate::compiler_frontend::hir::ids::BlockId;

// WHAT: Encodes whether an optional destruction-responsibility transfer can be recorded.
// WHY: Transfer paths use this single decision while mandatory borrow state remains unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MoveDecision {
    Borrow,
    Move,
}

// WHAT: Classifies optional transfer eligibility from future-use facts at one program point.
// WHY: An imprecise or path-dependent proof must conservatively remain a borrow.
pub(super) fn classify_move_decision(
    layout: &FunctionLayout,
    block_id: BlockId,
    roots: &RootSet,
    current_order: i32,
) -> MoveDecision {
    let mut saw_must = false;
    let mut saw_none = false;

    for root_index in roots.iter_ones() {
        match layout.future_use_kind(block_id, root_index, current_order) {
            FutureUseKind::Must => saw_must = true,
            FutureUseKind::None => saw_none = true,
            FutureUseKind::May => return MoveDecision::Borrow,
        }
    }

    if saw_must {
        MoveDecision::Borrow
    } else if saw_none {
        MoveDecision::Move
    } else {
        MoveDecision::Borrow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::analysis::borrow_checker::state::FunctionLayoutInputs;
    use crate::compiler_frontend::hir::ids::{HirNodeId, LocalId, RegionId};
    use rustc_hash::FxHashMap;

    #[test]
    fn path_dependent_future_use_falls_back_to_borrow() {
        let mut root = RootSet::empty(1);
        root.insert(0);

        let mut block_successors = FxHashMap::default();
        block_successors.insert(BlockId(0), vec![BlockId(1), BlockId(2)]);

        let mut may_use_from_block = FxHashMap::default();
        may_use_from_block.insert(BlockId(0), RootSet::empty(1));
        may_use_from_block.insert(BlockId(1), root.clone());
        may_use_from_block.insert(BlockId(2), RootSet::empty(1));

        let mut must_use_from_block = FxHashMap::default();
        must_use_from_block.insert(BlockId(0), RootSet::empty(1));
        must_use_from_block.insert(BlockId(1), RootSet::empty(1));
        must_use_from_block.insert(BlockId(2), RootSet::empty(1));

        let mut block_local_max_use_order = FxHashMap::default();
        block_local_max_use_order.insert(BlockId(0), vec![-1]);

        let layout = FunctionLayout::new(FunctionLayoutInputs {
            local_ids: vec![LocalId(0)],
            local_mutable: vec![true],
            local_regions: vec![RegionId(0)],
            local_first_write_order: vec![0],
            local_last_use_order: vec![-1],
            statement_order_by_id: FxHashMap::<HirNodeId, i32>::default(),
            terminator_order_by_block: FxHashMap::default(),
            block_local_max_use_order,
            block_successors,
            may_use_from_block,
            must_use_from_block,
        });

        assert_eq!(layout.future_use_kind(BlockId(0), 0, 0), FutureUseKind::May);
        assert_eq!(
            classify_move_decision(&layout, BlockId(0), &root, 0),
            MoveDecision::Borrow
        );
    }
}
