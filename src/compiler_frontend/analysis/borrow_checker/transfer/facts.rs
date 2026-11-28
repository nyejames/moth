//! Shared fact collection and per-statement access tracking.
//!
//! These helpers are intentionally lightweight because they run for every
//! statement/terminator transfer.

use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, AllocId, AllocationRetirement, BorrowState, CellId, Coverage, DependencySet,
    FieldKey, FunctionLayout, OrderKey, ResolvedPlace, RootSet, SmallSet, StoreReceipt, ValueAlt,
    ValueKind, WrapperAge, WrapperId, remap_dependencies,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    AccessKind, OptionalTransferStatus, ValueAccessClassification, ValueBorrowFact,
};
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::source::SourceSpan;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::hash_map::Entry;

/// The terminal operation. A projection's prefixes are always observed, not replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum PlaceUse {
    Read,
    Acquire,
    Replace,
    Mutate { deep: bool },
    ReadEntries,
    WriteEntries,
}

impl PlaceUse {
    pub(super) fn kind(self) -> AccessKind {
        match self {
            Self::Read | Self::ReadEntries => AccessKind::Shared,
            _ => AccessKind::Mutable,
        }
    }

    pub(super) fn touches_cell(self) -> bool {
        !matches!(self, Self::ReadEntries | Self::WriteEntries)
    }

    pub(super) fn for_transfer_probe(self) -> Self {
        match self.kind() {
            AccessKind::Shared => Self::Mutate { deep: true },
            AccessKind::Mutable => self,
        }
    }
}

/// Cell identities and graph alternatives stay separate, including when one is empty.
#[derive(Debug)]
pub(in super::super) struct AccessFootprint {
    pub address: AddressSet,
    pub traversal: SmallSet<CellId>,
    pub traversal_coverage: Coverage,
    pub content: Option<ResolvedPlace>,
    pub usage: PlaceUse,
}

impl AccessFootprint {
    pub(in super::super) fn cells(address: AddressSet, usage: PlaceUse) -> Self {
        Self {
            address,
            traversal: SmallSet::Empty,
            traversal_coverage: Coverage::Complete,
            content: None,
            usage,
        }
    }

    pub(super) fn implicit_slot(&self) -> Option<usize> {
        if self.usage.touches_cell()
            && self.content.is_none()
            && self.traversal.as_slice().is_empty()
            && let Some(CellId::Slot(slot)) = self.address.definite()
            && slot.age == WrapperAge::Current
        {
            return Some(slot.local);
        }
        None
    }
}

/// Identity is the stored occurrence, not the allocation it happens to hold.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in super::super) enum Carrier {
    Cell(CellId, CarrierValue, DependencySet),
    Route(WrapperId, CellId, DependencySet),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in super::super) enum CarrierValue {
    Scalar,
    Allocation(AllocId),
    Unknown,
    Absent,
}

impl Carrier {
    fn stored(cell: CellId, value: &ValueAlt) -> Option<Self> {
        Self::stored_matching(cell, value, |_| true)
    }

    fn stored_matching(
        cell: CellId,
        value: &ValueAlt,
        matches: impl FnOnce(&DependencySet) -> bool,
    ) -> Option<Self> {
        (!value.deps.is_empty() && matches(&value.deps))
            .then(|| Self::Cell(cell, CarrierValue::of(value), value.deps.clone()))
    }

    fn rekey(
        &mut self,
        wrapper: impl Fn(WrapperId) -> WrapperId,
        allocation: impl Fn(AllocId) -> AllocId,
    ) {
        let (cell, deps) = match self {
            Self::Cell(cell, value, deps) => {
                if let CarrierValue::Allocation(id) = value {
                    *id = allocation(*id);
                }
                (cell, deps)
            }
            Self::Route(owner, cell, deps) => {
                *owner = wrapper(*owner);
                (cell, deps)
            }
        };
        *cell = cell.remap(wrapper, &allocation);
        remap_dependencies(deps, allocation);
    }
}

impl CarrierValue {
    fn of(value: &ValueAlt) -> Self {
        match value.kind {
            ValueKind::Scalar => Self::Scalar,
            ValueKind::Allocation(value) => Self::Allocation(value.id),
            ValueKind::Unknown => Self::Unknown,
            ValueKind::Absent => Self::Absent,
        }
    }

