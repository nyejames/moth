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

#[cfg(test)]
#[path = "tests/move_decision_tests.rs"]
mod move_decision_tests;

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
