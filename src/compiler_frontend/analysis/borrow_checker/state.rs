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
use std::collections::BTreeSet;
use std::rc::Rc;

mod allocation_layer;

use allocation_layer::AllocationLayer;
pub(super) use allocation_layer::cyclic_blocks;
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

    pub(super) fn retired(self, retires: impl FnOnce(usize) -> bool) -> Self {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum CellId {
    Slot(WrapperId),
    Projected(AllocId, FieldKey),
}

impl CellId {
    pub(super) fn remap(
        self,
        wrapper: impl FnOnce(WrapperId) -> WrapperId,
        allocation: impl FnOnce(AllocId) -> AllocId,
    ) -> Self {
        match self {
            Self::Slot(slot) => Self::Slot(wrapper(slot)),
            Self::Projected(holder, key) => Self::Projected(allocation(holder), key),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Coverage {
    Complete,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum SmallSet<T> {
    Empty,
    One(T),
    Many(Vec<T>),
}

impl<T: Copy + Ord> FromIterator<T> for SmallSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
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
}

impl<T: Copy + Ord> SmallSet<T> {
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
    routes: CapturedRoutes,
}

impl AddressSet {
    pub(super) fn from_cells(cells: impl IntoIterator<Item = CellId>, coverage: Coverage) -> Self {
        Self {
            cells: SmallSet::from_iter(cells),
            coverage,
            definite: None,
            all_candidates_known: coverage == Coverage::Complete,
            routes: Vec::new(),
        }
    }

    pub(super) fn cells(&self) -> &[CellId] {
        self.cells.as_slice()
    }

    pub(super) fn definite(&self) -> Option<CellId> {
        self.definite
    }

    pub(super) fn retained_routes(&self) -> &[(CellId, DependencySet)] {
        &self.routes
    }

    pub(super) fn captures_current(&self, local: usize, names: impl Fn(AllocId) -> bool) -> bool {
        let names_cell = |cell| match cell {
            CellId::Slot(slot) => slot == WrapperId::current(local),
            CellId::Projected(holder, _) => names(holder),
        };
        self.cells().iter().copied().any(&names_cell)
            || self
                .routes
                .iter()
                .any(|(cell, deps)| names_cell(*cell) || dependencies_name(deps, &names))
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
        let remap_id = |mut id| {
            retirement.remap_id(state, &mut id);
            id
        };
        let remap_cell = |cell: CellId| cell.remap(|slot| retirement.remap_wrapper(slot), remap_id);
        self.cells.remap(remap_cell);
        remap_routes(&mut self.routes, remap_cell, remap_id);
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
pub(super) enum BindingTarget {
    OwnSlot,
    Link(WrapperId),
    Projected(AllocId, FieldKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Origin {
    pub wrapper: WrapperId,
    pub owner: Option<WrapperId>,
    pub isolated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct BindingAlt {
    pub target: BindingTarget,
    pub origin: Option<Origin>,
    pub writable: bool,
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
    routes: CapturedRoutes,
}

impl BindingRow {
    fn union_row_alts(&mut self, incoming: &Self) {
        if !self.routes.is_empty() || !incoming.routes.is_empty() {
            let implicit_routes = |alts: &SmallSet<BindingAlt>| {
                alts.as_slice()
                    .iter()
                    .filter_map(|alt| match alt.target {
                        BindingTarget::Projected(holder, key) => {
                            Some((CellId::Projected(holder, key), Vec::new()))
                        }
                        _ => None,
                    })
                    .collect::<CapturedRoutes>()
            };
            if self.routes.is_empty() {
                self.routes = implicit_routes(&self.alts);
            }
            if incoming.routes.is_empty() {
                self.routes.extend(implicit_routes(&incoming.alts));
            } else {
                self.routes.extend_from_slice(&incoming.routes);
            }
            self.routes.sort_unstable();
            self.routes.dedup();
        }
        self.union_alts(incoming.alts.as_slice());
    }

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

type CapturedRoutes = Vec<(CellId, DependencySet)>;

fn remap_routes(
    routes: &mut CapturedRoutes,
    mut remap_cell: impl FnMut(CellId) -> CellId,
    mut remap_alloc: impl FnMut(AllocId) -> AllocId,
) {
    for (cell, deps) in &mut *routes {
        *cell = remap_cell(*cell);
        remap_dependencies(deps, &mut remap_alloc);
    }
    routes.sort_unstable();
    routes.dedup();
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ReceiverDependency {
    pub entries: SmallSet<AllocId>,
    pub coverage: Coverage,
}

pub(super) type DependencySet = Vec<ReceiverDependency>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ValueKind {
    Scalar,
    Allocation(StoredValue),
    Unknown,
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ValueAlt {
    pub kind: ValueKind,
    pub deps: DependencySet,
}

impl ValueAlt {
    pub(super) fn plain(kind: ValueKind) -> Self {
        Self {
            kind,
            deps: Vec::new(),
        }
    }

    fn remap(&mut self, remap: &mut impl FnMut(AllocId) -> AllocId) {
        if let ValueKind::Allocation(value) = &mut self.kind {
            value.id = remap(value.id);
        }
        remap_dependencies(&mut self.deps, remap);
    }
}

pub(super) fn remap_dependencies(
    deps: &mut DependencySet,
    mut remap: impl FnMut(AllocId) -> AllocId,
) {
    for dep in &mut *deps {
        dep.entries.remap(&mut remap);
    }
    deps.sort_unstable();
    deps.dedup();
}

fn dependencies_name(deps: &DependencySet, names: &impl Fn(AllocId) -> bool) -> bool {
    deps.iter()
        .any(|dep| dep.entries.as_slice().iter().copied().any(names))
}

/// No side payload is needed while the compact allocation facts describe every alternative.
pub(super) type ValueAlternatives = Vec<ValueAlt>;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct CapturedValue {
    pub values: Vec<StoredValue>,
    pub views: Vec<AllocId>,
    pub alternatives: ValueAlternatives,
}

impl CapturedValue {
    pub(super) fn new(values: Vec<StoredValue>, views: Vec<AllocId>) -> Self {
        Self {
            values,
            views,
            alternatives: Vec::new(),
        }
    }

    pub(super) fn names_allocation(&self, names: &impl Fn(AllocId) -> bool) -> bool {
        self.values.iter().any(|value| names(value.id))
            || self.views.iter().copied().any(names)
            || self.alternatives.iter().any(|alt| {
                matches!(alt.kind, ValueKind::Allocation(value) if names(value.id))
                    || dependencies_name(&alt.deps, names)
            })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.values.is_empty() && self.alternatives.is_empty()
    }

    pub(super) fn into_scalar(mut self) -> Self {
        self.values.clear();
        self.views.clear();
        for alt in &mut self.alternatives {
            if matches!(alt.kind, ValueKind::Allocation(_)) {
                alt.kind = ValueKind::Scalar;
            }
        }
        self.alternatives.sort_unstable();
        self.alternatives.dedup();
        self
    }

    pub(super) fn readonly(&self) -> bool {
        self.values.iter().any(|value| !value.writable)
    }

    pub(super) fn restrict_permission(&mut self, readonly: bool) {
        if readonly {
            for value in &mut self.values {
                value.writable = false;
            }
            for alt in &mut self.alternatives {
                if let ValueKind::Allocation(value) = &mut alt.kind {
                    value.writable = false;
                }
            }
        }
    }

    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        self.remap_allocations(|mut id| {
            retirement.remap_id(state, &mut id);
            id
        });
    }

    pub(super) fn remap_allocations(&mut self, mut remap: impl FnMut(AllocId) -> AllocId) {
        for value in &mut self.values {
            value.id = remap(value.id);
        }
        for view in &mut self.views {
            *view = remap(*view);
        }
        remap_value_alternatives(&mut self.alternatives, remap);
    }
}

fn remap_value_alternatives(
    alternatives: &mut ValueAlternatives,
    mut remap: impl FnMut(AllocId) -> AllocId,
) {
    for alt in &mut *alternatives {
        alt.remap(&mut remap);
    }
    alternatives.sort_unstable();
    alternatives.dedup();
}

#[derive(Debug, Default)]
pub(super) struct StoreReceipt {
    pub replaced: [Option<(CellId, ValueAlternatives)>; 2],
}

/// Keep entry and exit coordinates separate: the same key can name two occurrences.
#[derive(Debug, Default)]
pub(super) struct PublicationCarriers {
    pub definite_old: Vec<(CellId, ValueAlternatives)>,
    pub introduced: Vec<(CellId, ValueAlternatives)>,
}

impl PublicationCarriers {
    pub(super) fn record(
        &mut self,
        entry_cell: CellId,
        old: &[ValueAlt],
        exit_cell: CellId,
        incoming: &[ValueAlt],
        definite: bool,
    ) {
        if definite {
            Self::add(
                &mut self.definite_old,
                entry_cell,
                old.iter().filter(|alt| !alt.deps.is_empty()),
            );
        }
        Self::add(
            &mut self.introduced,
            exit_cell,
            incoming.iter().filter(|alt| {
                !alt.deps.is_empty()
                    && (definite
                        || !old.iter().any(|old| {
                            let same_kind = match (old.kind, alt.kind) {
                                (ValueKind::Allocation(a), ValueKind::Allocation(b)) => {
                                    a.id == b.id
                                }
                                (a, b) => a == b,
                            };
                            same_kind && old.deps == alt.deps
                        }))
            }),
        );
    }

    fn add<'a>(
        rows: &mut Vec<(CellId, ValueAlternatives)>,
        cell: CellId,
        values: impl Iterator<Item = &'a ValueAlt>,
    ) {
        let mut values = values.peekable();
        if values.peek().is_none() {
            return;
        }
        let existing = match rows.binary_search_by_key(&cell, |row| row.0) {
            Ok(index) => {
                let existing = &mut rows[index].1;
                existing.extend(values.cloned());
                existing
            }
            Err(index) => {
                rows.insert(index, (cell, values.cloned().collect()));
                &mut rows[index].1
            }
        };
        existing.sort_unstable();
        existing.dedup();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Replacement {
    May,
    Must,
}

/// The snapshots describe the final member occurrences, not executed insertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FamilyMember {
    pub value: CapturedValue,
    pub at_most_once: bool,
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

    pub(super) fn captures_current(&self, local: usize, names: &impl Fn(AllocId) -> bool) -> bool {
        match self {
            Self::Write(write) => {
                write.addresses.captures_current(local, names)
                    || write.value.names_allocation(names)
                    || write
                        .slot_roots
                        .as_ref()
                        .is_some_and(|roots| roots.contains(local))
            }
            Self::EraseFamily { receivers, .. } => receivers.iter().copied().any(names),
        }
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

/// Projected directly from the current or retained wrapper row, including the
/// immediate link rather than its resolved terminal.
#[allow(dead_code)] // Used by boundary projection, not by access validation.
pub(super) struct BoundaryBinding<'a> {
    pub alternatives: &'a [BindingAlt],
    pub coverage: Coverage,
    pub routes: &'a [(CellId, DependencySet)],
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
    pub lookup_observations: bool,
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
            lookup_observations: false,
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

#[derive(Debug, Clone)]
pub(super) struct BorrowState {
    // One lattice state per function-local index.
    locals: Vec<LocalState>,
    value_root_users: BTreeSet<(usize, usize)>,
    derivation_users: BTreeSet<(usize, usize)>,
    // Allocation identities for values stored by reference inside other values.
    layer: AllocationLayer,
}

impl PartialEq for BorrowState {
    fn eq(&self, other: &Self) -> bool {
        self.locals == other.locals && self.layer == other.layer
    }
}

impl Eq for BorrowState {}

fn index_roots(map: &mut BTreeSet<(usize, usize)>, owner: usize, roots: &RootSet, insert: bool) {
    for root in roots.iter_ones() {
        if insert {
            map.insert((root, owner));
        } else {
            map.remove(&(root, owner));
        }
    }
}

impl BorrowState {
    #[cfg(feature = "benchmark_counters")]
    pub(super) fn validation_row_count(&self) -> usize {
        self.locals.len()
            + self.value_root_users.len()
            + self.derivation_users.len()
            + self.layer.row_count()
    }

    pub(super) fn new_uninitialized(local_count: usize) -> Self {
        let locals = (0..local_count)
            .map(|_| LocalState::uninit(local_count))
            .collect::<Vec<_>>();

        Self {
            locals,
            value_root_users: BTreeSet::new(),
            derivation_users: BTreeSet::new(),
            layer: AllocationLayer::new(),
        }
    }

    pub(super) fn initialize_parameter(&mut self, local_index: usize) {
        self.initialize_entry_binding(local_index, BindingSpec::Slot);
    }

    /// Entry anchors are chosen by admission; linking to one must not flatten its route.
    pub(super) fn initialize_entry_binding(&mut self, local: usize, binding: BindingSpec) {
        let capacity = self.locals.len();
        self.set_binding(
            local,
            binding,
            RootSet::empty(capacity),
            RootSet::empty(capacity),
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
        self.set_local_value_roots(local_index, new_state.value_roots);
        self.set_local_derivation(local_index, new_state.derived_from_roots);
        self.locals[local_index].init = new_state.init;
        self.refresh_linked_observations(WrapperId::current(local_index));
    }

    fn set_local_value_roots(&mut self, local: usize, roots: RootSet) {
        let old = std::mem::replace(&mut self.locals[local].value_roots, roots);
        index_roots(&mut self.value_root_users, local, &old, false);
        index_roots(
            &mut self.value_root_users,
            local,
            &self.locals[local].value_roots,
            true,
        );
    }

    fn set_local_derivation(&mut self, local: usize, roots: RootSet) {
        let old = std::mem::replace(&mut self.locals[local].derived_from_roots, roots);
        index_roots(&mut self.derivation_users, local, &old, false);
        index_roots(
            &mut self.derivation_users,
            local,
            &self.locals[local].derived_from_roots,
            true,
        );
    }

    fn rebuild_local_incidence(&mut self) {
        self.value_root_users.clear();
        self.derivation_users.clear();
        for (local, state) in self.locals.iter().enumerate() {
            index_roots(&mut self.value_root_users, local, &state.value_roots, true);
            index_roots(
                &mut self.derivation_users,
                local,
                &state.derived_from_roots,
                true,
            );
        }
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
        if changed {
            self.rebuild_local_incidence();
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
        self.rebuild_local_incidence();
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

#[derive(Debug, Clone)]
pub(super) struct RootSet {
    bits: RootBits,
    bit_len: usize,
}

/// A captured route usually names one root, irrespective of the function's width.
/// Sparse storage pays only for present roots; dense masks retain word-wise operations.
#[derive(Debug, Clone)]
enum RootBits {
    Empty,
    One(usize),
    Sparse(Rc<Vec<usize>>),
    Dense(Rc<[u64]>),
}

impl PartialEq for RootSet {
    fn eq(&self, other: &Self) -> bool {
        self.bit_len == other.bit_len
            && match (&self.bits, &other.bits) {
                (RootBits::Empty, RootBits::Empty) => true,
                (RootBits::One(a), RootBits::One(b)) => a == b,
                (RootBits::Sparse(a), RootBits::Sparse(b)) => Rc::ptr_eq(a, b) || a == b,
                (RootBits::Dense(a), RootBits::Dense(b)) => Rc::ptr_eq(a, b) || a == b,
                _ => self.iter_ones().eq(other.iter_ones()),
            }
    }
}

impl Eq for RootSet {}

impl RootSet {
    pub(super) fn empty(bit_len: usize) -> Self {
        Self {
            bits: RootBits::Empty,
            bit_len,
        }
    }

    pub(super) fn full(bit_len: usize) -> Self {
        let bits = match bit_len {
            0 => RootBits::Empty,
            1 => RootBits::One(0),
            _ => {
                let words = bit_len.div_ceil(64);
                RootBits::Dense(
                    (0..words)
                        .map(|i| {
                            if i + 1 == words {
                                Self::tail_word(bit_len)
                            } else {
                                u64::MAX
                            }
                        })
                        .collect(),
                )
            }
        };
        Self { bits, bit_len }
    }

    fn tail_word(bit_len: usize) -> u64 {
        u64::MAX >> ((64 - bit_len % 64) % 64)
    }

    pub(super) fn covers_all(&self, local_count: usize) -> bool {
        self.bit_len == local_count
            && match &self.bits {
                RootBits::Empty => local_count == 0,
                RootBits::One(_) => local_count == 1,
                RootBits::Sparse(roots) => roots.len() == local_count,
                RootBits::Dense(words) => words.split_last().is_some_and(|(tail, rest)| {
                    *tail == Self::tail_word(local_count)
                        && rest.iter().all(|&word| word == u64::MAX)
                }),
            }
    }

    fn dense_limit(&self) -> usize {
        self.bit_len.div_ceil(64).max(2)
    }

    fn dense_from_roots(bit_len: usize, roots: impl Iterator<Item = usize>) -> RootBits {
        let mut words: Rc<[u64]> = std::iter::repeat_n(0, bit_len.div_ceil(64)).collect();
        let buffer = Rc::get_mut(&mut words).expect("newly allocated root mask");
        for bit in roots {
            buffer[bit / 64] |= 1 << (bit % 64);
        }
        RootBits::Dense(words)
    }

    /// A once-dense mask can become a sparsely retained route after narrowing.
    fn compact_dense(&mut self) {
        let RootBits::Dense(words) = &self.bits else {
            return;
        };
        let limit = self.dense_limit();
        let mut count = 0;
        for word in words.iter() {
            count += word.count_ones() as usize;
            if count >= limit {
                return;
            }
        }
        self.bits = match count {
            0 => RootBits::Empty,
            1 => RootBits::One(self.iter_ones().next().expect("one surviving root")),
            _ => RootBits::Sparse(Rc::new(self.iter_ones().collect())),
        };
    }

    pub(super) fn insert(&mut self, bit: usize) {
        if bit >= self.bit_len {
            return;
        }
        let limit = self.dense_limit();
        match &mut self.bits {
            RootBits::Empty => self.bits = RootBits::One(bit),
            RootBits::One(old) if *old != bit => {
                let ordered = [(*old).min(bit), (*old).max(bit)];
                self.bits = if limit == 2 {
                    Self::dense_from_roots(self.bit_len, ordered.into_iter())
                } else {
                    RootBits::Sparse(Rc::new(ordered.to_vec()))
                };
            }
            RootBits::One(_) => {}
            RootBits::Sparse(roots) => {
                let Err(at) = roots.binary_search(&bit) else {
                    return;
                };
                if roots.len() + 1 >= limit {
                    self.bits =
                        Self::dense_from_roots(self.bit_len, roots.iter().copied().chain([bit]));
                } else {
                    Rc::make_mut(roots).insert(at, bit);
                }
            }
            RootBits::Dense(words) => {
                let (word, mask) = (bit / 64, 1 << (bit % 64));
                if words[word] & mask == 0 {
                    Rc::make_mut(words)[word] |= mask;
                }
            }
        }
    }

    pub(super) fn remove(&mut self, bit: usize) {
        if !self.contains(bit) {
            return;
        }
        match &mut self.bits {
            RootBits::One(_) => self.clear(),
            RootBits::Sparse(roots) => {
                let at = roots.binary_search(&bit).expect("present sparse root");
                if roots.len() == 2 {
                    self.bits = RootBits::One(roots[1 - at]);
                } else {
                    Rc::make_mut(roots).remove(at);
                }
            }
            RootBits::Dense(words) => {
                Rc::make_mut(words)[bit / 64] &= !(1 << (bit % 64));
                self.compact_dense();
            }
            RootBits::Empty => unreachable!(),
        }
    }

    pub(super) fn contains(&self, bit: usize) -> bool {
        bit < self.bit_len
            && match &self.bits {
                RootBits::Empty => false,
                RootBits::One(root) => bit == *root,
                RootBits::Sparse(roots) => roots.binary_search(&bit).is_ok(),
                RootBits::Dense(words) => words[bit / 64] & (1 << (bit % 64)) != 0,
            }
    }

    /// Returns whether any bit was added.
    pub(super) fn union_with(&mut self, other: &Self) -> bool {
        if other.is_empty() {
            return false;
        }
        if self.bit_len == other.bit_len {
            if self.is_empty() {
                self.bits.clone_from(&other.bits);
                return true;
            }
            if let RootBits::Dense(right) = &other.bits {
                if let RootBits::Dense(left) = &mut self.bits {
                    if !left.iter().zip(right.iter()).any(|(&a, &b)| b & !a != 0) {
                        return false;
                    }
                    for (a, b) in Rc::make_mut(left).iter_mut().zip(right.iter()) {
                        *a |= *b;
                    }
                    return true;
                }
                let before = std::mem::replace(&mut self.bits, RootBits::Dense(Rc::clone(right)));
                // A dense donor necessarily has more roots than the sparse receiver.
                let old = Self {
                    bits: before,
                    bit_len: self.bit_len,
                };
                for bit in old.iter_ones() {
                    self.insert(bit);
                }
                return true;
            }
        }
        let bit_len = self.bit_len;
        let mut changed = false;
        for bit in other.iter_ones().take_while(|&bit| bit < bit_len) {
            if !self.contains(bit) {
                self.insert(bit);
                changed = true;
            }
        }
        changed
    }

    pub(super) fn intersect_with(&mut self, other: &Self) -> bool {
        if !self.has_members_outside(other) {
            return false;
        }
        if self.bit_len != other.bit_len {
            *self = self.survivors(|bit| other.contains(bit));
            return true;
        }
        match (&mut self.bits, &other.bits) {
            (RootBits::Dense(left), RootBits::Dense(right)) => {
                for (a, b) in Rc::make_mut(left).iter_mut().zip(right.iter()) {
                    *a &= *b;
                }
                self.compact_dense();
            }
            (RootBits::Dense(_), _) => {
                let mut kept = Self::empty(self.bit_len);
                for bit in other.iter_ones().filter(|&bit| self.contains(bit)) {
                    kept.insert(bit);
                }
                *self = kept;
            }
            (RootBits::Sparse(roots), _) => {
                Rc::make_mut(roots).retain(|&bit| other.contains(bit));
                self.collapse_sparse();
            }
            _ => self.clear(),
        }
        true
    }

    fn has_members_outside(&self, other: &Self) -> bool {
        match (&self.bits, &other.bits) {
            (RootBits::Empty, _) => false,
            (_, RootBits::Empty) => true,
            (RootBits::Dense(left), RootBits::Dense(right)) if self.bit_len == other.bit_len => {
                left.iter().zip(right.iter()).any(|(&a, &b)| a & !b != 0)
            }
            _ => self.iter_ones().any(|bit| !other.contains(bit)),
        }
    }

    fn survivors(&self, keep: impl Fn(usize) -> bool) -> Self {
        let mut kept = Self::empty(self.bit_len);
        for bit in self.iter_ones().filter(|&bit| keep(bit)) {
            kept.insert(bit);
        }
        kept
    }

    fn collapse_sparse(&mut self) {
        if let RootBits::Sparse(roots) = &self.bits {
            match roots.as_slice() {
                [] => self.clear(),
                [one] => self.bits = RootBits::One(*one),
                _ => {}
            }
        }
    }

    pub(super) fn clear(&mut self) {
        self.bits = RootBits::Empty;
    }

    pub(super) fn subtract_with(&mut self, other: &Self) {
        if self.is_empty() || other.is_empty() {
            return;
        }
        let bit_len = self.bit_len;
        match (&mut self.bits, &other.bits) {
            (RootBits::Dense(left), RootBits::Dense(right)) => {
                if !left.iter().zip(right.iter()).any(|(&a, &b)| a & b != 0) {
                    return;
                }
                for (a, b) in Rc::make_mut(left).iter_mut().zip(right.iter()) {
                    *a &= !*b;
                }
                self.compact_dense();
            }
            (RootBits::Dense(left), _) => {
                let mut changed = false;
                for bit in other.iter_ones().take_while(|&bit| bit < bit_len) {
                    let (word, mask) = (bit / 64, 1 << (bit % 64));
                    if left[word] & mask != 0 {
                        Rc::make_mut(left)[word] &= !mask;
                        changed = true;
                    }
                }
                if changed {
                    self.compact_dense();
                }
            }
            (RootBits::Sparse(roots), _) => {
                if roots.iter().any(|&bit| other.contains(bit)) {
                    Rc::make_mut(roots).retain(|&bit| !other.contains(bit));
                    self.collapse_sparse();
                }
            }
            (RootBits::One(bit), _) if other.contains(*bit) => self.clear(),
            _ => {}
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        matches!(self.bits, RootBits::Empty)
    }

    pub(super) fn bit_len(&self) -> usize {
        self.bit_len
    }

    pub(super) fn iter_ones(&self) -> RootSetIter<'_> {
        match &self.bits {
            RootBits::Empty => RootSetIter::Sparse([].iter()),
            RootBits::One(bit) => RootSetIter::Sparse(std::slice::from_ref(bit).iter()),
            RootBits::Sparse(roots) => RootSetIter::Sparse(roots.iter()),
            RootBits::Dense(words) => RootSetIter::Dense {
                words: words.iter().enumerate(),
                offset: 0,
                current: 0,
            },
        }
    }
}

pub(super) enum RootSetIter<'a> {
    Sparse(std::slice::Iter<'a, usize>),
    Dense {
        words: std::iter::Enumerate<std::slice::Iter<'a, u64>>,
        offset: usize,
        current: u64,
    },
}

impl Iterator for RootSetIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        match self {
            Self::Sparse(roots) => roots.next().copied(),
            Self::Dense {
                words,
                offset,
                current,
            } => {
                while *current == 0 {
                    let (word, bits) = words.next()?;
                    *offset = word * 64;
                    *current = *bits;
                }
                let bit = *offset + current.trailing_zeros() as usize;
                *current &= *current - 1;
                Some(bit)
            }
        }
    }
}

#[cfg(test)]
#[test]
fn wide_root_routes_remain_sparse_and_fold_back_after_narrowing() {
    let width = 8192;
    let mut route = RootSet::empty(width);
    route.insert(width - 1);
    assert!(matches!(route.bits, RootBits::One(8191)));
    let mut other = RootSet::empty(width);
    other.insert(63);
    assert!(route.union_with(&other));
    assert!(matches!(&route.bits, RootBits::Sparse(roots) if roots.as_slice() == [63, 8191]));
    let mut dense = RootSet::full(width);
    assert!(dense.intersect_with(&route));
    assert_eq!(dense, route);
    assert!(matches!(&dense.bits, RootBits::Sparse(roots) if roots.len() == 2));
    let mut complement = RootSet::full(width);
    complement.subtract_with(&route);
    let mut dense_difference = RootSet::full(width);
    dense_difference.subtract_with(&complement);
    assert_eq!(dense_difference, route);
    assert!(matches!(&dense_difference.bits, RootBits::Sparse(roots) if roots.len() == 2));
    dense.subtract_with(&other);
    assert!(matches!(dense.bits, RootBits::One(8191)));
}