    fn compatible(self, other: Self) -> bool {
        self == other || matches!(self, Self::Unknown) || matches!(other, Self::Unknown)
    }
}

pub(in super::super) type Demand = FxHashSet<Carrier>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in super::super) struct ObservationSegment {
    pub upward_reads: Demand,
    pub definite_kills: Demand,
}

impl ObservationSegment {
    pub(in super::super) fn read(&mut self, reads: impl IntoIterator<Item = Carrier>) {
        for carrier in reads {
            self.read_carrier(carrier);
        }
    }

    pub(in super::super) fn observe(
        &mut self,
        state: &BorrowState,
        access: &AccessFootprint,
        terminal: bool,
        actor: Option<usize>,
    ) {
        visit_observed_carriers(state, access, terminal, actor, |carrier| {
            self.read_carrier(carrier)
        });
    }

    fn read_carrier(&mut self, carrier: Carrier) {
        if !self.definite_kills.contains(&carrier) {
            self.upward_reads.insert(carrier);
        }
    }

    fn append(&mut self, next: Self) {
        if self.upward_reads.is_empty() && self.definite_kills.is_empty() {
            *self = next;
            return;
        }
        self.read(next.upward_reads);
        self.definite_kills.extend(next.definite_kills);
    }

    pub(in super::super) fn apply(&self, demand: &mut Demand) {
        demand.retain(|carrier| !self.definite_kills.contains(carrier));
        demand.extend(self.upward_reads.iter().cloned());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct ObservationCheckpoint {
    pub order: OrderKey,
    pub actor: Option<usize>,
    pub span: Option<SourceSpan>,
    pub candidates: Demand,
    pub pending: Demand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) enum ObservationBoundary {
    Checkpoint(ObservationCheckpoint),
    Rekey(Vec<(Carrier, Carrier)>),
    Publication(Demand),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in super::super) struct ValueObservations {
    pub tail: ObservationSegment,
    pub splits: Vec<(ObservationSegment, ObservationBoundary)>,
}

impl ValueObservations {
    pub(in super::super) fn split(&mut self, boundary: ObservationBoundary) {
        self.splits.push((std::mem::take(&mut self.tail), boundary));
    }

    pub(in super::super) fn append(&mut self, next: Self) {
        let Self { tail, mut splits } = next;
        if let Some((initial, _)) = splits.first_mut() {
            let mut preceding = std::mem::replace(&mut self.tail, tail);
            preceding.append(std::mem::take(initial));
            *initial = preceding;
            if self.splits.is_empty() {
                self.splits = splits;
            } else {
                self.splits.append(&mut splits);
            }
        } else {
            self.tail.append(tail);
        }
    }

    pub(in super::super) fn publish(&mut self, receipt: StoreReceipt) {
        fn carriers(
            cells: Vec<(CellId, Vec<ValueAlt>)>,
            routes: Vec<(WrapperId, CellId, DependencySet)>,
        ) -> impl Iterator<Item = Carrier> {
            cells
                .into_iter()
                .flat_map(|(cell, values)| {
                    values
                        .into_iter()
                        .map(move |value| Carrier::Cell(cell, CarrierValue::of(&value), value.deps))
                })
                .chain(
                    routes
                        .into_iter()
                        .map(|(owner, cell, deps)| Carrier::Route(owner, cell, deps)),
                )
        }

        self.tail
            .definite_kills
            .extend(carriers(receipt.definite_old, receipt.definite_old_routes));
        if !receipt.introduced.is_empty() || !receipt.introduced_routes.is_empty() {
            let introduced = carriers(receipt.introduced, receipt.introduced_routes).collect();
            self.split(ObservationBoundary::Publication(introduced));
        }
    }

    #[cfg(test)]
    pub(in super::super) fn canonicalise_test_carriers(
        &mut self,
        map: impl Fn(AllocId) -> AllocId,
    ) {
        let remap_carrier = |carrier: &mut Carrier| carrier.rekey(|slot| slot, &map);
        let remap_demand = |demand: &mut Demand| {
            *demand = std::mem::take(demand)
                .into_iter()
                .map(|mut carrier| {
                    remap_carrier(&mut carrier);
                    carrier
                })
                .collect();
        };
        let remap_segment = |segment: &mut ObservationSegment| {
            remap_demand(&mut segment.upward_reads);
            remap_demand(&mut segment.definite_kills);
        };
        for (segment, boundary) in &mut self.splits {
            remap_segment(segment);
            match boundary {
                ObservationBoundary::Checkpoint(checkpoint) => {
                    remap_demand(&mut checkpoint.candidates);
                    remap_demand(&mut checkpoint.pending);
                }
                ObservationBoundary::Rekey(rows) => {
                    for (before, after) in rows.iter_mut() {
                        remap_carrier(before);
                        remap_carrier(after);
                    }
                    rows.sort_unstable();
                }
                ObservationBoundary::Publication(acquired) => remap_demand(acquired),
            }
        }
        remap_segment(&mut self.tail);
    }

    pub(in super::super) fn is_empty(&self) -> bool {
        self.splits.is_empty()
            && self.tail.upward_reads.is_empty()
            && self.tail.definite_kills.is_empty()
    }

    pub(in super::super) fn has_obligations(&self) -> bool {
        self.splits
            .iter()
            .any(|(_, boundary)| matches!(boundary, ObservationBoundary::Checkpoint(_)))
    }

    pub(in super::super) fn prune_retired_wrappers(&mut self, state: &mut BorrowState) {
        if state.has_retired_wrappers() {
            let before = self.before_rekey(state);
            state.prune_retired_wrappers();
            self.after_rekey(before, state, |slot| slot, |id| id);
        }
    }

    pub(in super::super) fn before_rekey(&self, state: &BorrowState) -> Vec<Carrier> {
        if state.has_dependency_carriers() {
            matching_carriers(state, |_| true).collect()
        } else {
            Vec::new()
        }
    }

    pub(in super::super) fn after_retirement(
        &mut self,
        before: Vec<Carrier>,
        state: &BorrowState,
        retirement: AllocationRetirement,
    ) {
        self.after_rekey(
            before,
            state,
            |slot| retirement.remap_wrapper(slot),
            |mut id| {
                retirement.remap_id(state, &mut id);
                id
            },
        );
    }

    pub(in super::super) fn after_rekey(
        &mut self,
        before: Vec<Carrier>,
        state: &BorrowState,
        wrapper: impl Fn(WrapperId) -> WrapperId,
        allocation: impl Fn(AllocId) -> AllocId,
    ) {
        if before.is_empty() {
            return;
        }
        let rekey = |old: &Carrier| {
            let mut new = old.clone();
            new.rekey(&wrapper, &allocation);
            new.exists_in(state).then_some(new)
        };
        let Some(first) = before
            .iter()
            .map(rekey)
            .zip(&before)
            .position(|(new, old)| new.as_ref() != Some(old))
        else {
            return;
        };
        // Keep every surviving preimage, including unchanged carriers that coalesced with a
        // renamed one. A carrier introduced by the operation must not flow backwards.
        let mut pairs = Vec::with_capacity(before.len());
        for (index, old) in before.into_iter().enumerate() {
            if index < first {
                pairs.push((old.clone(), old));
            } else if let Some(new) = rekey(&old) {
                pairs.push((old, new));
            }
        }
        self.split(ObservationBoundary::Rekey(pairs));
    }

    pub(in super::super) fn backward<'a>(
        &'a self,
        demand: &mut Demand,
        mut check: impl FnMut(&'a ObservationCheckpoint, &Demand),
    ) {
        self.tail.apply(demand);
        let mut exit_demand = Demand::default();
        for (segment, boundary) in self.splits.iter().rev() {
            match boundary {
                ObservationBoundary::Checkpoint(checkpoint) => check(checkpoint, demand),
                ObservationBoundary::Rekey(preimages) => {
                    std::mem::swap(demand, &mut exit_demand);
                    demand.extend(
                        preimages
                            .iter()
                            .filter(|(_, exit)| exit_demand.contains(exit))
                            .map(|(entry, _)| entry.clone()),
                    );
                    exit_demand.clear();
                }
                ObservationBoundary::Publication(introduced) => {
                    demand.retain(|carrier| !introduced.contains(carrier));
                }
            }
            segment.apply(demand);
        }
    }
}

pub(super) fn matching_carriers<'a>(
    state: &'a BorrowState,
    matches: impl Fn(&DependencySet) -> bool + Copy + 'a,
) -> impl Iterator<Item = Carrier> + 'a {
    state
        .value_cells()
        .flat_map(move |(cell, values)| {
            values
                .iter()
                .filter_map(move |value| Carrier::stored_matching(cell, value, matches))
        })
        .chain(
            state
                .value_routes()
                .filter(move |(_, _, deps)| matches(deps))
                .map(|(owner, cell, deps)| Carrier::Route(owner, cell, deps.clone())),
        )
}

