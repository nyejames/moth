//! Move-versus-borrow decision tests.
//!
//! WHAT: verifies optional transfer refinement falls back to borrow for path-dependent uses.
//! WHY: last-use-aware transfer refinement must remain conservative when future-use facts differ across paths.

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
