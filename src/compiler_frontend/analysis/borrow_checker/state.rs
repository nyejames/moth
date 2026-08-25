//! Borrow-checker state, layout, and bitset-backed dataflow primitives.
//!
//! This module owns the dense local indexing and abstract state representation used by the
//! forward transfer engine.

use crate::compiler_frontend::analysis::borrow_checker::types::{
    BorrowStateSnapshot, LocalBorrowSnapshot, LocalMode,
};
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, LocalId, RegionId};
use rustc_hash::FxHashMap;

// WHAT: Stable intra-function position key used by move/borrow decisions.
// WHY: Source line numbers are not precise enough when several accesses share a line.
pub(super) type OrderKey = i32;
pub(super) const UNKNOWN_ORDER_KEY: OrderKey = -1;

#[derive(Debug, Clone)]
pub(super) struct FunctionLayout {
    // WHAT: Dense local metadata keyed by stable local index.
    // WHY: Transfer rules and joins rely on O(1) lookups while iterating CFG edges.
    pub local_ids: Vec<LocalId>,
    pub local_index_by_id: FxHashMap<LocalId, usize>,
    pub local_mutable: Vec<bool>,
    pub local_regions: Vec<RegionId>,
    pub local_first_write_order: Vec<OrderKey>,
    pub local_last_use_order: Vec<OrderKey>,
    // WHAT: Per-node evaluation order used during transfer.
    // WHY: Transfer runs per statement/terminator and needs deterministic ordering.
    pub statement_order_by_id: FxHashMap<HirNodeId, OrderKey>,
    pub terminator_order_by_block: FxHashMap<BlockId, OrderKey>,
    // WHAT: Max local use order observed in each block.
    // WHY: Enables "future use in this block" checks without rescanning statements.
    pub block_local_max_use_order: FxHashMap<BlockId, Vec<OrderKey>>,
    pub block_successors: FxHashMap<BlockId, Vec<BlockId>>,
    pub may_use_from_block: FxHashMap<BlockId, RootSet>,
    pub must_use_from_block: FxHashMap<BlockId, RootSet>,
}

pub(super) struct FunctionLayoutInputs {
    // WHAT: Raw function facts collected during layout construction.
    // WHY: Keeping this separate from FunctionLayout lets callers build then validate atomically.
    pub local_ids: Vec<LocalId>,
    pub local_mutable: Vec<bool>,
    pub local_regions: Vec<RegionId>,
    pub local_first_write_order: Vec<OrderKey>,
    pub local_last_use_order: Vec<OrderKey>,
    pub statement_order_by_id: FxHashMap<HirNodeId, OrderKey>,
    pub terminator_order_by_block: FxHashMap<BlockId, OrderKey>,
    pub block_local_max_use_order: FxHashMap<BlockId, Vec<OrderKey>>,
    pub block_successors: FxHashMap<BlockId, Vec<BlockId>>,
    pub may_use_from_block: FxHashMap<BlockId, RootSet>,
    pub must_use_from_block: FxHashMap<BlockId, RootSet>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FutureUseKind {
    // WHAT: No reachable future read from this program point.
    // WHY: Call/assignment transfer may receive optional destruction responsibility when all roots
    //      are unused.
    None,
    // WHAT: Some paths use the root, others do not.
    // WHY: Mixed outcomes conservatively fall back to borrowing for optional transfer.
    May,
    // WHAT: Every reachable path uses the root again.
    // WHY: Operations must treat the root as borrowed to preserve later uses.
    Must,
}

impl FunctionLayout {
    pub(super) fn new(inputs: FunctionLayoutInputs) -> Self {
        let mut local_index_by_id =
            FxHashMap::with_capacity_and_hasher(inputs.local_ids.len(), Default::default());

        for (index, local_id) in inputs.local_ids.iter().enumerate() {
            local_index_by_id.insert(*local_id, index);
        }

        Self {
            local_ids: inputs.local_ids,
            local_index_by_id,
            local_mutable: inputs.local_mutable,
            local_regions: inputs.local_regions,
            local_first_write_order: inputs.local_first_write_order,
            local_last_use_order: inputs.local_last_use_order,
            statement_order_by_id: inputs.statement_order_by_id,
            terminator_order_by_block: inputs.terminator_order_by_block,
            block_local_max_use_order: inputs.block_local_max_use_order,
            block_successors: inputs.block_successors,
            may_use_from_block: inputs.may_use_from_block,
            must_use_from_block: inputs.must_use_from_block,
        }
    }