impl Carrier {
    fn exists_in(&self, state: &BorrowState) -> bool {
        match self {
            Self::Cell(cell, value, deps) => state
                .value_cell(*cell)
                .iter()
                .any(|alt| CarrierValue::of(alt) == *value && alt.deps == *deps),
            Self::Route(owner, cell, deps) => state
                .retained_value_routes(*owner)
                .iter()
                .any(|(candidate, evidence)| candidate == cell && evidence == deps),
        }
    }
}

fn matches_alternatives(value: &ValueAlt, alternatives: &[ValueAlt]) -> bool {
    let kind = CarrierValue::of(value);
    alternatives.iter().any(|alt| {
        kind.compatible(CarrierValue::of(alt))
            && value.deps.iter().all(|dep| alt.deps.contains(dep))
    })
}

pub(super) fn visit_observed_carriers(
    state: &BorrowState,
    access: &AccessFootprint,
    terminal: bool,
    actor: Option<usize>,
    mut visit: impl FnMut(Carrier),
) {
    if !state.has_dependency_carriers() {
        return;
    }
    let content = if terminal {
        access.content.as_ref()
    } else {
        None
    };
    let observed_alt = actor
        .filter(|_| terminal && access.traversal.as_slice().is_empty())
        .and_then(|local| state.observed_payload(local));
    let partial_terminal = terminal && access.address.may_coverage() == Coverage::Partial;
    let partial_prefix = access.traversal_coverage == Coverage::Partial;
    let deep = content.filter(|content| {
        matches!(
            access.usage,
            PlaceUse::Read | PlaceUse::Mutate { deep: true }
        ) && (!content.ids.is_empty() || content.coverage == Coverage::Partial)
    });
    let mut observe = |cell, values: &[ValueAlt], refine: bool| {
        for value in values {
            if let Some(carrier) = Carrier::stored_matching(cell, value, |_| {
                !refine
                    || observed_alt.map_or_else(
                        || {
                            content.is_none_or(|content| {
                                content.coverage == Coverage::Partial
                                    || matches_alternatives(value, &content.alternatives)
                            })
                        },
                        |alternatives| matches_alternatives(value, alternatives),
                    )
            }) {
                visit(carrier);
            }
        }
    };
    if partial_prefix || partial_terminal || deep.is_some() {
        for (cell, values) in state.value_cells() {
            let all = partial_prefix
                || partial_terminal
                || access.traversal.as_slice().contains(&cell)
                || matches!(cell, CellId::Projected(holder, _) if deep.is_some_and(|content| {
                    content.coverage == Coverage::Partial
                        || state.alloc_reaches_typed(&content.ids, holder, true, false)
                }));
            if all || terminal && access.address.cells().contains(&cell) {
                observe(cell, values, !all);
            }
        }
    } else {
        for &cell in access.traversal.as_slice() {
            observe(cell, state.value_cell(cell), false);
        }
        if terminal {
            for &cell in access.address.cells() {
                if !access.traversal.as_slice().contains(&cell) {
                    observe(cell, state.value_cell(cell), true);
                }
            }
        }
    }
    if let Some(actor) = actor {
        for owner in state.value_route_owners(actor) {
            for (cell, deps) in state.retained_value_routes(owner) {
                if !deps.is_empty()
                    && (access.traversal.as_slice().contains(cell)
                        || (terminal
                            || matches!(access.usage, PlaceUse::Acquire | PlaceUse::Replace))
                            && access.address.cells().contains(cell))
                {
                    visit(Carrier::Route(owner, *cell, deps.clone()));
                }
            }
        }
    }
}

