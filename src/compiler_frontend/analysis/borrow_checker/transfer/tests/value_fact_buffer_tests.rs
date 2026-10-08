//! Transfer-observation accumulation tests for the borrow checker's value fact buffer.
//!
//! WHAT: checks how completed use contexts combine transfer evidence for one HIR value row.
//! WHY: a non-consuming observation must conservatively downgrade a transfer candidate without
//!      changing the neutral merge used while collecting one use.

use super::facts::ValueFactBuffer;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    FunctionLayout, FunctionLayoutInputs, RootSet,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    OptionalTransferStatus, ValueAccessClassification,
};
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use rustc_hash::FxHashMap;

#[test]
fn completed_statement_uses_decline_transfer_in_either_order() {
    let value = HirValueId(7);
    let roots = RootSet::empty(1);

    for transfer_first in [false, true] {
        let mut facts = ValueFactBuffer::new(1);
        if transfer_first {
            facts.record_optional_transfer(value, OptionalTransferStatus::Transfer, &roots);
            facts.finish_use();
            facts.record(value, ValueAccessClassification::SharedRead, &roots);
        } else {
            facts.record(value, ValueAccessClassification::SharedRead, &roots);
            facts.finish_use();
            facts.record_optional_transfer(value, OptionalTransferStatus::Transfer, &roots);
        }
        facts.finish_use();

        assert_eq!(
            serialize(facts).optional_transfer,
            OptionalTransferStatus::Borrow,
            "an observed non-consuming use must downgrade the row in either statement order"
        );
    }
}

#[test]
fn transfer_survives_when_every_observed_use_proves_it_or_other_contexts_are_absent() {
    let value = HirValueId(7);
    let roots = RootSet::empty(1);

    let mut one_use = ValueFactBuffer::new(1);
    one_use.record_optional_transfer(value, OptionalTransferStatus::Transfer, &roots);
    one_use.finish_use();
    assert_eq!(
        serialize(one_use).optional_transfer,
        OptionalTransferStatus::Transfer,
        "a row absent from other contexts contributes no negative observation"
    );

    let mut all_transfer = ValueFactBuffer::new(1);
    for _ in 0..2 {
        all_transfer.record_optional_transfer(value, OptionalTransferStatus::Transfer, &roots);
        all_transfer.finish_use();
    }
    assert_eq!(
        serialize(all_transfer).optional_transfer,
        OptionalTransferStatus::Transfer,
        "the merge must preserve the optimization when every observed use proves transfer"
    );
}

fn serialize(
    facts: ValueFactBuffer,
) -> crate::compiler_frontend::analysis::borrow_checker::types::ValueBorrowFact {
    let layout = FunctionLayout::new(FunctionLayoutInputs {
        local_ids: vec![LocalId(0)],
        local_mutable: vec![true],
        local_first_write_order: vec![-1],
        local_last_use_order: vec![-1],
        statement_order_by_id: FxHashMap::default(),
        terminator_order_by_block: FxHashMap::default(),
        block_local_max_use_order: FxHashMap::default(),
        block_successors: FxHashMap::default(),
        visible_locals_by_block: FxHashMap::default(),
        may_use_from_block: FxHashMap::default(),
        must_use_from_block: FxHashMap::default(),
        may_assign_from_block: FxHashMap::default(),
    });
    facts.into_serialized(&layout).remove(0).1
}
