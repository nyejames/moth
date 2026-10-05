//! Borrow-checker state, layout, and bitset-backed dataflow primitives.
//!
//! This module owns the dense local indexing and abstract state representation used by the
//! forward transfer engine.

use crate::compiler_frontend::analysis::borrow_checker::types::{
    BorrowStateSnapshot, LocalBorrowSnapshot, LocalMode,
};
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, LocalId};
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
    // Shared by assignment reachability and forward state kills at lexical region exits.
    pub visible_locals_by_block: FxHashMap<BlockId, RootSet>,
    pub may_use_from_block: FxHashMap<BlockId, RootSet>,
    pub must_use_from_block: FxHashMap<BlockId, RootSet>,
    // Direct assignment reachability is separate from definition-killing read liveness.
    pub may_assign_from_block: FxHashMap<BlockId, RootSet>,
}

pub(super) struct FunctionLayoutInputs {
    // WHAT: Raw function facts collected during layout construction.
    // WHY: Keeping this separate from FunctionLayout lets callers build then validate atomically.
    pub local_ids: Vec<LocalId>,
    pub local_mutable: Vec<bool>,
    pub local_first_write_order: Vec<OrderKey>,
    pub local_last_use_order: Vec<OrderKey>,
    pub statement_order_by_id: FxHashMap<HirNodeId, OrderKey>,
    pub terminator_order_by_block: FxHashMap<BlockId, OrderKey>,
    pub block_local_max_use_order: FxHashMap<BlockId, Vec<OrderKey>>,
    pub block_successors: FxHashMap<BlockId, Vec<BlockId>>,
    pub visible_locals_by_block: FxHashMap<BlockId, RootSet>,
    pub may_use_from_block: FxHashMap<BlockId, RootSet>,
    pub must_use_from_block: FxHashMap<BlockId, RootSet>,
    pub may_assign_from_block: FxHashMap<BlockId, RootSet>,
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
            local_first_write_order: inputs.local_first_write_order,
            local_last_use_order: inputs.local_last_use_order,
            statement_order_by_id: inputs.statement_order_by_id,
            terminator_order_by_block: inputs.terminator_order_by_block,
            block_local_max_use_order: inputs.block_local_max_use_order,
            block_successors: inputs.block_successors,
            visible_locals_by_block: inputs.visible_locals_by_block,
            may_use_from_block: inputs.may_use_from_block,
            must_use_from_block: inputs.must_use_from_block,
            may_assign_from_block: inputs.may_assign_from_block,
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

    /// Whether a reachable successor directly assigns this local on any path.
    ///
    /// In-block writes are already covered by the inclusive `future_use_kind` activity query.
    /// This fact does not classify write-through aliases or change read/transfer liveness.
    pub(super) fn local_has_future_assignment(
        &self,
        block_id: BlockId,
        local_index: usize,
    ) -> bool {
        let Some(successors) = self.block_successors.get(&block_id) else {
            return false;
        };

        successors.iter().any(|successor| {
            self.may_assign_from_block
                .get(successor)
                .map(|roots| roots.contains(local_index))
                .unwrap_or(false)
        })
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
}

impl BorrowState {
    pub(super) fn new_uninitialized(local_count: usize) -> Self {
        let locals = (0..local_count)
            .map(|_| LocalState::uninit(local_count))
            .collect::<Vec<_>>();

        Self { locals }
    }

    pub(super) fn initialize_parameter(&mut self, local_index: usize) {
        let local_count = self.locals.len();
        self.update_local_state(local_index, LocalState::slot(local_count));
    }

    pub(super) fn local_state(&self, local_index: usize) -> &LocalState {
        &self.locals[local_index]
    }

    /// The roots a local's current binding keeps alive.
    ///
    /// A definite slot stores the current value in its own binding cell. If that value aliases an
    /// older allocation, the older roots replace the slot's own allocation root. A SLOT | ALIAS
    /// join remains conservative because either binding representation may have reached the join.
    pub(super) fn effective_roots(&self, local_index: usize) -> RootSet {
        let state = &self.locals[local_index];
        let has_slot_binding = state.mode.contains(LocalMode::SLOT);
        let has_alias_binding = state.mode.contains(LocalMode::ALIAS);
        let has_value_roots = !state.value_roots.is_empty();

        let mut roots = RootSet::empty(self.locals.len());
        if has_slot_binding && (has_alias_binding || !has_value_roots) {
            roots.insert(local_index);
        }
        if has_alias_binding || (has_slot_binding && has_value_roots) {
            roots.union_with(&state.value_roots);
        }
        roots
    }

    pub(super) fn update_local_state(&mut self, local_index: usize, new_state: LocalState) {
        self.locals[local_index] = new_state;
    }

    /// Join `other` into this state, reporting whether this state grew.
    ///
    /// WHY: the dataflow engine joins at every CFG edge. Joining in place avoids allocating a
    /// whole O(locals²) state per edge only to compare it with the existing one.
    pub(super) fn join_in_place(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (left, right) in self.locals.iter_mut().zip(&other.locals) {
            let mode = left.mode.union(right.mode);
            changed |= mode != left.mode;
            left.mode = mode;
            changed |= left.value_roots.union_with(&right.value_roots);
            changed |= left
                .direct_alias_roots
                .union_with(&right.direct_alias_roots);
        }
        changed
    }

    /// Reset locals outside the lexical visibility mask and drop their roots from visible locals.
    ///
    /// WHY: runs on every block visit and CFG edge, so it rewrites each local in place rather
    /// than cloning and comparing every local's root sets.
    pub(super) fn kill_invisible(&mut self, visible_mask: &RootSet) {
        for (local_index, local) in self.locals.iter_mut().enumerate() {
            if !visible_mask.contains(local_index) {
                local.reset(LocalMode::UNINIT);
                continue;
            }

            // Locals without value roots keep their direct-alias roots untouched.
            if local.value_roots.is_empty() {
                continue;
            }
            local.value_roots.intersect_with(visible_mask);
            local.direct_alias_roots.intersect_with(visible_mask);
            if local.value_roots.is_empty() {
                let mode = if local.mode.contains(LocalMode::SLOT) {
                    LocalMode::SLOT
                } else {
                    LocalMode::UNINIT
                };
                local.reset(mode);
            }
        }
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

    /// Reset to a root-free binding of `mode` in place, keeping the root-set allocations.
    fn reset(&mut self, mode: LocalMode) {
        self.mode = mode;
        self.value_roots.clear();
        self.direct_alias_roots.clear();
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

    /// Returns whether any bit was added.
    pub(super) fn union_with(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (left, right) in self.words.iter_mut().zip(other.words.iter()) {
            let next = *left | *right;
            changed |= next != *left;
            *left = next;
        }
        changed
    }

    pub(super) fn intersect_with(&mut self, other: &Self) {
        for (left, right) in self.words.iter_mut().zip(other.words.iter()) {
            *left &= *right;
        }
    }

    pub(super) fn clear(&mut self) {
        self.words.fill(0);
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