pub(super) fn selected_members<'a>(
    state: &'a BorrowState,
    receiver: &'a ResolvedPlace,
) -> impl Iterator<Item = Carrier> + 'a {
    state
        .has_dependency_carriers()
        .then(|| state.value_cells())
        .into_iter()
        .flatten()
        .filter(move |(cell, _)| match *cell {
            CellId::Projected(holder, FieldKey::Elem) => {
                receiver.holder_coverage() == Coverage::Partial
                    || state.alloc_paths_may_share(&[holder], &[], &receiver.ids, &receiver.views)
            }
            _ => false,
        })
        .flat_map(|(cell, values)| {
            values
                .iter()
                .filter_map(move |value| Carrier::stored(cell, value))
        })
}

#[derive(Debug)]
pub(super) struct AccessReservation {
    pub footprint: AccessFootprint,
    pub actor: Option<usize>,
    pub span: Option<SourceSpan>,
}

#[derive(Debug)]
pub(super) struct StatementAccessTracker {
    first: Option<AccessReservation>,
    slots: FxHashMap<usize, AccessReservation>,
    places: Vec<AccessReservation>,
    pub(super) shared_roots: RootSet,
    pub(super) mutable_roots: RootSet,
    /// Roots whose storage this statement writes, separate from roots it accesses exclusively.
    pub(super) written_roots: RootSet,
}