    pub(super) fn local_count(&self) -> usize {
        self.local_ids.len()
    }

    pub(super) fn index_of(&self, local_id: LocalId) -> Option<usize> {
        self.local_index_by_id.get(&local_id).copied()
    }

    pub(super) fn statement_order_or_unknown(&self, statement_id: HirNodeId) -> OrderKey {
        self.statement_order_by_id
            .get(&statement_id)
            .copied()
            .unwrap_or(UNKNOWN_ORDER_KEY)
    }

    pub(super) fn terminator_order_or_unknown(&self, block_id: BlockId) -> OrderKey {
        self.terminator_order_by_block
            .get(&block_id)
            .copied()
            .unwrap_or(UNKNOWN_ORDER_KEY)
    }

    pub(super) fn local_is_expired(&self, local_index: usize, current_order: OrderKey) -> bool {
        let last_use = self.local_last_use_order[local_index];
        last_use >= 0 && last_use < current_order
    }

    pub(super) fn future_use_kind(
        &self,
        block_id: BlockId,
        local_index: usize,
        current_order: OrderKey,
    ) -> FutureUseKind {
        if self.local_has_future_use_in_block(block_id, local_index, current_order) {
            return FutureUseKind::Must;
        }

        let Some(successors) = self.block_successors.get(&block_id) else {
            return FutureUseKind::None;
        };
        if successors.is_empty() {
            return FutureUseKind::None;
        }

        let mut may = false;
        let mut must = true;

        for successor in successors {
            let successor_may = self
                .may_use_from_block
                .get(successor)
                .map(|roots| roots.contains(local_index))
                .unwrap_or(false);
            let successor_must = self
                .must_use_from_block
                .get(successor)
                .map(|roots| roots.contains(local_index))
                .unwrap_or(false);

            may |= successor_may;
            must &= successor_must;
        }

        if !may {
            FutureUseKind::None
        } else if must {
            FutureUseKind::Must
        } else {
            FutureUseKind::May
        }
    }

