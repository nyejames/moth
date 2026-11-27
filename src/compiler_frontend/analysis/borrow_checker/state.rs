//! Borrow-checker state, layout, and bitset-backed dataflow primitives.
//!
//! This module owns the dense local indexing and abstract state representation used by the
//! forward transfer engine.

use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
#[cfg(any(test, feature = "show_borrow_checker"))]
use crate::compiler_frontend::analysis::borrow_checker::types::{
    BorrowStateSnapshot, LocalBorrowSnapshot,
};
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, LocalId};
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::rc::Rc;

mod allocation_layer;

use allocation_layer::AllocationLayer;
pub(super) use allocation_layer::{
    AllocId, AllocationRetirement, FieldKey, PlaceResolution, ResolvedPlace, StoredValue,
};

/// An occurrence of a local binding, whether it owns a slot or links to another place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct WrapperId {
    pub local: usize,
    pub age: WrapperAge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum WrapperAge {
    Current,
    Prior,
}

impl WrapperId {
    pub(super) fn current(local: usize) -> Self {
        Self {
            local,
            age: WrapperAge::Current,
        }
    }

    fn retired(self, retires: impl FnOnce(usize) -> bool) -> Self {
        if self.age == WrapperAge::Current && retires(self.local) {
            Self {
                age: WrapperAge::Prior,
                ..self
            }
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CellId {
    Slot(WrapperId),
    Projected(AllocId, FieldKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Coverage {
    Complete,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SmallSet<T> {
    Empty,
    One(T),
    Many(Vec<T>),
}

impl<T: Copy + Ord> SmallSet<T> {
    fn from_iter(items: impl IntoIterator<Item = T>) -> Self {
        let mut items = items.into_iter();
        let Some(first) = items.next() else {
            return Self::Empty;
        };
        let Some(second) = items.by_ref().find(|&item| item != first) else {
            return Self::One(first);
        };
        let mut known = vec![first, second];
        known.extend(items);
        known.sort_unstable();
        known.dedup();
        Self::Many(known)
    }

    pub(super) fn insert(&mut self, item: T) {
        match self {
            Self::Empty => *self = Self::One(item),
            Self::One(old) if item != *old => {
                *self = if item < *old {
                    Self::Many(vec![item, *old])
                } else {
                    Self::Many(vec![*old, item])
                };
            }
            Self::One(_) => {}
            Self::Many(items) => {
                if let Err(index) = items.binary_search(&item) {
                    items.insert(index, item);
                }
            }
        }
    }

    fn remap(&mut self, mut remap: impl FnMut(T) -> T) {
        match self {
            Self::Empty => {}
            Self::One(item) => *item = remap(*item),
            Self::Many(items) => {
                let mut changed = false;
                for item in items.iter_mut() {
                    let next = remap(*item);
                    changed |= next != *item;
                    *item = next;
                }
                if changed {
                    items.sort_unstable();
                    items.dedup();
                    if let [only] = items.as_slice() {
                        *self = Self::One(*only);
                    }
                }
            }
        }
    }

    pub(super) fn as_slice(&self) -> &[T] {
        match self {
            Self::Empty => &[],
            Self::One(item) => std::slice::from_ref(item),
            Self::Many(items) => items,
        }
    }
}

/// An operand capture, with address evidence independent of its value provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AddressSet {
    cells: SmallSet<CellId>,
    pub coverage: Coverage,
    definite: Option<CellId>,
    all_candidates_known: bool,
}

impl AddressSet {
    pub(super) fn from_cells(cells: impl IntoIterator<Item = CellId>, coverage: Coverage) -> Self {
        Self {
            cells: SmallSet::from_iter(cells),
            coverage,
            definite: None,
            all_candidates_known: coverage == Coverage::Complete,
        }
    }

    pub(super) fn cells(&self) -> &[CellId] {
        self.cells.as_slice()
    }

    pub(super) fn definite(&self) -> Option<CellId> {
        self.definite
    }

    /// A missing binding alternative need not hide a possible storage location.
    pub(super) fn may_coverage(&self) -> Coverage {
        if self.all_candidates_known {
            Coverage::Complete
        } else {
            Coverage::Partial
        }
    }

    pub(super) fn with_singleton_proof(mut self) -> Self {
        if self.coverage == Coverage::Complete
            && let [
                cell @ (CellId::Slot(WrapperId {
                    age: WrapperAge::Current,
                    ..
                })
                | CellId::Projected(_, FieldKey::Field(_) | FieldKey::Payload { .. })),
            ] = self.cells()
        {
            self.definite = Some(*cell);
        }
        self
    }

    pub(super) fn remap_allocations(
        &mut self,
        state: &BorrowState,
        retirement: AllocationRetirement,
    ) {
        self.cells.remap(|cell| match cell {
            CellId::Slot(wrapper) => CellId::Slot(retirement.remap_wrapper(wrapper)),
            CellId::Projected(mut holder, key) => {
                retirement.remap_id(state, &mut holder);
                CellId::Projected(holder, key)
            }
        });
        // A definite destination was already a singleton; a prior slot is never exact.
        if self.definite.is_some() {
            self.definite = match self.cells() {
                [
                    CellId::Slot(WrapperId {
                        age: WrapperAge::Prior,
                        ..
                    }),
                ] => None,
                [cell] => Some(*cell),
                _ => unreachable!("remapping cannot add a destination to a singleton"),
            };
        }
    }
}

/// A slot or a captured alias supplied by a definition or an entry fixture.
pub(super) enum BindingSpec {
    Slot,
    Link {
        source: WrapperId,
        writable: bool,
    },
    Projected {
        addresses: AddressSet,
        origin: WrapperId,
        owner: Option<WrapperId>,
        writable: bool,
        isolated: bool,
        route_roots: RootSet,
    },
    #[cfg(test)]
    Alias {
        addresses: AddressSet,
        route_roots: RootSet,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BindingTarget {
    OwnSlot,
    Link(WrapperId),
    Projected(AllocId, FieldKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Origin {
    wrapper: WrapperId,
    owner: Option<WrapperId>,
    isolated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct BindingAlt {
    target: BindingTarget,
    origin: Option<Origin>,
    writable: bool,
}

const OWN_SLOT: BindingAlt = BindingAlt {
    target: BindingTarget::OwnSlot,
    writable: true,
    origin: None,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct BindingRow {
    wrapper: WrapperId,
    alts: SmallSet<BindingAlt>,
    coverage: Coverage,
    /// Named routes for attribution and effect provenance, not storage identity.
    route_roots: RootSet,
    /// Slot provenance when the local's may-set cannot describe the owned cell on its own.
    slot_roots: Option<RootSet>,
}

impl BindingRow {
    fn union_alts(&mut self, incoming: &[BindingAlt]) {
        if incoming
            .iter()
            .all(|alt| self.alts.as_slice().contains(alt))
        {
            return;
        }
        let mut alts = SmallSet::from_iter(self.alts.as_slice().iter().chain(incoming).copied());
        if let SmallSet::Many(items) = &mut alts {
            items.dedup_by(|later, earlier| {
                if later.target != earlier.target || later.origin != earlier.origin {
                    return false;
                }
                earlier.writable &= later.writable;
                true
            });
            if let [only] = items.as_slice() {
                alts = SmallSet::One(*only);
            }
        }
        self.alts = alts;
    }

    fn has_slot(&self) -> bool {
        self.alts.as_slice().contains(&OWN_SLOT)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Init {
    No,
    Yes,
    Maybe,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct CapturedValue {
    pub values: Vec<StoredValue>,
    pub views: Vec<AllocId>,
}

impl CapturedValue {
    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        for value in &mut self.values {
            retirement.remap_value(state, value);
        }
        for view in &mut self.views {
            retirement.remap_id(state, view);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Replacement {
    May,
    Must,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TypedStore {
    Write(TypedWrite),
    EraseFamily {
        receivers: Vec<AllocId>,
        coverage: Coverage,
        keys: EntryFamilies,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TypedWrite {
    addresses: AddressSet,
    value: CapturedValue,
    replace: Replacement,
    distinct_occurrence: bool,
    observer: Option<(usize, bool)>,
    slot_roots: Option<RootSet>,
    batch_receiver: bool,
    explicit_slot_payload: bool,
}

impl TypedStore {
    pub(super) fn write_projected(
        holders: impl IntoIterator<Item = AllocId>,
        key: FieldKey,
        coverage: Coverage,
        value: CapturedValue,
        distinct_occurrence: bool,
    ) -> Self {
        Self::Write(TypedWrite {
            addresses: AddressSet::from_cells(
                holders
                    .into_iter()
                    .map(|holder| CellId::Projected(holder, key)),
                coverage,
            ),
            value,
            replace: Replacement::May,
            distinct_occurrence,
            observer: None,
            slot_roots: None,
            batch_receiver: true,
            explicit_slot_payload: false,
        })
    }

    pub(super) fn write_cells(addresses: AddressSet, value: CapturedValue) -> Self {
        Self::Write(TypedWrite {
            addresses,
            value,
            replace: Replacement::May,
            distinct_occurrence: false,
            observer: None,
            slot_roots: None,
            batch_receiver: false,
            explicit_slot_payload: false,
        })
    }

    pub(super) fn write_through(
        addresses: AddressSet,
        value: CapturedValue,
        replace: Replacement,
        actor: usize,
    ) -> Self {
        Self::Write(TypedWrite {
            addresses,
            value,
            replace,
            distinct_occurrence: true,
            observer: (replace == Replacement::Must).then_some((actor, false)),
            slot_roots: None,
            batch_receiver: false,
            explicit_slot_payload: false,
        })
    }

    pub(super) fn receive_result(
        addresses: AddressSet,
        value: CapturedValue,
        actor: usize,
        readonly: bool,
    ) -> Self {
        Self::Write(TypedWrite {
            addresses,
            value,
            replace: Replacement::Must,
            distinct_occurrence: true,
            observer: Some((actor, readonly)),
            slot_roots: None,
            batch_receiver: false,
            explicit_slot_payload: false,
        })
    }

    pub(super) fn replace_cells(addresses: AddressSet, value: CapturedValue) -> Self {
        let Self::Write(mut write) = Self::write_cells(addresses, value) else {
            unreachable!()
        };
        write.replace = Replacement::Must;
        write.distinct_occurrence = true;
        Self::Write(write)
    }

    pub(super) fn with_slot_roots(mut self, roots: RootSet) -> Self {
        if let Self::Write(write) = &mut self {
            write.slot_roots = Some(roots);
        }
        self
    }

    pub(super) fn with_explicit_slot_payload(mut self) -> Self {
        if let Self::Write(write) = &mut self {
            write.explicit_slot_payload = true;
        }
        self
    }

    pub(super) fn with_actor_observation(mut self, actor: usize) -> Self {
        if let Self::Write(write) = &mut self {
            write.observer = Some((actor, false));
        }
        self
    }

    pub(super) fn captures_slot(&self, local: usize) -> bool {
        matches!(self, Self::Write(write)
            if write.addresses.cells().contains(&CellId::Slot(WrapperId::current(local))))
    }

    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        match self {
            Self::Write(TypedWrite {
                addresses, value, ..
            }) => {
                addresses.remap_allocations(state, retirement);
                if let Some(CellId::Projected(holder, _)) = addresses.definite()
                    && !state.alloc_singular(holder)
                {
                    addresses.definite = None;
                }
                value.remap(state, retirement);
            }
            Self::EraseFamily { receivers, .. } => {
                for receiver in receivers {
                    retirement.remap_id(state, receiver);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EntryFamilies {
    Elements,
    MapEntries,
}

/// Keep captures borrowed unless retirement changes at least one identity.
pub(super) fn remap_invoke_capture<T: Copy + PartialEq>(
    capture: &[T],
    remap: impl Fn(T) -> T,
) -> Cow<'_, [T]> {
    for (index, &original) in capture.iter().enumerate() {
        let rebased = remap(original);
        if rebased != original {
            let mut result = Vec::with_capacity(capture.len());
            result.extend_from_slice(&capture[..index]);
            result.push(rebased);
            result.extend(capture[index + 1..].iter().copied().map(remap));
            return Cow::Owned(result);
        }
    }
    Cow::Borrowed(capture)
}

/// Read-only access to binding role, route evidence and projected addresses.
#[derive(Clone, Copy)]
pub(super) struct BindingView<'a> {
    state: &'a BorrowState,
    local: usize,
}

impl<'a> BindingView<'a> {
    fn local_state(self) -> &'a LocalState {
        &self.state.locals[self.local]
    }

    pub(super) fn mode(self) -> LocalMode {
        let init = self.local_state().init;
        if init == Init::No {
            return LocalMode::UNINIT;
        }
        let mut mode = match self.state.binding_row(WrapperId::current(self.local)) {
            None => LocalMode::SLOT,
            Some(row) if !row.has_slot() => LocalMode::ALIAS,
            Some(row) if row.coverage == Coverage::Partial || row.alts.as_slice().len() > 1 => {
                LocalMode::SLOT.union(LocalMode::ALIAS)
            }
            Some(_) => LocalMode::SLOT,
        };
        if init == Init::Maybe {
            mode = mode.union(LocalMode::UNINIT);
        }
        mode
    }

    pub(super) fn is_alias_only(self) -> bool {
        let mode = self.mode();
        mode.contains(LocalMode::ALIAS) && !mode.contains(LocalMode::SLOT)
    }

    pub(super) fn value_roots(self) -> &'a RootSet {
        &self.local_state().value_roots
    }

    pub(super) fn derivation(self) -> &'a RootSet {
        &self.local_state().derived_from_roots
    }

    pub(super) fn direct_slots(self) -> RootSet {
        let mut roots = RootSet::empty(self.state.locals.len());
        if self.mode().contains(LocalMode::ALIAS) {
            for &cell in self.state.resolve_cells(self.local).cells() {
                if let CellId::Slot(wrapper) = cell
                    && wrapper.age == WrapperAge::Current
                    && wrapper.local != self.local
                {
                    roots.insert(wrapper.local);
                }
            }
        }
        roots
    }

    pub(super) fn route_roots(self) -> RootSet {
        self.state.binding_route_roots(self.local)
    }

    pub(super) fn attribution_roots(self) -> RootSet {
        let mut roots = RootSet::empty(self.state.locals.len());
        if self.mode().contains(LocalMode::SLOT) {
            roots.insert(self.local);
        }
        roots.union_with(&self.direct_slots());
        roots.union_with(&self.route_roots());
        roots
    }
}

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
    /// Whether a mutable access to each local's type can change storage through an alias.
    ///
    /// WHY: a definite slot storing a produced value keeps its own binding cell. For value types
    /// the write replaces that cell, while heap-allocated types may expose the older allocation.
    pub local_has_in_place_mutation_surface: Vec<bool>,
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
    /// The entry block if no reachable edge can feed it after parameter initialization.
    pub acyclic_entry_block: Option<BlockId>,
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
    pub local_has_in_place_mutation_surface: Vec<bool>,
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
        let local_count = inputs.local_ids.len();

        // WHAT: every local-keyed column, per-block order array and reachability mask is indexed
        // by the same dense local positions.
        // WHY: the layout is columnar, so a column that disagrees with `local_ids` either panics
        // when a short one is indexed or silently keeps facts no local owns when it is long. A
        // `RootSet` mask of the wrong width is the quiet case: the set combinators zip words and
        // would compare only the shared prefix in release builds.
        debug_assert_eq!(
            inputs.local_mutable.len(),
            local_count,
            "local_mutable must have one entry per local"
        );
        debug_assert_eq!(
            inputs.local_has_in_place_mutation_surface.len(),
            local_count,
            "local_has_in_place_mutation_surface must have one entry per local"
        );
        debug_assert_eq!(
            inputs.local_first_write_order.len(),
            local_count,
            "local_first_write_order must have one entry per local"
        );
        debug_assert_eq!(
            inputs.local_last_use_order.len(),
            local_count,
            "local_last_use_order must have one entry per local"
        );
        debug_assert!(
            inputs
                .block_local_max_use_order
                .values()
                .all(|max_use_order| max_use_order.len() == local_count),
            "block_local_max_use_order must have one entry per local for every block"
        );
        for (column, masks) in [
            ("visible_locals_by_block", &inputs.visible_locals_by_block),
            ("may_use_from_block", &inputs.may_use_from_block),
            ("must_use_from_block", &inputs.must_use_from_block),
            ("may_assign_from_block", &inputs.may_assign_from_block),
        ] {
            debug_assert!(
                masks.values().all(|mask| mask.bit_len == local_count),
                "{column} masks must span every local"
            );
        }

        let mut local_index_by_id =
            FxHashMap::with_capacity_and_hasher(local_count, Default::default());

        for (index, local_id) in inputs.local_ids.iter().enumerate() {
            local_index_by_id.insert(*local_id, index);
        }

        // WHY: a duplicate local ID would silently last-win in the position map while the
        // positional columns keep both rows, so every later lookup would read the wrong row.
        debug_assert_eq!(
            local_index_by_id.len(),
            local_count,
            "local_ids must not repeat a local"
        );

        Self {
            local_ids: inputs.local_ids,
            local_index_by_id,
            local_mutable: inputs.local_mutable,
            local_has_in_place_mutation_surface: inputs.local_has_in_place_mutation_surface,
            local_first_write_order: inputs.local_first_write_order,
            local_last_use_order: inputs.local_last_use_order,
            statement_order_by_id: inputs.statement_order_by_id,
            terminator_order_by_block: inputs.terminator_order_by_block,
            block_local_max_use_order: inputs.block_local_max_use_order,
            block_successors: inputs.block_successors,
            acyclic_entry_block: None,
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
    // Allocation identities for values stored by reference inside other values.
    layer: AllocationLayer,
}

impl BorrowState {
    pub(super) fn new_uninitialized(local_count: usize) -> Self {
        let locals = (0..local_count)
            .map(|_| LocalState::uninit(local_count))
            .collect::<Vec<_>>();

        Self {
            locals,
            layer: AllocationLayer::new(),
        }
    }

    pub(super) fn initialize_parameter(&mut self, local_index: usize) {
        self.set_binding(
            local_index,
            BindingSpec::Slot,
            RootSet::empty(self.locals.len()),
            RootSet::empty(self.locals.len()),
        );
    }

    pub(super) fn binding(&self, local: usize) -> BindingView<'_> {
        BindingView { state: self, local }
    }

    pub(super) fn binding_mode(&self, local: usize) -> LocalMode {
        self.binding(local).mode()
    }

    pub(super) fn set_binding(
        &mut self,
        local: usize,
        spec: BindingSpec,
        value_roots: RootSet,
        derived_from_roots: RootSet,
    ) {
        self.update_local_state(
            local,
            LocalState {
                derived_from_roots,
                ..LocalState::slot_from_value_roots(value_roots)
            },
        );
        self.install_binding_spec(local, spec);
    }

    /// The value-provenance roots a local's current binding keeps alive.
    ///
    /// Allocation provenance is separate from the cells that access designates.
    pub(super) fn effective_roots(&self, local_index: usize) -> RootSet {
        let (includes_self, includes_value_roots) = self.effective_root_sources(local_index);
        let mut roots = RootSet::empty(self.locals.len());
        if includes_self {
            roots.insert(local_index);
        }
        if includes_value_roots {
            roots.union_with(&self.locals[local_index].value_roots);
        }
        roots
    }

    /// Alias derivation ledger a value read from `source_index` carries into a new binding.
    ///
    /// WHAT: the source binding's transitive derivation chain, including the source itself when
    ///       it is a pure alias.
    /// WHY: jump arguments and call results transport the same value into a fresh slot. Keeping
    ///      the ledger on that slot lets reads through it stay exempt from the exclusive alias
    ///      they derive from instead of looking like foreign accesses. The ledger is a must-set,
    ///      so a path that did not derive still loses the exemption at a join.
    pub(super) fn derived_roots_from_source(&self, source_index: usize) -> RootSet {
        let source_state = &self.locals[source_index];
        let mut roots = source_state.derived_from_roots.clone();
        if self.binding(source_index).is_alias_only()
            && source_state.init == Init::Yes
            && self.resolve_cells(source_index).coverage == Coverage::Complete
        {
            roots.insert(source_index);
        }
        roots
    }

    /// Which sources of `effective_roots` apply to a local: its own allocation root, and its
    /// value roots.
    fn effective_root_sources(&self, local_index: usize) -> (bool, bool) {
        let state = &self.locals[local_index];
        if state.init == Init::No {
            return (false, false);
        }
        let has_value_roots = !state.value_roots.is_empty();
        let includes_self = match self.binding_row(WrapperId::current(local_index)) {
            None => !has_value_roots,
            Some(row) => row.has_slot() && row.slot_roots.as_ref().is_some_and(RootSet::is_empty),
        };
        (includes_self, has_value_roots)
    }

    fn implicit_slot_provenance(&self, local: usize) -> bool {
        match self.binding_row(WrapperId::current(local)) {
            None => {
                self.locals[local].init != Init::No && self.locals[local].value_roots.is_empty()
            }
            Some(row) => row.slot_roots.as_ref().is_some_and(RootSet::is_empty),
        }
    }

    pub(super) fn update_local_state(&mut self, local_index: usize, new_state: LocalState) {
        self.remove_current_row(local_index);
        self.locals[local_index] = new_state;
    }

    fn join_local_facts(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for index in 0..self.locals.len() {
            let implicit = !other.locals[index].value_roots.is_empty()
                && self.implicit_slot_provenance(index)
                || !self.locals[index].value_roots.is_empty()
                    && other.implicit_slot_provenance(index);
            let left = &mut self.locals[index];
            let right = &other.locals[index];
            let init = if left.init == right.init {
                left.init
            } else {
                Init::Maybe
            };
            changed |= init != left.init;
            left.init = init;
            changed |= left.value_roots.union_with(&right.value_roots);
            if implicit && !left.value_roots.contains(index) {
                left.value_roots.insert(index);
                changed = true;
            }
            // Derivation is a must fact: every incoming path must have derived from the alias.
            changed |= left
                .derived_from_roots
                .intersect_with(&right.derived_from_roots);
        }
        changed
    }

    /// Hide names outside the lexical mask, without invalidating retained storage.
    pub(super) fn kill_invisible(&mut self, visible_mask: &RootSet) {
        // WHY: the sweep is O(locals) and runs on every CFG edge. A mask that covers every local
        // (a function whose blocks all sit in one lexical region) hides none and keeps every root,
        // so the pass would rewrite every local without changing one.
        if visible_mask.covers_all(self.locals.len()) {
            return;
        }
        self.narrow_layer(visible_mask);

        for local_index in 0..self.locals.len() {
            let local = &mut self.locals[local_index];
            if !visible_mask.contains(local_index) {
                local.reset();
            } else {
                local.value_roots.intersect_with(visible_mask);
                local.derived_from_roots.intersect_with(visible_mask);
            }
        }
    }

    #[cfg(any(test, feature = "show_borrow_checker"))]
    pub(super) fn to_snapshot(&self, local_ids: &[LocalId]) -> BorrowStateSnapshot {
        let mut locals = Vec::with_capacity(self.locals.len());

        for (index, local_state) in self.locals.iter().enumerate() {
            let alias_roots = local_state
                .value_roots
                .iter_ones()
                .map(|root_index| local_ids[root_index])
                .collect::<Vec<_>>();
            let write_targets = self
                .binding(index)
                .route_roots()
                .iter_ones()
                .map(|root_index| local_ids[root_index])
                .collect::<Vec<_>>();

            locals.push(LocalBorrowSnapshot {
                local: local_ids[index],
                mode: self.binding_mode(index),
                alias_roots,
                write_targets,
            });
        }

        BorrowStateSnapshot { locals }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LocalState {
    init: Init,
    // Allocation provenance, never a destination.
    pub value_roots: RootSet,
    // Must-set of alias bindings this binding was created through, with the full transitive
    // chain. Reads through a derived binding are not foreign accesses to a mutably active alias
    // in that chain. Joins intersect this set: a path that did not derive from the alias loses
    // the exemption.
    pub derived_from_roots: RootSet,
}

impl LocalState {
    fn root_free(init: Init, local_count: usize) -> Self {
        Self {
            init,
            value_roots: RootSet::empty(local_count),
            derived_from_roots: RootSet::empty(local_count),
        }
    }

    pub(super) fn uninit(local_count: usize) -> Self {
        Self::root_free(Init::No, local_count)
    }

    pub(super) fn slot(local_count: usize) -> Self {
        Self::root_free(Init::Yes, local_count)
    }

    pub(super) fn slot_from_value_roots(value_roots: RootSet) -> Self {
        let count = value_roots.bit_len();
        Self {
            value_roots,
            ..Self::slot(count)
        }
    }

    fn reset(&mut self) {
        self.init = Init::No;
        self.value_roots.clear();
        self.derived_from_roots.clear();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RootSet {
    // WHAT: Copy-on-write words; clones share the allocation until an update writes. `Some` holds
    // exactly `ceil(bit_len / 64)` words with at least one non-zero word, and `None` is the empty
    // set and owns no allocation. `is_empty` is `words.is_none()`, so an update that clears the
    // last bit must store `None`.
    // WHY: borrow states clone every local's root set at block and edge joins, and most of those
    // clones are never updated, so a shared buffer avoids copying words that no rule will change.
    // Most locals hold no roots, so an empty set carries no buffer and its clone, drop, emptiness
    // test and joins stay O(1) instead of walking zeroed words.
    words: Option<Rc<[u64]>>,
    bit_len: usize,
}

impl RootSet {
    pub(super) fn empty(bit_len: usize) -> Self {
        Self {
            words: None,
            bit_len,
        }
    }

    pub(super) fn full(bit_len: usize) -> Self {
        let word_len = bit_len.div_ceil(64);
        let tail = match bit_len % 64 {
            0 => u64::MAX,
            remainder => (1u64 << remainder) - 1,
        };
        Self {
            // WHY: a zero-width universe has no bits, and every set without bits is `None`.
            // An exact-length iterator collects straight into the shared buffer, so the words
            // are written once instead of being built in a `Vec` and copied.
            words: (word_len != 0).then(|| {
                (0..word_len)
                    .map(|index| {
                        if index + 1 == word_len {
                            tail
                        } else {
                            u64::MAX
                        }
                    })
                    .collect()
            }),
            bit_len,
        }
    }

    /// Whether every index below `local_count` is a member and the set spans that universe.
    ///
    /// WHY: the engine narrows every CFG edge against a block's visibility mask, and that sweep is
    /// O(locals). A mask covering every local hides none and removes no root bit, so callers can
    /// skip the sweep instead of rewriting every local for no change.
    pub(super) fn covers_all(&self, local_count: usize) -> bool {
        if self.bit_len != local_count {
            return false;
        }
        let word_len = local_count.div_ceil(64);
        let words = self.word_slice();
        // A set with no words is an empty set; it covers every index only when there are none.
        if words.len() != word_len {
            return false;
        }
        let Some((tail, leading)) = words.split_last() else {
            return word_len == 0;
        };
        let tail_bits = match local_count % 64 {
            0 => u64::MAX,
            remainder => (1u64 << remainder) - 1,
        };
        *tail == tail_bits && leading.iter().all(|word| *word == u64::MAX)
    }

    /// The words as a slice, with the empty set viewed as having none.
    fn word_slice(&self) -> &[u64] {
        self.words.as_deref().unwrap_or(&[])
    }

    /// Mutable words, allocated for an empty set and copied first when a clone still shares them.
    ///
    /// WHY: the combinators test whether an update changes any bit before calling this on shared
    /// words, so a no-op update never copies them.
    fn words_mut(&mut self) -> &mut [u64] {
        let word_len = self.bit_len.div_ceil(64);
        let words = self
            .words
            .get_or_insert_with(|| std::iter::repeat_n(0, word_len).collect());
        if Rc::strong_count(words) != 1 {
            *words = Rc::from(&words[..]);
        }
        // Invariant: this private allocation never has weak references, so `Rc::get_mut` fails
        // only while the strong count is above one, and the copy above leaves exactly one owner.
        Rc::get_mut(words).expect("root words are unshared before mutation")
    }

    /// Whether any clone still shares this set's words. Only shared words need the change tests
    /// that keep no-op updates from copying; unshared words are written in place either way.
    fn words_are_shared(&self) -> bool {
        self.words
            .as_ref()
            .is_some_and(|words| Rc::strong_count(words) != 1)
    }

    pub(super) fn insert(&mut self, bit_index: usize) {
        if bit_index >= self.bit_len {
            return;
        }

        // WHY: only a shared buffer needs the bit test to avoid a copy of unchanged words; an
        // unshared write is idempotent and costs a read anyway.
        if self.words_are_shared() && self.contains(bit_index) {
            return;
        }

        let word_index = bit_index / 64;
        let bit_offset = bit_index % 64;
        self.words_mut()[word_index] |= 1u64 << bit_offset;
    }

    pub(super) fn remove(&mut self, bit_index: usize) {
        if !self.contains(bit_index) {
            return;
        }
        let word_index = bit_index / 64;
        let bit_offset = bit_index % 64;
        self.words_mut()[word_index] &= !(1u64 << bit_offset);
        // The empty set owns no words.
        if self.word_slice().iter().all(|word| *word == 0) {
            self.words = None;
        }
    }

    pub(super) fn contains(&self, bit_index: usize) -> bool {
        if bit_index >= self.bit_len {
            return false;
        }

        let word_index = bit_index / 64;
        let bit_offset = bit_index % 64;
        self.word_slice()
            .get(word_index)
            .is_some_and(|word| (*word & (1u64 << bit_offset)) != 0)
    }

    /// Returns whether any bit was added.
    pub(super) fn union_with(&mut self, other: &Self) -> bool {
        let Some(other_words) = other.words.as_ref() else {
            return false;
        };

        // WHY: the first bit into an empty set shares the other set's buffer instead of copying
        // it, and copy-on-write copies it later only if either side is written.
        if self.words.is_none() {
            self.words = Some(Rc::clone(other_words));
            return true;
        }

        // WHY: a share test keeps a no-op union from copying shared words; unshared words are
        // written in place by the loop below, which reports the same change flag.
        if self.words_are_shared() {
            let adds_bits = self
                .word_slice()
                .iter()
                .zip(other_words.iter())
                .any(|(left, right)| right & !left != 0);
            if !adds_bits {
                return false;
            }
        }

        let mut changed = false;
        for (left, right) in self.words_mut().iter_mut().zip(other_words.iter()) {
            let next = *left | *right;
            changed |= next != *left;
            *left = next;
        }
        changed
    }

    /// Keep only roots `other` also holds, reporting whether any root was removed.
    pub(super) fn intersect_with(&mut self, other: &Self) -> bool {
        // WHY: an empty set has nothing to mask, and an empty mask leaves no bit, so neither case
        // reads or copies words.
        if self.words.is_none() {
            return false;
        }
        let Some(other_words) = other.words.as_ref() else {
            self.words = None;
            return true;
        };

        // WHY: an unshared buffer is masked in place; only shared words need the removal test
        // first, so that a mask that removes nothing never copies them.
        if self.words_are_shared() {
            let removes_bits = self
                .word_slice()
                .iter()
                .zip(other_words.iter())
                .any(|(left, right)| left & !right != 0);
            if !removes_bits {
                return false;
            }
        }

        // WHY: a mask can remove every bit, so the surviving bits are gathered while masking; an
        // empty result drops the buffer, keeping `None` as the only empty representation.
        let mut surviving = 0u64;
        let mut removed = false;
        for (left, right) in self.words_mut().iter_mut().zip(other_words.iter()) {
            let next = *left & *right;
            removed |= next != *left;
            *left = next;
            surviving |= next;
        }
        if surviving == 0 {
            self.words = None;
        }
        removed
    }

    fn has_members_outside(&self, other: &Self) -> bool {
        debug_assert_eq!(self.bit_len, other.bit_len);
        match (self.words.as_deref(), other.words.as_deref()) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(left), Some(right)) => left.iter().zip(right).any(|(&a, &b)| a & !b != 0),
        }
    }

    pub(super) fn clear(&mut self) {
        // WHY: dropping the buffer clears the set without copying or zeroing words that a clone
        // may still share.
        self.words = None;
    }

    pub(super) fn subtract_with(&mut self, other: &Self) {
        // WHY: an empty set has nothing to remove and an empty subtrahend removes nothing, so
        // neither case reads or copies words.
        let Some(other_words) = other.words.as_ref() else {
            return;
        };
        if self.words.is_none() {
            return;
        }

        // WHY: unshared words subtract in place; only shared words need the test for bits that
        // `other` would actually remove.
        if self.words_are_shared() {
            let removes_bits = self
                .word_slice()
                .iter()
                .zip(other_words.iter())
                .any(|(left, right)| left & right != 0);
            if !removes_bits {
                return;
            }
        }

        let mut surviving = 0u64;
        for (left, right) in self.words_mut().iter_mut().zip(other_words.iter()) {
            *left &= !*right;
            surviving |= *left;
        }
        if surviving == 0 {
            self.words = None;
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.words.is_none()
    }

    /// Width of the universe this set indexes.
    pub(super) fn bit_len(&self) -> usize {
        self.bit_len
    }

    pub(super) fn iter_ones(&self) -> RootSetIter<'_> {
        let words = self.word_slice();
        RootSetIter {
            words,
            bit_len: self.bit_len,
            word_index: 0,
            current_word: words.first().copied().unwrap_or(0),
        }
    }
}

pub(super) struct RootSetIter<'a> {
    words: &'a [u64],
    bit_len: usize,
    word_index: usize,
    current_word: u64,
}

impl<'a> Iterator for RootSetIter<'a> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.word_index >= self.words.len() {
                return None;
            }

            if self.current_word != 0 {
                let trailing = self.current_word.trailing_zeros() as usize;
                let bit_index = self.word_index * 64 + trailing;
                self.current_word &= self.current_word - 1;

                if bit_index < self.bit_len {
                    return Some(bit_index);
                }

                continue;
            }

            self.word_index += 1;
            if self.word_index < self.words.len() {
                self.current_word = self.words[self.word_index];
            }
        }
    }
}