impl StatementAccessTracker {
    pub(super) fn new(root_count: usize) -> Self {
        Self {
            first: None,
            slots: FxHashMap::default(),
            places: Vec::new(),
            shared_roots: RootSet::empty(root_count),
            mutable_roots: RootSet::empty(root_count),
            written_roots: RootSet::empty(root_count),
        }
    }

    /// Index only implicit slot accesses; all other pairs use the same typed predicate.
    pub(super) fn conflict(
        &self,
        footprint: &AccessFootprint,
        mut overlaps: impl FnMut(&AccessReservation) -> bool,
    ) -> Option<&AccessReservation> {
        if let Some(first) = self.first.as_ref().filter(|first| overlaps(first)) {
            return Some(first);
        }
        if let Some(slot) = footprint.implicit_slot() {
            if let Some(other) = self.slots.get(&slot).filter(|other| overlaps(other)) {
                return Some(other);
            }
        } else if let Some(other) = self.slots.values().find(|other| overlaps(other)) {
            return Some(other);
        }
        self.places.iter().find(|other| overlaps(other))
    }

    pub(super) fn reserve(&mut self, reservation: AccessReservation) {
        if self.first.is_none() {
            self.first = Some(reservation);
        } else if let Some(slot) = reservation.footprint.implicit_slot() {
            match self.slots.entry(slot) {
                Entry::Vacant(entry) => {
                    entry.insert(reservation);
                }
                Entry::Occupied(mut entry) => {
                    if reservation.footprint.usage.kind() == AccessKind::Mutable {
                        entry.insert(reservation);
                    }
                }
            }
        } else {
            self.places.push(reservation);
        }
    }

    pub(super) fn reservations(&self) -> impl Iterator<Item = &AccessReservation> {
        self.first
            .iter()
            .chain(self.slots.values())
            .chain(&self.places)
    }

    pub(super) fn clear_reservations(&mut self) {
        self.first = None;
        self.slots.clear();
        self.places.clear();
    }

    pub(super) fn report(&mut self, roots: &RootSet, access: AccessKind) {
        match access {
            AccessKind::Shared => self.shared_roots.union_with(roots),
            AccessKind::Mutable => self.mutable_roots.union_with(roots),
        };
    }

    /// Records roots whose storage the statement actually writes.
    ///
    /// An exclusive acquisition need not write the referent, so access facts alone would
    /// overstate a function's mutation effects.
    pub(super) fn record_write(&mut self, roots: &RootSet) {
        self.written_roots.union_with(roots);
    }
}

#[derive(Debug, Clone)]
pub(super) struct ValueFactBuffer {
    local_count: usize,
    facts: FxHashMap<HirValueId, BufferedValueFact>,
    current_use_values: Vec<HirValueId>,
}