    fn local_has_future_use_in_block(
        &self,
        block_id: BlockId,
        local_index: usize,
        current_order: OrderKey,
    ) -> bool {
        self.block_local_max_use_order
            .get(&block_id)
            .and_then(|max_use_order| max_use_order.get(local_index))
            .map(|order| *order > current_order)
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BorrowState {
    // One lattice state per function-local index.
    locals: Vec<LocalState>,
    // Cached count of locals whose effective roots include each root index.
    // This keeps mutable conflict checks O(1) per root.
    root_ref_counts: Vec<u32>,
}

impl BorrowState {
    pub(super) fn new_uninitialized(local_count: usize) -> Self {
        let locals = (0..local_count)
            .map(|_| LocalState::uninit(local_count))
            .collect::<Vec<_>>();

        Self {
            locals,
            root_ref_counts: vec![0; local_count],
        }
    }

    pub(super) fn initialize_parameter(&mut self, local_index: usize) {
        let local_count = self.locals.len();
        self.update_local_state(local_index, LocalState::slot(local_count));
    }

    pub(super) fn local_state(&self, local_index: usize) -> &LocalState {
        &self.locals[local_index]
    }

    pub(super) fn has_any_alias_conflict(&self) -> bool {
        self.root_ref_counts.iter().any(|count| *count > 1)
    }

    pub(super) fn effective_roots(&self, local_index: usize) -> RootSet {
        self.effective_roots_from_state(local_index, &self.locals[local_index])
    }

    pub(super) fn update_local_state(&mut self, local_index: usize, new_state: LocalState) {
        let old_roots = self.effective_roots(local_index);
        for root_index in old_roots.iter_ones() {
            if self.root_ref_counts[root_index] > 0 {
                self.root_ref_counts[root_index] -= 1;
            }
        }

        self.locals[local_index] = new_state;

        let new_roots = self.effective_roots(local_index);
        for root_index in new_roots.iter_ones() {
            self.root_ref_counts[root_index] += 1;
        }
    }

    pub(super) fn join(&self, other: &Self) -> Self {
        let local_count = self.locals.len();
        let mut joined_locals = Vec::with_capacity(local_count);

        for index in 0..local_count {
            let left = &self.locals[index];
            let right = &other.locals[index];

            let mut value_roots = left.value_roots.clone();
            value_roots.union_with(&right.value_roots);
            let mut direct_alias_roots = left.direct_alias_roots.clone();
            direct_alias_roots.union_with(&right.direct_alias_roots);

            joined_locals.push(LocalState {
                mode: left.mode.union(right.mode),
                value_roots,
                direct_alias_roots,
            });
        }

        let mut joined = Self {
            locals: joined_locals,
            root_ref_counts: vec![0; local_count],
        };
        joined.recompute_root_ref_counts();
        joined
    }

    pub(super) fn kill_invisible(&mut self, visible_mask: &RootSet) -> bool {
        let local_count = self.locals.len();
        let mut changed = false;

        for local_index in 0..local_count {
            if !visible_mask.contains(local_index) {
                let replacement = LocalState::uninit(local_count);
                if self.locals[local_index] != replacement {
                    self.locals[local_index] = replacement;
                    changed = true;
                }
                continue;
            }

            let mut next = self.locals[local_index].clone();
            if !next.value_roots.is_empty() {
                next.value_roots.intersect_with(visible_mask);
                next.direct_alias_roots.intersect_with(visible_mask);
                if next.value_roots.is_empty() {
                    next = if next.mode.contains(LocalMode::SLOT) {
                        LocalState::slot(local_count)
                    } else {
                        LocalState::uninit(local_count)
                    };
                }
            }

            if next != self.locals[local_index] {
                self.locals[local_index] = next;
                changed = true;
            }
        }

        if changed {
            self.recompute_root_ref_counts();
        }

        changed
    }

    pub(super) fn to_snapshot(&self, local_ids: &[LocalId]) -> BorrowStateSnapshot {
        let mut locals = Vec::with_capacity(self.locals.len());

        for (index, local_state) in self.locals.iter().enumerate() {
            let alias_roots = local_state
                .value_roots
                .iter_ones()
                .map(|root_index| local_ids[root_index])
                .collect::<Vec<_>>();

            locals.push(LocalBorrowSnapshot {
                local: local_ids[index],
                mode: local_state.mode,
                alias_roots,
            });
        }

        BorrowStateSnapshot { locals }
    }

    fn effective_roots_from_state(&self, local_index: usize, state: &LocalState) -> RootSet {
        let local_count = self.locals.len();
        let mut roots = RootSet::empty(local_count);

        let has_slot_binding = state.mode.contains(LocalMode::SLOT);
        let has_alias_binding = state.mode.contains(LocalMode::ALIAS);

        // A definite slot stores the current value in its own binding cell. If that value
        // aliases an older allocation, the older roots replace the slot's own allocation root.
        // A SLOT | ALIAS join remains conservative because either binding representation may
        // have reached the join.
        if has_slot_binding && (has_alias_binding || state.value_roots.is_empty()) {
            roots.insert(local_index);
        }

        if has_alias_binding || (has_slot_binding && !state.value_roots.is_empty()) {
            roots.union_with(&state.value_roots);
        }

        roots
    }

    fn recompute_root_ref_counts(&mut self) {
        self.root_ref_counts.fill(0);

        for local_index in 0..self.locals.len() {
            let roots = self.effective_roots(local_index);
            for root_index in roots.iter_ones() {
                self.root_ref_counts[root_index] += 1;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LocalState {
    // Binding storage mode and value provenance are deliberately separate. A call result has a
    // SLOT binding even when its current value aliases roots owned by an argument.
    pub mode: LocalMode,
    pub value_roots: RootSet,
    pub direct_alias_roots: RootSet,
}

impl LocalState {
    pub(super) fn uninit(local_count: usize) -> Self {
        Self {
            mode: LocalMode::UNINIT,
            value_roots: RootSet::empty(local_count),
            direct_alias_roots: RootSet::empty(local_count),
        }
    }

    pub(super) fn slot(local_count: usize) -> Self {
        Self {
            mode: LocalMode::SLOT,
            value_roots: RootSet::empty(local_count),
            direct_alias_roots: RootSet::empty(local_count),
        }
    }

    pub(super) fn slot_with_value_roots(value_roots: RootSet, direct_alias_roots: RootSet) -> Self {
        Self {
            mode: LocalMode::SLOT,
            value_roots,
            direct_alias_roots,
        }
    }

    pub(super) fn alias_with_direct(value_roots: RootSet, direct_alias_roots: RootSet) -> Self {
        Self {
            mode: LocalMode::ALIAS,
            value_roots,
            direct_alias_roots,
        }
    }

    pub(super) fn has_value_aliases(&self) -> bool {
        !self.value_roots.is_empty()
    }

    pub(super) fn is_alias_only(&self) -> bool {
        self.mode.contains(LocalMode::ALIAS) && !self.mode.contains(LocalMode::SLOT)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RootSet {
    words: Vec<u64>,
    bit_len: usize,
}

impl RootSet {
    pub(super) fn empty(bit_len: usize) -> Self {
        let word_len = bit_len.div_ceil(64);
        Self {
            words: vec![0; word_len],
            bit_len,
        }
    }

    pub(super) fn full(bit_len: usize) -> Self {
        let word_len = bit_len.div_ceil(64);
        let mut words = vec![u64::MAX; word_len];
        if !bit_len.is_multiple_of(64) {
            let remainder = bit_len % 64;
            let mask = (1u64 << remainder) - 1;
            if let Some(last) = words.last_mut() {
                *last = mask;
            }
        }
        Self { words, bit_len }
    }

    pub(super) fn insert(&mut self, bit_index: usize) {
        if bit_index >= self.bit_len {
            return;
        }

        let word_index = bit_index / 64;
        let bit_offset = bit_index % 64;
        self.words[word_index] |= 1u64 << bit_offset;
    }

    pub(super) fn contains(&self, bit_index: usize) -> bool {
        if bit_index >= self.bit_len {
            return false;
        }

        let word_index = bit_index / 64;
        let bit_offset = bit_index % 64;
        (self.words[word_index] & (1u64 << bit_offset)) != 0
    }

    pub(super) fn union_with(&mut self, other: &Self) {
        for (left, right) in self.words.iter_mut().zip(other.words.iter()) {
            *left |= *right;
        }
    }

    pub(super) fn intersect_with(&mut self, other: &Self) {
        for (left, right) in self.words.iter_mut().zip(other.words.iter()) {
            *left &= *right;
        }
    }

    pub(super) fn subtract_with(&mut self, other: &Self) {
        for (left, right) in self.words.iter_mut().zip(other.words.iter()) {
            *left &= !*right;
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    pub(super) fn iter_ones(&self) -> RootSetIter<'_> {
        RootSetIter {
            set: self,
            word_index: 0,
            current_word: if self.words.is_empty() {
                0
            } else {
                self.words[0]
            },
        }
    }
}

pub(super) struct RootSetIter<'a> {
    set: &'a RootSet,
    word_index: usize,
    current_word: u64,
}

impl<'a> Iterator for RootSetIter<'a> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.word_index >= self.set.words.len() {
                return None;
            }

            if self.current_word != 0 {
                let trailing = self.current_word.trailing_zeros() as usize;
                let bit_index = self.word_index * 64 + trailing;
                self.current_word &= self.current_word - 1;

                if bit_index < self.set.bit_len {
                    return Some(bit_index);
                }

                continue;
            }

            self.word_index += 1;
            if self.word_index < self.set.words.len() {
                self.current_word = self.set.words[self.word_index];
            }
        }
    }
}