#[derive(Debug, Clone)]
struct BufferedValueFact {
    classification: ValueAccessClassification,
    roots: RootSet,
    current_use_transfer: OptionalTransferStatus,
    completed_use_transfer: Option<OptionalTransferStatus>,
    observed_in_current_use: bool,
}

impl BufferedValueFact {
    fn new(local_count: usize) -> Self {
        Self {
            classification: ValueAccessClassification::None,
            roots: RootSet::empty(local_count),
            current_use_transfer: OptionalTransferStatus::NotAttempted,
            completed_use_transfer: None,
            observed_in_current_use: false,
        }
    }
}

impl ValueFactBuffer {
    pub(super) fn new(local_count: usize) -> Self {
        Self {
            local_count,
            facts: FxHashMap::default(),
            current_use_values: Vec::new(),
        }
    }

    pub(super) fn record(
        &mut self,
        value_id: HirValueId,
        classification: ValueAccessClassification,
        roots: &RootSet,
    ) {
        let entry = self.entry_for_current_use(value_id);
        entry.classification = entry.classification.merge(classification);
        entry.roots.union_with(roots);
    }

    pub(super) fn record_optional_transfer(
        &mut self,
        value_id: HirValueId,
        status: OptionalTransferStatus,
        roots: &RootSet,
    ) {
        let entry = self.entry_for_current_use(value_id);
        entry.roots.union_with(roots);
        entry.current_use_transfer = entry.current_use_transfer.merge(status);
    }

    pub(super) fn record_optional_transfer_status(
        &mut self,
        value_id: HirValueId,
        status: OptionalTransferStatus,
    ) {
        let entry = self.entry_for_current_use(value_id);
        entry.current_use_transfer = entry.current_use_transfer.merge(status);
    }

    /// Finish collecting one statement or terminator use context.
    ///
    /// A row touched only by ordinary access has an observed `NotAttempted` result. Merging that
    /// completed observation with a transfer candidate must decline the row-wide permission.
    pub(super) fn finish_use(&mut self) {
        for value_id in self.current_use_values.drain(..) {
            let entry = self
                .facts
                .get_mut(&value_id)
                .expect("a touched value row should have a buffered fact");
            entry.completed_use_transfer = Some(match entry.completed_use_transfer {
                Some(completed) => completed.merge_completed_use(entry.current_use_transfer),
                None => entry.current_use_transfer,
            });
            entry.current_use_transfer = OptionalTransferStatus::NotAttempted;
            entry.observed_in_current_use = false;
        }
    }

    pub(super) fn into_serialized(
        self,
        layout: &FunctionLayout,
    ) -> Vec<(HirValueId, ValueBorrowFact)> {
        debug_assert!(
            self.current_use_values.is_empty(),
            "all value-fact use contexts must be completed before serialization"
        );
        self.facts
            .into_iter()
            .map(|(value_id, entry)| {
                (
                    value_id,
                    ValueBorrowFact {
                        classification: entry.classification,
                        roots: roots_to_local_ids(layout, &entry.roots),
                        optional_transfer: entry
                            .completed_use_transfer
                            .expect("every buffered value row should have a completed use"),
                    },
                )
            })
            .collect::<Vec<_>>()
    }

    fn entry_for_current_use(&mut self, value_id: HirValueId) -> &mut BufferedValueFact {
        match self.facts.entry(value_id) {
            Entry::Occupied(mut occupied) => {
                if !occupied.get().observed_in_current_use {
                    occupied.get_mut().observed_in_current_use = true;
                    self.current_use_values.push(value_id);
                }
                occupied.into_mut()
            }
            Entry::Vacant(vacant) => {
                self.current_use_values.push(value_id);
                let mut fact = BufferedValueFact::new(self.local_count);
                fact.observed_in_current_use = true;
                vacant.insert(fact)
            }
        }
    }
}

pub(super) fn roots_to_local_ids(layout: &FunctionLayout, roots: &RootSet) -> Vec<LocalId> {
    roots
        .iter_ones()
        .map(|index| layout.local_ids[index])
        .collect::<Vec<_>>()
}
